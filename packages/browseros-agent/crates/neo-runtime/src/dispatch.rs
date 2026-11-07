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
//!         ├─ admit against the in-flight caps   past them → an early failure too
//!         └─ run.tasks.spawn(serve)             runs while JS spins, outlives the realm
//!              ├─ click::click(&page, &options, &progress)   prepares the tab first
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use neo_runtime_spec::injected::ReadableElementState;
use neo_runtime_spec::serve::{
    BrowserContextCall, FrameAriaSnapshotParams, FrameCall, FrameCheckParams, FrameCheckResult,
    FrameClickParams, FrameDblclickParams, FrameEvalOnSelectorAllParams, FrameFillParams,
    FrameFillResult, FrameGetAttributeParams, FrameGotoParams, FrameHoverParams,
    FrameInnerHTMLParams, FrameInnerTextParams, FrameInputValueParams, FrameInputValueResult,
    FrameIsCheckedParams, FrameIsCheckedResult, FrameIsDisabledParams, FrameIsDisabledResult,
    FrameIsEditableParams, FrameIsEditableResult, FrameIsEnabledParams, FrameIsEnabledResult,
    FrameIsHiddenParams, FrameIsHiddenResult, FrameIsVisibleParams, FrameIsVisibleResult,
    FramePressParams, FramePressResult, FrameQueryCountParams, FrameSelectOptionParams,
    FrameSelectOptionResult, FrameTapParams, FrameTextContentParams, FrameTitleParams,
    FrameTypeParams, FrameTypeResult, FrameUncheckParams, FrameUncheckResult,
    FrameWaitForSelectorParams, FrameWaitForSelectorResult, Metadata, PageCall, RequestCall,
    ResponseCall,
};
use neo_runtime_spec::{CallDecodeError, Command, MethodFlag, MethodMeta};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::time;
use tokio::time::Instant;
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::check::{self, CheckOptions};
use crate::click::{self, PointerOptions};
use crate::devices::{self, DeviceCall};
use crate::fill::{self, FillOptions, InputValueOptions, SelectOptions};
use crate::forms::FormTab;
use crate::frames::FrameScope;
use crate::keyboard::KeyboardTab;
use crate::lists;
use crate::navigate::{self, GotoOptions};
use crate::network::{HopId, ObjectKind as NetworkObject};
use crate::page::FrameTab;
use crate::press::{self, KeyInput, PressOptions};
use crate::progress::{
    CallEntry, CallError, CallStart, PlaywrightError, Progress, RunScope, Trace,
};
use crate::reads::{self, AttributeOptions, ReadOptions, SnapshotOptions};
use crate::responses::{self, NetworkCall};
use crate::run::{CallLocation, CallRecord, CallResult, DroppedCalls, RunEnd};
use crate::states::{self, StateOptions};
use crate::tab_calls;
use crate::tabs::{PageEntry, Tabs};
use crate::waits::{
    self, NeoWaitForLoadStateParams, NeoWaitForUrlParams, WaitForLoadStateOptions,
    WaitForSelectorOptions, WaitForUrlOptions,
};

/// The shared budget for creation settlement, call draining and closing-state reads after
/// the realm ends. Tasks still running keep their state and records read `Unfinished`.
/// Unkept creations close their tabs when their late acknowledgements arrive.
const RECORDS_WAIT: Duration = Duration::from_secs(1);

/// The most browser calls one run may have unfinished at once. Each holds its params, its call
/// log and a task in Rust, outside the realm's memory limit, and a script can start calls without
/// awaiting them, so without a cap one run could hold any amount of the server's memory.
/// Playwright has no such cap; a script that awaits its calls, or a `Promise.all` over a page's
/// worth of locators, stays far below it.
const MAX_CALLS_IN_FLIGHT: usize = 64;

/// The most request bytes (each call's message as the SDK sent it) one run's unfinished calls
/// may total, for the same reason: a few calls with huge selectors would otherwise hold several
/// copies of each outside the realm.
const MAX_REQUEST_BYTES_IN_FLIGHT: usize = 8 * 1024 * 1024;

/// Neo has no `ElementHandle` objects to pass as `selectOption` values.
const ELEMENTS_UNSUPPORTED: &str = "Neo does not support ElementHandle values in selectOption yet";

// Playwright implements these on its eventful client. Neo has a call/reply bridge, so its
// Frame extensions keep the whole wait in Rust. Wire shapes live in waits.rs and neo-commands.ts.
static WAIT_FOR_URL_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "neo:waitForURL",
    title: Some("Wait for URL"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[],
};
static WAIT_FOR_LOAD_STATE_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "neo:waitForLoadState",
    title: Some("Wait for load state"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[],
};
/// Arbitrary page JS remains outside Neo's typed glue boundary.
const EVAL_ON_SELECTOR_ALL_UNSERVED: &str =
    "Neo serves Frame.evalOnSelectorAll only for locator.allTextContents";

// `serve` replies to a pointer action with `{id}` and no `result`, which is right only while
// Playwright's spec declares no `returns` for `Frame.click`, `dblclick`, `hover` and `tap`. A
// spec that gives one a result stops this compiling.
const _: () = assert!(
    <FrameClickParams as Command>::VOID_RESULT
        && <FrameDblclickParams as Command>::VOID_RESULT
        && <FrameHoverParams as Command>::VOID_RESULT
        && <FrameTapParams as Command>::VOID_RESULT,
    "a pointer action gained a result: reply with it in dispatch::serve",
);

/// Owns every browser call of one run. Cheap to clone: the bridge holds one, `Runtime::run`
/// another. Call tasks hold their `Progress`, entry and the trace, never a `Dispatcher`, so the
/// last clone drops once the realm is gone and the run is done with it (or its future was
/// dropped). Then `_ending` cancels the run's end token, and every call stops at its next await.
#[derive(Clone)]
pub(crate) struct Dispatcher(Arc<Inner>);

struct Inner {
    /// Owns every session attached for this run; frame calls resolve their own tab here.
    tabs: Tabs,
    run: RunScope,
    trace: Trace,
    /// What the run's unfinished calls hold, bounded at admission.
    in_flight: Arc<InFlight>,
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
    route: Result<ObjectCall, PlaywrightError>,
}

#[derive(Clone)]
enum ObjectKind {
    Context,
    Page,
    Frame,
    /// A hop's `Request` or `Response`, behind `goto`'s answer.
    Network(NetworkObject, HopId),
    Unknown,
}

enum ObjectCall {
    Context(BrowserContextCall),
    Page(PageCall),
    /// `page.keyboard`, `page.mouse` and `page.touchscreen` (`devices.rs`).
    Device(DeviceCall),
    Frame(Route),
    Network(NetworkCall),
}

/// What a message asks for. One variant per served (object, method).
#[derive(Debug)]
#[expect(
    clippy::enum_variant_names,
    reason = "Routes name their protocol interface and method"
)]
enum Route {
    /// `Frame.click`, `dblclick`, `hover` and `tap`: one policy (`click.rs`).
    FramePointer(PointerOptions),
    FrameGoto(GotoOptions),
    FrameTitle,
    FrameTextContent(ReadOptions),
    FrameInnerText(ReadOptions),
    FrameGetAttribute(AttributeOptions),
    FrameQueryCount(String),
    FrameAllTextContents(String),
    FrameAllInnerTexts(String),
    FrameInnerHtml(ReadOptions),
    FrameAriaSnapshot(SnapshotOptions),
    FrameFill(FillOptions),
    FramePress(PressOptions),
    /// `Frame.type`, served by press's policy.
    FrameType(PressOptions),
    FrameInputValue(InputValueOptions),
    FrameIsVisible(StateOptions),
    FrameIsHidden(StateOptions),
    FrameIsChecked(StateOptions),
    FrameIsEnabled(StateOptions),
    FrameIsDisabled(StateOptions),
    FrameIsEditable(StateOptions),
    FrameSetChecked(CheckOptions),
    FrameSelectOption(SelectOptions),
    FrameWaitForSelector(WaitForSelectorOptions),
    FrameWaitForUrl(WaitForUrlOptions),
    FrameWaitForLoadState(WaitForLoadStateOptions),
}

