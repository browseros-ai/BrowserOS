//! Browser calls: the server half of Playwright's wire inside one run. Every
//! `__neo_runtime.call` message comes through [`Dispatcher::call`], the only path a browser call
//! can take. It names the call's target, decodes its params against the spec, opens its trace
//! entry, runs it on its own tokio task with a [`Progress`], and turns the result into
//! Playwright's reply.
//!
//! ```text
//!  JS: __neo_runtime.call('{"id":7,"guid":"frame@T","method":"click",…}')
//!    └─ Dispatcher::call  (sync, inside the native call)            returns PendingReply
//!         ├─ parse the envelope; unreadable → BridgeError (rejects the promise)
//!         ├─ route: guid → frame, FrameCall::decode, frame selectors
//!         ├─ Trace += CallEntry                 every message with an id leaves a record
//!         ├─ early failure → reply now, no call log (Playwright's dispatcher does the same)
//!         └─ run.tasks.spawn(serve)             runs while JS spins, outlives the realm
//!              └─ entry.finish(…); reply {id} | {id, error:{error:{message,name}}, log}
//!  run end: Dispatcher::finish(reason)
//!    record the reason → cancel `ended` → wait for the run's tasks (≤ RECORDS_WAIT) → records
//! ```
//!
//! Why a task per call and not the realm's scheduler, where `setTimeout` runs: rquickjs polls
//! futures started from JS only while the script is parked on an `await`, so a call would stall
//! while the script spins or a sync getter blocks, and the realm would drop it mid-call when the
//! script returns. Playwright's server runs a call whatever its client does; so does this. The
//! call starts inside the native call itself, not when the realm first polls the returned
//! future, so a call the script never awaits still runs.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use neo_runtime_spec::serve::{FrameCall, FrameClickParams, Metadata};
use neo_runtime_spec::{CallDecodeError, MethodFlag, MethodMeta};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::time;
use tokio_util::sync::DropGuard;

use crate::bridge;
use crate::page::PageSession;
use crate::progress::{
    CallEntry, CallError, CallStart, PlaywrightError, Progress, RunScope, Trace,
};
use crate::run::{CallLocation, CallRecord, CallResult, DroppedCalls, PageId, RunEnd};

/// How long `finish` waits, after the realm is gone, for the run's calls to end so their
/// records read final results and deliveries. A task still running past it keeps running and
/// its record reads `Unfinished`. The wait adds to the run's wall time only when a call is
/// still in flight at the end.
const RECORDS_WAIT: Duration = Duration::from_secs(1);

/// Neo's message for a selector that enters a frame. Playwright has none: Neo serves the main
/// frame only, and without this check the injected engine would match nothing there and the call
/// would time out without saying why.
const FRAMES_UNSUPPORTED: &str = "Neo does not support frames yet: the selector enters a frame";

/// What `Frame.click` replies until Neo serves it.
const CLICK_NOT_SERVED: &str = "Neo does not serve Frame.click yet";

/// Owns every browser call of one run. Cheap to clone: the bridge holds one, `Runtime::run`
/// another. Call tasks hold their `Progress`, entry and the trace, never a `Dispatcher`, so the
/// last clone drops once the realm is gone and the run is done with it (or its future was
/// dropped). Then `_ending` cancels the run's end token, and every call stops at its next await.
#[derive(Clone)]
pub(crate) struct Dispatcher(Arc<Inner>);

struct Inner {
    /// Owns the run's CDP session (`page.rs`): the session stays attached while this lives.
    page: Arc<PageSession>,
    page_guid: String,
    frame_guid: String,
    run: RunScope,
    trace: Trace,
    next_call_id: AtomicU64,
    /// Cancels `run.ended` when the last `Dispatcher` drops. A reason `finish` recorded first
    /// wins; otherwise calls read [`RunEnd::Cancelled`].
    _ending: DropGuard,
}

