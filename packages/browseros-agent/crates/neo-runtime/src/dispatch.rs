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
//!         ├─ Aborts += message id               `__neo_runtime.abort(id, reason)` reaches it
//!         └─ run.tasks.spawn(serve)             runs while JS spins, outlives the realm
//!              ├─ events.ready(page)            what the page's subscriptions switched on
//!              ├─ click::click(&page, &options, &progress)   prepares the tab first
//!              ├─ entry.finish(…)
//!              └─ events.barrier(); reply ({id} | {id, error:{error:{message,name}}, log}, W)
//!  run end: Dispatcher::finish(reason)
//!    record the reason → cancel `ended` → mark the run's input ended on every tab
//!    → close the events, answer dialogs left open → start each tab's input release (a task)
//!    → wait for the run's tasks, releases included (≤ RECORDS_WAIT)
//!    → hide the run's highlights (≤ HIGHLIGHTS_WAIT) → records
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

use futures_util::future;
use neo_runtime_spec::injected::ReadableElementState;
use neo_runtime_spec::serve::{
    ArtifactCall, BrowserContextCall, DialogCall, FrameAriaSnapshotJSONParams,
    FrameAriaSnapshotParams, FrameCall, FrameCheckParams, FrameCheckResult, FrameClickParams,
    FrameContentParams, FrameDblclickParams, FrameDragAndDropParams, FrameDragAndDropResult,
    FrameDropParams, FrameDropResult, FrameFillParams, FrameFillResult, FrameGetAttributeParams,
    FrameGotoParams, FrameHoverParams, FrameInnerHTMLParams, FrameInnerTextParams,
    FrameInputValueParams, FrameInputValueResult, FrameIsCheckedParams, FrameIsCheckedResult,
    FrameIsDisabledParams, FrameIsDisabledResult, FrameIsEditableParams, FrameIsEditableResult,
    FrameIsEnabledParams, FrameIsEnabledResult, FrameIsHiddenParams, FrameIsHiddenResult,
    FrameIsVisibleParams, FrameIsVisibleResult, FramePressParams, FramePressResult,
    FrameQueryCountParams, FrameResolveSelectorParams, FrameSelectOptionParams,
    FrameSelectOptionResult, FrameSetContentParams, FrameSetContentResult,
    FrameSetInputFilesParams, FrameSetInputFilesResult, FrameTapParams, FrameTextContentParams,
    FrameTitleParams, FrameTypeParams, FrameTypeResult, FrameUncheckParams, FrameUncheckResult,
    FrameWaitForSelectorParams, FrameWaitForSelectorResult, FrameWaitForTimeoutParams,
    FrameWaitForTimeoutResult, Metadata, PageCall, RequestCall, ResponseCall, RouteCall,
};
use neo_runtime_spec::{CallDecodeError, Command, MethodFlag, MethodMeta};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::time;
use tokio::time::Instant;
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::debug;

use crate::abort::{Aborts, Registration};
use crate::bindings::{self, BindingRoute, Outcome};
use crate::capture::{
    self, Capture, ELEMENT_HANDLE_SCREENSHOT_META, ElementScreenshotParams, NEO_SCREENSHOT_META,
    NeoScreenshotParams,
};
use crate::check::{self, CheckOptions};
use crate::click::{self, PointerOptions};
use crate::content::{self, SetContentOptions, WriteOrder, WriteTurn};
use crate::devices::{self, DeviceCall};
use crate::dialogs;
use crate::downloads;
use crate::drag::{self, DragOptions};
use crate::drop_on;
use crate::element_ops::{
    self, BOUNDING_BOX_META, DispatchOptions, ElementTarget, NeoElementParams, NeoSelectTextParams,
    SCROLL_INTO_VIEW_META, SELECT_TEXT_META,
};
use crate::evaluate::{self, EvalCall};
use crate::events::{ALL_PAGES, EventHub};
use crate::fill::{self, FillOptions, InputValueOptions, SelectOptions};
use crate::forms::FormTab;
use crate::frame_element;
use crate::frames::FrameScope;
use crate::highlight::{self, HighlightOptions};
use crate::history::{
    self, Cursors, Ending, FinishNavigationOptions, FrameCursor, NeoFinishNavigationParams,
    NeoNavigationResult, NeoNextNavigationParams, NeoReleaseNavigationParams,
    NeoWaitForNavigationParams, WaitForNavigationOptions,
};
use crate::init_scripts::{
    self, CONTEXT_ADD_INIT_SCRIPT_META, DISPOSABLE_DISPOSE_META, InitRoute,
    PAGE_ADD_INIT_SCRIPT_META,
};
use crate::keyboard::KeyboardTab;
use crate::lists;
use crate::locator_handlers::{self, Resolving};
use crate::navigate::{self, GotoOptions};
use crate::network::{HopId, ObjectKind as NetworkObject};
use crate::page::{DialogAnswer, FrameTab, PageSession};
use crate::press::{self, PressOptions};
use crate::progress::{
    CallEntry, CallError, CallStart, PlaywrightError, Progress, RunScope, Trace,
};
use crate::reads::{self, AttributeOptions, ReadOptions, SnapshotOptions};
use crate::responses::{self, NetworkCall};
use crate::routes;
use crate::run::{CallLocation, CallRecord, CallResult, DroppedCalls, RunEnd};
use crate::session_state::StateFile;
use crate::set_input_files;
use crate::states::{self, StateOptions};
use crate::tab_calls;
use crate::tabs::{PageEntry, Tabs};
use crate::waits::{
    self, NeoWaitForLoadStateParams, NeoWaitForUrlParams, WaitForLoadStateOptions,
    WaitForSelectorOptions, WaitForUrlOptions, WaitTab,
};

/// The shared budget for creation settlement, call draining and closing-state reads after
/// the realm ends. Tasks still running keep their state and records read `Unfinished`.
/// Unkept creations close their tabs when their late acknowledgements arrive.
const RECORDS_WAIT: Duration = Duration::from_secs(1);

/// Neo: how long run end may take removing the run's init scripts and ending its bindings in
/// its pages (`init_scripts.rs`, `bindings.rs`), apart from [`RECORDS_WAIT`]: a session it does
/// not reach keeps a script the next run's documents would run, or a binding whose calls never
/// settle. Free for a run that used neither.
pub(crate) const PAGE_END_WAIT: Duration = Duration::from_secs(3);
/// Neo: the budget for removing the run's highlights at its end, after [`RECORDS_WAIT`] and
/// apart from it, so a document that does not answer delays the outcome by at most this and
/// never holds back an input release. An overlay it cannot remove in time stays, as one in a
/// document that navigated away does.
const HIGHLIGHTS_WAIT: Duration = Duration::from_secs(1);

/// The most browser calls one run may have unfinished at once. Each holds its params, its call
/// log and a task in Rust, outside the realm's memory limit, and a script can start calls without
/// awaiting them, so without a cap one run could hold any amount of the server's memory.
/// Playwright has no such cap; a script that awaits its calls, or a `Promise.all` over a page's
/// worth of locators, stays far below it. A URL-predicate wait's finish and release skip it
/// (`ending_cursor`): the wait they end may hold some of it, and the cursor cap bounds them.
const MAX_CALLS_IN_FLIGHT: usize = 64;

/// The most request bytes (each call's message as the SDK sent it) one run's unfinished calls
/// may total, for the same reason: a few calls with huge selectors would otherwise hold several
/// copies of each outside the realm.
const MAX_REQUEST_BYTES_IN_FLIGHT: usize = 8 * 1024 * 1024;

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
// Playwright's client `waitForNavigation` (`@apiCall({ title: 'Wait for navigation' })`). A
// glob or RegExp `url` is one call; a predicate is a cursor the realm steps through
// (`history.rs`), one navigation per `neo:nextNavigation`, closed by `neo:finishNavigation`.
static WAIT_FOR_NAVIGATION_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "neo:waitForNavigation",
    title: Some("Wait for navigation"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[],
};
static NEXT_NAVIGATION_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "neo:nextNavigation",
    title: Some("Wait for navigation"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[],
};
static FINISH_NAVIGATION_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "neo:finishNavigation",
    title: Some("Wait for navigation"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[],
};
static RELEASE_NAVIGATION_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "neo:releaseNavigation",
    title: Some("Wait for navigation"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[],
};
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
    /// The URL-predicate navigation waits open between their calls.
    cursors: Arc<Cursors<FrameCursor>>,
    /// The admitted calls a script's `signal` can still abort, by message id (`abort.rs`).
    aborts: Arc<Aborts>,
    next_call_id: AtomicU64,
    /// The `setContent` writes into each frame, in arrival order (`content.rs`).
    write_order: WriteOrder,
    /// Each page's `localStorage` and `sessionStorage` calls, in arrival order (`web_storage.rs`):
    /// Playwright's server sends them in the order the script made them.
    storage_order: WriteOrder,
    /// Cancels `run.ended` when the last `Dispatcher` drops. A reason `finish` recorded first
    /// wins; otherwise calls read [`RunEnd::Cancelled`].
    _ending: DropGuard,
}

/// What admitted one call ([`Inner::admit`]): its share of the in-flight caps, and the claim of
/// a call that ends calls in flight.
struct Admission {
    share: Option<Admitted>,
    ending: Option<Ending<FrameCursor>>,
    resolving: Option<Resolving>,
}

impl Inner {
    /// Admits `route`: a share of the in-flight caps, or none for a call that ends calls in
    /// flight, with its claim. A navigation's release, a running locator handler's completion
    /// (which frees the action waiting on it) and the context's close: the cap never refuses
    /// them, or a full run could not let go.
    fn admit(
        &self,
        route: &ObjectCall,
        target: Option<&(Arc<PageEntry>, FrameScope)>,
        bytes: usize,
    ) -> Result<Admission, PlaywrightError> {
        let ending = ending_cursor(route).and_then(|id| self.cursors.begin_ending(id));
        let resolving = completed_handler(route).and_then(|uid| {
            let page = target?.0.attached_session()?;
            locator_handlers::begin_resolving(&page, uid)
        });
        let releases = ending.is_some()
            || resolving.is_some()
            || matches!(route, ObjectCall::Context(BrowserContextCall::Close(_)));
        let share = if releases {
            None
        } else {
            Some(self.in_flight.admit(bytes)?)
        };
        Ok(Admission {
            share,
            ending,
            resolving,
        })
    }

