//! `__neo_runtime`, the only door from a run's JS into Rust. neo-sdk's `runtime-bridge.ts`
//! calls `get` for every synchronous Playwright getter and `call` for every browser call, and
//! the realm's `console` calls `log`.
//!
//! ```text
//!  page.url()                                    JS, on the run's thread
//!    └─ __neo_runtime.get('{"guid":"page@<id>","field":"url"}')
//!         └─ Getters::get: parse → (object, field) → getters::url   ← blocks this thread
//!              └─ CDP request ──▶ BrowserOS ──▶ reply, read by browseros-cdp's reader task
//!         ◀─ '"https://…"'                       JSON text of the value
//!
//!  locator.click()                               JS, on the run's thread
//!    └─ __neo_runtime.callWithEvents('{"id":1,"guid":"frame@<id>","method":"click",…}')
//!         └─ Dispatcher::call: starts the call on its own tokio task, returns at once
//!         ◀─ Promise of {reply: '{"id":1}' | '{"id":1,"error":{…},"log":[…]}', events: W}
//!    (`call` is the same without `events`: the reply text alone, for white-box tests)
//!
//!  controller.abort()                            JS, a call's `abort` listener (connection.ts)
//!    └─ __neo_runtime.abort(1, 'reason')         sync: aborts message 1's call (`abort.rs`)
//!
//!  page.on('console', f)                         JS, when a listener count leaves 0
//!    └─ __neo_runtime.subscribe('{"guid":"page@<id>","event":"console","enabled":true}')
//!  runtime-bridge.ts, before a reply's caller continues, and on its idle wake
//!    └─ __neo_runtime.drain(W) ◀─ '[{"seq":1,"guid":"context@default","method":"console",…}]'
//!    └─ __neo_runtime.nextEvents() ◀─ Promise that resolves once an event is queued
//! ```
//!
//! A call's promise only waits for the reply its task sends; this file does no CDP for it. So a
//! call keeps going while the script spins or a getter blocks, and dropping the realm drops only
//! the wait, never the call. The reply carries the event watermark W (`events.rs`): the SDK runs
//! the listeners of every event up to W before the call's caller continues, Playwright's order.

use std::future::Future;
use std::mem;
use std::sync::{Arc, Mutex};

use rquickjs::function::{Async, Func};
use rquickjs::{Ctx, Exception, Object};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::task;

use crate::deadline::{Deadline, Stop};
use crate::dispatch::Dispatcher;
use crate::events::{self, CONTEXT_GUID};
use crate::getters::{self, FrameField};
use crate::network::{HopId, LogError, ObjectKind as NetworkObject};
use crate::page_events;
use crate::progress::PlaywrightError;
use crate::run::{PageId, RunError};
use crate::tabs::Tabs;
use crate::url_match::{self, SerializedUrlMatch};

/// The guid the realm gives a run's page, in the `<type>@<id>` form Playwright's protocol uses.
pub(crate) fn page_guid(page: &PageId) -> String {
    format!("page@{}", page.as_str())
}

/// The guid the realm gives a run's main frame. Chromium gives a page's main frame its target's
/// id (measured in `tests/sync_getters.rs`), so the frame is `frame@<targetId>`.
pub(crate) fn frame_guid(page: &PageId) -> String {
    format!("frame@{}", page.as_str())
}