impl Dispatcher {
    pub(crate) fn new(tabs: Tabs, run: RunScope) -> Self {
        Self(Arc::new(Inner {
            tabs,
            _ending: run.ended.clone().drop_guard(),
            run,
            trace: Trace::default(),
            in_flight: Arc::default(),
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
        let target = inner.tabs.resolve(&guid);
        let object = if guid == "context@default" {
            ObjectKind::Context
        } else if target.is_none() {
            ObjectKind::Unknown
        } else if guid.starts_with("frame@") {
            ObjectKind::Frame
        } else if let Some((kind, _, hop)) = HopId::parse(&guid) {
            ObjectKind::Network(kind, hop)
        } else {
            ObjectKind::Page
        };
        let Routed { meta, route } = route(object, &method, params.clone());
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
        let admitted = route.and_then(|route| {
            let admitted = inner.in_flight.admit(message.len())?;
            Ok((route, admitted))
        });
        let (route, admitted) = match admitted {
            Ok(admitted) => admitted,
            Err(error) => {
                entry.finish(error.to_result());
                inner.trace.finished(&entry);
                return PendingReply::ready(Ok(encode_reply(&id, Err(&error), None)));
            }
        };
        // Close must finish its own acknowledgement after cancelling this page's other
        // calls. A context call also survives the original run page being closed.
        let closed = if matches!(route, ObjectCall::Page(PageCall::Close(_))) {
            CancellationToken::new()
        } else {
            target
                .as_ref()
                .map_or_else(CancellationToken::new, |(page, _)| page.closed.clone())
        };
        let progress = Progress::new(
            Arc::clone(&entry),
            metadata.timeout.unwrap_or(0.0),
            inner.run.clone(),
            closed,
        );
        let (reply, receiver) = oneshot::channel();
        let served = serve_call(
            inner.tabs.clone(),
            target,
            route,
            progress,
            CallReply {
                entry,
                trace: inner.trace.clone(),
                id,
                reply,
            },
        );
        inner.run.tasks.spawn(async move {
            // The call's share of the in-flight caps, given back however the task ends.
            let _admitted = admitted;
            served.await;
        });
        PendingReply(PendingInner::Waiting(receiver))
    }

    /// Ends the run's calls after the realm is gone: records why the run ended (the first
    /// reason wins), and shares [`RECORDS_WAIT`] across creation settlement, task draining
    /// and closing-state reads before returning the records and dropped-calls summary.
    pub(crate) async fn finish(&self, reason: RunEnd) -> (Vec<CallRecord>, Option<DroppedCalls>) {
        let deadline = Instant::now() + RECORDS_WAIT;
        let run = &self.0.run;
        run.end(reason);
        let _created = time::timeout_at(deadline, self.0.tabs.finish_creations()).await;
        run.tasks.close();
        let _waited = time::timeout_at(deadline, run.tasks.wait()).await;
        // Keys and buttons this run left held are released on each tab it touched, under a
        // turn, inside the same budget (design section 4.6); a closed tab is skipped.
        let _released = time::timeout_at(deadline, self.0.tabs.release_input()).await;
        let _refreshed = time::timeout_at(deadline, self.0.tabs.refresh_closing()).await;
        self.0.trace.take(reason)
    }
}

/// Names a message's target and decodes its params. Pure.
fn route(object: ObjectKind, method: &str, params: Option<Value>) -> Routed {
    let decoded = match object {
        ObjectKind::Context => BrowserContextCall::decode(method, params)
            .map(|call| (call.meta(), Ok(ObjectCall::Context(call)))),
        ObjectKind::Page => PageCall::decode(method, params).map(|call| {
            let meta = call.meta();
            let routed = match DeviceCall::from_page_call(call) {
                Ok(device) => device.map(ObjectCall::Device),
                Err(call) => Ok(ObjectCall::Page(call)),
            };
            (meta, routed)
        }),
        ObjectKind::Frame => {
            if method == WAIT_FOR_URL_META.method {
                return Routed {
                    meta: Some(&WAIT_FOR_URL_META),
                    route: decode_neo_params::<NeoWaitForUrlParams>(params, &WAIT_FOR_URL_META)
                        .and_then(|params| WaitForUrlOptions::from_params(&params))
                        .map(|options| ObjectCall::Frame(Route::FrameWaitForUrl(options))),
                };
            }
            if method == WAIT_FOR_LOAD_STATE_META.method {
                return Routed {
                    meta: Some(&WAIT_FOR_LOAD_STATE_META),
                    route: decode_neo_params::<NeoWaitForLoadStateParams>(
                        params,
                        &WAIT_FOR_LOAD_STATE_META,
                    )
                    .and_then(WaitForLoadStateOptions::from_params)
                    .map(|options| ObjectCall::Frame(Route::FrameWaitForLoadState(options))),
                };
            }
            // ElementHandleChannel is uninhabited in Neo, so reject non-empty arrays before
            // serde tries to decode their members. Keep the method's input flag and title.
            if method == "selectOption"
                && params
                    .as_ref()
                    .and_then(|params| params.get("elements"))
                    .and_then(Value::as_array)
                    .is_some_and(|elements| !elements.is_empty())
            {
                return Routed {
                    meta: Some(FrameSelectOptionParams::META),
                    route: Err(PlaywrightError::error(ELEMENTS_UNSUPPORTED)),
                };
            }
            FrameCall::decode(method, params)
                .map(|call| (call.meta(), route_frame_call(call).map(ObjectCall::Frame)))
        }
        ObjectKind::Network(NetworkObject::Request, hop) => RequestCall::decode(method, params)
            .map(|call| {
                let meta = call.meta();
                (
                    meta,
                    Ok(ObjectCall::Network(NetworkCall::Request(hop, call))),
                )
            }),
        ObjectKind::Network(NetworkObject::Response, hop) => ResponseCall::decode(method, params)
            .map(|call| {
                let meta = call.meta();
                (
                    meta,
                    Ok(ObjectCall::Network(NetworkCall::Response(hop, call))),
                )
            }),
        ObjectKind::Unknown => {
            return Routed {
                meta: None,
                route: Err(PlaywrightError::target_closed()),
            };
        }
    };
    match decoded {
        Ok((meta, route)) => Routed {
            meta: Some(meta),
            route,
        },
        Err(error) => Routed {
            meta: match &error {
                CallDecodeError::InvalidParams { meta, .. } => Some(*meta),
                CallDecodeError::UnknownMethod { .. } => None,
            },
            route: Err(PlaywrightError::error(error.to_string())),
        },
    }
}

fn decode_neo_params<T: DeserializeOwned>(
    params: Option<Value>,
    meta: &MethodMeta,
) -> Result<T, PlaywrightError> {
    serde_json::from_value(params.unwrap_or_else(|| json!({}))).map_err(|error| {
        PlaywrightError::error(format!(
            "invalid params for {}.{}: {error}",
            meta.interface, meta.method
        ))
    })
}

/// Serving a new `Frame` command in `spec/serve.yml` adds a `FrameCall` variant, and this match
/// stops compiling until the command is routed.
fn route_frame_call(call: FrameCall) -> Result<Route, PlaywrightError> {
    match call {
        FrameCall::IsVisible(params) => Ok(Route::FrameIsVisible(StateOptions::new(
            params.selector,
            params.strict,
        ))),
        FrameCall::IsHidden(params) => Ok(Route::FrameIsHidden(StateOptions::new(
            params.selector,
            params.strict,
        ))),
        FrameCall::IsChecked(params) => Ok(Route::FrameIsChecked(StateOptions::new(
            params.selector,
            params.strict,
        ))),
        FrameCall::IsEnabled(params) => Ok(Route::FrameIsEnabled(StateOptions::new(
            params.selector,
            params.strict,
        ))),
        FrameCall::IsDisabled(params) => Ok(Route::FrameIsDisabled(StateOptions::new(
            params.selector,
            params.strict,
        ))),
        FrameCall::IsEditable(params) => Ok(Route::FrameIsEditable(StateOptions::new(
            params.selector,
            params.strict,
        ))),
        FrameCall::Click(params) => PointerOptions::from_click(params).map(Route::FramePointer),
        FrameCall::Dblclick(params) => {
            PointerOptions::from_dblclick(params).map(Route::FramePointer)
        }
        FrameCall::Hover(params) => Ok(Route::FramePointer(PointerOptions::from_hover(params))),
        FrameCall::Tap(params) => Ok(Route::FramePointer(PointerOptions::from_tap(params))),
        FrameCall::Goto(params) => GotoOptions::from_params(params).map(Route::FrameGoto),
        FrameCall::Title(_) => Ok(Route::FrameTitle),
        FrameCall::TextContent(params) => {
            Ok(Route::FrameTextContent(ReadOptions::from_params(params)))
        }
        FrameCall::InnerText(params) => {
            Ok(Route::FrameInnerText(ReadOptions::from_inner_text(params)))
        }
        FrameCall::InnerHTML(params) => {
            Ok(Route::FrameInnerHtml(ReadOptions::from_inner_html(params)))
        }
        FrameCall::GetAttribute(params) => Ok(Route::FrameGetAttribute(
            AttributeOptions::from_params(params),
        )),
        FrameCall::QueryCount(params) => Ok(Route::FrameQueryCount(params.selector)),
        FrameCall::EvalOnSelectorAll(params)
            if params.expression == lists::ALL_TEXT_CONTENTS_EXPRESSION
                && params.is_function == Some(true) =>
        {
            Ok(Route::FrameAllTextContents(params.selector))
        }
        FrameCall::EvalOnSelectorAll(params)
            if params.expression == lists::ALL_INNER_TEXTS_EXPRESSION
                && params.is_function == Some(true) =>
        {
            Ok(Route::FrameAllInnerTexts(params.selector))
        }
        FrameCall::EvalOnSelectorAll(_) => {
            Err(PlaywrightError::error(EVAL_ON_SELECTOR_ALL_UNSERVED))
        }
        FrameCall::AriaSnapshot(params) => Ok(Route::FrameAriaSnapshot(
            SnapshotOptions::from_params(params),
        )),
        FrameCall::Fill(params) => Ok(Route::FrameFill(FillOptions::from_params(params))),
        FrameCall::Press(params) => Ok(Route::FramePress(PressOptions::from_params(params))),
        FrameCall::Type(params) => Ok(Route::FrameType(PressOptions::from_type(params))),
        FrameCall::InputValue(params) => Ok(Route::FrameInputValue(
            InputValueOptions::from_params(params),
        )),
        FrameCall::Check(params) => Ok(Route::FrameSetChecked(CheckOptions::from_check(params))),
        FrameCall::Uncheck(params) => {
            Ok(Route::FrameSetChecked(CheckOptions::from_uncheck(params)))
        }
        FrameCall::SelectOption(params) => {
            Ok(Route::FrameSelectOption(SelectOptions::from_params(params)))
        }
        FrameCall::WaitForSelector(params) => {
            WaitForSelectorOptions::from_params(params).map(Route::FrameWaitForSelector)
        }
    }
}

struct CallReply {
    entry: Arc<CallEntry>,
    trace: Trace,
    id: Value,
    reply: oneshot::Sender<String>,
}

/// Attachment can block or be cancelled independently of a frame handler. Once attached,
/// `serve` retains its original contract: the `PageSession` always belongs to this call's tab.
async fn serve_call(
    tabs: Tabs,
    target: Option<(Arc<PageEntry>, FrameScope)>,
    route: ObjectCall,
    progress: Progress,
    reply: CallReply,
) {
    let result = match (route, target) {
        (ObjectCall::Context(call), _) => tab_calls::context(&tabs, call, &progress).await,
        (ObjectCall::Page(call), Some((page, _))) => {
            tab_calls::page(&tabs, &page, call, &progress).await
        }
        (ObjectCall::Frame(route), Some((page, scope))) => {
            let session = match page.attached_session() {
                Some(session) => Ok(session),
                None => match progress.race(tabs.session(&page)).await {
                    Ok(result) => result.map_err(CallError::from),
                    Err(error) => Err(error),
                },
            };
            match session {
                Ok(page) => {
                    serve(
                        FrameTab { page, scope },
                        route,
                        progress,
                        reply.entry,
                        reply.trace,
                        reply.id,
                        reply.reply,
                    )
                    .await;
                    return;
                }
                Err(error) => Err(error),
            }
        }
        (ObjectCall::Device(call), Some((page, _))) => {
            let session = match page.attached_session() {
                Some(session) => Ok(session),
                None => match progress.race(tabs.session(&page)).await {
                    Ok(result) => result.map_err(CallError::from),
                    Err(error) => Err(error),
                },
            };
            match session {
                Ok(page) => {
                    let tab = FrameTab {
                        page,
                        scope: FrameScope::Main,
                    };
                    devices::serve(&tab, &call, &progress).await.map(|()| None)
                }
                Err(error) => Err(error),
            }
        }
        // A hop's objects exist only after a goto on this page's session, which stays attached
        // for the run.
        (ObjectCall::Network(call), Some((page, _))) => match page.attached_session() {
            Some(session) => responses::serve(&session, call, &progress).await,
            None => Err(PlaywrightError::target_closed().into()),
        },
        _ => Err(PlaywrightError::target_closed().into()),
    };
    finish_reply(result, reply);
}

/// One call's task: serves the route, ends the trace entry and sends the reply. A call the run
/// ended leaves its entry open, so its record reads `Unfinished`, and sends nothing: nobody
/// waits for it. The task holds the page, so the session stays attached while it runs.
async fn serve(
    page: FrameTab,
    route: Route,
    progress: Progress,
    entry: Arc<CallEntry>,
    trace: Trace,
    id: Value,
    reply: oneshot::Sender<String>,
) {
    // Navigation and page reads need the real page's navigation/read seams, which the
    // forms tests' fake does not implement, so they are answered here; every other route goes
    // through `serve_route`.
    if let Some(error) = unsupported_child_call(&page.scope, &route) {
        finish_reply(
            Err(error.into()),
            CallReply {
                entry,
                trace,
                id,
                reply,
            },
        );
        return;
    }
    let result = match route {
        Route::FrameGoto(options) => navigate::goto(&page, &options, &progress)
            .await
            .and_then(|result| encode_result::<FrameGotoParams>(&result)),
        Route::FrameTitle => navigate::title(&page, &progress)
            .await
            .and_then(|result| encode_result::<FrameTitleParams>(&result)),
        Route::FrameTextContent(options) => reads::text_content(&page, &options, &progress)
            .await
            .and_then(|result| encode_result::<FrameTextContentParams>(&result)),
        Route::FrameInnerText(options) => reads::inner_text(&page, &options, &progress)
            .await
            .and_then(|result| encode_result::<FrameInnerTextParams>(&result)),
        Route::FrameGetAttribute(options) => reads::get_attribute(&page, &options, &progress)
            .await
            .and_then(|result| encode_result::<FrameGetAttributeParams>(&result)),
        Route::FrameQueryCount(selector) => lists::query_count(&page, &selector, &progress)
            .await
            .and_then(|result| encode_result::<FrameQueryCountParams>(&result)),
        Route::FrameAllTextContents(selector) => {
            lists::all_text_contents(&page, &selector, &progress)
                .await
                .and_then(|result| encode_result::<FrameEvalOnSelectorAllParams>(&result))
        }
        Route::FrameAllInnerTexts(selector) => lists::all_inner_texts(&page, &selector, &progress)
            .await
            .and_then(|result| encode_result::<FrameEvalOnSelectorAllParams>(&result)),
        Route::FrameInnerHtml(options) => reads::inner_html(&page, &options, &progress)
            .await
            .and_then(|result| encode_result::<FrameInnerHTMLParams>(&result)),
        Route::FrameAriaSnapshot(options) => reads::aria_snapshot(&page, &options, &progress)
            .await
            .and_then(|result| encode_result::<FrameAriaSnapshotParams>(&result)),
        Route::FrameWaitForSelector(options) => {
            waits::wait_for_selector(&page, &options, &progress)
                .await
                .and_then(|()| {
                    encode_result::<FrameWaitForSelectorParams>(&FrameWaitForSelectorResult {
                        element: None,
                    })
                })
        }
        // Neo extensions answer an empty result.
        Route::FrameWaitForUrl(options) => waits::wait_for_url(&page, &options, &progress)
            .await
            .map(|()| Some(json!({}))),
        Route::FrameWaitForLoadState(options) => {
            waits::wait_for_load_state(&page, &options, &progress)
                .await
                .map(|()| Some(json!({})))
        }
        route @ (Route::FrameIsVisible(_)
        | Route::FrameIsHidden(_)
        | Route::FrameIsChecked(_)
        | Route::FrameIsEnabled(_)
        | Route::FrameIsDisabled(_)
        | Route::FrameIsEditable(_)) => serve_state(&page, route, &progress).await,
        route => serve_route(&page, route, &progress).await,
    };
    finish_reply(
        result,
        CallReply {
            entry,
            trace,
            id,
            reply,
        },
    );
}

/// The state reads `serve` answers on the call's page (`states.rs`): `isVisible` and `isHidden`
/// read once; the others wait for a match and read one `ElementState`.
async fn serve_state(
    page: &FrameTab,
    route: Route,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    match route {
        Route::FrameIsVisible(options) => states::is_visible(page, &options, progress)
            .await
            .and_then(|value| {
                encode_result::<FrameIsVisibleParams>(&FrameIsVisibleResult { value })
            }),
        Route::FrameIsHidden(options) => states::is_hidden(page, &options, progress)
            .await
            .and_then(|value| encode_result::<FrameIsHiddenParams>(&FrameIsHiddenResult { value })),
        Route::FrameIsChecked(options) => {
            states::element_state(page, &options, ReadableElementState::Checked, progress)
                .await
                .and_then(|value| {
                    encode_result::<FrameIsCheckedParams>(&FrameIsCheckedResult { value })
                })
        }
        Route::FrameIsEnabled(options) => {
            states::element_state(page, &options, ReadableElementState::Enabled, progress)
                .await
                .and_then(|value| {
                    encode_result::<FrameIsEnabledParams>(&FrameIsEnabledResult { value })
                })
        }
        Route::FrameIsDisabled(options) => {
            states::element_state(page, &options, ReadableElementState::Disabled, progress)
                .await
                .and_then(|value| {
                    encode_result::<FrameIsDisabledParams>(&FrameIsDisabledResult { value })
                })
        }
        Route::FrameIsEditable(options) => {
            states::element_state(page, &options, ReadableElementState::Editable, progress)
                .await
                .and_then(|value| {
                    encode_result::<FrameIsEditableParams>(&FrameIsEditableResult { value })
                })
        }
        _ => {
            Err(PlaywrightError::error("neo-runtime: `serve_state` serves state reads only").into())
        }
    }
}

fn unsupported_child_call(scope: &FrameScope, route: &Route) -> Option<PlaywrightError> {
    if matches!(scope, FrameScope::Main) {
        return None;
    }
    let method = match route {
        Route::FrameGoto(_) => "goto",
        _ => return None,
    };
    Some(PlaywrightError::error(format!(
        "Neo does not serve {method} on child frames yet"
    )))
}

fn finish_reply(result: Result<Option<Value>, CallError>, target: CallReply) {
    let CallReply {
        entry,
        trace,
        id,
        reply,
    } = target;
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

/// The typed handler dispatch, also used by the skeleton's tests with click's existing fake.
async fn serve_route<T: FormTab + KeyboardTab>(
    tab: &T,
    route: Route,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    match route {
        Route::FramePointer(options) => click::pointer_action(tab, &options, progress)
            .await
            .map(|()| None),
        Route::FrameFill(options) => fill::fill(tab, &options, progress)
            .await
            .and_then(|()| encode_result::<FrameFillParams>(&FrameFillResult {})),
        Route::FramePress(options) => press::press(tab, &options, progress)
            .await
            .and_then(|()| encode_result::<FramePressParams>(&FramePressResult {})),
        Route::FrameType(options) => press::press(tab, &options, progress)
            .await
            .and_then(|()| encode_result::<FrameTypeParams>(&FrameTypeResult {})),
        Route::FrameSetChecked(options) => {
            let checked = options.checked;
            check::set_checked(tab, &options, progress)
                .await
                .and_then(|()| {
                    if checked {
                        encode_result::<FrameCheckParams>(&FrameCheckResult {})
                    } else {
                        encode_result::<FrameUncheckParams>(&FrameUncheckResult {})
                    }
                })
        }
        Route::FrameInputValue(options) => fill::input_value(tab, &options, progress)
            .await
            .and_then(|value| {
                encode_result::<FrameInputValueParams>(&FrameInputValueResult { value })
            }),
        Route::FrameSelectOption(options) => fill::select_option(tab, &options, progress)
            .await
            .and_then(|values| {
                encode_result::<FrameSelectOptionParams>(&FrameSelectOptionResult { values })
            }),
        // `serve` answers these on the call's page before it delegates here.
        Route::FrameGoto(_)
        | Route::FrameTitle
        | Route::FrameTextContent(_)
        | Route::FrameInnerText(_)
        | Route::FrameGetAttribute(_)
        | Route::FrameQueryCount(_)
        | Route::FrameAllTextContents(_)
        | Route::FrameAllInnerTexts(_)
        | Route::FrameInnerHtml(_)
        | Route::FrameIsVisible(_)
        | Route::FrameIsHidden(_)
        | Route::FrameIsChecked(_)
        | Route::FrameIsEnabled(_)
        | Route::FrameIsDisabled(_)
        | Route::FrameIsEditable(_)
        | Route::FrameAriaSnapshot(_)
        | Route::FrameWaitForSelector(_)
        | Route::FrameWaitForUrl(_)
        | Route::FrameWaitForLoadState(_) => Err(PlaywrightError::error(
            "neo-runtime: navigation and page reads are served by `serve`",
        )
        .into()),
    }
}

/// Like `Command::reply`, without retaining the raw params once their options are decoded.
pub(crate) fn encode_result<C: Command>(result: &C::Result) -> Result<Option<Value>, CallError> {
    if C::VOID_RESULT {
        return Ok(None);
    }
    serde_json::to_value(result).map(Some).map_err(|error| {
        PlaywrightError::error(format!("encoding {} result: {error}", C::META.method)).into()
    })
}

/// What a run's unfinished calls hold now, against [`MAX_CALLS_IN_FLIGHT`] and
/// [`MAX_REQUEST_BYTES_IN_FLIGHT`]: admitted in `Dispatcher::call`, given back from whichever
/// thread ends the call's task.
#[derive(Default)]
struct InFlight(Mutex<Load>);

#[derive(Default)]
struct Load {
    calls: usize,
    bytes: usize,
}

/// One admitted call's share of [`InFlight`], given back when it drops: when the call's task
/// ends, whether it replied, the run ended under it, or the runtime dropped it.
struct Admitted {
    in_flight: Arc<InFlight>,
    bytes: usize,
}

impl InFlight {
    /// Takes a share for a call whose message is `bytes` long, or refuses it with the cap it
    /// would pass. Neo-only, so the message names Neo: Playwright's server has no such cap.
    fn admit(self: &Arc<Self>, bytes: usize) -> Result<Admitted, PlaywrightError> {
        let Ok(mut load) = self.0.lock() else {
            return Err(PlaywrightError::error(
                "neo-runtime: the run's in-flight calls cannot be counted",
            ));
        };
        if load.calls >= MAX_CALLS_IN_FLIGHT {
            return Err(PlaywrightError::error(format!(
                "neo-runtime: a run may have at most {MAX_CALLS_IN_FLIGHT} browser calls in \
                 flight; await one before starting another"
            )));
        }
        if load.bytes.saturating_add(bytes) > MAX_REQUEST_BYTES_IN_FLIGHT {
            return Err(PlaywrightError::error(format!(
                "neo-runtime: a run's browser calls in flight may hold at most \
                 {MAX_REQUEST_BYTES_IN_FLIGHT} bytes of requests; this one is {bytes} bytes and \
                 {} are held",
                load.bytes
            )));
        }
        load.calls += 1;
        load.bytes += bytes;
        Ok(Admitted {
            in_flight: Arc::clone(self),
            bytes,
        })
    }
}

impl Drop for Admitted {
    fn drop(&mut self) {
        if let Ok(mut load) = self.in_flight.0.lock() {
            load.calls -= 1;
            load.bytes -= self.bytes;
        }
    }
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

    use neo_runtime_spec::injected::AriaSnapshotMode;
    use neo_runtime_spec::serve::FrameWaitForSelectorParamsState;

    use super::*;
    use neo_runtime_spec::injected::Point;

    use crate::click::tests::FakeTab;
    use crate::deadline::Deadline;
    use crate::fake_browser::{FakeBrowser, PAGE, Replies};
    use crate::forms::SelectOption;
    use crate::page::TabInputs;
    use crate::progress::{self, ErrorName};
    use crate::run::Delivery;
    use crate::run::PageId;
    use tokio::runtime::Handle;
    use tokio_util::task::TaskTracker;

    #[tokio::test(flavor = "multi_thread")]
    async fn tests_that_an_attached_frame_call_starts_when_first_polled_after_run_end() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let tasks = TaskTracker::new();
        let deadline = Deadline::new(Duration::from_secs(5), CancellationToken::new());
        let run = RunScope::new(deadline, tasks.clone());
        let (tabs, _) = Tabs::open(
            browser.cdp().await,
            Handle::current(),
            TabInputs::default(),
            tasks,
            &PageId::new(PAGE.to_string()),
        )
        .await
        .unwrap();
        let (page, scope) = tabs.resolve(&format!("frame@{PAGE}")).unwrap();
        let entry = Arc::new(CallEntry::new(CallStart {
            call_id: 1,
            guid: format!("frame@{PAGE}"),
            method: "click".to_string(),
            title: Some("Click"),
            params: json!({"selector":"#nothing"}),
            location: None,
            input: true,
        }));
        let trace = Trace::default();
        trace.push(Arc::clone(&entry));
        let progress = Progress::new(Arc::clone(&entry), 0.0, run.clone(), page.closed.clone());
        let route = route_frame("click", json!({"selector":"#nothing"}))
            .route
            .unwrap_or_else(|error| panic!("{error:?}"));
        let (reply, _receiver) = oneshot::channel();
        // This future is deliberately not polled until the run has ended. No scheduler
        // interleaving can let the call start early and hide the regression.
        let call = serve_call(
            tabs,
            Some((page, scope)),
            route,
            progress,
            CallReply {
                entry,
                trace: trace.clone(),
                id: json!(1),
                reply,
            },
        );
        run.end(RunEnd::Settled);
        call.await;
        let (records, _) = trace.take(RunEnd::Settled);
        assert!(
            records[0]
                .log
                .iter()
                .any(|line| line.contains("waiting for #nothing")),
            "attached call never entered click policy: {records:?}"
        );
        assert_eq!(
            records[0].result,
            CallResult::Unfinished {
                reason: RunEnd::Settled
            },
            "the ended run still cancels the click"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tests_that_closing_pages_share_one_finish_budget() {
        let browser = FakeBrowser::start(Replies::default()).await;
        browser.page.lock().unwrap().tabs.push(json!({
            "targetId":"other", "url":"about:blank", "windowId":1,
            "liveBrowserContextId":"profile", "isActive":false,
        }));
        let tasks = TaskTracker::new();
        let run = RunScope::new(
            Deadline::new(Duration::from_secs(10), CancellationToken::new()),
            tasks.clone(),
        );
        let (tabs, _) = Tabs::open(
            browser.cdp().await,
            Handle::current(),
            TabInputs::default(),
            tasks,
            &PageId::new(PAGE.to_string()),
        )
        .await
        .unwrap();
        tabs.pages().await.unwrap();
        for target in [PAGE, "other"] {
            tabs.resolve(&format!("page@{target}"))
                .unwrap()
                .0
                .request_close();
        }
        browser.page.lock().unwrap().tab_info_delay = Some(Duration::from_secs(3));
        let dispatcher = Dispatcher::new(tabs, run);
        let started = Instant::now();
        dispatcher.finish(RunEnd::Settled).await;
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "each closing page received another finish budget: {:?}",
            started.elapsed()
        );
    }

    fn route_frame(method: &str, params: Value) -> Routed {
        route(ObjectKind::Frame, method, Some(params))
    }

    fn error_of(routed: Routed) -> PlaywrightError {
        match routed.route {
            Ok(_) => panic!("expected the call to be refused"),
            Err(error) => error,
        }
    }

    /// The frame route inside a routed Frame message.
    fn frame_route(route: Result<ObjectCall, PlaywrightError>) -> Result<Route, PlaywrightError> {
        route.map(|call| match call {
            ObjectCall::Frame(route) => route,
            ObjectCall::Context(_)
            | ObjectCall::Page(_)
            | ObjectCall::Device(_)
            | ObjectCall::Network(_) => {
                panic!("a Frame message routed to another object")
            }
        })
    }

    #[test]
    fn tests_that_request_and_response_guids_route_to_their_commands() {
        let hop = HopId {
            request: "LOADER".to_string(),
            hop: 1,
        };
        let routed = route(
            ObjectKind::Network(NetworkObject::Response, hop.clone()),
            "body",
            None,
        );
        assert_eq!(
            routed.meta.and_then(|meta| meta.title),
            Some("Get response body")
        );
        let Ok(ObjectCall::Network(NetworkCall::Response(routed_hop, ResponseCall::Body(_)))) =
            routed.route
        else {
            panic!("Response.body routes to the response's hop");
        };
        assert_eq!(routed_hop, hop);
        let routed = route(
            ObjectKind::Network(NetworkObject::Request, hop.clone()),
            "rawRequestHeaders",
            None,
        );
        assert!(matches!(
            routed.route,
            Ok(ObjectCall::Network(NetworkCall::Request(
                _,
                RequestCall::RawRequestHeaders(_)
            )))
        ));
        for (kind, method) in [
            (NetworkObject::Request, "body"),
            (NetworkObject::Response, "response"),
        ] {
            let error = error_of(route(ObjectKind::Network(kind, hop.clone()), method, None));
            assert_eq!(error.name, ErrorName::Error, "{method} is not served there");
        }
    }

    fn forms_params(method: &str, selector: &str) -> Value {
        match method {
            "fill" => {
                json!({ "selector": selector, "strict": true, "value": "hello", "force": true })
            }
            "press" => {
                json!({ "selector": selector, "strict": true, "key": "Shift+Enter", "delay": 25.0, "noWaitAfter": true })
            }
            "inputValue" => json!({ "selector": selector, "strict": true }),
            "check" | "uncheck" => {
                json!({ "selector": selector, "strict": true, "force": true, "scroll": "none", "position": { "x": 2, "y": 3 }, "trial": true })
            }
            "selectOption" => {
                json!({ "selector": selector, "strict": true, "elements": [], "options": [{ "valueOrLabel": "one", "value": "1", "label": "One", "index": 0 }], "force": true })
            }
            _ => panic!("unknown test method"),
        }
    }

    const FORMS: [&str; 6] = [
        "fill",
        "press",
        "inputValue",
        "check",
        "uncheck",
        "selectOption",
    ];

    #[test]
    fn tests_that_each_forms_command_decodes_routes_and_retains_its_options() {
        for method in FORMS {
            let routed = route_frame(method, forms_params(method, "input"));
            let meta = routed.meta.expect("served method metadata");
            assert_eq!(meta.has(MethodFlag::Input), method != "inputValue");
            assert!(meta.title.is_some());
            match frame_route(routed.route).expect("served form route") {
                Route::FrameFill(options) => {
                    assert_eq!(
                        options,
                        FillOptions {
                            selector: "input".into(),
                            strict: true,
                            value: "hello".into(),
                            force: true
                        }
                    );
                }
                Route::FramePress(options) => {
                    assert_eq!(
                        options,
                        PressOptions {
                            selector: "input".into(),
                            strict: true,
                            keys: KeyInput::Press("Shift+Enter".into()),
                            delay: Some(25.0),
                            wait_after: false
                        }
                    );
                }
                Route::FrameInputValue(options) => {
                    assert_eq!(
                        options,
                        InputValueOptions {
                            selector: "input".into(),
                            strict: true
                        }
                    );
                }
                Route::FrameSetChecked(options) => {
                    assert_eq!(options.checked, method == "check");
                    assert_eq!(options.click.selector, "input");
                    assert!(options.click.strict && options.click.force && options.click.trial);
                    assert!(!options.click.scroll);
                    assert_eq!(options.click.position, Some(Point { x: 2.0, y: 3.0 }));
                }
                Route::FrameSelectOption(options) => {
                    assert!(options.strict && options.force);
                    assert_eq!(options.selector, "input");
                    assert_eq!(
                        options.options,
                        [SelectOption {
                            value_or_label: Some("one".into()),
                            value: Some("1".into()),
                            label: Some("One".into()),
                            index: Some(0)
                        }]
                    );
                }
                Route::FramePointer(_) => panic!("a form is not a pointer action"),
                Route::FrameGoto(_)
                | Route::FrameTitle
                | Route::FrameTextContent(_)
                | Route::FrameInnerText(_)
                | Route::FrameGetAttribute(_)
                | Route::FrameQueryCount(_)
                | Route::FrameAllTextContents(_)
                | Route::FrameAllInnerTexts(_)
                | Route::FrameInnerHtml(_)
                | Route::FrameIsVisible(_)
                | Route::FrameIsHidden(_)
                | Route::FrameIsChecked(_)
                | Route::FrameIsEnabled(_)
                | Route::FrameIsDisabled(_)
                | Route::FrameIsEditable(_)
                | Route::FrameAriaSnapshot(_)
                | Route::FrameWaitForSelector(_)
                | Route::FrameWaitForUrl(_)
                | Route::FrameWaitForLoadState(_) => {
                    panic!("a form is not a navigation, read or wait")
                }
            }
        }
    }

    #[test]
    fn tests_that_all_forms_route_frames_and_refuse_malformed_params() {
        for method in FORMS {
            let routed = route_frame(
                method,
                forms_params(method, "iframe >> internal:control=enter-frame >> input"),
            );
            assert!(routed.route.is_ok());
            let mut params = forms_params(method, "input");
            params["selector"] = json!(5);
            let error = error_of(route_frame(method, params));
            assert!(
                error
                    .message
                    .starts_with(&format!("invalid params for Frame.{method}: ")),
                "{error:?}"
            );
        }
        let mut params = forms_params("selectOption", "select");
        params["elements"] = json!([{ "guid": "element@1" }]);
        let routed = route_frame("selectOption", params);
        assert_eq!(
            routed.meta.and_then(|meta| meta.title),
            Some("Select option")
        );
        assert_eq!(
            error_of(routed),
            PlaywrightError::error(ELEMENTS_UNSUPPORTED)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_press_is_served_with_a_void_reply() {
        let tab = FakeTab::ready();
        let progress = Progress::for_test(1000.0);
        let route = frame_route(route_frame("press", forms_params("press", "input")).route)
            .expect("routed");
        let result = serve_route(&tab, route, &progress).await;
        assert_eq!(result, Ok(None));
        assert_eq!(
            progress.delivery().expect("test cell").delivery(),
            Delivery::Done
        );
        assert_eq!(
            encode_reply(&json!(7), Ok(None), Some(&progress.folded_log())),
            json!({ "id": 7 }).to_string()
        );
    }

    #[test]
    fn tests_that_forms_success_results_have_the_playwright_shapes() {
        assert_eq!(
            encode_result::<FrameInputValueParams>(&FrameInputValueResult { value: "x".into() })
                .expect("encodes"),
            Some(json!({ "value": "x" }))
        );
        assert_eq!(
            encode_result::<FrameSelectOptionParams>(&FrameSelectOptionResult {
                values: vec!["1".into(), "2".into()]
            })
            .expect("encodes"),
            Some(json!({ "values": ["1", "2"] }))
        );
        assert_eq!(
            encode_result::<FrameFillParams>(&FrameFillResult {}).expect("void"),
            None
        );
        assert_eq!(
            encode_result::<FramePressParams>(&FramePressResult {}).expect("void"),
            None
        );
        assert_eq!(
            encode_result::<FrameCheckParams>(&FrameCheckResult {}).expect("void"),
            None
        );
        assert_eq!(
            encode_result::<FrameUncheckParams>(&FrameUncheckResult {}).expect("void"),
            None
        );
    }

    #[test]
    fn a_frame_click_routes_with_its_options_and_title() {
        let routed = route_frame("click", json!({ "selector": "#go", "strict": true }));
        assert_eq!(routed.meta.and_then(|meta| meta.title), Some("Click"));
        let Ok(ObjectCall::Frame(Route::FramePointer(options))) = routed.route else {
            panic!("a click on the run's frame routes to Frame.click");
        };
        assert_eq!((options.selector.as_str(), options.strict), ("#go", true));
    }

    #[test]
    fn tests_that_page_device_commands_route_to_the_devices_with_a_delivery_cell() {
        for (method, params, title) in [
            (
                "keyboardDown",
                json!({ "key": "Shift" }),
                "Key down \"{key}\"",
            ),
            (
                "keyboardType",
                json!({ "text": "hi", "delay": 5 }),
                "Type \"{text}\"",
            ),
            (
                "mouseMove",
                json!({ "x": 1, "y": 2, "steps": 3 }),
                "Mouse move",
            ),
            (
                "mouseClick",
                json!({ "x": 1, "y": 2, "clickCount": 2 }),
                "Click",
            ),
            (
                "mouseWheel",
                json!({ "deltaX": 0, "deltaY": 9 }),
                "Mouse wheel",
            ),
            ("touchscreenTap", json!({ "x": 1, "y": 2 }), "Tap"),
        ] {
            let routed = route(ObjectKind::Page, method, Some(params));
            assert_eq!(
                routed.meta.and_then(|meta| meta.title),
                Some(title),
                "{method}"
            );
            assert!(
                routed.meta.is_some_and(|meta| meta.has(MethodFlag::Input)),
                "{method} is an input method"
            );
            assert!(
                matches!(routed.route, Ok(ObjectCall::Device(_))),
                "{method} routes to the devices"
            );
        }
        let routed = route(
            ObjectKind::Page,
            "mouseMove",
            Some(json!({ "x": 1, "y": 2, "steps": 1001 })),
        );
        assert_eq!(
            error_of(routed),
            PlaywrightError::error("Neo does not support steps above 1000")
        );
        assert!(matches!(
            route(ObjectKind::Page, "bringToFront", Some(json!({}))).route,
            Ok(ObjectCall::Page(PageCall::BringToFront(_)))
        ));
    }

    #[test]
    fn tests_that_dblclick_hover_and_tap_route_to_the_pointer_policy() {
        for (method, title, name) in [
            ("dblclick", "Double click", "dblclick"),
            ("hover", "Hover", "hover"),
            ("tap", "Tap", "tap"),
        ] {
            let routed = route_frame(method, json!({ "selector": "#go", "noWaitAfter": true }));
            assert_eq!(routed.meta.and_then(|meta| meta.title), Some(title));
            assert!(
                routed.meta.is_some_and(|meta| meta.has(MethodFlag::Input)),
                "{method} is an input method with a delivery cell"
            );
            let Ok(ObjectCall::Frame(Route::FramePointer(options))) = routed.route else {
                panic!("{method} routes to the pointer policy");
            };
            assert_eq!(options.action.name(), name);
            assert!(!options.wait_after, "{method} waits for no navigation");
        }
    }

    #[test]
    fn tests_that_navigation_and_title_are_served() {
        for (method, params, title) in [
            ("goto", json!({ "url": "https://example.test" }), "Navigate"),
            ("title", json!({}), "Get page title"),
        ] {
            let routed = route_frame(method, params);
            assert_eq!(routed.meta.and_then(|meta| meta.title), Some(title));
            assert!(routed.route.is_ok(), "{method} must be served");
        }
    }

    #[test]
    fn tests_that_waits_route_without_an_input_flag() {
        for (method, params, title) in [
            (
                "waitForSelector",
                json!({"selector":"#item","strict":true,"omitReturnValue":true}),
                "Wait for selector",
            ),
            (
                "neo:waitForURL",
                json!({"url":{"glob":"https://a.test/"}}),
                "Wait for URL",
            ),
            ("neo:waitForLoadState", json!({}), "Wait for load state"),
        ] {
            let routed = route_frame(method, params);
            let meta = routed.meta.expect("wait metadata");
            assert_eq!(meta.title, Some(title));
            assert_eq!(meta.interface, "Frame");
            assert!(!meta.has(MethodFlag::Input));
            assert!(routed.route.is_ok(), "{method}");
        }
        let Ok(ObjectCall::Frame(Route::FrameWaitForSelector(options))) = route_frame(
            "waitForSelector",
            json!({"selector":"#item","omitReturnValue":true}),
        )
        .route
        else {
            panic!("selector route");
        };
        assert_eq!(options.state, FrameWaitForSelectorParamsState::Visible);
        assert!(!options.strict);
    }

    #[test]
    fn tests_that_the_verbatim_neo_wait_messages_decode() {
        // The wire examples in med-neo-waits/design.md, with concrete metadata locations.
        let messages = [
            r#"{"id":7,"guid":"frame@T","method":"neo:waitForURL","params":{"url":{"glob":"**/done.html"},"waitUntil":"load"},"metadata":{"location":{"file":"test.js","line":1,"column":1},"timeout":30000}}"#,
            r#"{"id":8,"guid":"frame@T","method":"neo:waitForURL","params":{"url":{"regexSource":"done\\.html$","regexFlags":"i"},"waitUntil":"commit"},"metadata":{}}"#,
            r#"{"id":9,"guid":"frame@T","method":"neo:waitForLoadState","params":{"state":"domcontentloaded"},"metadata":{}}"#,
        ];
        for (index, text) in messages.into_iter().enumerate() {
            let message: Message = serde_json::from_str(text).expect("wire envelope");
            assert_eq!(message.id, json!(index + 7));
            let routed = route(ObjectKind::Frame, &message.method, message.params);
            assert!(routed.route.is_ok(), "wire message {index}");
            assert!(!routed.meta.expect("metadata").has(MethodFlag::Input));
        }
    }

    #[test]
    fn tests_that_wait_refusals_happen_during_routing() {
        for omit in [json!({}), json!({"omitReturnValue":false})] {
            let mut params = json!({"selector":"#item"});
            params
                .as_object_mut()
                .expect("object")
                .extend(omit.as_object().expect("object").clone());
            assert_eq!(
                error_of(route_frame("waitForSelector", params)).name,
                ErrorName::Error
            );
        }
        assert!(route_frame("waitForSelector", json!({"selector":"iframe >> internal:control=enter-frame >> button", "omitReturnValue": true})).route.is_ok());
        for (method, params) in [
            (
                "neo:waitForURL",
                json!({"url":{"glob":""},"waitUntil":"networkidle"}),
            ),
            ("neo:waitForLoadState", json!({"state":"networkidle"})),
        ] {
            assert_eq!(error_of(route_frame(method, params)).name, ErrorName::Error);
        }
        for (method, params) in [
            (
                "waitForSelector",
                json!({"selector":"a","state":"wrong","omitReturnValue":true}),
            ),
            (
                "neo:waitForURL",
                json!({"url":{"glob":""},"waitUntil":"wrong"}),
            ),
            ("neo:waitForURL", json!({"url":{}})),
            (
                "neo:waitForURL",
                json!({"url":{"glob":"","regexSource":"a"}}),
            ),
            ("neo:waitForURL", json!({})),
            ("neo:waitForLoadState", json!({"state":"wrong"})),
        ] {
            assert_eq!(error_of(route_frame(method, params)).name, ErrorName::Error);
        }
        for object in [ObjectKind::Page, ObjectKind::Context, ObjectKind::Unknown] {
            assert!(
                route(object, "neo:waitForLoadState", Some(json!({})))
                    .route
                    .is_err()
            );
        }
    }

    #[test]
    fn tests_that_wait_success_encodes_an_empty_result() {
        let result = encode_result::<FrameWaitForSelectorParams>(&FrameWaitForSelectorResult {
            element: None,
        })
        .expect("wait result");
        let reply: Value =
            serde_json::from_str(&encode_reply(&json!(7), Ok(result.as_ref()), None))
                .expect("reply");
        assert_eq!(reply, json!({"id":7,"result":{}}));
    }

    #[test]
    fn tests_that_navigation_and_title_keep_their_generated_reply_shapes() {
        use neo_runtime_spec::serve::{FrameGotoResult, FrameTitleResult};

        for (result, expected) in [
            (
                serde_json::to_value(FrameGotoResult { response: None }).expect("goto result"),
                json!({"id":7,"result":{}}),
            ),
            (
                serde_json::to_value(FrameTitleResult {
                    value: "Page title".to_string(),
                })
                .expect("title result"),
                json!({"id":7,"result":{"value":"Page title"}}),
            ),
        ] {
            let actual: Value =
                serde_json::from_str(&encode_reply(&json!(7), Ok(Some(&result)), None))
                    .expect("reply");
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn text_content_and_page_snapshots_route_with_their_options_and_titles() {
        let routed = route_frame("textContent", json!({ "selector": "li", "strict": true }));
        assert_eq!(
            routed.meta.and_then(|meta| meta.title),
            Some("Get text content")
        );
        let Ok(ObjectCall::Frame(Route::FrameTextContent(options))) = routed.route else {
            panic!("textContent on the run's frame routes to Frame.textContent");
        };
        assert_eq!(
            options,
            ReadOptions {
                selector: "li".to_string(),
                strict: true,
            }
        );
        let Ok(ObjectCall::Frame(Route::FrameTextContent(options))) =
            route_frame("textContent", json!({ "selector": "li" })).route
        else {
            panic!("page.textContent sends no strict");
        };
        assert!(!options.strict, "page and frame reads are not strict");

        let routed = route_frame("ariaSnapshot", json!({ "mode": "ai", "depth": 3 }));
        assert_eq!(
            routed.meta.and_then(|meta| meta.title),
            Some("Aria snapshot")
        );
        let Ok(ObjectCall::Frame(Route::FrameAriaSnapshot(options))) = routed.route else {
            panic!("ariaSnapshot without a selector routes to Frame.ariaSnapshot");
        };
        assert_eq!(
            options,
            SnapshotOptions {
                selector: None,
                mode: AriaSnapshotMode::Ai,
                depth: Some(3),
                boxes: None,
            }
        );
    }

    #[test]
    fn tests_that_inner_text_and_get_attribute_route_without_input_flags() {
        for strict in [None, Some(true), Some(false)] {
            for method in ["innerText", "getAttribute"] {
                let mut params = json!({ "selector": "li" });
                if method == "getAttribute" {
                    params["name"] = json!("data-kind");
                }
                if let Some(strict) = strict {
                    params["strict"] = json!(strict);
                }
                let routed = route_frame(method, params);
                let meta = routed.meta.expect("served read metadata");
                assert!(!meta.has(MethodFlag::Input), "{method} is a read");
                match frame_route(routed.route).expect("read route") {
                    Route::FrameInnerText(options) => assert_eq!(
                        options,
                        ReadOptions {
                            selector: "li".to_string(),
                            strict: strict.unwrap_or(false),
                        }
                    ),
                    Route::FrameGetAttribute(options) => assert_eq!(
                        options,
                        AttributeOptions {
                            selector: "li".to_string(),
                            strict: strict.unwrap_or(false),
                            name: "data-kind".to_string(),
                        }
                    ),
                    other => panic!("expected a text read, got {other:?}"),
                }
            }
        }
    }

    #[test]
    fn tests_that_text_reads_route_frame_selectors() {
        for method in ["innerText", "getAttribute", "textContent"] {
            assert!(route_frame(method, json!({"selector": "iframe >> internal:control=enter-frame >> li", "name": "title"})).route.is_ok());
        }
    }

    #[test]
    fn tests_that_text_reads_keep_empty_and_missing_wire_values_distinct() {
        use neo_runtime_spec::serve::{FrameGetAttributeResult, FrameInnerTextResult};
        for (result, expected) in [
            (
                encode_result::<FrameInnerTextParams>(&FrameInnerTextResult {
                    value: String::new(),
                }),
                json!({ "id": 7, "result": { "value": "" } }),
            ),
            (
                encode_result::<FrameGetAttributeParams>(&FrameGetAttributeResult {
                    value: Some(String::new()),
                }),
                json!({ "id": 7, "result": { "value": "" } }),
            ),
            (
                encode_result::<FrameGetAttributeParams>(&FrameGetAttributeResult {
                    value: Some("fruit".to_string()),
                }),
                json!({ "id": 7, "result": { "value": "fruit" } }),
            ),
            (
                encode_result::<FrameGetAttributeParams>(&FrameGetAttributeResult { value: None }),
                json!({ "id": 7, "result": {} }),
            ),
        ] {
            let result = result.expect("typed result").expect("reads have a result");
            let reply: Value =
                serde_json::from_str(&encode_reply(&json!(7), Ok(Some(&result)), None))
                    .expect("reply");
            assert_eq!(reply, expected);
        }
    }

    /// Every state read Neo serves: the two that read once and the four that wait for a match.
    const STATE_READS: [&str; 6] = [
        "isVisible",
        "isHidden",
        "isChecked",
        "isEnabled",
        "isDisabled",
        "isEditable",
    ];

    #[test]
    fn tests_that_state_reads_decode_the_selector_and_strict_option_without_input() {
        for method in STATE_READS {
            for strict in [None, Some(false), Some(true)] {
                let mut params = json!({ "selector": "#control" });
                if let Some(strict) = strict {
                    params["strict"] = json!(strict);
                }
                let routed = route_frame(method, params);
                let meta = routed.meta.expect("state read metadata");
                assert!(!meta.has(MethodFlag::Input), "state reads have no delivery");
                let (actual_method, options) = match frame_route(routed.route).expect("state route")
                {
                    Route::FrameIsVisible(options) => ("isVisible", options),
                    Route::FrameIsHidden(options) => ("isHidden", options),
                    Route::FrameIsChecked(options) => ("isChecked", options),
                    Route::FrameIsEnabled(options) => ("isEnabled", options),
                    Route::FrameIsDisabled(options) => ("isDisabled", options),
                    Route::FrameIsEditable(options) => ("isEditable", options),
                    route => panic!("unexpected state route: {route:?}"),
                };
                assert_eq!(actual_method, method);
                assert_eq!(
                    options,
                    StateOptions {
                        selector: "#control".to_string(),
                        strict: strict.unwrap_or(false),
                    }
                );
            }
        }
    }

    #[test]
    fn tests_that_state_reads_route_frames_and_refuse_malformed_params() {
        for method in STATE_READS {
            assert!(
                route_frame(
                    method,
                    json!({ "selector": "iframe >> internal:control=enter-frame >> #control" })
                )
                .route
                .is_ok()
            );
            assert!(route_frame(method, json!({ "selector": 5 })).route.is_err());
            assert!(
                route_frame(method, json!({ "selector": "#control", "strict": 1 }))
                    .route
                    .is_err()
            );
        }
    }

    #[test]
    fn tests_that_locator_snapshots_route_and_unserved_frame_reads_reject() {
        assert!(
            route_frame(
                "textContent",
                json!({ "selector": "iframe >> internal:control=enter-frame >> li" }),
            )
            .route
            .is_ok()
        );
        let routed = route_frame("ariaSnapshot", json!({ "selector": "#list", "mode": "ai" }));
        assert_eq!(
            routed.meta.and_then(|meta| meta.title),
            Some("Aria snapshot")
        );
        let Ok(ObjectCall::Frame(Route::FrameAriaSnapshot(options))) = routed.route else {
            panic!("a locator snapshot routes to Frame.ariaSnapshot");
        };
        assert_eq!(options.selector.as_deref(), Some("#list"));
        assert_eq!(options.mode, AriaSnapshotMode::Ai);
    }

    /// Playwright's dispatcher sends `{id, result}` for every command with `returns`; the
    /// client reads an absent `value` as `null`.
    #[test]
    fn reads_reply_with_their_generated_result_shapes() {
        use neo_runtime_spec::serve::{FrameAriaSnapshotResult, FrameTextContentResult};

        for (result, expected) in [
            (
                serde_json::to_value(FrameTextContentResult {
                    value: Some("Apples".to_string()),
                })
                .expect("text result"),
                json!({ "id": 7, "result": { "value": "Apples" } }),
            ),
            (
                serde_json::to_value(FrameTextContentResult {
                    value: Some(String::new()),
                })
                .expect("text result"),
                json!({ "id": 7, "result": { "value": "" } }),
            ),
            (
                serde_json::to_value(FrameTextContentResult { value: None }).expect("text"),
                json!({ "id": 7, "result": {} }),
            ),
            (
                serde_json::to_value(FrameAriaSnapshotResult {
                    snapshot: "- button \"Go\" [ref=e2]".to_string(),
                })
                .expect("snapshot result"),
                json!({ "id": 7, "result": { "snapshot": "- button \"Go\" [ref=e2]" } }),
            ),
        ] {
            let actual: Value =
                serde_json::from_str(&encode_reply(&json!(7), Ok(Some(&result)), None))
                    .expect("reply");
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn a_click_option_neo_does_not_serve_is_refused_before_any_work() {
        let routed = route_frame("click", json!({ "selector": "#go", "clickCount": 1001 }));
        assert_eq!(routed.meta.and_then(|meta| meta.title), Some("Click"));
        assert_eq!(
            error_of(routed),
            PlaywrightError::error("Neo does not support a clickCount above 1000")
        );
    }

    #[test]
    fn unserved_methods_and_objects_reply_as_playwrights_dispatcher_would() {
        let cases = [
            (
                route_frame("expect", json!({ "selector": "#go" })),
                None,
                PlaywrightError::error("Neo does not serve Frame.expect"),
            ),
            (
                route_frame(
                    "goto",
                    json!({ "url": "https://example.test", "waitUntil": "networkidle" }),
                ),
                Some("Navigate"),
                PlaywrightError::error("Neo does not support waitUntil: \"networkidle\" yet"),
            ),
            (
                route(ObjectKind::Page, "click", None),
                None,
                PlaywrightError::error("Neo does not serve Page.click"),
            ),
            (
                route(ObjectKind::Unknown, "click", None),
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
    fn calls_past_the_in_flight_caps_are_refused_until_one_ends() {
        let in_flight = Arc::new(InFlight::default());
        let mut admitted: Vec<Admitted> = (0..MAX_CALLS_IN_FLIGHT)
            .map(|_| in_flight.admit(100).expect("under the call cap"))
            .collect();
        assert_eq!(
            in_flight.admit(100).err(),
            Some(PlaywrightError::error(
                "neo-runtime: a run may have at most 64 browser calls in flight; await one before \
                 starting another"
            ))
        );
        admitted.pop();
        assert!(
            in_flight.admit(100).is_ok(),
            "an ended call gives its share back"
        );
        drop(admitted);

        let five = in_flight.admit(5 << 20).expect("5 MiB of 8");
        assert_eq!(
            in_flight.admit(4 << 20).err(),
            Some(PlaywrightError::error(
                "neo-runtime: a run's browser calls in flight may hold at most 8388608 bytes of \
                 requests; this one is 4194304 bytes and 5242880 are held"
            ))
        );
        let three = in_flight.admit(3 << 20).expect("exactly 8 MiB");
        drop((five, three));
        assert!(
            in_flight.admit(MAX_REQUEST_BYTES_IN_FLIGHT + 1).is_err(),
            "one request alone past the cap"
        );
        assert!(
            in_flight.admit(MAX_REQUEST_BYTES_IN_FLIGHT).is_ok(),
            "every share was given back"
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

    /// The replies Neo sends for a click that succeeds, one the page refuses, one that times out
    /// and one whose tab closed, built from Neo's own errors, folding and envelope, and pinned
    /// in `tests/fixtures/replies.json`. neo-sdk's `connection.test.ts` rebuilds each into the
    /// error the script sees, so both halves of the reply path read one file.
    /// Set `NEO_RUNTIME_WRITE_FIXTURES=1` to rewrite the file after a deliberate change.
    #[test]
    fn tests_that_replies_match_the_committed_fixture() {
        let id = json!(1);
        let waiting = "waiting for locator('#go')";
        let mut replies = BTreeMap::new();
        replies.insert("ok", encode_reply(&id, Ok(None), None));
        // A strict mode violation's message is the page's own, passed through.
        let strict = PlaywrightError::error(
            "Error: strict mode violation: locator('#go') resolved to 2 elements",
        );
        let log = progress::compress_call_log(&[waiting]);
        replies.insert("error", encode_reply(&id, Err(&strict), Some(&log)));
        let log = progress::compress_call_log(&[
            waiting,
            "  locator resolved to <button id=\"go\" disabled=\"\">Go</button>",
            "attempting click action",
            "  element is not enabled",
        ]);
        let timeout = PlaywrightError::timeout(500.0);
        replies.insert("timeout", encode_reply(&id, Err(&timeout), Some(&log)));
        replies.insert(
            "targetClosed",
            encode_reply(&id, Err(&PlaywrightError::target_closed()), Some(&[])),
        );
        let value =
            encode_result::<FrameInputValueParams>(&FrameInputValueResult { value: "x".into() })
                .expect("value encodes");
        replies.insert("inputValueOk", encode_reply(&id, Ok(value.as_ref()), None));
        let values = encode_result::<FrameSelectOptionParams>(&FrameSelectOptionResult {
            values: vec!["a".into()],
        })
        .expect("values encode");
        replies.insert(
            "selectOptionOk",
            encode_reply(&id, Ok(values.as_ref()), None),
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

    fn all_text_contents_params(selector: &str) -> Value {
        json!({
            "selector": selector,
            "expression": lists::ALL_TEXT_CONTENTS_EXPRESSION,
            "isFunction": true,
            "arg": {"value": {"v": "undefined"}, "handles": []},
        })
    }

    #[test]
    fn tests_that_collection_commands_decode_and_route_their_selectors() {
        let Ok(ObjectCall::Frame(Route::FrameQueryCount(selector))) =
            route_frame("queryCount", json!({"selector": ".item"})).route
        else {
            panic!("queryCount routes");
        };
        assert_eq!(selector, ".item");
        let Ok(ObjectCall::Frame(Route::FrameAllTextContents(selector))) =
            route_frame("evalOnSelectorAll", all_text_contents_params(".item")).route
        else {
            panic!("allTextContents routes");
        };
        assert_eq!(selector, ".item");
        let mut inner_texts = all_text_contents_params(".fruit");
        inner_texts["expression"] = json!(lists::ALL_INNER_TEXTS_EXPRESSION);
        let Ok(ObjectCall::Frame(Route::FrameAllInnerTexts(selector))) =
            route_frame("evalOnSelectorAll", inner_texts).route
        else {
            panic!("allInnerTexts routes");
        };
        assert_eq!(selector, ".fruit");
    }

    #[test]
    fn tests_that_inner_html_decodes_its_selector_and_strictness_as_a_read() {
        let routed = route_frame("innerHTML", json!({"selector": "#list", "strict": true}));
        let meta = routed.meta.expect("innerHTML metadata");
        assert!(!meta.has(MethodFlag::Input), "a read has no delivery");
        let Ok(ObjectCall::Frame(Route::FrameInnerHtml(options))) = routed.route else {
            panic!("innerHTML routes");
        };
        assert_eq!(
            options,
            ReadOptions {
                selector: "#list".to_string(),
                strict: true,
            }
        );
        assert!(
            route_frame("innerHTML", json!({"selector": 1}))
                .route
                .is_err()
        );
    }

    #[test]
    fn tests_that_collection_eval_refuses_other_expressions_and_non_functions() {
        let mut other_expression = all_text_contents_params(".item");
        other_expression["expression"] = json!("ee => ee.length");
        let mut not_a_function = all_text_contents_params(".item");
        not_a_function["isFunction"] = json!(false);
        let mut absent_function = all_text_contents_params(".item");
        absent_function
            .as_object_mut()
            .expect("params")
            .remove("isFunction");
        let mut inner_texts_not_a_function = all_text_contents_params(".item");
        inner_texts_not_a_function["expression"] = json!(lists::ALL_INNER_TEXTS_EXPRESSION);
        inner_texts_not_a_function["isFunction"] = json!(false);
        // Playwright's TypeScript body, which is not JavaScript, is not Neo's served expression.
        let mut typescript_body = all_text_contents_params(".item");
        typescript_body["expression"] = json!("ee => ee.map(e => (e as HTMLElement).innerText)");
        for params in [
            other_expression,
            not_a_function,
            absent_function,
            inner_texts_not_a_function,
            typescript_body,
        ] {
            assert_eq!(
                error_of(route_frame("evalOnSelectorAll", params)),
                PlaywrightError::error(EVAL_ON_SELECTOR_ALL_UNSERVED)
            );
        }
    }

    #[test]
    fn tests_that_collection_commands_route_frames_and_restrict_evaluation() {
        let selector = "iframe >> internal:control=enter-frame >> .item";
        assert!(
            route_frame("queryCount", json!({"selector": selector}))
                .route
                .is_ok()
        );
        assert!(
            route_frame("evalOnSelectorAll", all_text_contents_params(selector))
                .route
                .is_ok()
        );
        let mut params = all_text_contents_params(selector);
        params["expression"] = json!("ee => ee.length");
        assert_eq!(
            error_of(route_frame("evalOnSelectorAll", params)),
            PlaywrightError::error(EVAL_ON_SELECTOR_ALL_UNSERVED)
        );
    }

    #[test]
    fn tests_that_serialized_arguments_accept_no_handles() {
        assert!(
            FrameCall::decode("evalOnSelectorAll", Some(all_text_contents_params(".item"))).is_ok()
        );
        for handle in [json!({"guid": "handle@1"}), json!("handle@1"), json!(null)] {
            let mut params = all_text_contents_params(".item");
            params["arg"]["handles"] = json!([handle]);
            assert!(FrameCall::decode("evalOnSelectorAll", Some(params)).is_err());
        }
    }

    #[test]
    fn tests_that_collection_replies_have_playwrights_generated_shapes() {
        use neo_runtime_spec::serve::{FrameEvalOnSelectorAllResult, FrameQueryCountResult};

        for (result, expected) in [
            (
                encode_result::<FrameQueryCountParams>(&FrameQueryCountResult { value: 3 })
                    .expect("count encodes"),
                json!({"id": 7, "result": {"value": 3}}),
            ),
            (
                encode_result::<FrameEvalOnSelectorAllParams>(&FrameEvalOnSelectorAllResult {
                    value: lists::serialize_strings(vec!["x".to_string()]),
                })
                .expect("texts encode"),
                json!({"id": 7, "result": {"value": {"a": [{"s": "x"}], "id": 1}}}),
            ),
        ] {
            let reply: Value =
                serde_json::from_str(&encode_reply(&json!(7), Ok(result.as_ref()), None))
                    .expect("reply");
            assert_eq!(reply, expected);
        }
    }
}