    /// A `setContent` call takes its place in its frame's write order as it arrives, and a
    /// storage call its place in its page's storage order.
    fn ordered(&self, guid: &str, mut call: ObjectCall) -> ObjectCall {
        match &mut call {
            ObjectCall::Frame(Route::FrameSetContent(_, turn)) => {
                *turn = Some(self.write_order.turn(guid));
            }
            ObjectCall::Storage(_, turn) => *turn = Some(self.storage_order.turn(guid)),
            _ => {}
        }
        call
    }
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

/// The reply text a call resolves the script's promise with, and the event watermark W the SDK
/// delivers up to before the caller continues (`events.rs`): ready at once for an early failure,
/// or a oneshot the call's task fills. A dropped sender means the run ended under the call;
/// nobody sees that error, because the realm is going away too.
pub(crate) struct PendingReply(PendingInner);

/// A reply's text and its event watermark.
type Reply = (String, u64);

enum PendingInner {
    Ready(Option<Result<Reply, BridgeError>>),
    Waiting(oneshot::Receiver<Reply>),
}

impl PendingReply {
    fn ready(reply: Result<Reply, BridgeError>) -> Self {
        Self(PendingInner::Ready(Some(reply)))
    }
}

impl Future for PendingReply {
    type Output = Result<Reply, BridgeError>;

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
    /// A run's `JSHandle` or `ElementHandle` (`evaluate.rs`), by guid.
    Handle(String),
    /// An init script's or a binding's `Disposable` (`init_scripts.rs`, `bindings.rs`), by guid.
    Disposable(String),
    /// A binding call the realm answers (`bindings.rs`), by guid.
    BindingCall(String),
    /// A JavaScript dialog the run's `dialog` event handed out, by its index on the tab.
    Dialog(u64),
    /// A paused request the run's `route` event handed out (Playwright's `Route`), by its index
    /// on the tab; named apart from this module's call-routing `Route`.
    Intercepted(u64),
    /// A download the run's `download` event handed out, by the browser's guid
    /// (`downloads.rs`). It belongs to the run, not to its tab, which may close first.
    Download(String),
    Unknown,
}

enum ObjectCall {
    Context(BrowserContextCall),
    /// `context.storageState` and `context.setStorageState`, with the `path` the spec's params
    /// lack (`neo_path`).
    StorageState(StateFile, Option<String>),
    Page(PageCall),
    /// `page.keyboard`, `page.mouse` and `page.touchscreen` (`devices.rs`).
    Device(DeviceCall),
    Frame(Route),
    Network(NetworkCall),
    /// An init script command (`init_scripts.rs`).
    Init(InitRoute),
    /// `Page.screenshot`, `Page.pdf` and `neo:screenshot`, with Neo's `path` (`capture.rs`).
    Capture(Capture),
    /// A binding command (`bindings.rs`).
    Binding(BindingRoute),
    Dialog(u64, DialogCall),
    /// A paused request's answer or `route.fetch` (`routes.rs`).
    Intercepted(u64, routes::InterceptedCall),
    Download(String, ArtifactCall),
    /// `page.localStorage` and `page.sessionStorage` (`web_storage.rs`), with the call's place
    /// in its page's storage order, taken as it arrives ([`Inner::ordered`]).
    Storage(PageCall, Option<WriteTurn>),
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
    FrameInnerHtml(ReadOptions),
    FrameAriaSnapshot(SnapshotOptions),
    FrameFill(FillOptions),
    FramePress(PressOptions),
    /// `Frame.type`, served by press's policy.
    FrameType(PressOptions),
    /// `Frame.dragAndDrop`: two pointer actions over Chromium's drag manager.
    FrameDragAndDrop(DragOptions),
    FrameInputValue(InputValueOptions),
    FrameIsVisible(StateOptions),
    FrameIsHidden(StateOptions),
    FrameIsChecked(StateOptions),
    FrameIsEnabled(StateOptions),
    FrameIsDisabled(StateOptions),
    FrameIsEditable(StateOptions),
    FrameSetChecked(CheckOptions),
    FrameSelectOption(SelectOptions),
    /// `Frame.setInputFiles` (`set_input_files.rs`).
    FrameSetInputFiles(FrameSetInputFilesParams),
    /// `Frame.drop`, behind `Locator.drop` (`drop_on.rs`).
    FrameDrop(FrameDropParams),
    FrameWaitForSelector(WaitForSelectorOptions),
    /// `Frame.frameElement` (`frame_element.rs`).
    FrameFrameElement,
    FrameWaitForUrl(WaitForUrlOptions),
    FrameWaitForLoadState(WaitForLoadStateOptions),
    // Document content and timing (content.rs, waits.rs, reads.rs).
    FrameContent(bool),
    /// The options, and the write's place in its frame's order (`Dispatcher::call` gives it).
    FrameSetContent(SetContentOptions, Option<WriteTurn>),
    FrameWaitForTimeout(f64),
    FrameResolveSelector(String),
    FrameAriaSnapshotJson(SnapshotOptions),
    // Element operations (element_ops.rs, highlight.rs).
    FrameFocus(ElementTarget),
    FrameBlur(ElementTarget),
    FrameDispatchEvent(DispatchOptions),
    FrameScrollIntoViewIfNeeded(ElementTarget),
    FrameSelectText(ElementTarget, bool),
    FrameBoundingBox(ElementTarget),
    FrameHighlight(HighlightOptions),
    FrameHideHighlight(HighlightOptions),
    FrameWaitForNavigation(WaitForNavigationOptions),
    FrameNextNavigation(NeoNextNavigationParams),
    FrameFinishNavigation(FinishNavigationOptions),
    FrameReleaseNavigation(NeoReleaseNavigationParams),
    /// Page evaluation and the `JSHandle` commands (on a handle's frame), served by `evaluate.rs`.
    FrameEval(EvalCall),
}

impl Dispatcher {
    pub(crate) fn new(tabs: Tabs, run: RunScope) -> Self {
        Self(Arc::new(Inner {
            tabs,
            _ending: run.ended.clone().drop_guard(),
            run,
            trace: Trace::default(),
            in_flight: Arc::default(),
            cursors: Arc::default(),
            aborts: Arc::default(),
            next_call_id: AtomicU64::new(1),
            write_order: WriteOrder::default(),
            storage_order: WriteOrder::default(),
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
        // A handle's calls run on the frame it belongs to.
        let target = inner.tabs.resolve(&guid).or_else(|| {
            inner
                .run
                .handles
                .frame_guid(&guid)
                .and_then(|frame| inner.tabs.resolve(&frame))
        });
        let object = object_kind(&guid, target.is_some());
        let Routed { meta, route } = route(object, &method, params.clone());
        let route = route.map(|call| inner.ordered(&guid, call));
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
            let admission = inner.admit(&route, target.as_ref(), message.len())?;
            Ok((route, admission))
        });
        let (route, admission) = match admitted {
            Ok(admitted) => admitted,
            Err(error) => {
                entry.finish(error.to_result());
                inner.trace.finished(&entry);
                let watermark = inner.tabs.events().seq();
                return PendingReply::ready(Ok((encode_reply(&id, Err(&error), None), watermark)));
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
        let progress = if matches!(route, ObjectCall::Context(BrowserContextCall::Close(_))) {
            progress.closing_the_context()
        } else {
            progress
        };
        // Registered before the call returns to JS, so the abort listener the SDK adds right
        // after always finds it. The SDK's ids are integers; any other id cannot be aborted.
        let abortable = id
            .as_u64()
            .map(|message_id| inner.aborts.register(message_id, &entry));
        let (reply, receiver) = oneshot::channel();
        let served = serve_call(
            inner.tabs.clone(),
            Arc::clone(&inner.cursors),
            target,
            route,
            progress,
            CallReply {
                entry,
                trace: inner.trace.clone(),
                id,
                reply,
                events: inner.tabs.events().clone(),
                share: admission.share,
                ending: admission.ending,
                resolving: admission.resolving,
                abortable,
            },
        );
        inner.run.tasks.spawn(served);
        PendingReply(PendingInner::Waiting(receiver))
    }

    /// `__neo_runtime.abort`: the script's `signal` aborted the call it sent as message `id`.
    /// The call stops at its next race or check and replies `AbortError` with `reason`; an id
    /// no call holds any more (it replied already) is ignored, as Playwright ignores one.
    pub(crate) fn abort(&self, id: u64, reason: String) {
        self.0.aborts.abort(id, reason);
    }

    /// Ends the run's calls after the realm is gone: records why the run ended (the first
    /// reason wins) and that its input is over, starts releasing the input it left held,
    /// shares [`RECORDS_WAIT`] across creation settlement, task draining (the releases
    /// included) and closing-state reads, then gives the run's highlights
    /// [`HIGHLIGHTS_WAIT`] of their own before returning the records and dropped-calls summary.
    pub(crate) async fn finish(&self, reason: RunEnd) -> (Vec<CallRecord>, Option<DroppedCalls>) {
        let deadline = Instant::now() + RECORDS_WAIT;
        let run = &self.0.run;
        run.end(reason);
        // Before anything below waits: what the run holds is held by no live run from now on,
        // however its release goes (`TabInput::mark_ended`).
        self.0.tabs.mark_input_ended();
        // What the run installed on the tabs it leaves open goes first, as `context.close()`
        // lets go of it, each part on a budget of its own: removal never waits on a call still
        // running, so no document loaded after the run returns runs one of its init scripts, no page keeps one of its
        // bindings, and none gets its session overrides (`Tabs::release_run_scoped`).
        self.0.tabs.release_run_scoped(run).await;
        // The realm is gone with its listeners: from now on the watchers answer dialogs
        // themselves, and a dialog a listener left open gets the default answer before the
        // sessions detach (`Dialogs::left_open`).
        self.0.tabs.events().close();
        let _answered = time::timeout_at(deadline, answer_left_open(&self.0.tabs)).await;
        // Every request a route handler left paused goes on, and no tab keeps pausing requests
        // for a run that is over (`routes.rs`).
        let _routed = time::timeout_at(deadline, async {
            let sessions = self.0.tabs.attached_sessions();
            future::join_all(sessions.iter().map(|session| routes::finish(session))).await;
        })
        .await;
        // Keys and buttons this run left held are released on each tab it touched, under a
        // turn (design section 4.6), once the dialogs it left open are answered. Each tab's
        // release is a task of the run's tracker, started before the tracker closes, so the
        // drain below waits for it and `Runtime::drain` for what the budget leaves running:
        // no budget cuts a release off, and no other cleanup goes before one.
        self.0.tabs.release_input();
        let _created = time::timeout_at(deadline, self.0.tabs.finish_creations()).await;
        run.tasks.close();
        let _waited = time::timeout_at(deadline, run.tasks.wait()).await;
        // Releases each session's handle group on its page's tracked task, before the sessions
        // detach; a run whose future is dropped releases them as `RunScope` drops.
        run.handles.close();
        let _refreshed = time::timeout_at(deadline, self.0.tabs.refresh_closing()).await;
        // The run's highlights show over the user's tab; none may outlive the run that made it.
        // They go last, on their own budget: a document that does not answer (a busy
        // out-of-process frame) delays the outcome, never a release.
        let sessions = self.0.tabs.attached_sessions();
        let _hidden = time::timeout(HIGHLIGHTS_WAIT, async {
            for session in &sessions {
                highlight::hide_at_run_end(session).await;
            }
        })
        .await;
        self.0.trace.take(reason)
    }
}

/// An init script command: `addInitScript` answers its `Disposable`, as Playwright's
/// `{ disposable }`; `dispose` answers nothing.
async fn init_call(
    tabs: &Tabs,
    target: Option<(Arc<PageEntry>, FrameScope)>,
    route: InitRoute,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let scripts = progress.init_scripts();
    let guid = match route {
        InitRoute::Page(source) => {
            let (page, _) = target.ok_or_else(PlaywrightError::target_closed)?;
            let session = progress.race(tabs.session(&page)).await??;
            progress
                .race(scripts.add(tabs.cdp(), Some(&session), &source))
                .await??
        }
        InitRoute::Context(source) => {
            let guid = progress
                .race(scripts.add(tabs.cdp(), None, &source))
                .await??;
            // Every page the run has a session on shows it from its next document, a reload
            // included, not only those a later call touches.
            for (_, session) in tabs.attached() {
                scripts.ensure(tabs.cdp(), &session).await;
            }
            guid
        }
        InitRoute::Dispose(guid) if bindings::owns(&guid) => {
            progress
                .race(progress.bindings().dispose(tabs, scripts, &guid))
                .await??;
            return Ok(Some(json!({})));
        }
        InitRoute::Dispose(guid) => {
            progress.race(scripts.dispose(&guid)).await??;
            return Ok(Some(json!({})));
        }
    };
    Ok(Some(json!({ "disposable": { "guid": guid } })))
}

/// The init script commands (`init_scripts.rs`), served by name ahead of the spec's own; the
/// object and params back when the message is another command.
fn route_init(
    object: ObjectKind,
    method: &str,
    params: Option<Value>,
) -> Result<Routed, (ObjectKind, Option<Value>)> {
    match object {
        ObjectKind::Context if method == CONTEXT_ADD_INIT_SCRIPT_META.method => Ok(Routed {
            meta: Some(&CONTEXT_ADD_INIT_SCRIPT_META),
            route: init_scripts::source_of(&CONTEXT_ADD_INIT_SCRIPT_META, params)
                .map(|source| ObjectCall::Init(InitRoute::Context(source))),
        }),
        ObjectKind::Page if method == PAGE_ADD_INIT_SCRIPT_META.method => Ok(Routed {
            meta: Some(&PAGE_ADD_INIT_SCRIPT_META),
            route: init_scripts::source_of(&PAGE_ADD_INIT_SCRIPT_META, params)
                .map(|source| ObjectCall::Init(InitRoute::Page(source))),
        }),
        ObjectKind::Context | ObjectKind::Page
            if method == bindings::PAGE_EXPOSE_BINDING_META.method =>
        {
            let (meta, route) =
                bindings::decode_expose(matches!(object, ObjectKind::Context), params);
            Ok(Routed {
                meta: Some(meta),
                route: route.map(ObjectCall::Binding),
            })
        }
        ObjectKind::BindingCall(guid) => Ok(match bindings::decode_call(&guid, method, params) {
            Some((meta, route)) => Routed {
                meta: Some(meta),
                route: route.map(ObjectCall::Binding),
            },
            None => Routed {
                meta: None,
                // Playwright: `server/dispatchers/dispatcher.ts` `DispatcherConnection.dispatch`: `Mismatching dispatcher: "${dispatcher._type}" does not implement "${method}"`
                route: Err(PlaywrightError::error(format!(
                    "Mismatching dispatcher: \"BindingCall\" does not implement \"{method}\""
                ))),
            },
        }),
        ObjectKind::Disposable(guid) => {
            Ok(Routed {
                meta: Some(&DISPOSABLE_DISPOSE_META),
                route: if method == DISPOSABLE_DISPOSE_META.method {
                    Ok(ObjectCall::Init(InitRoute::Dispose(guid)))
                } else {
                    // Playwright: `server/dispatchers/dispatcher.ts` `DispatcherConnection.dispatch`: `Mismatching dispatcher: "${dispatcher._type}" does not implement "${method}"`
                    Err(PlaywrightError::error(format!(
                        "Mismatching dispatcher: \"Disposable\" does not implement \"{method}\""
                    )))
                },
            })
        }
        object => Err((object, params)),
    }
}

/// A binding command: `exposeBinding` answers its `Disposable`, as Playwright's
/// `{ disposable }`; a binding call's answer answers nothing.
async fn binding_call(
    tabs: &Tabs,
    target: Option<(Arc<PageEntry>, FrameScope)>,
    route: BindingRoute,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let bindings = progress.bindings();
    let scripts = progress.init_scripts();
    let (outcome, guid) = match route {
        BindingRoute::Page { name, no_global } => {
            let (page, _) = target.ok_or_else(PlaywrightError::target_closed)?;
            let session = progress.race(tabs.session(&page)).await??;
            let guid = progress
                .race(bindings.expose(tabs, scripts, Some(&session), &name, no_global))
                .await??;
            return Ok(Some(json!({ "disposable": { "guid": guid } })));
        }
        BindingRoute::Context { name, no_global } => {
            let guid = progress
                .race(bindings.expose(tabs, scripts, None, &name, no_global))
                .await??;
            return Ok(Some(json!({ "disposable": { "guid": guid } })));
        }
        BindingRoute::Resolve { guid, result } => (Outcome::Result(result), guid),
        BindingRoute::Reject { guid, error } => (Outcome::Error(error), guid),
    };
    bindings
        .answer(tabs, &guid, outcome, progress.handles(), progress)
        .await?;
    Ok(Some(json!({})))
}

/// Names a message's target and decodes its params. Pure.
/// A Page call that is not a device call: a storage call keeps its own route, ordered per page.
fn storage_or_page(call: PageCall) -> ObjectCall {
    match call {
        call @ (PageCall::WebStorageItems(_)
        | PageCall::WebStorageGetItem(_)
        | PageCall::WebStorageSetItem(_)
        | PageCall::WebStorageRemoveItem(_)
        | PageCall::WebStorageClear(_)) => ObjectCall::Storage(call, None),
        call => ObjectCall::Page(call),
    }
}

fn route(object: ObjectKind, method: &str, params: Option<Value>) -> Routed {
    let (object, params) = match route_init(object, method, params) {
        Ok(routed) => return routed,
        Err(unrouted) => unrouted,
    };
    let decoded = match object {
        ObjectKind::Context => {
            let path = neo_path(params.as_ref());
            BrowserContextCall::decode(method, params)
                .map(|call| (call.meta(), context_call(call, path)))
        }
        ObjectKind::Page => {
            let path = neo_path(params.as_ref());
            PageCall::decode(method, params).map(|call| {
                let meta = call.meta();
                let routed = match DeviceCall::from_page_call(call) {
                    Ok(device) => device.map(ObjectCall::Device),
                    Err(call) => page_call(call, path),
                };
                (meta, routed)
            })
        }
        ObjectKind::Frame => {
            if let Some(routed) = route_neo_screenshot(method, params.as_ref()) {
                return routed;
            }
            if let Some(routed) = route_frame_extension(method, params.as_ref()) {
                return routed;
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
        ObjectKind::Handle(guid) => {
            if let Some(routed) = route_handle_screenshot(&guid, method, params.as_ref()) {
                return routed;
            }
            let (meta, call) = evaluate::route_handle(&guid, method, params.as_ref());
            return Routed {
                meta,
                route: call.map(|call| ObjectCall::Frame(Route::FrameEval(call))),
            };
        }
        ObjectKind::Dialog(index) => DialogCall::decode(method, params)
            .map(|call| (call.meta(), Ok(ObjectCall::Dialog(index, call)))),
        ObjectKind::Intercepted(index) if method == routes::FETCH_META.method => {
            return Routed {
                meta: Some(&routes::FETCH_META),
                route: decode_neo_params::<routes::NeoRouteFetchParams>(
                    params,
                    &routes::FETCH_META,
                )
                .map(|params| {
                    ObjectCall::Intercepted(index, routes::InterceptedCall::Fetch(params))
                }),
            };
        }
        ObjectKind::Intercepted(index) => RouteCall::decode(method, params).map(|call| {
            let meta = call.meta();
            let route = ObjectCall::Intercepted(index, routes::InterceptedCall::Answer(call));
            (meta, Ok(route))
        }),
        ObjectKind::Download(guid) => ArtifactCall::decode(method, params)
            .map(|call| (call.meta(), Ok(ObjectCall::Download(guid, call)))),
        // `route_init` answers every Disposable and BindingCall message.
        ObjectKind::Disposable(_) | ObjectKind::BindingCall(_) | ObjectKind::Unknown => {
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

/// Neo's `neo:screenshot` on a Frame guid (`locator.screenshot`, `capture.rs`), or `None` for
/// another method.
fn route_neo_screenshot(method: &str, params: Option<&Value>) -> Option<Routed> {
    (method == NEO_SCREENSHOT_META.method).then(|| Routed {
        meta: Some(&NEO_SCREENSHOT_META),
        route: decode_neo_params::<NeoScreenshotParams>(params.cloned(), &NEO_SCREENSHOT_META)
            .map(|params| ObjectCall::Capture(Capture::Locator(Box::new(params)))),
    })
}

/// `ElementHandle.screenshot` on a handle guid (`capture.rs`), or `None` for another method.
fn route_handle_screenshot(guid: &str, method: &str, params: Option<&Value>) -> Option<Routed> {
    (method == ELEMENT_HANDLE_SCREENSHOT_META.method).then(|| Routed {
        meta: Some(&ELEMENT_HANDLE_SCREENSHOT_META),
        route: decode_neo_params::<ElementScreenshotParams>(
            params.cloned(),
            &ELEMENT_HANDLE_SCREENSHOT_META,
        )
        .map(|params| {
            ObjectCall::Capture(Capture::Element {
                guid: guid.to_string(),
                params: Box::new(params),
            })
        }),
    })
}

/// Neo's `path` beside Playwright's params of a page call (`capture.rs`); Playwright's own
/// decode ignores the key.
fn neo_path(params: Option<&Value>) -> Result<Option<String>, PlaywrightError> {
    match params.and_then(|params| params.get("path")) {
        None => Ok(None),
        Some(Value::String(path)) => Ok(Some(path.clone())),
        Some(other) => Err(PlaywrightError::error(format!(
            "path: expected string, got {}",
            json_kind(other)
        ))),
    }
}

/// The JSON kind of `value`, as a validator names it.
fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// A decoded context call: the storage-state ones go with their `path`.
fn context_call(
    call: BrowserContextCall,
    path: Result<Option<String>, PlaywrightError>,
) -> Result<ObjectCall, PlaywrightError> {
    Ok(match call {
        BrowserContextCall::StorageState(params) => {
            ObjectCall::StorageState(StateFile::Export(params), path?)
        }
        BrowserContextCall::SetStorageState(params) => {
            ObjectCall::StorageState(StateFile::Import(params), path?)
        }
        call => ObjectCall::Context(call),
    })
}

/// A decoded page call: the ones that answer bytes go to `capture.rs` with their `path`.
fn page_call(
    call: PageCall,
    path: Result<Option<String>, PlaywrightError>,
) -> Result<ObjectCall, PlaywrightError> {
    Ok(match call {
        PageCall::Screenshot(params) => ObjectCall::Capture(Capture::Page {
            params: Box::new(params),
            path: path?,
        }),
        PageCall::Pdf(params) => ObjectCall::Capture(Capture::Pdf {
            params: Box::new(params),
            path: path?,
        }),
        call => storage_or_page(call),
    })
}

/// A frame's calls that are not one generated `FrameCall` (Neo's URL and load-state waits,
/// element commands, navigation waits, evaluation) or that `FrameCall` must not see
/// (`selectOption` with element handles). `None` hands the call to `FrameCall::decode`.
fn route_frame_extension(method: &str, params: Option<&Value>) -> Option<Routed> {
    if method == WAIT_FOR_URL_META.method {
        return Some(Routed {
            meta: Some(&WAIT_FOR_URL_META),
            route: decode_neo_params::<NeoWaitForUrlParams>(params.cloned(), &WAIT_FOR_URL_META)
                .and_then(|params| WaitForUrlOptions::from_params(&params))
                .map(|options| ObjectCall::Frame(Route::FrameWaitForUrl(options))),
        });
    }
    if let Some(routed) = route_element_command(method, params.cloned()) {
        return Some(routed);
    }
    if let Some(routed) = route_navigation_wait(method, params) {
        return Some(routed);
    }
    if let Some(routed) = route_frame_element(method) {
        return Some(routed);
    }
    if let Some((meta, call)) = evaluate::route_frame(method, params) {
        return Some(Routed {
            meta: Some(meta),
            route: call.map(|call| ObjectCall::Frame(Route::FrameEval(call))),
        });
    }
    if method == WAIT_FOR_LOAD_STATE_META.method {
        return Some(Routed {
            meta: Some(&WAIT_FOR_LOAD_STATE_META),
            route: decode_neo_params::<NeoWaitForLoadStateParams>(
                params.cloned(),
                &WAIT_FOR_LOAD_STATE_META,
            )
            .map(WaitForLoadStateOptions::from_params)
            .map(|options| ObjectCall::Frame(Route::FrameWaitForLoadState(options))),
        });
    }
    // The generated `ElementHandleChannel` has no values, so `elements` (Playwright's
    // `ElementHandle` values) leaves the wire before the generated decode, as guids. Keep the
    // method's input flag and title.
    if method == "selectOption"
        && let Some(elements) = params
            .and_then(|params| params.get("elements"))
            .and_then(Value::as_array)
        && !elements.is_empty()
    {
        return Some(Routed {
            meta: Some(FrameSelectOptionParams::META),
            route: select_option_with_elements(params, elements),
        });
    }
    None
}

/// Playwright: `server/dispatchers/dispatcher.ts` `DispatcherConnection._tChannelImplFromWire`: `expected guid for ${names.toString()}`
///
/// `selectOption` with `elements`: each element's guid, and the rest of its params decoded as
/// the generated `FrameCall` decodes them. The guids name handles only once the call runs
/// (`fill::select_option`).
fn select_option_with_elements(
    params: Option<&Value>,
    elements: &[Value],
) -> Result<ObjectCall, PlaywrightError> {
    let guids = elements
        .iter()
        .enumerate()
        .map(|(index, element)| {
            element
                .get("guid")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    PlaywrightError::error(format!(
                        "elements[{index}]: expected guid for ElementHandle"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut rest = params.cloned().unwrap_or_else(|| json!({}));
    if let Some(object) = rest.as_object_mut() {
        object.remove("elements");
    }
    let call = FrameCall::decode("selectOption", Some(rest))
        .map_err(|error| PlaywrightError::error(error.to_string()))?;
    match route_frame_call(call)? {
        Route::FrameSelectOption(options) => Ok(ObjectCall::Frame(Route::FrameSelectOption(
            options.with_elements(guids),
        ))),
        _ => Err(PlaywrightError::error(
            "neo-runtime: selectOption decoded as another call",
        )),
    }
}

/// The Neo element commands (`element_ops.rs`), or `None` for any other method.
fn route_element_command(method: &str, params: Option<Value>) -> Option<Routed> {
    let (meta, route) = if method == SCROLL_INTO_VIEW_META.method {
        let route = decode_neo_params::<NeoElementParams>(params, &SCROLL_INTO_VIEW_META)
            .map(|params| Route::FrameScrollIntoViewIfNeeded(ElementTarget::from_neo(params)));
        (&SCROLL_INTO_VIEW_META, route)
    } else if method == SELECT_TEXT_META.method {
        let route =
            decode_neo_params::<NeoSelectTextParams>(params, &SELECT_TEXT_META).map(|params| {
                let force = params.force.unwrap_or(false);
                let target = ElementTarget::from_neo(NeoElementParams {
                    selector: params.selector,
                    strict: params.strict,
                });
                Route::FrameSelectText(target, force)
            });
        (&SELECT_TEXT_META, route)
    } else if method == BOUNDING_BOX_META.method {
        let route = decode_neo_params::<NeoElementParams>(params, &BOUNDING_BOX_META)
            .map(|params| Route::FrameBoundingBox(ElementTarget::from_neo(params)));
        (&BOUNDING_BOX_META, route)
    } else {
        return None;
    };
    Some(Routed {
        meta: Some(meta),
        route: route.map(ObjectCall::Frame),
    })
}

/// Neo's navigation-wait commands on a Frame guid (`history.rs`), or `None` for another method.
fn route_navigation_wait(method: &str, params: Option<&Value>) -> Option<Routed> {
    let params = params.cloned();
    let (meta, route) = if method == WAIT_FOR_NAVIGATION_META.method {
        let route =
            decode_neo_params::<NeoWaitForNavigationParams>(params, &WAIT_FOR_NAVIGATION_META)
                .and_then(|params| WaitForNavigationOptions::from_params(&params))
                .map(Route::FrameWaitForNavigation);
        (&WAIT_FOR_NAVIGATION_META, route)
    } else if method == NEXT_NAVIGATION_META.method {
        let route = decode_neo_params::<NeoNextNavigationParams>(params, &NEXT_NAVIGATION_META)
            .map(Route::FrameNextNavigation);
        (&NEXT_NAVIGATION_META, route)
    } else if method == FINISH_NAVIGATION_META.method {
        let route = decode_neo_params::<NeoFinishNavigationParams>(params, &FINISH_NAVIGATION_META)
            .map(|params| FinishNavigationOptions::from_params(&params))
            .map(Route::FrameFinishNavigation);
        (&FINISH_NAVIGATION_META, route)
    } else if method == RELEASE_NAVIGATION_META.method {
        let route =
            decode_neo_params::<NeoReleaseNavigationParams>(params, &RELEASE_NAVIGATION_META)
                .map(Route::FrameReleaseNavigation);
        (&RELEASE_NAVIGATION_META, route)
    } else {
        return None;
    };
    Some(Routed {
        meta: Some(meta),
        route: route.map(ObjectCall::Frame),
    })
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
        FrameCall::Goto(params) => Ok(Route::FrameGoto(GotoOptions::from_params(params))),
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
        FrameCall::AriaSnapshot(params) => Ok(Route::FrameAriaSnapshot(
            SnapshotOptions::from_params(params),
        )),
        FrameCall::Fill(params) => Ok(Route::FrameFill(FillOptions::from_params(params))),
        FrameCall::Press(params) => Ok(Route::FramePress(PressOptions::from_params(params))),
        FrameCall::Type(params) => Ok(Route::FrameType(PressOptions::from_type(params))),
        FrameCall::DragAndDrop(params) => {
            DragOptions::from_params(params).map(Route::FrameDragAndDrop)
        }
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
        FrameCall::SetInputFiles(params) => Ok(Route::FrameSetInputFiles(params)),
        FrameCall::Drop(params) => Ok(Route::FrameDrop(params)),
        FrameCall::WaitForSelector(params) => Ok(Route::FrameWaitForSelector(
            WaitForSelectorOptions::from_params(params),
        )),
        FrameCall::Content(params) => {
            Ok(Route::FrameContent(params.include_shadow.unwrap_or(false)))
        }
        FrameCall::SetContent(params) => Ok(Route::FrameSetContent(
            SetContentOptions::from_params(params),
            None,
        )),
        FrameCall::WaitForTimeout(params) => Ok(Route::FrameWaitForTimeout(params.wait_timeout)),
        FrameCall::ResolveSelector(params) => Ok(Route::FrameResolveSelector(params.selector)),
        FrameCall::AriaSnapshotJSON(params) => Ok(Route::FrameAriaSnapshotJson(
            SnapshotOptions::from_json_params(params),
        )),
        FrameCall::Focus(params) => Ok(Route::FrameFocus(ElementTarget::from_focus(params))),
        FrameCall::Blur(params) => Ok(Route::FrameBlur(ElementTarget::from_blur(params))),
        FrameCall::DispatchEvent(params) => {
            DispatchOptions::from_params(params).map(Route::FrameDispatchEvent)
        }
        FrameCall::Highlight(params) => Ok(Route::FrameHighlight(
            HighlightOptions::from_highlight(params),
        )),
        FrameCall::HideHighlight(params) => Ok(Route::FrameHideHighlight(
            HighlightOptions::from_hide(params),
        )),
    }
}

struct CallReply {
    entry: Arc<CallEntry>,
    trace: Trace,
    id: Value,
    reply: oneshot::Sender<Reply>,
    /// The run's events, whose barrier the reply waits for.
    events: EventHub,
    /// The call's share of the in-flight caps; none for a wait's finish or release.
    share: Option<Admitted>,
    /// A wait's finish or release: its claim on the wait it ends (`Cursors::begin_ending`).
    ending: Option<Ending<FrameCursor>>,
    /// A running locator handler's completion: its claim on that handler
    /// (`locator_handlers::begin_resolving`).
    resolving: Option<Resolving>,
    /// The call's place in the abort registry; `None` for an id that is not an integer.
    abortable: Option<Registration>,
}

/// Attachment can block or be cancelled independently of a frame handler. Once attached,
/// `serve` retains its original contract: the `PageSession` always belongs to this call's tab.
async fn serve_call(
    tabs: Tabs,
    cursors: Arc<Cursors<FrameCursor>>,
    target: Option<(Arc<PageEntry>, FrameScope)>,
    route: ObjectCall,
    progress: Progress,
    reply: CallReply,
) {
    // A call the script aborted before its task first ran never starts its handler, even one
    // that would not race its progress first. A run that ended meanwhile outranks the abort
    // (`Progress::race`'s order): the handler starts and its first race reports the end.
    if let Err(error) = progress.pending_abort() {
        finish_reply(Err(error), reply).await;
        return;
    }
    // A page's call waits for what the run's subscriptions switched on there (console messages,
    // the dialog watcher), so the call cannot cause an event before its producer listens; a
    // context call (`newPage`) waits for target discovery's baseline. A dialog's answer never
    // waits: an open dialog can hold that very enable, and only the answer frees the page.
    let gate = match (&route, &target) {
        // A route's answer never waits either: the paused request may be what the page's
        // enable, or the script's goto, waits for. A download's calls start nothing in a page;
        // `cancel` must not wait for a page's enables.
        (ObjectCall::Dialog(..) | ObjectCall::Download(..) | ObjectCall::Intercepted(..), _) => {
            None
        }
        (ObjectCall::Context(_) | ObjectCall::StorageState(..), _) => Some(ALL_PAGES),
        (_, Some((page, _))) => Some(page.id.as_str()),
        (_, None) => None,
    };
    if let Some(gate) = gate
        && reply.events.enabling(gate)
        && let Err(error) = progress.race(reply.events.ready(gate)).await
    {
        finish_reply(Err(error), reply).await;
        return;
    }
    let result = match (route, target) {
        (ObjectCall::Context(call), _) => tab_calls::context(&tabs, call, &progress).await,
        (ObjectCall::StorageState(file, path), _) => {
            tab_calls::state_file(&tabs, file, path, &progress).await
        }
        (ObjectCall::Init(route), target) => init_call(&tabs, target, route, &progress).await,
        (ObjectCall::Storage(call, turn), Some((page, _))) => {
            tab_calls::storage(&tabs, &page, call, turn, &progress).await
        }
        (ObjectCall::Binding(route), target) => binding_call(&tabs, target, route, &progress).await,
        (ObjectCall::Page(call), Some((page, _))) => {
            // A page call that navigates (`reload`, `goBack`, …) shows the run's scripts first,
            // as a Frame call does.
            if progress.init_scripts().has_scripts()
                && let Ok(Ok(session)) = progress.race(tabs.session(&page)).await
            {
                progress.init_scripts().ensure(tabs.cdp(), &session).await;
            }
            tab_calls::page(&tabs, &page, call, &progress).await
        }
        (ObjectCall::Capture(call), Some((page, scope))) => {
            capture::serve(&tabs, &page, scope, call, &progress).await
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
                    // A page the run's init scripts have not reached yet gets them first.
                    progress.init_scripts().ensure(tabs.cdp(), &page).await;
                    // A page attached just now may have started a context subscription's
                    // enable; the call waits for that too.
                    if reply.events.enabling(page_id(&page))
                        && let Err(error) = progress.race(reply.events.ready(page_id(&page))).await
                    {
                        finish_reply(Err(error), reply).await;
                        return;
                    }
                    serve(FrameTab { page, scope }, &cursors, route, progress, reply).await;
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
        // A dialog exists only after its tab's watcher saw it open on this run's session.
        (ObjectCall::Dialog(index, call), Some((page, _))) => match page.attached_session() {
            Some(session) => dialogs::serve(&session, index, call, &progress).await,
            None => Err(PlaywrightError::target_closed().into()),
        },
        // A paused request exists only after its tab's watcher handed it to this run.
        (ObjectCall::Intercepted(index, call), Some((page, _))) => match page.attached_session() {
            Some(session) => routes::serve_intercepted(&session, index, call, &progress).await,
            None => Err(PlaywrightError::target_closed().into()),
        },
        (ObjectCall::Download(guid, call), _) => {
            downloads::serve(&tabs, &guid, call, &progress).await
        }
        _ => Err(PlaywrightError::target_closed().into()),
    };
    finish_reply(result, reply).await;
}

fn page_id(page: &PageSession) -> &str {
    page.events.target()
}

/// Gives every dialog the run handed to a listener and nobody answered its default answer, and
/// waits for the browser to take each, so the tab is not left frozen when its session detaches.
async fn answer_left_open(tabs: &Tabs) {
    let answers = tabs.attached().into_iter().filter_map(|(_, session)| {
        let answer = session.events.dialogs.left_open()?;
        Some(async move {
            let accept = answer == DialogAnswer::Accept;
            let answered = session
                .send("Page.handleJavaScriptDialog", json!({ "accept": accept }))
                .await;
            if let Err(error) = answered {
                debug!(%error, "answering a dialog left open at run end failed");
            }
        })
    });
    future::join_all(answers).await;
}

/// One call's task: serves the route, ends the trace entry and sends the reply. A call the run
/// ended leaves its entry open, so its record reads `Unfinished`, and sends nothing: nobody
/// waits for it. The task holds the page, so the session stays attached while it runs.
async fn serve(
    page: FrameTab,
    cursors: &Cursors<FrameCursor>,
    route: Route,
    progress: Progress,
    reply: CallReply,
) {
    // Navigation and page reads need the real page's navigation/read seams, which the
    // forms tests' fake does not implement, so they are answered here; every other route goes
    // through `serve_route`.
    if let Some(error) = unsupported_child_call(&page.scope, &route) {
        finish_reply(Err(error.into()), reply).await;
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
        Route::FrameInnerHtml(options) => reads::inner_html(&page, &options, &progress)
            .await
            .and_then(|result| encode_result::<FrameInnerHTMLParams>(&result)),
        Route::FrameAriaSnapshot(options) => reads::aria_snapshot(&page, &options, &progress)
            .await
            .and_then(|result| encode_result::<FrameAriaSnapshotParams>(&result)),
        Route::FrameWaitForSelector(options) => {
            waits::wait_for_selector_element(&page, &options, &progress)
                .await
                .and_then(wait_for_selector_reply)
        }
        // `ElementHandle` is no interface of the generated spec, so the element travels as the
        // record the SDK builds it from, as `waitForSelector`'s does.
        Route::FrameFrameElement => frame_element::frame_element(&page, &progress)
            .await
            .map(|created| Some(json!({ "element": created }))),
        Route::FrameEval(call) => evaluate::serve(&page, call, &progress).await,
        // Neo extensions answer an empty result.
        Route::FrameWaitForUrl(options) => waits::wait_for_url(&page, &options, &progress)
            .await
            .map(|()| Some(json!({}))),
        Route::FrameWaitForLoadState(options) => {
            waits::wait_for_load_state(&page, &options, &progress)
                .await
                .map(|()| Some(json!({})))
        }
        Route::FrameWaitForNavigation(options) => {
            let waited = if matches!(page.scope, FrameScope::Main) {
                history::wait_for_navigation(&page, &options, &progress).await
            } else {
                match WaitTab::observe(&page) {
                    Ok(events) => {
                        history::wait_for_navigation_on(&page, events, &options, &progress).await
                    }
                    Err(error) => Err(navigate::page_error(error).into()),
                }
            };
            waited.and_then(|response| encode_neo(&NeoNavigationResult { response }))
        }
        Route::FrameNextNavigation(params) => {
            history::next_frame_navigation(cursors, page, &params, &progress)
                .await
                .and_then(|result| encode_neo(&result))
        }
        Route::FrameFinishNavigation(options) => {
            history::finish_frame_navigation(cursors, &options, &progress)
                .await
                .and_then(|result| encode_neo(&result))
        }
        Route::FrameReleaseNavigation(params) => {
            encode_neo(&history::release_frame_navigation(cursors, &params))
        }
        route @ (Route::FrameIsVisible(_)
        | Route::FrameIsHidden(_)
        | Route::FrameIsChecked(_)
        | Route::FrameIsEnabled(_)
        | Route::FrameIsDisabled(_)
        | Route::FrameIsEditable(_)) => serve_state(&page, route, &progress).await,
        route @ (Route::FrameContent(_)
        | Route::FrameSetContent(..)
        | Route::FrameWaitForTimeout(_)
        | Route::FrameResolveSelector(_)
        | Route::FrameAriaSnapshotJson(_)) => serve_document(&page, route, &progress).await,
        route @ (Route::FrameSetInputFiles(_) | Route::FrameDrop(_)) => {
            serve_files(&page, route, &progress).await
        }
        route @ (Route::FrameFocus(_)
        | Route::FrameBlur(_)
        | Route::FrameDispatchEvent(_)
        | Route::FrameScrollIntoViewIfNeeded(_)
        | Route::FrameSelectText(..)
        | Route::FrameBoundingBox(_)
        | Route::FrameHighlight(_)
        | Route::FrameHideHighlight(_)) => serve_element_op(&page, route, &progress).await,
        route => serve_route(&page, route, &progress).await,
    };
    finish_reply(result, reply).await;
}

/// The file calls `serve` answers on the call's page: `setInputFiles` and `drop`, which read
/// the run's artifact directory. Each replies `{}`.
async fn serve_files(
    page: &FrameTab,
    route: Route,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    match route {
        Route::FrameSetInputFiles(params) => {
            set_input_files::set_input_files(page, params, progress)
                .await
                .and_then(|()| {
                    encode_result::<FrameSetInputFilesParams>(&FrameSetInputFilesResult {})
                })
        }
        Route::FrameDrop(params) => drop_on::drop_on(page, params, progress)
            .await
            .and_then(|()| encode_result::<FrameDropParams>(&FrameDropResult {})),
        _ => Err(PlaywrightError::error(
            "neo-runtime: `serve_files` serves setInputFiles and drop only",
        )
        .into()),
    }
}

/// The element operations `serve` answers on the call's page (`element_ops.rs`,
/// `highlight.rs`); each replies `{}` but `neo:boundingBox`, which answers `{value?}`.
async fn serve_element_op(
    page: &FrameTab,
    route: Route,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    match route {
        Route::FrameFocus(target) => element_ops::focus(page, &target, progress)
            .await
            .map(|()| None),
        Route::FrameBlur(target) => element_ops::blur(page, &target, progress)
            .await
            .map(|()| None),
        Route::FrameDispatchEvent(options) => element_ops::dispatch_event(page, &options, progress)
            .await
            .map(|()| None),
        Route::FrameScrollIntoViewIfNeeded(target) => {
            element_ops::scroll_into_view_if_needed(page, &target, progress)
                .await
                .map(|()| Some(json!({})))
        }
        Route::FrameSelectText(target, force) => {
            element_ops::select_text(page, &target, force, progress)
                .await
                .map(|()| Some(json!({})))
        }
        Route::FrameBoundingBox(target) => element_ops::bounding_box(page, &target, progress)
            .await
            .and_then(|result| {
                serde_json::to_value(result).map(Some).map_err(|error| {
                    PlaywrightError::error(format!("encoding neo:boundingBox result: {error}"))
                        .into()
                })
            }),
        Route::FrameHighlight(options) => highlight::highlight(page, &options, progress)
            .await
            .map(|()| None),
        Route::FrameHideHighlight(options) => highlight::hide_highlight(page, &options, progress)
            .await
            .map(|()| None),
        _ => Err(PlaywrightError::error(
            "neo-runtime: `serve_element_op` serves element operations only",
        )
        .into()),
    }
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

/// The document-wide calls `serve` answers on the call's page (`content.rs`, `waits.rs`,
/// `reads.rs`).
async fn serve_document(
    page: &FrameTab,
    route: Route,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    match route {
        Route::FrameContent(include_shadow) => content::content(page, include_shadow, progress)
            .await
            .and_then(|result| encode_result::<FrameContentParams>(&result)),
        Route::FrameSetContent(options, turn) => {
            content::set_content(page, &options, turn, progress)
                .await
                .and_then(|()| encode_result::<FrameSetContentParams>(&FrameSetContentResult {}))
        }
        Route::FrameWaitForTimeout(wait) => {
            waits::wait_for_timeout(page, waits::timer_delay(wait), progress)
                .await
                .and_then(|()| {
                    encode_result::<FrameWaitForTimeoutParams>(&FrameWaitForTimeoutResult {})
                })
        }
        Route::FrameResolveSelector(selector) => {
            content::resolve_selector(page, &selector, progress)
                .await
                .and_then(|result| encode_result::<FrameResolveSelectorParams>(&result))
        }
        Route::FrameAriaSnapshotJson(options) => {
            reads::aria_snapshot_json(page, &options, progress)
                .await
                .and_then(|result| encode_result::<FrameAriaSnapshotJSONParams>(&result))
        }
        _ => Err(PlaywrightError::error(
            "neo-runtime: `serve_document` serves the document calls only",
        )
        .into()),
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

/// Ends the call's trace entry, then replies once every event that preceded the reply has
/// reached the queue (`EventHub::barrier`), with the watermark the SDK delivers up to.
async fn finish_reply(result: Result<Option<Value>, CallError>, target: CallReply) {
    let CallReply {
        entry,
        trace,
        id,
        reply,
        events,
        share,
        ending,
        resolving,
        abortable,
    } = target;
    // The outcome is decided: an abort from now on is ignored, as Playwright drops a call's
    // controller before it replies.
    drop(abortable);
    let (record, answer) = match result {
        Err(CallError::RunEnded(_)) => return,
        Ok(value) => (CallResult::Ok, Ok(value)),
        Err(CallError::Playwright(error)) => (error.to_result(), Err(error)),
    };
    let log = entry.finish(record);
    trace.finished(&entry);
    let answer = answer.as_ref().map(Option::as_ref);
    let text = encode_reply(&id, answer, Some(&log));
    events.barrier().await;
    // Given back before the reply leaves, so a script that awaited this call can start another
    // at once. A call that never replies gives them back as its task ends.
    drop((share, ending, resolving));
    // The script may have stopped waiting (the realm is gone); the record already holds the
    // outcome.
    let _sent = reply.send((text, events.seq()));
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
        Route::FrameDragAndDrop(options) => drag::drag_and_drop(tab, &options, progress)
            .await
            .and_then(|()| encode_result::<FrameDragAndDropParams>(&FrameDragAndDropResult {})),
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
        | Route::FrameInnerHtml(_)
        | Route::FrameIsVisible(_)
        | Route::FrameIsHidden(_)
        | Route::FrameIsChecked(_)
        | Route::FrameIsEnabled(_)
        | Route::FrameIsDisabled(_)
        | Route::FrameIsEditable(_)
        | Route::FrameAriaSnapshot(_)
        | Route::FrameWaitForSelector(_)
        | Route::FrameFrameElement
        | Route::FrameWaitForUrl(_)
        | Route::FrameWaitForLoadState(_)
        | Route::FrameContent(_)
        | Route::FrameSetContent(..)
        | Route::FrameWaitForTimeout(_)
        | Route::FrameResolveSelector(_)
        | Route::FrameAriaSnapshotJson(_)
        | Route::FrameFocus(_)
        | Route::FrameBlur(_)
        | Route::FrameDispatchEvent(_)
        | Route::FrameScrollIntoViewIfNeeded(_)
        | Route::FrameSelectText(..)
        | Route::FrameBoundingBox(_)
        | Route::FrameHighlight(_)
        | Route::FrameHideHighlight(_)
        | Route::FrameSetInputFiles(_)
        | Route::FrameDrop(_)
        | Route::FrameWaitForNavigation(_)
        | Route::FrameNextNavigation(_)
        | Route::FrameFinishNavigation(_)
        | Route::FrameReleaseNavigation(_)
        | Route::FrameEval(_) => Err(PlaywrightError::error(
            "neo-runtime: navigation and page reads are served by `serve`",
        )
        .into()),
    }
}

/// What a message's `guid` names: the context, an object of a resolved tab (a JS handle, a frame,
/// a network hop, a dialog, or the page itself), or nothing Neo knows.
fn object_kind(guid: &str, resolved: bool) -> ObjectKind {
    if guid == "context@default" {
        ObjectKind::Context
    } else if guid.starts_with(init_scripts::DISPOSABLE_PREFIX) {
        ObjectKind::Disposable(guid.to_owned())
    } else if guid.starts_with(bindings::BINDING_CALL_PREFIX) {
        ObjectKind::BindingCall(guid.to_owned())
    } else if let Some(download) = guid.strip_prefix(downloads::DOWNLOAD_PREFIX) {
        ObjectKind::Download(download.to_owned())
    } else if !resolved {
        ObjectKind::Unknown
    } else if guid.starts_with(evaluate::HANDLE_PREFIX) {
        ObjectKind::Handle(guid.to_owned())
    } else if guid.starts_with("frame@") {
        ObjectKind::Frame
    } else if let Some((kind, _, hop)) = HopId::parse(guid) {
        ObjectKind::Network(kind, hop)
    } else if let Some((_, index)) = dialogs::parse(guid) {
        ObjectKind::Dialog(index)
    } else if let Some((_, index)) = routes::parse(guid) {
        ObjectKind::Intercepted(index)
    } else {
        ObjectKind::Page
    }
}

/// A Neo extension's result as its reply's `result`.
fn encode_neo(result: &impl serde::Serialize) -> Result<Option<Value>, CallError> {
    serde_json::to_value(result)
        .map(Some)
        .map_err(|error| PlaywrightError::error(format!("encoding a Neo result: {error}")).into())
}

/// `Frame.waitForSelector`'s reply. `ElementHandle` is no interface of the generated spec, so a
/// returned element travels as the record the SDK builds it from (`evaluate.rs`).
fn wait_for_selector_reply(
    element: Option<evaluate::CreatedHandle>,
) -> Result<Option<Value>, CallError> {
    match element {
        Some(created) => Ok(Some(json!({ "element": created }))),
        None => encode_result::<FrameWaitForSelectorParams>(&FrameWaitForSelectorResult {
            element: None,
        }),
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

/// `Frame.frameElement`, routed by hand: its result requires an `ElementHandle`, which the
/// generated spec cannot create (`frame_element.rs`).
fn route_frame_element(method: &str) -> Option<Routed> {
    (method == frame_element::FRAME_ELEMENT_META.method).then(|| Routed {
        meta: Some(&frame_element::FRAME_ELEMENT_META),
        route: Ok(ObjectCall::Frame(Route::FrameFrameElement)),
    })
}

/// The cursor a URL-predicate wait's finish or release names. Each skips the in-flight caps for
/// the one call that claims its open cursor (`Cursors::begin_ending`): the wait it ends may hold
/// that capacity, and the claim counts against the cursor cap instead.
/// The locator handler a `Page.resolveLocatorHandlerNoReply` completes.
fn completed_handler(call: &ObjectCall) -> Option<i64> {
    match call {
        ObjectCall::Page(PageCall::ResolveLocatorHandlerNoReply(params)) => Some(params.uid),
        _ => None,
    }
}

fn ending_cursor(call: &ObjectCall) -> Option<u64> {
    match call {
        ObjectCall::Frame(Route::FrameFinishNavigation(options)) => Some(options.cursor),
        ObjectCall::Frame(Route::FrameReleaseNavigation(params)) => Some(params.cursor),
        _ => None,
    }
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

/// One admitted call's share of [`InFlight`], given back when it drops: just before the call's
/// reply is sent ([`finish_reply`]), or when its task ends without one (the run ended under
/// it, or the runtime dropped it).
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
    use crate::press::KeyInput;
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
    use crate::screenshot::TabScreenshots;
    use crate::session_state::Permissions;
    use crate::switch::Switches;
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
            TabScreenshots::default(),
            Switches::default(),
            Permissions::default(),
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
            Arc::default(),
            Some((page, scope)),
            route,
            progress,
            CallReply {
                entry,
                trace: trace.clone(),
                id: json!(1),
                reply,
                events: EventHub::default(),
                share: None,
                ending: None,
                resolving: None,
                abortable: None,
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
            TabScreenshots::default(),
            Switches::default(),
            Permissions::default(),
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
            | ObjectCall::StorageState(..)
            | ObjectCall::Page(_)
            | ObjectCall::Device(_)
            | ObjectCall::Network(_)
            | ObjectCall::Init(_)
            | ObjectCall::Capture(_)
            | ObjectCall::Binding(_)
            | ObjectCall::Dialog(..)
            | ObjectCall::Intercepted(..)
            | ObjectCall::Download(..)
            | ObjectCall::Storage(..) => {
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
            "type" => json!({ "selector": selector, "strict": true, "text": "hi", "delay": 10.0 }),
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

    const FORMS: [&str; 7] = [
        "fill",
        "press",
        "type",
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
            assert_form_route(
                method,
                frame_route(routed.route).expect("served form route"),
            );
        }
    }

    /// The options one forms command's route must retain; every other route fails the test.
    fn assert_form_route(method: &str, route: Route) {
        match route {
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
            Route::FrameType(options) => {
                assert_eq!(
                    options,
                    PressOptions {
                        selector: "input".into(),
                        strict: true,
                        keys: KeyInput::Type("hi".into()),
                        delay: Some(10.0),
                        wait_after: false
                    },
                    "a type waits for no navigation"
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
            // Every other route is no form; naming each would only grow this match.
            other => panic!("{method} is a form, not {other:?}"),
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
        // `elements` (Playwright's ElementHandle values) leave the wire as guids, the rest
        // decodes as usual; a member that is no channel is refused as Playwright's dispatcher
        // refuses it.
        let mut params = forms_params("selectOption", "select");
        params["elements"] = json!([{ "guid": "handle@1" }, { "guid": "handle@2" }]);
        let routed = route_frame("selectOption", params);
        assert_eq!(
            routed.meta.and_then(|meta| meta.title),
            Some("Select option")
        );
        match frame_route(routed.route).expect("routed") {
            Route::FrameSelectOption(options) => {
                assert_eq!(options.elements, ["handle@1", "handle@2"]);
                assert_eq!(options.selector, "select");
            }
            _ => panic!("not a selectOption"),
        }
        let mut params = forms_params("selectOption", "select");
        params["elements"] = json!([{ "guid": "handle@1" }, "two"]);
        assert_eq!(
            error_of(route_frame("selectOption", params)),
            PlaywrightError::error("elements[1]: expected guid for ElementHandle")
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
        // A wait that returns its element (`locator.evaluate`'s `_withElement`) is served too.
        for (omit, returns) in [
            (json!({}), true),
            (json!({"omitReturnValue":false}), true),
            (json!({"omitReturnValue":true}), false),
        ] {
            let mut params = json!({"selector":"#item"});
            params
                .as_object_mut()
                .expect("object")
                .extend(omit.as_object().expect("object").clone());
            match frame_route(route_frame("waitForSelector", params).route) {
                Ok(Route::FrameWaitForSelector(options)) => {
                    assert_eq!(options.return_element, returns, "{omit}");
                }
                other => panic!("waitForSelector routed to {other:?}"),
            }
        }
        assert!(route_frame("waitForSelector", json!({"selector":"iframe >> internal:control=enter-frame >> button", "omitReturnValue": true})).route.is_ok());
        // `networkidle` is served by every wait (`network_idle.rs`).
        for (method, params) in [
            (
                "neo:waitForURL",
                json!({"url":{"glob":""},"waitUntil":"networkidle"}),
            ),
            ("neo:waitForLoadState", json!({"state":"networkidle"})),
            (
                "neo:waitForNavigation",
                json!({"url":{"glob":""},"waitUntil":"networkidle"}),
            ),
            (
                "neo:finishNavigation",
                json!({"cursor":1,"index":0,"waitUntil":"networkidle"}),
            ),
            (
                "goto",
                json!({"url":"https://example.test","waitUntil":"networkidle"}),
            ),
        ] {
            assert!(route_frame(method, params).route.is_ok(), "{method}");
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
    fn tests_that_the_navigation_wait_messages_decode_and_refuse() {
        // The SDK's wire for these commands, which neo-sdk's tests/unit/history.test.ts pins.
        for (method, params) in [
            (
                "neo:waitForNavigation",
                json!({"url":{"glob":""},"waitUntil":"load"}),
            ),
            (
                "neo:waitForNavigation",
                json!({"url":{"regexSource":"done$","regexFlags":"i"},"waitUntil":"commit"}),
            ),
            ("neo:nextNavigation", json!({"response":true})),
            ("neo:nextNavigation", json!({"cursor":4,"response":false})),
            (
                "neo:finishNavigation",
                json!({"cursor":4,"index":2,"waitUntil":"domcontentloaded"}),
            ),
            ("neo:releaseNavigation", json!({"cursor":4})),
        ] {
            let routed = route_frame(method, params);
            let meta = routed.meta.expect("navigation wait metadata");
            assert_eq!(
                (meta.title, meta.interface),
                (Some("Wait for navigation"), "Frame")
            );
            assert!(!meta.has(MethodFlag::Input));
            assert!(routed.route.is_ok(), "{method}");
        }
        for (method, params) in [
            ("neo:waitForNavigation", json!({"url":{}})),
            ("neo:waitForNavigation", json!({"url":{"glob":"**/{a"}})),
            ("neo:nextNavigation", json!({})),
            ("neo:nextNavigation", json!({"response":true,"extra":1})),
            (
                "neo:finishNavigation",
                json!({"index":0,"waitUntil":"load"}),
            ),
            (
                "neo:finishNavigation",
                json!({"cursor":1,"waitUntil":"load"}),
            ),
            ("neo:releaseNavigation", json!({})),
        ] {
            assert_eq!(
                error_of(route_frame(method, params)).name,
                ErrorName::Error,
                "{method}"
            );
        }
        // Every frame's requests are recorded, so a child frame's navigation waits are served
        // with their Response, as the main frame's; only `goto` stays refused on a child.
        let child = FrameScope::Frame("child".to_string());
        for (method, params, refused) in [
            ("neo:waitForNavigation", json!({"url":{"glob":""}}), false),
            ("neo:nextNavigation", json!({"response":true}), false),
            ("neo:nextNavigation", json!({"response":false}), false),
            ("neo:releaseNavigation", json!({"cursor":1}), false),
            ("goto", json!({"url":"https://example.test"}), true),
        ] {
            let Ok(ObjectCall::Frame(route)) = route_frame(method, params).route else {
                panic!("{method} routes");
            };
            assert_eq!(
                unsupported_child_call(&child, &route).is_some(),
                refused,
                "{method}"
            );
            assert!(unsupported_child_call(&FrameScope::Main, &route).is_none());
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
    fn tests_that_capture_calls_route_with_neos_path() {
        let screenshot = route(
            ObjectKind::Page,
            "screenshot",
            Some(json!({"type": "jpeg", "quality": 50, "path": "shots/a.jpg"})),
        );
        assert_eq!(screenshot.meta.map(|meta| meta.method), Some("screenshot"));
        match screenshot.route {
            Ok(ObjectCall::Capture(Capture::Page { params, path })) => {
                assert_eq!(params.quality, Some(50));
                assert_eq!(path.as_deref(), Some("shots/a.jpg"));
            }
            Ok(_) => panic!("expected a page screenshot"),
            Err(error) => panic!("expected a page screenshot, got {error:?}"),
        }
        let pdf = route(ObjectKind::Page, "pdf", None);
        assert_eq!(pdf.meta.map(|meta| meta.title), Some(Some("PDF")));
        assert!(
            matches!(
                pdf.route,
                Ok(ObjectCall::Capture(Capture::Pdf { path: None, .. }))
            ),
            "a pdf without a path writes nothing"
        );
        let bad_path = error_of(route(
            ObjectKind::Page,
            "screenshot",
            Some(json!({"path": 5})),
        ));
        assert_eq!(bad_path.message, "path: expected string, got number");

        let locator = route_frame(
            "neo:screenshot",
            json!({"selector": "#a", "strict": true, "path": "a.png", "omitBackground": true}),
        );
        assert_eq!(
            locator.meta.map(|meta| (meta.method, meta.title)),
            Some(("neo:screenshot", Some("Screenshot")))
        );
        match locator.route {
            Ok(ObjectCall::Capture(Capture::Locator(params))) => {
                assert_eq!(params.selector, "#a");
                assert_eq!(params.screenshot.path.as_deref(), Some("a.png"));
                assert_eq!(params.screenshot.omit_background, Some(true));
            }
            Ok(_) => panic!("expected a locator screenshot"),
            Err(error) => panic!("expected a locator screenshot, got {error:?}"),
        }
        let handle = route(
            ObjectKind::Handle("handle@run.1".to_string()),
            "screenshot",
            Some(json!({"type": "jpeg", "quality": 50, "path": "a.jpg"})),
        );
        assert_eq!(
            handle
                .meta
                .map(|meta| (meta.interface, meta.method, meta.title)),
            Some(("ElementHandle", "screenshot", Some("Screenshot")))
        );
        match handle.route {
            Ok(ObjectCall::Capture(Capture::Element { guid, params })) => {
                assert_eq!(guid, "handle@run.1");
                assert_eq!(params.quality, Some(50));
                assert_eq!(params.path.as_deref(), Some("a.jpg"));
            }
            Ok(_) => panic!("expected an element handle screenshot"),
            Err(error) => panic!("expected an element handle screenshot, got {error:?}"),
        }
        // Every other handle method is still the handle's evaluation (`evaluate.rs`).
        assert!(matches!(
            route(
                ObjectKind::Handle("handle@run.1".to_string()),
                "jsonValue",
                Some(json!({})),
            )
            .route,
            Ok(ObjectCall::Frame(Route::FrameEval(_)))
        ));
        let no_selector = error_of(route_frame("neo:screenshot", json!({})));
        assert!(
            no_selector.message.contains("neo:screenshot"),
            "{no_selector:?}"
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
    fn tests_that_only_a_waits_finish_and_release_skip_the_in_flight_caps() {
        let ends = |method: &str, params: Value| {
            let route = route_frame(method, params)
                .route
                .unwrap_or_else(|error| panic!("{method}: {error:?}"));
            ending_cursor(&route)
        };
        assert_eq!(
            ends(
                "neo:finishNavigation",
                json!({"cursor":4,"index":2,"waitUntil":"load"})
            ),
            Some(4)
        );
        assert_eq!(ends("neo:releaseNavigation", json!({"cursor":5})), Some(5));
        assert_eq!(
            ends("neo:nextNavigation", json!({"cursor":4,"response":false})),
            None,
            "a next waits like any call"
        );
        assert_eq!(ends("click", json!({"selector":"#nothing"})), None);
        let cursors = Arc::new(Cursors::<FrameCursor>::default());
        assert!(
            cursors.begin_ending(4).is_none(),
            "a cursor that is not open claims nothing, so its call is admitted as any other"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tests_that_a_call_gives_its_share_back_before_its_reply_is_sent() {
        let in_flight = Arc::new(InFlight::default());
        let entry = Arc::new(CallEntry::new(CallStart {
            call_id: 1,
            guid: format!("frame@{PAGE}"),
            method: "neo:nextNavigation".to_string(),
            title: Some("Wait for navigation"),
            params: json!({"response":false}),
            location: None,
            input: false,
        }));
        let trace = Trace::default();
        trace.push(Arc::clone(&entry));
        let (reply, receiver) = oneshot::channel();
        let replied = tokio::spawn(finish_reply(
            Ok(None),
            CallReply {
                entry,
                trace,
                id: json!(1),
                reply,
                events: EventHub::default(),
                share: Some(in_flight.admit(10).expect("under the caps")),
                ending: None,
                resolving: None,
                abortable: None,
            },
        ));
        let _reply = receiver.await.expect("the call replied");
        let held = in_flight.0.lock().map(|load| load.calls).ok();
        assert_eq!(
            held,
            Some(0),
            "a script that awaited the reply finds its capacity free"
        );
        replied.await.expect("finish_reply");
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
            "expression": "ee => ee.map(e => e.textContent || '')",
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
        // `allTextContents` is Playwright's `$$eval`: the evaluation route serves it in the main
        // world.
        let Ok(ObjectCall::Frame(Route::FrameEval(EvalCall::EvalOnSelectorAll {
            selector,
            scope: None,
            ..
        }))) = route_frame("evalOnSelectorAll", all_text_contents_params(".item")).route
        else {
            panic!("allTextContents routes to the evaluation route");
        };
        assert_eq!(selector, ".item");
        let mut inner_texts = all_text_contents_params(".fruit");
        inner_texts["expression"] = json!("ee => ee.map(e => e.innerText)");
        let Ok(ObjectCall::Frame(Route::FrameEval(EvalCall::EvalOnSelectorAll {
            selector,
            scope: None,
            ..
        }))) = route_frame("evalOnSelectorAll", inner_texts).route
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
    fn tests_that_collection_eval_serves_any_expression_as_playwrights_eval_on_selector_all() {
        let mut other_expression = all_text_contents_params(".item");
        other_expression["expression"] = json!("ee => ee.length");
        let mut not_a_function = all_text_contents_params(".item");
        not_a_function["isFunction"] = json!(false);
        let mut absent_function = all_text_contents_params(".item");
        absent_function
            .as_object_mut()
            .expect("params")
            .remove("isFunction");
        for params in [other_expression, not_a_function, absent_function] {
            assert!(
                matches!(
                    route_frame("evalOnSelectorAll", params).route,
                    Ok(ObjectCall::Frame(Route::FrameEval(
                        EvalCall::EvalOnSelectorAll { .. }
                    )))
                ),
                "every $$eval is served"
            );
        }
        let mut missing_arg = all_text_contents_params(".item");
        missing_arg.as_object_mut().expect("params").remove("arg");
        assert_eq!(
            error_of(route_frame("evalOnSelectorAll", missing_arg)).name,
            ErrorName::Error
        );
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
        assert!(
            route_frame("evalOnSelectorAll", params).route.is_ok(),
            "a frame-crossing $$eval is served"
        );
    }

    #[test]
    fn tests_that_collection_evaluation_accepts_handle_guids() {
        let mut params = all_text_contents_params(".item");
        params["arg"]["handles"] = json!(["handle@1"]);
        assert!(route_frame("evalOnSelectorAll", params).route.is_ok());
        for handle in [json!({"guid": "handle@1"}), json!(null)] {
            let mut params = all_text_contents_params(".item");
            params["arg"]["handles"] = json!([handle]);
            assert!(route_frame("evalOnSelectorAll", params).route.is_err());
        }
    }

    #[test]
    fn tests_that_count_replies_have_playwrights_generated_shape() {
        use neo_runtime_spec::serve::FrameQueryCountResult;

        let result = encode_result::<FrameQueryCountParams>(&FrameQueryCountResult { value: 3 })
            .expect("count encodes");
        let reply: Value =
            serde_json::from_str(&encode_reply(&json!(7), Ok(result.as_ref()), None))
                .expect("reply");
        assert_eq!(reply, json!({"id": 7, "result": {"value": 3}}));
    }
}

#[cfg(test)]
mod review_abort_priority {
    use super::*;
    use crate::deadline::Deadline;
    use crate::fake_browser::{FakeBrowser, PAGE, Replies};
    use crate::page::TabInputs;
    use crate::run::PageId;
    use crate::screenshot::TabScreenshots;
    use crate::session_state::Permissions;
    use crate::switch::Switches;
    use tokio::runtime::Handle;
    use tokio_util::task::TaskTracker;

    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_end_wins_over_an_abort_before_the_handler_starts() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let tasks = TaskTracker::new();
        let run = RunScope::new(
            Deadline::new(Duration::from_secs(30), CancellationToken::new()),
            tasks.clone(),
        );
        let (tabs, _) = Tabs::open(
            browser.cdp().await,
            Handle::current(),
            TabInputs::default(),
            TabScreenshots::default(),
            Switches::default(),
            Permissions::default(),
            tasks,
            &PageId::new(PAGE.to_string()),
        )
        .await
        .unwrap_or_else(|error| panic!("opening the fake tab: {error:?}"));
        let (page, scope) = tabs
            .resolve(&format!("frame@{PAGE}"))
            .unwrap_or_else(|| panic!("the fake tab must have a main frame"));
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
        let route = route(
            ObjectKind::Frame,
            "click",
            Some(json!({"selector":"#nothing"})),
        )
        .route
        .unwrap_or_else(|error| panic!("decoding the click: {error:?}"));
        let (reply, _receiver) = oneshot::channel();
        let call = serve_call(
            tabs,
            Arc::default(),
            Some((page, scope)),
            route,
            progress,
            CallReply {
                entry: Arc::clone(&entry),
                trace: trace.clone(),
                id: json!(1),
                reply,
                events: EventHub::default(),
                share: None,
                ending: None,
                resolving: None,
                abortable: None,
            },
        );
        // Neither stop can race scheduling: the handler future has never been polled.
        entry.abort("stop call".to_string());
        run.end(RunEnd::Settled);
        call.await;
        let (records, _) = trace.take(RunEnd::Settled);
        assert_eq!(
            records[0].result,
            CallResult::Unfinished {
                reason: RunEnd::Settled
            },
            "run end has priority over the signal abort at admission"
        );
    }
}