/// Installs `__neo_runtime` as a global of the realm `ctx`.
pub(crate) fn install<'js>(
    ctx: &Ctx<'js>,
    getters: Getters,
    logs: Logs,
    dispatcher: Dispatcher,
) -> rquickjs::Result<()> {
    let tabs = getters.tabs.clone();
    let bridge = Object::new(ctx.clone())?;
    bridge.set(
        "get",
        Func::from(move |ctx: Ctx<'js>, request: String| getters.get(&ctx, &request)),
    )?;
    let log = logs.clone();
    bridge.set("log", Func::from(move |line: String| log.push(line)))?;
    let aborts = dispatcher.clone();
    bridge.set(
        "abort",
        // Synchronous, inside the signal's `abort` listener: the call stops at its next race or
        // check and replies `AbortError`. An id that is not a call in flight is ignored.
        Func::from(move |id: i64, reason: String| {
            if let Ok(id) = u64::try_from(id) {
                aborts.abort(id, reason);
            }
        }),
    )?;
    let calls = dispatcher.clone();
    bridge.set(
        "call",
        // The reply text alone: what the realm's tests drive Rust with directly. The SDK uses
        // `callWithEvents`, so its listeners run before the reply's caller continues.
        Func::from(Async(move |ctx: Ctx<'js>, message: String| {
            let reply = calls.call(&message);
            async move {
                reply
                    .await
                    .map(|(reply, _)| reply)
                    .map_err(|error| throw_named(&ctx, "Error", &error.to_string()))
            }
        })),
    )?;
    bridge.set(
        "callWithEvents",
        // rquickjs makes the promise now and polls this future only while the script awaits.
        // That is enough because the future only waits for the reply: `Dispatcher::call` has
        // already started the call on a tokio task. An `Err` rejects the promise with a named
        // `Error`; only an unreadable envelope (an SDK bug) or a run ending under the call does.
        Func::from(Async(move |ctx: Ctx<'js>, message: String| {
            let reply = dispatcher.call(&message);
            async move {
                let (reply, watermark) = reply
                    .await
                    .map_err(|error| throw_named(&ctx, "Error", &error.to_string()))?;
                let answer = Object::new(ctx.clone())?;
                answer.set("reply", reply)?;
                answer.set("events", i64::try_from(watermark).unwrap_or(i64::MAX))?;
                Ok::<_, rquickjs::Error>(answer)
            }
        })),
    )?;
    let subscriptions = tabs.clone();
    bridge.set(
        "subscribe",
        Func::from(move |ctx: Ctx<'js>, request: String| {
            subscribe(&subscriptions, &request)
                .map_err(|message| throw_named(&ctx, "Error", &message))
        }),
    )?;
    let hub = tabs.events().clone();
    bridge.set(
        "drain",
        // Synchronous: the SDK runs the drained events' listeners in the same JS job, before the
        // reply that asked for them reaches its caller. `until` < 0 takes every queued event.
        Func::from(move |until: i64| {
            let text = hub.drain(u64::try_from(until).ok());
            for note in hub.take_notes() {
                logs.push(note);
            }
            text
        }),
    )?;
    let hub = tabs.events().clone();
    bridge.set(
        "nextEvents",
        // The idle wake: the SDK keeps one pending while it listens to anything. It resolves once
        // an undelivered event exists and never keeps the run alive (the realm ends with the
        // script, dropping it).
        Func::from(Async(move || {
            let hub = hub.clone();
            async move {
                hub.next().await;
                Ok::<(), rquickjs::Error>(())
            }
        })),
    )?;
    ctx.globals().set("__neo_runtime", bridge)
}

/// `__neo_runtime.subscribe`: the script's listening to `event` on a page or the context turned
/// on or off. On, it switches on what the event's producer needs in each page it covers, and
/// that page's next call waits for it (`EventHub::ready`); off, a page whose console nobody
/// listens to any more frees the objects its console events hold.
fn subscribe(tabs: &Tabs, request: &str) -> Result<(), String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Subscription {
        guid: String,
        event: String,
        enabled: bool,
    }
    let Subscription {
        guid,
        event,
        enabled,
    } = serde_json::from_str(request).map_err(|error| format!("bad subscribe request: {error}"))?;
    let needs = events::needs(&guid, &event)
        .ok_or_else(|| format!("Neo does not produce {event:?} events on {guid:?}"))?;
    let pages = if guid == CONTEXT_GUID {
        tabs.attached()
            .into_iter()
            .map(|(entry, _)| entry)
            .collect()
    } else {
        let (entry, _) = tabs
            .resolve(&guid)
            .ok_or_else(|| format!("unknown guid {guid:?}"))?;
        vec![entry]
    };
    tabs.events().subscribe(&guid, &event, enabled);
    for entry in pages {
        if enabled {
            page_events::enable(tabs, entry, needs);
        } else if needs.runtime
            && let Some(session) = entry.attached_session()
            && session.events.console_unwanted()
        {
            session.events.release_console_objects();
        }
    }
    Ok(())
}

/// The most `console` text and lines a run keeps. Kept lines live in Rust, outside the realm's
/// memory limit, so without a cap one script could hold any amount of the server's memory.
const MAX_LOG_BYTES: usize = 1024 * 1024;
const MAX_LOG_LINES: usize = 1000;