/// Playwright's client message, `{id, guid, method, params, metadata}` (`connection.ts`).
#[derive(Deserialize)]
struct Message {
    /// Echoed in the reply; opaque to Rust (the SDK sends a counter).
    id: Value,
    guid: String,
    method: String,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    metadata: Option<Metadata>,
}

/// A message the bridge cannot answer with a Playwright reply. Only a broken SDK sends one, and
/// the bridge rejects the script's promise with it.
#[derive(Debug, thiserror::Error)]
pub(crate) enum BridgeError {
    #[error("bad protocol message: {0}")]
    BadMessage(String),
    #[error("the run ended before the call replied")]
    NoReply,
}

/// The reply text a call resolves the script's promise with: ready at once for an early
/// failure, or a oneshot the call's task fills. A dropped sender means the run ended under the
/// call; nobody sees that error, because the realm is going away too.
pub(crate) struct PendingReply(PendingInner);

enum PendingInner {
    Ready(Option<Result<String, BridgeError>>),
    Waiting(oneshot::Receiver<String>),
}

impl PendingReply {
    fn ready(reply: Result<String, BridgeError>) -> Self {
        Self(PendingInner::Ready(Some(reply)))
    }
}

impl Future for PendingReply {
    type Output = Result<String, BridgeError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut self.0 {
            PendingInner::Ready(reply) => {
                Poll::Ready(reply.take().unwrap_or(Err(BridgeError::NoReply)))
            }
            PendingInner::Waiting(receiver) => Pin::new(receiver)
                .poll(cx)
                .map(|reply| reply.map_err(|_| BridgeError::NoReply)),
        }
    }
}

/// A message after routing: the method's metadata when the call named one Neo knows, and what
/// to run or why not.
struct Routed {
    meta: Option<&'static MethodMeta>,
    route: Result<Route, PlaywrightError>,
}

/// What a message asks for. One variant per served (object, method).
enum Route {
    FrameClick(
        #[cfg_attr(
            not(test),
            expect(dead_code, reason = "Frame.click's handler reads them")
        )]
        FrameClickParams,
    ),
}

impl Dispatcher {
    pub(crate) fn new(page: Arc<PageSession>, run: RunScope, page_id: &PageId) -> Self {
        Self(Arc::new(Inner {
            page,
            page_guid: bridge::page_guid(page_id),
            frame_guid: bridge::frame_guid(page_id),
            _ending: run.ended.clone().drop_guard(),
            run,
            trace: Trace::default(),
            next_call_id: AtomicU64::new(1),
        }))
    }

    /// `__neo_runtime.call`: one Playwright message in, one Playwright reply out, as JSON text.
    /// Synchronous so the call starts before the realm polls anything. Every message with a
    /// readable envelope gets a reply, failures included, and a trace record.
    pub(crate) fn call(&self, message: &str) -> PendingReply {
        let inner = &self.0;
        let Message {
            id,
            guid,
            method,
            params,
            metadata,
        } = match serde_json::from_str(message) {
            Ok(message) => message,
            Err(error) => {
                return PendingReply::ready(Err(BridgeError::BadMessage(error.to_string())));
            }
        };
        let Routed { meta, route } = route(
            &inner.page_guid,
            &inner.frame_guid,
            &guid,
            &method,
            params.clone(),
        );
        let metadata = metadata.unwrap_or(Metadata {
            location: None,
            title: None,
            internal: None,
            timeout: None,
        });
        let entry = Arc::new(CallEntry::new(CallStart {
            call_id: inner.next_call_id.fetch_add(1, Ordering::Relaxed),
            guid,
            method,
            title: meta.and_then(|meta| meta.title),
            params: params.unwrap_or_else(|| json!({})),
            location: call_location(&metadata),
            input: meta.is_some_and(|meta| meta.has(MethodFlag::Input)),
        }));
        inner.trace.push(Arc::clone(&entry));
        let route = match route {
            Ok(route) => route,
            Err(error) => {
                entry.finish(error.to_result());
                inner.trace.finished(&entry);
                return PendingReply::ready(Ok(encode_reply(&id, Err(&error), None)));
            }
        };
        let progress = Progress::new(
            Arc::clone(&entry),
            metadata.timeout.unwrap_or(0.0),
            inner.run.clone(),
            inner.page.closed(),
        );
        let (reply, receiver) = oneshot::channel();
        inner.run.tasks.spawn(serve(
            route,
            progress,
            entry,
            inner.trace.clone(),
            id,
            reply,
        ));
        PendingReply(PendingInner::Waiting(receiver))
    }