/// Lines the script wrote with `console`. Shared because the realm owns the JS `log` function
/// while the run reads the lines back, also after a deadline dropped the realm mid-script.
///
/// Keeps the first lines up to [`MAX_LOG_LINES`] and [`MAX_LOG_BYTES`]. The first line past
/// either cap ends keeping: it and every later line are only counted, so the kept lines are
/// always the start of what the script wrote, and [`Logs::take`] ends them with a note saying
/// how many were dropped. Logging itself never fails, so the cap never throws into the script.
#[derive(Debug, Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<LogLines>>);

#[derive(Debug, Default)]
struct LogLines {
    kept: Vec<String>,
    kept_bytes: usize,
    /// Set by the first line that did not fit.
    overflow: Option<Overflow>,
}

#[derive(Debug)]
struct Overflow {
    limit: LogLimit,
    dropped: usize,
}

#[derive(Debug, Clone, Copy)]
enum LogLimit {
    Lines,
    Bytes,
}

impl Logs {
    pub(crate) fn push(&self, line: String) {
        if let Ok(mut lines) = self.0.lock() {
            lines.push(line);
        }
    }

    pub(crate) fn take(&self) -> Vec<String> {
        let Ok(mut lines) = self.0.lock() else {
            return Vec::new();
        };
        let LogLines {
            mut kept, overflow, ..
        } = mem::take(&mut *lines);
        kept.extend(overflow.map(|overflow| overflow.note()));
        kept
    }
}

impl LogLines {
    fn push(&mut self, line: String) {
        if let Some(overflow) = &mut self.overflow {
            overflow.dropped += 1;
            return;
        }
        if let Some(limit) = self.limit_passed_by(&line) {
            self.overflow = Some(Overflow { limit, dropped: 1 });
        } else {
            self.kept_bytes += line.len();
            self.kept.push(line);
        }
    }

    fn limit_passed_by(&self, line: &str) -> Option<LogLimit> {
        if self.kept.len() >= MAX_LOG_LINES {
            Some(LogLimit::Lines)
        } else if self.kept_bytes + line.len() > MAX_LOG_BYTES {
            Some(LogLimit::Bytes)
        } else {
            None
        }
    }
}

impl Overflow {
    fn note(&self) -> String {
        let lines = if self.dropped == 1 { "line" } else { "lines" };
        let limit = match self.limit {
            LogLimit::Lines => format!("{MAX_LOG_LINES} lines"),
            LogLimit::Bytes => format!("{} MiB of console text", MAX_LOG_BYTES >> 20),
        };
        format!(
            "neo-runtime dropped the last {} console {lines}: a run keeps at most {limit}",
            self.dropped
        )
    }
}