    /// Ends the run's calls after the realm is gone: records why the run ended (the first
    /// reason wins), cancels every call, waits up to [`RECORDS_WAIT`] for the run's tasks, and
    /// returns the records and the dropped-calls summary.
    pub(crate) async fn finish(&self, reason: RunEnd) -> (Vec<CallRecord>, Option<DroppedCalls>) {
        let run = &self.0.run;
        run.end(reason);
        run.tasks.close();
        let _waited = time::timeout(RECORDS_WAIT, run.tasks.wait()).await;
        self.0.trace.take(reason)
    }
}

/// Names a message's target and decodes its params. Pure.
fn route(
    page_guid: &str,
    frame_guid: &str,
    guid: &str,
    method: &str,
    params: Option<Value>,
) -> Routed {
    if guid == frame_guid {
        return match FrameCall::decode(method, params) {
            Ok(call) => Routed {
                meta: Some(call.meta()),
                route: route_frame_call(call),
            },
            Err(error) => Routed {
                meta: match &error {
                    CallDecodeError::InvalidParams { meta, .. } => Some(*meta),
                    CallDecodeError::UnknownMethod { .. } => None,
                },
                route: Err(PlaywrightError::error(error.to_string())),
            },
        };
    }
    let error = if guid == page_guid {
        PlaywrightError::error(format!("Neo does not serve Page.{method}"))
    } else {
        // Playwright's dispatcher answers a guid it does not know with `TargetClosedError`.
        PlaywrightError::target_closed()
    };
    Routed {
        meta: None,
        route: Err(error),
    }
}

/// Serving a new `Frame` command in `spec/serve.yml` adds a `FrameCall` variant, and this match
/// stops compiling until the command is routed.
fn route_frame_call(call: FrameCall) -> Result<Route, PlaywrightError> {
    match call {
        FrameCall::Click(params) if enters_frame(&params.selector) => {
            Err(PlaywrightError::error(FRAMES_UNSUPPORTED))
        }
        FrameCall::Click(params) => Ok(Route::FrameClick(params)),
        FrameCall::Goto(_) => Err(not_served_yet("goto")),
        FrameCall::Title(_) => Err(not_served_yet("title")),
        FrameCall::TextContent(_) => Err(not_served_yet("textContent")),
        FrameCall::AriaSnapshot(_) => Err(not_served_yet("ariaSnapshot")),
    }
}

fn not_served_yet(method: &str) -> PlaywrightError {
    PlaywrightError::error(format!("Neo does not serve Frame.{method} yet"))
}

/// One call's task: serves the route, ends the trace entry and sends the reply. A call the run
/// ended leaves its entry open, so its record reads `Unfinished`, and sends nothing: nobody
/// waits for it.
#[expect(
    clippy::unused_async,
    reason = "Frame.click's handler is the first to await"
)]
async fn serve(
    route: Route,
    _progress: Progress,
    entry: Arc<CallEntry>,
    trace: Trace,
    id: Value,
    reply: oneshot::Sender<String>,
) {
    let result: Result<Option<Value>, CallError> = match route {
        Route::FrameClick(_) => Err(PlaywrightError::error(CLICK_NOT_SERVED).into()),
    };
    let (record, answer) = match result {
        Err(CallError::RunEnded(_)) => return,
        Ok(value) => (CallResult::Ok, Ok(value)),
        Err(CallError::Playwright(error)) => (error.to_result(), Err(error)),
    };
    let log = entry.finish(record);
    trace.finished(&entry);
    let answer = answer.as_ref().map(Option::as_ref);
    // The script may have stopped waiting (the realm is gone); the record already holds the
    // outcome.
    let _sent = reply.send(encode_reply(&id, answer, Some(&log)));
}