/// Everything `__neo_runtime.get` needs to answer for one run.
pub(crate) struct Getters {
    tabs: Tabs,
    deadline: Deadline,
    tokio: Handle,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum GetterField {
    Url,
    IsClosed,
    Pages,
    Initializer,
    /// On a response: its request and every hop before, for the SDK to build.
    Chain,
    /// On a request: `timing()`, which Playwright's client updates from events.
    Timing,
    /// On a request: `failure()`'s text, set by Playwright's `requestFailed` event.
    Failure,
    /// On a response: `"finished"`, `"pending"` or `"closed"`, which `finished()` waits on.
    Finished,
    Frames,
    Name,
    ParentFrame,
    ChildFrames,
    IsDetached,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectRequest {
    guid: String,
    field: GetterField,
}

#[derive(Deserialize)]
enum MatchField {
    #[serde(rename = "matchesURL")]
    MatchesUrl,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MatchRequest {
    #[serde(rename = "field")]
    _field: MatchField,
    url: String,
    matcher: SerializedUrlMatch,
}

#[derive(Deserialize)]
enum ParseField {
    #[serde(rename = "parseURL")]
    ParseUrl,
}

/// A URL predicate's argument: the parts of `new URL(url)`, or `null` where it would throw.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParseRequest {
    #[serde(rename = "field")]
    _field: ParseField,
    url: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum GetRequest {
    Object(ObjectRequest),
    Match(MatchRequest),
    Parse(ParseRequest),
}

/// Why a getter threw. Each maps to the JS error the script sees.
#[derive(Debug)]
enum GetError {
    BadRequest(String),
    Page(PlaywrightError),
    Stopped(Stop),
}

impl Getters {
    pub(crate) fn new(tabs: Tabs, deadline: Deadline, tokio: Handle) -> Self {
        Self {
            tabs,
            deadline,
            tokio,
        }
    }

    fn get(&self, ctx: &Ctx<'_>, request: &str) -> rquickjs::Result<String> {
        match self.answer(request) {
            Ok(value) => Ok(value.to_string()),
            Err(error) => Err(self.throw(ctx, error)),
        }
    }

    fn answer(&self, request: &str) -> Result<Value, GetError> {
        let request: GetRequest = serde_json::from_str(request)
            .map_err(|error| GetError::BadRequest(format!("bad getter request: {error}")))?;
        let request = match request {
            GetRequest::Object(request) => request,
            GetRequest::Match(request) => {
                return self.wait(async {
                    getters::matches_url(&request.url, &request.matcher).map(|value| json!(value))
                });
            }
            GetRequest::Parse(request) => {
                return serde_json::to_value(url_match::parse_url(&request.url))
                    .map_err(|error| GetError::BadRequest(format!("encoding a URL: {error}")));
            }
        };
        if request.guid == "context@default" && matches!(request.field, GetterField::Pages) {
            return self.wait(async {
                serde_json::to_value(self.tabs.pages().await?)
                    .map_err(|error| PlaywrightError::error(error.to_string()))
            });
        }
        if let Some((kind, _, hop)) = HopId::parse(&request.guid) {
            return self.network(&request.guid, kind, &hop, request.field);
        }
        let (page, scope) = self
            .tabs
            .resolve(&request.guid)
            .ok_or_else(|| GetError::BadRequest(format!("unknown guid {:?}", request.guid)))?;
        if request.guid.starts_with("frame@") {
            let field = match request.field {
                GetterField::Url => FrameField::Url,
                GetterField::Name => FrameField::Name,
                GetterField::ParentFrame => FrameField::ParentFrame,
                GetterField::ChildFrames => FrameField::ChildFrames,
                GetterField::IsDetached => FrameField::IsDetached,
                _ => return Err(GetError::BadRequest("not a frame getter".to_string())),
            };
            return self.wait(async { getters::frame(&self.tabs, &page, &scope, field).await });
        }
        if !request.guid.starts_with("page@") {
            return Err(GetError::BadRequest(format!(
                "unknown guid {:?}",
                request.guid
            )));
        }
        match request.field {
            GetterField::Url => self.wait(self.tabs.url(&page)).map(|url| json!(url)),
            GetterField::IsClosed => self
                .wait(self.tabs.is_closed(&page))
                .map(|closed| json!(closed)),
            GetterField::Initializer => self.wait(async {
                serde_json::to_value(self.tabs.initializer(&page).await?)
                    .map_err(|error| PlaywrightError::error(error.to_string()))
            }),
            GetterField::Pages => Err(GetError::BadRequest(
                "pages is a context getter".to_string(),
            )),
            GetterField::Chain
            | GetterField::Timing
            | GetterField::Failure
            | GetterField::Finished => Err(GetError::BadRequest(format!(
                "{:?} is a request or response getter",
                request.field
            ))),
            GetterField::Frames => self.wait(async { getters::frames(&self.tabs, &page).await }),
            GetterField::Name
            | GetterField::ParentFrame
            | GetterField::ChildFrames
            | GetterField::IsDetached => Err(GetError::BadRequest("not a page getter".to_string())),
        }
    }

    /// A hop's live client state, read from the network log of the session that recorded it.
    /// Nothing here waits: every value is already in Rust.
    fn network(
        &self,
        guid: &str,
        kind: NetworkObject,
        hop: &HopId,
        field: GetterField,
    ) -> Result<Value, GetError> {
        let unknown = || GetError::BadRequest(format!("unknown guid {guid:?}"));
        let (page, _) = self.tabs.resolve(guid).ok_or_else(unknown)?;
        // A hop exists only after a goto on this session, which stays attached for the run.
        let session = page.attached_session().ok_or_else(unknown)?;
        let log = &session.network;
        let value = match (kind, field) {
            (NetworkObject::Response, GetterField::Chain) => log
                .chain(hop)
                .and_then(|chain| serde_json::to_value(chain).map_err(|_| LogError::Poisoned)),
            (NetworkObject::Request, GetterField::Timing) => log
                .timing(hop)
                .and_then(|timing| serde_json::to_value(timing).map_err(|_| LogError::Poisoned)),
            (NetworkObject::Request, GetterField::Failure) => {
                log.failure(hop).map(|text| json!(text))
            }
            (NetworkObject::Response, GetterField::Finished) => log.finished(hop).map(|finished| {
                json!(if finished {
                    "finished"
                } else if page.closed.is_cancelled() {
                    "closed"
                } else {
                    "pending"
                })
            }),
            (_, field) => return Err(GetError::BadRequest(format!("no {field:?} on {guid:?}"))),
        };
        value.map_err(|error| GetError::Page(PlaywrightError::error(error.to_string())))
    }

    /// Waits on the current thread for one CDP reply, so a JS getter can return a plain
    /// value. This is safe because a run's JS is single-threaded: while the getter waits the
    /// script has nothing else it could do. `block_in_place` hands this tokio worker's other
    /// tasks to another thread first, so other runs and browseros-cdp's reader task, which
    /// delivers the reply, keep going. That needs tokio's multi-threaded runtime, which
    /// `Runtime::run` checks before a realm exists. The wait ends early at the run's deadline
    /// or on cancellation.
    fn wait<T>(
        &self,
        read: impl Future<Output = Result<T, PlaywrightError>>,
    ) -> Result<T, GetError> {
        task::block_in_place(|| {
            self.tokio.block_on(async {
                tokio::select! {
                    stop = self.deadline.reached() => Err(GetError::Stopped(stop)),
                    result = read => result.map_err(GetError::Page),
                }
            })
        })
    }

    fn throw(&self, ctx: &Ctx<'_>, error: GetError) -> rquickjs::Error {
        let error = match error {
            GetError::BadRequest(message) => RunError::new("Error", message),
            GetError::Page(error) => RunError::new(error.name.as_str(), error.message),
            GetError::Stopped(Stop::Timeout) => RunError::timeout(self.deadline.timeout()),
            GetError::Stopped(Stop::Cancelled) => {
                RunError::new("Error", "Run cancelled".to_string())
            }
        };
        throw_named(ctx, &error.name, &error.message)
    }
}

/// Throws a JS `Error` whose `name` is `name`, the way Playwright's own error classes read.
fn throw_named(ctx: &Ctx<'_>, name: &str, message: &str) -> rquickjs::Error {
    let exception = match Exception::from_message(ctx.clone(), message) {
        Ok(exception) => exception,
        Err(error) => return error,
    };
    if let Err(error) = exception.as_object().set("name", name) {
        return error;
    }
    exception.throw()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::GetRequest;
    use crate::getters;

    #[test]
    fn tests_that_matches_url_uses_the_wait_for_url_wire_shape_without_a_guid() {
        for wire in [
            r#"{"field":"matchesURL","url":"https://a.test/","matcher":{"glob":"https://A.test/"}}"#,
            r#"{"field":"matchesURL","url":"https://a.test/","matcher":{"regexSource":"A\\.test","regexFlags":"i"}}"#,
        ] {
            let GetRequest::Match(request) = serde_json::from_str(wire).unwrap() else {
                panic!("expected the pure URL matcher getter");
            };
            assert!(getters::matches_url(&request.url, &request.matcher).unwrap());
        }
    }

    #[test]
    fn tests_that_frame_getter_requests_preserve_the_opaque_guid() {
        for field in ["url", "name", "parentFrame", "childFrames", "isDetached"] {
            let wire = json!({"guid": "frame@tab/child", "field": field});
            let GetRequest::Object(request) = serde_json::from_value(wire).unwrap() else {
                panic!("expected a frame getter");
            };
            assert_eq!(request.guid, "frame@tab/child");
        }
        assert!(serde_json::from_str::<GetRequest>(r#"{"field":"matchesURL","url":"https://a.test/","matcher":{"glob":"**"},"guid":"frame@tab"}"#).is_err());
    }
}