/// Whether a selector enters a frame. `internal:control=enter-frame` and
/// `internal:control=any-frame` appear unescaped even inside nested `has=` and `and=` parts.
fn enters_frame(selector: &str) -> bool {
    selector.contains("internal:control=enter-frame")
        || selector.contains("internal:control=any-frame")
}

/// `metadata.location` as a record keeps it: the line and column, without the file, which is
/// always the agent's script.
fn call_location(metadata: &Metadata) -> Option<CallLocation> {
    let location = metadata.location.as_ref()?;
    Some(CallLocation {
        line: u32::try_from(location.line?).ok()?,
        column: location
            .column
            .and_then(|column| u32::try_from(column).ok()),
    })
}

/// Playwright's reply (`dispatcher.ts`): `{id}` or `{id, result}` on success, and
/// `{id, error: {error: {message, name}}, log}` on failure. A served call's failure always
/// carries its folded `log`; an early failure (unknown object, bad params) carries none, as in
/// Playwright. Pure. Its outputs are pinned in `tests/fixtures/replies.json`, which neo-sdk's
/// `rebuildError` test reads, so the envelope that both languages write is pinned once.
fn encode_reply(
    id: &Value,
    answer: Result<Option<&Value>, &PlaywrightError>,
    log: Option<&[String]>,
) -> String {
    let reply = match answer {
        Ok(None) => json!({ "id": id }),
        Ok(Some(result)) => json!({ "id": id, "result": result }),
        Err(error) => {
            let mut reply = json!({
                "id": id,
                "error": { "error": { "message": error.message, "name": error.name.as_str() } },
            });
            if let (Some(log), Some(fields)) = (log, reply.as_object_mut()) {
                fields.insert("log".to_string(), json!(log));
            }
            reply
        }
    };
    reply.to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::click_goldens::{golden, raw_log};
    use crate::progress::{ErrorName, compress_call_log};

    const PAGE: &str = "page@T";
    const FRAME: &str = "frame@T";

    fn route_frame(method: &str, params: Value) -> Routed {
        route(PAGE, FRAME, FRAME, method, Some(params))
    }

    fn error_of(routed: Routed) -> PlaywrightError {
        match routed.route {
            Ok(_) => panic!("expected the call to be refused"),
            Err(error) => error,
        }
    }

    #[test]
    fn a_frame_click_routes_with_its_params_and_title() {
        let routed = route_frame("click", json!({ "selector": "#go", "strict": true }));
        assert_eq!(routed.meta.and_then(|meta| meta.title), Some("Click"));
        let Ok(Route::FrameClick(params)) = routed.route else {
            panic!("a click on the run's frame routes to Frame.click");
        };
        assert_eq!(
            (params.selector.as_str(), params.strict),
            ("#go", Some(true))
        );
    }

    #[test]
    fn a_selector_that_enters_a_frame_is_refused_at_once() {
        for selector in [
            "iframe >> internal:control=enter-frame >> button",
            "li >> internal:has=\"iframe >> internal:control=enter-frame >> b\"",
            "internal:control=any-frame >> button",
        ] {
            let error = error_of(route_frame("click", json!({ "selector": selector })));
            assert_eq!(
                error,
                PlaywrightError::error(FRAMES_UNSUPPORTED),
                "{selector}"
            );
        }
        assert!(!enters_frame("text=\"enter-frame\""));
    }

    #[test]
    fn unserved_methods_and_objects_reply_as_playwrights_dispatcher_would() {
        let cases = [
            (
                route_frame("dblclick", json!({ "selector": "#go" })),
                None,
                PlaywrightError::error("Neo does not serve Frame.dblclick"),
            ),
            (
                route_frame("goto", json!({ "url": "https://example.test" })),
                Some("Navigate"),
                PlaywrightError::error("Neo does not serve Frame.goto yet"),
            ),
            (
                route(PAGE, FRAME, PAGE, "click", None),
                None,
                PlaywrightError::error("Neo does not serve Page.click"),
            ),
            (
                route(PAGE, FRAME, "frame@another-tab", "click", None),
                None,
                PlaywrightError::target_closed(),
            ),
        ];
        for (routed, title, expected) in cases {
            assert_eq!(routed.meta.and_then(|meta| meta.title), title);
            assert_eq!(error_of(routed), expected);
        }
    }

    #[test]
    fn invalid_params_name_the_method_and_keep_its_title() {
        let routed = route_frame("click", json!({ "selector": 5 }));
        assert_eq!(routed.meta.and_then(|meta| meta.title), Some("Click"));
        let error = error_of(routed);
        assert_eq!(error.name, ErrorName::Error);
        assert!(
            error
                .message
                .starts_with("invalid params for Frame.click: "),
            "{error:?}"
        );
    }

    #[test]
    fn replies_have_playwrights_envelope() {
        let id = json!(7);
        let error = PlaywrightError::error("boom");
        assert_eq!(encode_reply(&id, Ok(None), None), r#"{"id":7}"#);
        assert_eq!(
            encode_reply(&id, Ok(Some(&json!({ "value": "x" }))), None),
            r#"{"id":7,"result":{"value":"x"}}"#
        );
        assert_eq!(
            encode_reply(&id, Err(&error), None),
            r#"{"error":{"error":{"message":"boom","name":"Error"}},"id":7}"#,
            "an early failure carries no log"
        );
        assert_eq!(
            encode_reply(&id, Err(&error), Some(&[])),
            r#"{"error":{"error":{"message":"boom","name":"Error"}},"id":7,"log":[]}"#,
            "a served call's failure always carries its log"
        );
    }

    /// The replies Neo sends for Playwright's recorded clicks, built from Neo's own errors,
    /// folding and envelope, and pinned in `tests/fixtures/replies.json`. neo-sdk's
    /// `connection.test.ts` rebuilds each into the error the script sees and compares it with
    /// the golden's message, so both halves of the reply path are checked against Playwright.
    /// Set `NEO_RUNTIME_WRITE_FIXTURES=1` to rewrite the file after a deliberate change.
    #[test]
    fn replies_for_the_click_goldens_match_the_committed_fixture() {
        let id = json!(1);
        let mut replies = BTreeMap::new();
        replies.insert("ok", encode_reply(&id, Ok(None), None));
        for name in ["strict", "hidden", "covered", "disabled"] {
            let golden = golden(name);
            let error = match golden.timeout_ms() {
                Some(timeout_ms) => {
                    let error = PlaywrightError::timeout(timeout_ms);
                    assert_eq!(error.message, golden.server_message(), "the {name} golden");
                    error
                }
                // A strict mode violation's message is the page's own, passed through.
                None => PlaywrightError::error(golden.server_message()),
            };
            let log = compress_call_log(&raw_log(name));
            replies.insert(name, encode_reply(&id, Err(&error), Some(&log)));
        }
        replies.insert(
            "targetClosed",
            encode_reply(&id, Err(&PlaywrightError::target_closed()), Some(&[])),
        );
        let mut fixture = serde_json::to_string_pretty(&replies).expect("strings serialize");
        fixture.push('\n');

        let crate_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
        let path = crate_dir.join("tests/fixtures/replies.json");
        if env::var_os("NEO_RUNTIME_WRITE_FIXTURES").is_some() {
            fs::write(&path, &fixture).expect("the fixture is writable");
        }
        let committed = fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == fixture,
            "{} is stale; rerun with NEO_RUNTIME_WRITE_FIXTURES=1. Expected:\n{fixture}",
            path.display()
        );
    }
}
