//! The background task that turns CDP's target, navigation, dialog and console events into the
//! event log's records. One per `Playwright`, and the log's only writer.
//!
//! - **Discovery.** On start, after a lag in the event stream and on every connection epoch
//!   change: `Target.getTargets` (the tabs that already exist are never reported as opened),
//!   then `Target.setDiscoverTargets` on the root session (nothing in core sends it) and
//!   `Browser.setDownloadBehavior { behavior: 'default', eventsEnabled }` (what core's own
//!   download tool sends), so downloads keep Chrome's destination and report events.
//! - **Downloads.** `Browser.downloadWillBegin` lists the download in the hub's download table
//!   before the `download` record names it, so an op handed that record always finds it there;
//!   `Browser.downloadProgress` reports how it ended, with the saved file's path.
//! - **New tabs.** `Target.targetCreated` has no URL yet (measured, every popup kind), so a tab
//!   is recorded once `Target.targetInfoChanged` gives it one, or after 1 s with `about:blank`.
//!   A tab with an opener is a popup: a short task looks up its window type first, so a popup
//!   that opened as a separate window is recorded as one (D9), and reports back here.
//! - **Sessions.** Page events arrive on page sessions. `Target.attachedToTarget` names each
//!   session's tab (core's attaches and its OOPIF auto-attaches alike); a session attached
//!   before the watcher started is looked up in core's page registry.
//! - **Dialogs.** An `alert()` is already accepted by core's page signals (a listed deviation).
//!   Any other dialog is held for the runs listening for dialogs on its tab, then
//!   [`DIALOG_GRACE`] later held for whoever listens by then, or closed by Neo; the reap tick
//!   closes held dialogs whose runs all ended (`dialog_table.rs`).
//! - **Frames.** `Page.frameAttached`, a removing `Page.frameDetached` and the main frame's
//!   `Page.frameNavigated` go to the `FrameAttachLog`, from one session per native target, so frames are
//!   numbered in the order Playwright numbers them (`world/frame_attach_log.rs`). That commit
//!   and a closed tab also drop the tab's snapshot baseline, and a lag or a new connection drops
//!   all of them (`SnapshotBaselines`), so a delta never diffs a new document against the one it
//!   replaced.
//! - **Binding calls.** `Runtime.bindingCalled` for upstream's `__playwright__binding__` goes,
//!   while some run exposes a binding on that tab, to each such run's own queue, never the
//!   bounded log, so no call the page awaits is evicted unread. It comes on a channel of its own
//!   (`events_targeted`), unbounded, so a call is never lost to the broadcast ring's lag and a
//!   page never waits forever on an answer nobody heard; the broadcast is drained first, so the
//!   `Runtime.executionContextCreated` that named the caller's frame is already in hand.
//! - **Workers.** Each dedicated worker the page sessions auto-attach is armed to auto-attach its
//!   own nested workers and released from its start-up pause, with `Runtime` left off: while it
//!   is on, every console call in the worker serializes its arguments for the inspector, which a
//!   site can time to detect an open inspector. A worker session takes a `Runtime` lease only while some
//!   run listens for console messages or page errors on its tab ([`WorkerRuntime`]).
//! - **Console and page errors** are recorded only while some run listens for them on that tab,
//!   so a chatty page cannot push popups and dialogs out of the ring. In hosted mode they never
//!   fire, by design: the browser reports neither, even with `Runtime` on (measured: no
//!   `Runtime.consoleAPICalled` and no `Runtime.exceptionThrown` with
//!   `BROWSERCLAW_HOSTED_MODE=1`), and Neo leases nothing for them. The browser's own log
//!   entries reach no listener there either: one is passed on only under a console lease on its
//!   tab (`event_op.rs`, `lease_runtime`, says why).

use std::collections::{HashMap, HashSet, VecDeque};
use std::mem;
use std::sync::Arc;
use std::time::Duration;

use browseros_cdp::CdpEvent;
use browseros_core::{
    BrowserSession, BrowserView, CoreError, FrameId, ProtocolSession, RuntimeLease, SessionId,
    TabId, TargetId,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::{self, Instant, MissedTickBehavior};

use crate::events::dialog_table::{ClosedBy, HeldDialog, Unanswered};
use crate::events::event_hub::EventWriter;
use crate::events::frame_lifecycle::FrameLifecycleTracker;
use crate::events::log_lease::LogLease;
use crate::events::page_event::{
    BindingCalled, ConsoleMessage, ConsoleParams, DialogId, DialogKind, DialogOpened,
    DownloadStarted, ExceptionParams, FileChooserOpened, LogEntryParams, OpenedPage, PageError,
    PageEvent, WindowKind,
};
use crate::events::recorded_reports::{RecordedReports, ReportKey};
use crate::events::worker_table::WorkerContext;
use crate::events::{PageEventKind, WatcherProgress, navigation_event};
use crate::snapshot::SnapshotBaselines;
use crate::world::{BackendNodeId, FrameAttachLog, FrameEvent};

/// upstream's `PageBinding.kBindingName`: the CDP binding every exposed binding's stub calls.
pub(crate) const BINDING_NAME: &str = "__playwright__binding__";

/// The event a call of [`BINDING_NAME`] arrives as, on a targeted channel of its own.
pub(crate) const BINDING_CALLED: &str = "Runtime.bindingCalled";

/// How long a dialog nobody listens for stays open, for a listener that lands just after the
/// click that opened it (build-design graft 7).
const DIALOG_GRACE: Duration = Duration::from_millis(250);

/// How long a new tab may go without a URL before it is recorded anyway.
const URL_WAIT: Duration = Duration::from_secs(1);

/// How often held dialogs are checked for runs that ended.
const REAP: Duration = Duration::from_secs(1);

/// The watcher's clock for discovery retries and URL deadlines.
const TICK: Duration = Duration::from_millis(250);

/// The running watcher. Dropping it (with its `Playwright`) stops the task.
#[derive(Debug)]
pub(crate) struct TargetWatcher {
    task: Option<JoinHandle<()>>,
    requests: UnboundedSender<WatcherRequest>,
}

impl Drop for TargetWatcher {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

#[derive(Debug)]
struct WatcherRequest {
    closed: Option<TargetId>,
    done: oneshot::Sender<()>,
}

/// A handle on the running watcher for what outlives an op: a console lease's release has it
/// take the frames' `Log` off before `Runtime` goes (`log_lease.rs`).
#[derive(Clone, Debug)]
pub(crate) struct WatcherHandle {
    requests: UnboundedSender<WatcherRequest>,
}

impl WatcherHandle {
    /// Waits, at most 2 s, for the watcher to handle the events received so far and bring
    /// every worker's `Runtime` and frame's `Log` in line with who listens now. A watcher that
    /// is gone answers at once.
    pub(crate) async fn synchronize(&self) {
        let (done, wait) = oneshot::channel();
        if self
            .requests
            .send(WatcherRequest { closed: None, done })
            .is_ok()
        {
            let _ = time::timeout(Duration::from_secs(2), wait).await;
        }
    }

    /// A handle no watcher answers, for tests.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        let (requests, _gone) = mpsc::unbounded_channel();
        Self { requests }
    }
}

impl TargetWatcher {
    /// A handle on this watcher that outlives the op holding it ([`WatcherHandle`]).
    pub(crate) fn handle(&self) -> WatcherHandle {
        WatcherHandle {
            requests: self.requests.clone(),
        }
    }

    /// Foreground replies must see every CDP fact received before their command replied.
    /// A confirmed explicit close also retires workers before Chromium finishes tearing
    /// down a renderer with unresolved promises (Playwright issue 43204).
    pub(crate) async fn synchronize(&self, closed: Option<TargetId>) {
        let (done, wait) = oneshot::channel();
        if self.requests.send(WatcherRequest { closed, done }).is_ok() {
            let _ = time::timeout(Duration::from_secs(2), wait).await;
        }
    }

    /// Starts watching `browser`, recording page events through `writer` and frame lifecycles
    /// into `attach_log`, dropping a tab's entry in `baselines` when its document goes, and
    /// counting what it handled into `progress`. Outside a tokio runtime
    /// there is nothing to run it on, so nothing is recorded and no one waits for it.
    pub(crate) fn spawn(
        browser: Arc<BrowserSession>,
        writer: EventWriter,
        attach_log: Arc<FrameAttachLog>,
        progress: Arc<WatcherProgress>,
        baselines: Arc<SnapshotBaselines>,
    ) -> Self {
        let (requests, pending) = mpsc::unbounded_channel();
        let Ok(runtime) = Handle::try_current() else {
            progress.stopped();
            return Self {
                task: None,
                requests,
            };
        };
        let (watcher, found) = Watcher::new(browser, writer, attach_log, progress, baselines);
        Self {
            task: Some(runtime.spawn(watcher.run(found, pending))),
            requests,
        }
    }
}

/// The task's state. Owned by the task alone, so nothing here needs a lock.
struct Watcher {
    browser: Arc<BrowserSession>,
    /// An unbound view, for core's session-to-page registry.
    view: BrowserView,
    writer: EventWriter,
    attach_log: Arc<FrameAttachLog>,
    /// Each tab's snapshot baseline: forgotten when the tab's main frame commits a new
    /// document or the tab closes, all of them after a lag.
    baselines: Arc<SnapshotBaselines>,
    progress: Arc<WatcherProgress>,
    lifecycle: FrameLifecycleTracker,
    /// The connection epoch discovery ran for; `None` until it succeeds, and after a lag.
    epoch: Option<u64>,
    /// Retained across lag-driven rediscovery, so reconnect still invalidates endpoints.
    connection_epoch: Option<u64>,
    /// The tabs the watcher knows.
    pages: HashMap<TargetId, KnownPage>,
    /// Popups whose window lookup was in flight when they closed.
    closed_early: HashSet<TargetId>,
    /// Which tab each page or OOPIF session belongs to.
    sessions: HashMap<SessionId, TargetId>,
    /// Core's own page sessions, which it attaches with `Runtime` off: a console report on one
    /// that no lease holds on is a replay another toggle caused (core's console harvest).
    page_sessions: HashSet<SessionId>,
    /// Only the chosen auto-attach endpoint publishes a worker's console and close.
    worker_sessions: HashMap<SessionId, String>,
    /// `Runtime` on each chosen worker session: off unless a run listens on its tab.
    worker_runtime: HashMap<SessionId, WorkerRuntime>,
    /// `Log` on each out-of-process frame's session: off unless a run listens for console
    /// messages on its tab.
    frame_logs: HashMap<SessionId, FrameLog>,
    /// The console messages and exceptions the log recorded, so a lease's replay of one is
    /// dropped by identity.
    recorded: RecordedReports,
    /// Older Chromium has no workerScriptLoaded event and uses context creation instead.
    worker_script_events: Option<bool>,
    /// Watcher-owned root sessions only auto-attach descendants; page Runtime stays off.
    root_observers: HashMap<SessionId, RootObserver>,
    /// Core may attach concurrently. One source per native target prevents each CDP session
    /// from publishing the same lifecycle transition; OOPIFs have their own native target.
    lifecycle_sources: LifecycleSources,
    /// Which tab each out-of-process frame belongs to (an OOPIF's frame id is its target id).
    frames: HashMap<String, TargetId>,
    /// The frame of each main-world execution context, by session and context id, from
    /// `Runtime.executionContextCreated`: where a binding call came from.
    contexts: HashMap<(SessionId, i64), FrameId>,
    /// Dialogs in their grace, oldest first; every grace is as long, so this is by deadline.
    graces: VecDeque<Grace>,
    /// Opening metadata survives automatic answers; CDP close carries no dialog identity.
    pending_dialogs: HashMap<SessionId, DialogOpened>,
    next_reap: Instant,
    /// Window lookups report their popup back on this channel, so only this task writes the
    /// log. The receiving end is a local of the task's loop.
    found: UnboundedSender<OpenedPage>,
}

/// A worker session's `Runtime` reporting, which its console messages and uncaught errors need.
/// While it is on, every console call in the worker serializes its arguments for the inspector,
/// and a site can time that to detect an open inspector, so it is on only while some run listens for
/// console messages or page errors on the worker's tab. `worker.evaluate` needs none of it.
enum WorkerRuntime {
    /// Off: no run asks.
    Off,
    /// On until the lease drops: the session's last lease switches `Runtime` off again.
    On { _lease: RuntimeLease },
}

/// An out-of-process frame session's `Log` reporting. Upstream enables `Log` on every frame
/// session, and a frame's own entries arrive only there: a frame refused for `X-Frame-Options`
/// reports the refusal on its session as it attaches (measured), and a later `Log.enable`
/// replays it. On only while some run listens for console messages on the frame's tab, whose
/// page session holds `Runtime` and `Log` for that listener meanwhile (`log_lease.rs`), and
/// only on the frame's chosen session ([`LifecycleSources`]), so an entry is recorded once.
enum FrameLog {
    /// Off: no run asks, or another session of the frame reports.
    Off,
    /// On until the lease drops: the session's last lease switches `Log` off again.
    On { lease: LogLease },
}

/// The watcher owns only this root attachment, fenced to the socket that created it.
struct RootObserver {
    target: TargetId,
    root: ProtocolSession,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let observers = mem::take(&mut self.root_observers);
        if observers.is_empty() {
            return;
        }
        if let Ok(runtime) = Handle::try_current() {
            runtime.spawn(async move {
                for (session, observer) in observers {
                    let _ = time::timeout(
                        Duration::from_secs(1),
                        observer.root.send_value(
                            "Target.detachFromTarget",
                            json!({"sessionId": session.as_str()}),
                        ),
                    )
                    .await;
                }
            });
        }
    }
}

struct KnownPage {
    opener: Option<TargetId>,
    /// The native tab id, which the popup's record carries (`OpenedPage::tab`).
    tab: Option<TabId>,
    state: PageState,
}

enum PageState {
    /// Recorded as opened, or open when discovery ran: its closing is recorded.
    Reported,
    /// Created and waiting for a URL until `deadline`.
    Pending {
        deadline: Instant,
        window_id: Option<i64>,
    },
    /// A popup whose window type is being looked up.
    LookingUp,
}

struct Grace {
    ends: Instant,
    dialog: DialogId,
    target: Option<TargetId>,
}

/// The fields of CDP's `Target.TargetInfo` the watcher reads.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetInfo {
    target_id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    parent_frame_id: Option<String>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    opener_id: Option<String>,
    #[serde(default)]
    subtype: Option<String>,
    /// BrowserOS's: the window that holds the tab.
    #[serde(default)]
    window_id: Option<i64>,
    /// BrowserOS's: the tab's native id, already on a popup's `targetCreated` (measured).
    #[serde(default)]
    tab_id: Option<TabId>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetEvent {
    target_info: TargetInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Attached {
    session_id: SessionId,
    target_info: TargetInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Detached {
    session_id: SessionId,
}

/// `Target.targetDestroyed`'s params, and the target `Target.targetCrashed` names.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Destroyed {
    target_id: String,
}

/// `Runtime.executionContextCreated`, for the frame of a page's main-world contexts.
#[derive(Deserialize)]
struct ContextCreated {
    context: ContextDescription,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextDescription {
    id: i64,
    #[serde(default)]
    aux_data: ContextAux,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextAux {
    #[serde(default)]
    is_default: bool,
    #[serde(default)]
    frame_id: Option<FrameId>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextDestroyed {
    execution_context_id: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DialogParams {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    default_prompt: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameAttached {
    frame_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameDetached {
    frame_id: String,
    /// `remove` or `swap` (the frame moved to another process and lives on).
    #[serde(default)]
    reason: String,
}

#[derive(Deserialize)]
struct FrameNavigated {
    frame: NavigatedFrame,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NavigatedFrame {
    id: String,
    #[serde(default)]
    url: String,
    /// The `#...` part, which CDP reports apart from `url`.
    #[serde(default)]
    url_fragment: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileChooserParams {
    frame_id: FrameId,
    backend_node_id: Option<BackendNodeId>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadParams {
    frame_id: String,
    guid: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    suggested_filename: String,
}

/// `Browser.downloadProgress`. `filePath` comes with `completed` only.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProgressParams {
    guid: String,
    state: String,
    #[serde(default)]
    file_path: Option<String>,
}

impl Watcher {
    fn new(
        browser: Arc<BrowserSession>,
        writer: EventWriter,
        attach_log: Arc<FrameAttachLog>,
        progress: Arc<WatcherProgress>,
        baselines: Arc<SnapshotBaselines>,
    ) -> (Self, UnboundedReceiver<OpenedPage>) {
        let (found, found_rx) = mpsc::unbounded_channel();
        let watcher = Self {
            view: BrowserView::unbound(Arc::clone(&browser), None),
            browser,
            writer,
            attach_log,
            baselines,
            progress,
            lifecycle: FrameLifecycleTracker::default(),
            epoch: None,
            connection_epoch: None,
            pages: HashMap::new(),
            closed_early: HashSet::new(),
            sessions: HashMap::new(),
            page_sessions: HashSet::new(),
            worker_sessions: HashMap::new(),
            worker_runtime: HashMap::new(),
            frame_logs: HashMap::new(),
            recorded: RecordedReports::default(),
            worker_script_events: None,
            root_observers: HashMap::new(),
            lifecycle_sources: LifecycleSources::default(),
            frames: HashMap::new(),
            contexts: HashMap::new(),
            graces: VecDeque::new(),
            pending_dialogs: HashMap::new(),
            next_reap: Instant::now() + REAP,
            found,
        };
        (watcher, found_rx)
    }

    async fn run(
        mut self,
        mut found: UnboundedReceiver<OpenedPage>,
        mut requests: UnboundedReceiver<WatcherRequest>,
    ) {
        // Counters and subscriptions are captured together: progress excludes messages
        // that arrived before this watcher subscribed, including the binding stream.
        let (mut events, before) = self.browser.cdp_events_counted();
        let (mut bindings, calls_before) = self.browser.cdp_events_targeted_counted(BINDING_CALLED);
        self.progress.start(before, calls_before);
        let mut tick = time::interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            let grace_ends = self.graces.front().map(|grace| grace.ends);
            tokio::select! {
                received = events.recv() => match received {
                    Ok(event) => {
                        self.handle(event).await;
                        self.progress.handled(1);
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        self.lost_events();
                        self.progress.handled(skipped);
                    }
                    Err(RecvError::Closed) => {
                        self.progress.stopped();
                        return;
                    }
                },
                Some(call) = bindings.recv() => {
                    // Every event sent before the call, its context's creation among them.
                    self.drain(&mut events).await;
                    self.binding_called(call).await;
                    self.progress.call_handled();
                }
                Some(request) = requests.recv() => {
                    // Initial synchronous page facts need attachments even for tabs the
                    // run has never acted on; otherwise their pre-existing workers vanish.
                    if self.epoch.is_none() { self.tick().await; }
                    self.drain(&mut events).await;
                    if let Some(target) = request.closed {
                        for worker in self.writer.hub().workers.list(&target) { self.close_worker(&worker.id); }
                    }
                    // Every op synchronizes before it replies, so a listener the op added has
                    // its tab's workers and frames reporting before the script's next call.
                    self.lease_worker_runtime().await;
                    self.lease_frame_logs().await;
                    let _ = request.done.send(());
                }
                Some(opened) = found.recv() => self.found(opened),
                _ = tick.tick() => self.tick().await,
                () = time::sleep_until(grace_ends.unwrap_or_else(Instant::now)), if grace_ends.is_some() => {
                    self.end_graces();
                }
            }
        }
    }

    /// Keeps [`Watcher::contexts`] current: a main-world context made, gone, or every context of
    /// the session gone (a navigation clears them). Any other event changes nothing.
    fn context_changed(&mut self, method: &str, session: Option<SessionId>, params: Value) {
        let Some(session) = session else {
            return;
        };
        match method {
            "Inspector.workerScriptLoaded" => self.writer.hub().workers.script_loaded(&session),
            "Runtime.executionContextCreated" => {
                if let Some(ContextCreated { context }) = parse(params)
                    && context.aux_data.is_default
                    && let Some(frame) = context.aux_data.frame_id
                {
                    self.contexts.insert((session, context.id), frame);
                }
            }
            "Runtime.executionContextDestroyed" => {
                if let Some(ContextDestroyed {
                    execution_context_id,
                }) = parse(params)
                {
                    self.contexts.remove(&(session, execution_context_id));
                }
            }
            "Runtime.executionContextsCleared" => {
                self.contexts.retain(|(known, _), _| *known != session);
            }
            _ => {}
        }
    }

    /// A session went: its tab and its contexts are forgotten.
    fn session_detached(&mut self, session: &SessionId) {
        self.recorded.forget(session);
        self.root_observers.remove(session);
        self.worker_runtime.remove(session);
        self.frame_logs.remove(session);
        if let Some(id) = self.worker_sessions.remove(session) {
            self.close_worker(&id);
        }
        self.contexts.retain(|(known, _), _| known != session);
        self.sessions.remove(session);
        self.page_sessions.remove(session);
    }

    /// Events were dropped: rediscover, which records what closed meanwhile, and tell the frame
    /// log that commits may be missing, and the snapshot baselines that any tab's document may
    /// have changed unseen.
    fn lost_events(&mut self) {
        self.epoch = None;
        self.attach_log.lost_events();
        self.baselines.events_lost();
    }

    /// Handles every broadcast event already queued, without waiting for more.
    async fn drain(&mut self, events: &mut Receiver<CdpEvent>) {
        loop {
            // Counted as the run loop counts, so ops waiting on `progress` see these too.
            match events.try_recv() {
                Ok(event) => {
                    self.handle(event).await;
                    self.progress.handled(1);
                }
                Err(TryRecvError::Lagged(skipped)) => {
                    self.lost_events();
                    self.progress.handled(skipped);
                }
                Err(TryRecvError::Empty | TryRecvError::Closed) => return,
            }
        }
    }

    /// Records a call of [`BINDING_NAME`] on a tab where some run exposes a binding.
    async fn binding_called(&mut self, event: CdpEvent) {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Called {
            name: String,
            payload: String,
            execution_context_id: i64,
        }
        let Some(session) = event.session_id else {
            return;
        };
        let Some(called) = parse::<Called>(event.params) else {
            return;
        };
        if called.name != BINDING_NAME {
            return;
        }
        if let Some(target) = self.wanted(&session, PageEventKind::Binding).await {
            // The main frame's id is its target's; the pump names it `null`.
            let frame = self
                .contexts
                .get(&(session, called.execution_context_id))
                .filter(|frame| frame.0 != target.0)
                .cloned();
            self.writer.deliver_binding(&BindingCalled {
                target,
                context: called.execution_context_id,
                frame,
                payload: called.payload,
            });
        }
    }

    async fn tick(&mut self) {
        let epoch = self.browser.connection().connection_epoch();
        if self.epoch != Some(epoch) && self.browser.is_connected() {
            let changed = self.connection_epoch.is_some_and(|known| known != epoch);
            if self.discover(changed).await.is_ok() {
                self.epoch = Some(epoch);
                self.connection_epoch = Some(epoch);
            }
        }
        self.observe_context().await;
        let now = Instant::now();
        let overdue: Vec<(TargetId, Option<i64>)> = self
            .pages
            .iter()
            .filter_map(|(target, page)| match page.state {
                PageState::Pending {
                    deadline,
                    window_id,
                } if deadline <= now => Some((target.clone(), window_id)),
                PageState::Pending { .. } | PageState::Reported | PageState::LookingUp => None,
            })
            .collect();
        for (target, window_id) in overdue {
            self.ready(&target, "about:blank".to_owned(), window_id);
        }
        if now >= self.next_reap {
            self.next_reap = now + REAP;
            let abandoned = self.writer.hub().dialogs().reap();
            for dialog in abandoned {
                self.close_unanswered(dialog);
            }
        }
        // Catches a `waitForEvent` that started mid-op, and lets go once a run stops listening.
        self.lease_worker_runtime().await;
        self.lease_frame_logs().await;
    }

    /// Learns the tabs that exist and turns discovery on. `changed`: the connection is a new
    /// one, so every session id the watcher knew died with the old socket.
    async fn discover(&mut self, changed: bool) -> Result<(), CoreError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Targets {
            target_infos: Vec<TargetInfo>,
        }
        let targets: Targets = decode(
            self.browser
                .cdp("Target.getTargets", json!({}), None)
                .await?,
        )?;
        if self.connection_epoch.is_some() && !changed {
            self.recover_workers(&targets.target_infos).await;
        }
        // Reconcile worker lifetimes after broadcast lag, just as discovery reconciles tabs.
        let live_workers: HashSet<&str> = targets
            .target_infos
            .iter()
            .filter(|info| info.kind == "worker")
            .map(|info| info.target_id.as_str())
            .collect();
        let gone_workers: Vec<String> = self
            .worker_sessions
            .values()
            .filter(|id| !live_workers.contains(id.as_str()))
            .cloned()
            .collect();
        for id in gone_workers {
            self.close_worker(&id);
        }
        let live: HashSet<TargetId> = targets
            .target_infos
            .into_iter()
            .filter(is_tab)
            .map(|info| TargetId(info.target_id))
            .collect();
        let gone: Vec<TargetId> = self
            .pages
            .keys()
            .filter(|target| !live.contains(*target))
            .cloned()
            .collect();
        for target in gone {
            self.destroyed(&target);
        }
        for target in live {
            self.pages.entry(target).or_insert(KnownPage {
                opener: None,
                tab: None,
                state: PageState::Reported,
            });
        }
        if changed {
            // Whatever the tabs did while the old socket was down went unseen.
            self.baselines.events_lost();
            self.lifecycle = FrameLifecycleTracker::default();
            for id in self.worker_sessions.values().cloned().collect::<Vec<_>>() {
                self.close_worker(&id);
            }
            self.worker_sessions.clear();
            self.worker_runtime.clear();
            self.frame_logs.clear();
            self.recorded.clear();
            self.worker_script_events = None;
            self.root_observers.clear();
            self.sessions.clear();
            self.lifecycle_sources = LifecycleSources::default();
            self.frames.clear();
            self.contexts.clear();
            self.pending_dialogs.clear();
            self.writer.hub().dialogs().forget_sessions();
        }
        self.browser
            .cdp(
                "Target.setDiscoverTargets",
                json!({ "discover": true }),
                None,
            )
            .await?;
        // Older natives may refuse it; downloads then simply report nothing.
        let _ = self
            .browser
            .cdp(
                "Browser.setDownloadBehavior",
                json!({ "behavior": "default", "eventsEnabled": true }),
                None,
            )
            .await;
        Ok(())
    }

    async fn handle(&mut self, event: CdpEvent) {
        let CdpEvent {
            method,
            params,
            session_id,
        } = event;
        match method.as_str() {
            "Page.frameAttached"
            | "Page.frameNavigated"
            | "Page.navigatedWithinDocument"
            | "Page.frameDetached" => {
                self.frame_changed(&method, session_id.as_ref(), params)
                    .await;
            }
            "Target.targetCreated" => {
                if let Some(TargetEvent { target_info }) = parse(params) {
                    self.created(target_info);
                    self.observe_context().await;
                }
            }
            "Target.targetInfoChanged" => {
                if let Some(TargetEvent { target_info }) = parse(params) {
                    self.info_changed(target_info);
                }
            }
            "Target.targetDestroyed" => {
                if let Some(Destroyed { target_id }) = parse(params) {
                    let target = TargetId(target_id);
                    self.destroyed(&target);
                }
            }
            "Target.targetCrashed" => {
                if let Some(Destroyed { target_id }) = parse(params) {
                    self.crashed(TargetId(target_id));
                }
            }
            "Target.attachedToTarget" => {
                if let Some(attached) = parse(params) {
                    self.attached(session_id.as_ref(), attached).await;
                }
            }
            "Target.detachedFromTarget" => {
                if let Some(Detached { session_id }) = parse(params) {
                    self.dialog_closed(&session_id);
                    self.session_detached(&session_id);
                    self.lifecycle_sources.detach(&session_id);
                }
            }
            "Page.loadEventFired" | "Page.domContentEventFired" => {
                if let Some(session) = session_id {
                    self.page_load(&method, &session).await;
                }
            }
            "Page.javascriptDialogOpening" => {
                if let (Some(session), Some(dialog)) = (session_id, parse(params)) {
                    self.dialog_opened(session, dialog).await;
                }
            }
            "Page.fileChooserOpened" => {
                // Core owns interception on agent tabs and their remote descendants. Keep
                // every chooser in the bounded log: a wait may be admitted after its click,
                // but its synchronous `since` stamp still predates this event.
                if let Some(session) = session_id
                    && let Some(target) = self.target_of(&session).await
                    && let Some(params) = parse::<FileChooserParams>(params)
                    && let Some(node) = params.backend_node_id
                {
                    self.writer
                        .record(PageEvent::FileChooser(FileChooserOpened {
                            target,
                            session,
                            frame: params.frame_id,
                            node,
                        }));
                }
            }
            "Page.javascriptDialogClosed" => {
                if let Some(session) = session_id {
                    self.dialog_closed(&session);
                }
            }
            "Runtime.consoleAPICalled" | "Runtime.exceptionThrown" => {
                if let Some(session) = session_id {
                    self.runtime_report(&method, session, params).await;
                }
            }
            "Log.entryAdded" => {
                if let Some(session) = session_id {
                    self.log_entry(session, params).await;
                }
            }
            "Browser.downloadWillBegin" => {
                if let Some(download) = parse(params) {
                    self.download(download);
                }
            }
            "Browser.downloadProgress" => self.download_progress(params),
            _ => self.context_changed(&method, session_id, params),
        }
    }

    fn created(&mut self, info: TargetInfo) {
        let target = TargetId(info.target_id.clone());
        if !is_tab(&info) || self.pages.contains_key(&target) {
            return;
        }
        self.pages.insert(
            target.clone(),
            KnownPage {
                opener: info.opener_id.map(TargetId),
                tab: info.tab_id,
                state: PageState::Pending {
                    deadline: Instant::now() + URL_WAIT,
                    window_id: info.window_id,
                },
            },
        );
        if !info.url.is_empty() {
            self.ready(&target, info.url, info.window_id);
        }
    }

    fn info_changed(&mut self, info: TargetInfo) {
        let target = TargetId(info.target_id);
        let Some(page) = self.pages.get_mut(&target) else {
            return;
        };
        page.tab = info.tab_id.or(page.tab);
        let pending = matches!(page.state, PageState::Pending { .. });
        if pending && !info.url.is_empty() {
            self.ready(&target, info.url, info.window_id);
        }
    }

    /// A new tab has its URL: recorded now, or once a popup's window type is known.
    fn ready(&mut self, target: &TargetId, url: String, window_id: Option<i64>) {
        let Some(page) = self.pages.get_mut(target) else {
            return;
        };
        let tab = page.tab;
        let Some(opener) = page.opener.clone() else {
            page.state = PageState::Reported;
            self.writer.record(PageEvent::Opened(OpenedPage {
                target: target.clone(),
                opener: None,
                url,
                window: WindowKind::Tab,
                tab,
            }));
            return;
        };
        page.state = PageState::LookingUp;
        let browser = Arc::clone(&self.browser);
        let found = self.found.clone();
        let target = target.clone();
        tokio::spawn(async move {
            let window = window_kind(&browser, &target, window_id).await;
            // The watcher may be gone already; then nobody reads the answer.
            let _ = found.send(OpenedPage {
                target,
                opener: Some(opener),
                url,
                window,
                tab,
            });
        });
    }

    /// A popup's window lookup came back.
    fn found(&mut self, opened: OpenedPage) {
        let target = opened.target.clone();
        if let Some(page) = self.pages.get_mut(&target) {
            page.state = PageState::Reported;
        }
        self.writer.record(PageEvent::Opened(opened));
        if self.closed_early.remove(&target) {
            self.writer.record(PageEvent::Closed(target));
        }
    }

    fn destroyed(&mut self, target: &TargetId) {
        self.root_observers
            .retain(|_, observer| observer.target != *target);
        self.close_worker(target.as_str());
        for worker in self.writer.hub().workers.list(target) {
            self.close_worker(&worker.id);
        }
        self.frames.remove(target.as_str());
        self.attach_log.forget(target);
        self.baselines.tab_closed(target);
        self.lifecycle.forget(target);
        let Some(page) = self.pages.remove(target) else {
            return;
        };
        let sessions: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, tab)| *tab == target)
            .map(|(session, _)| session.clone())
            .collect();
        for session in &sessions {
            self.dialog_closed(session);
            self.sessions.remove(session);
            self.frame_logs.remove(session);
            self.recorded.forget(session);
            self.lifecycle_sources.detach(session);
        }
        match page.state {
            PageState::Reported => {
                self.writer.record(PageEvent::Closed(target.clone()));
            }
            PageState::LookingUp => {
                self.closed_early.insert(target.clone());
            }
            // A popup closed before its URL came (`window.open()` then `close()` at once):
            // upstream still emits `popup`, with a page already closed. Its window is gone, so
            // it reads as a tab, as any lookup that cannot tell does (`window_kind`).
            PageState::Pending { .. } if page.opener.is_some() => {
                self.writer.record(PageEvent::Opened(OpenedPage {
                    target: target.clone(),
                    opener: page.opener,
                    url: "about:blank".to_owned(),
                    window: WindowKind::Tab,
                    tab: page.tab,
                }));
                self.writer.record(PageEvent::Closed(target.clone()));
            }
            // A tab nobody opened from a page, never reported: its closing is not news either.
            PageState::Pending { .. } => {}
        }
    }

    /// A tab's renderer crashed: `page.on('crash')` news once the tab was reported opened, as
    /// its close is. Discovery sends it (the signal `newPage`'s readiness wait also ends on);
    /// the tab stays listed until `Target.targetDestroyed`.
    fn crashed(&mut self, target: TargetId) {
        if matches!(
            self.pages.get(&target).map(|page| &page.state),
            Some(PageState::Reported)
        ) {
            self.writer.record(PageEvent::Crashed(target));
        }
    }

    fn console(&mut self, session: &SessionId, target: TargetId, params: ConsoleParams) {
        let mut message = ConsoleMessage::from_params(target, params);
        message.worker = self
            .worker_sessions
            .get(session)
            .and_then(|id| self.writer.hub().workers.get(&message.target, id))
            .map(|worker| worker.info.clone());
        self.writer.record(PageEvent::Console(message));
    }

    fn close_worker(&mut self, id: &str) {
        if let Some(worker) = self.writer.hub().workers.remove(id) {
            if let Some(session) = worker.session().session_id() {
                self.worker_sessions.remove(session);
                self.worker_runtime.remove(session);
                self.sessions.remove(session);
                self.recorded.forget(session);
            }
            self.writer.record(PageEvent::Worker {
                target: worker.page.clone(),
                worker: worker.info.clone(),
                closed: true,
            });
        }
    }

    async fn worker_script_events(&mut self) -> bool {
        #[derive(Deserialize)]
        struct Version {
            product: String,
        }
        if let Some(supported) = self.worker_script_events {
            return supported;
        }
        let version = self
            .browser
            .cdp("Browser.getVersion", json!({}), None)
            .await;
        let supported = version
            .ok()
            .and_then(parse::<Version>)
            .and_then(|version| {
                version
                    .product
                    .split_once('/')
                    .and_then(|(_, version)| version.split('.').next())
                    .and_then(|major| major.parse::<u32>().ok())
            })
            .is_some_and(|major| major >= 143);
        self.worker_script_events = Some(supported);
        supported
    }

    /// Reattach a worker still starting after event loss: Chromium reports
    /// `Inspector.workerScriptLoaded` at attach when the script already ran, so the lost report
    /// comes again. Retaining the old worker ID preserves client identity.
    async fn recover_workers(&mut self, targets: &[TargetInfo]) {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct AttachedWorker {
            session_id: SessionId,
        }
        for target in targets.iter().filter(|info| info.kind == "worker") {
            let known = self
                .sessions
                .values()
                .find_map(|page| self.writer.hub().workers.get(page, &target.target_id));
            if known
                .as_ref()
                .is_some_and(|worker| *worker.context.borrow() == WorkerContext::Ready)
            {
                continue;
            }
            let owner = known
                .as_ref()
                .map(|worker| worker.page.clone())
                .or_else(|| {
                    let frame = target.parent_frame_id.as_deref()?;
                    self.pages
                        .keys()
                        .find(|page| page.as_str() == frame)
                        .cloned()
                        .or_else(|| self.frames.get(frame).cloned())
                        .or_else(|| {
                            self.contexts.iter().find_map(|((session, _), known)| {
                                (known.as_str() == frame)
                                    .then(|| self.sessions.get(session).cloned())
                                    .flatten()
                            })
                        })
                });
            let Some(owner) = owner else {
                continue;
            };
            let reply = self
                .browser
                .cdp(
                    "Target.attachToTarget",
                    json!({"targetId": target.target_id, "flatten": true}),
                    None,
                )
                .await;
            let Some(attached) = reply.ok().and_then(parse::<AttachedWorker>) else {
                continue;
            };
            if let Some(worker) = known {
                if let Some(old_session) = worker.session().session_id() {
                    self.worker_sessions.remove(old_session);
                    self.worker_runtime.remove(old_session);
                    self.sessions.remove(old_session);
                    self.recorded.forget(old_session);
                }
                let session = self
                    .browser
                    .protocol_session(attached.session_id.clone())
                    .at_connection_epoch();
                let wait_for_script = self.worker_script_events().await;
                // Retain the watch channel: waiting evaluations follow startup onto this
                // new Runtime agent instead of mistaking endpoint replacement for closure.
                worker.rebind(session.clone(), wait_for_script);
                self.sessions.insert(attached.session_id.clone(), owner);
                self.worker_sessions
                    .insert(attached.session_id.clone(), worker.info.id.clone());
                self.resume_worker(&session, &attached.session_id).await;
            } else {
                let info = super::worker_table::WorkerInfo {
                    id: target.target_id.clone(),
                    url: target.url.clone(),
                };
                self.start_worker(owner, info, attached.session_id, true)
                    .await;
            }
        }
    }

    async fn start_worker(
        &mut self,
        tab: TargetId,
        info: super::worker_table::WorkerInfo,
        session_id: SessionId,
        announce: bool,
    ) {
        let session = self
            .browser
            .protocol_session(session_id.clone())
            .at_connection_epoch();
        self.sessions.insert(session_id.clone(), tab.clone());
        let wait_for_script = self.worker_script_events().await;
        if !self.writer.hub().workers.insert(
            tab.clone(),
            info.clone(),
            session.clone(),
            wait_for_script,
        ) {
            // Another page/observer session can auto-attach the same worker. Its startup
            // pause is independent of our canonical endpoint and must never be left held.
            let _ = session
                .send_value("Runtime.runIfWaitingForDebugger", json!({}))
                .await;
            return;
        }
        self.worker_sessions
            .insert(session_id.clone(), info.id.clone());
        if announce {
            self.writer.record(PageEvent::Worker {
                target: tab,
                worker: info,
                closed: false,
            });
        }
        self.resume_worker(&session, &session_id).await;
    }

    /// Readies a chosen worker session and releases the worker from its start-up pause, which
    /// must happen whatever else fails: a worker left paused never runs. Recursive attachment is
    /// armed as upstream arms it, so nested workers belong to the same page. `Runtime` goes on
    /// only if a run already listens on the tab, and before the release, so the worker's first
    /// console call is reported.
    async fn resume_worker(&mut self, session: &ProtocolSession, id: &SessionId) {
        let _ = session
            .send_value(
                "Target.setAutoAttach",
                json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true}),
            )
            .await;
        self.worker_runtime.insert(id.clone(), WorkerRuntime::Off);
        self.lease_worker(id).await;
        let _ = session
            .send_value("Runtime.runIfWaitingForDebugger", json!({}))
            .await;
    }

    /// Brings every chosen worker session's `Runtime` in line with who listens now.
    async fn lease_worker_runtime(&mut self) {
        let sessions: Vec<SessionId> = self.worker_sessions.keys().cloned().collect();
        for session in sessions {
            self.lease_worker(&session).await;
        }
    }

    /// Takes a `Runtime` lease on worker session `id` while some run asks for console messages
    /// or page errors on its tab, and drops it once none does ([`WorkerRuntime`]). A run asks
    /// through a listener only where it leased its page's `Runtime` for them, so a tab's
    /// workers report exactly when its page does.
    ///
    /// The lease's floor is where the browser's replay of the worker's stored history starts to
    /// count (`RuntimeLeases::floor_ms`): the `since` of the earliest ask, so a listener or a
    /// wait still sees everything the worker logged after it began, and nothing from before,
    /// however long the watcher took to get here.
    async fn lease_worker(&mut self, id: &SessionId) {
        let since = self
            .sessions
            .get(id)
            .and_then(|tab| self.writer.hub().workers_asked_since(tab));
        let since = match (since, self.worker_runtime.get(id)) {
            // Not a chosen worker session, or already as it should be.
            (_, None)
            | (Some(_), Some(WorkerRuntime::On { .. }))
            | (None, Some(WorkerRuntime::Off)) => {
                return;
            }
            (None, Some(WorkerRuntime::On { .. })) => {
                // Replacing the held lease drops it.
                self.worker_runtime.insert(id.clone(), WorkerRuntime::Off);
                return;
            }
            (Some(since), Some(WorkerRuntime::Off)) => since,
        };
        let session = self
            .browser
            .protocol_session(id.clone())
            .at_connection_epoch();
        // A failed enable leaves it off; the next pass tries again while the worker lives.
        if let Ok(lease) = self
            .browser
            .page_signals
            .runtime_leases()
            .acquire(
                &session,
                Duration::from_millis(since).as_secs_f64() * 1000.0,
            )
            .await
        {
            self.worker_runtime
                .insert(id.clone(), WorkerRuntime::On { _lease: lease });
        }
    }

    /// Brings every out-of-process frame session's `Log` in line with who listens now.
    async fn lease_frame_logs(&mut self) {
        let sessions: Vec<SessionId> = self.frame_logs.keys().cloned().collect();
        for session in sessions {
            self.lease_frame_log(&session).await;
        }
    }

    /// Takes a `Log` lease on frame session `id` while a console lease holds its tab and the
    /// session is its frame's chosen one, and drops it once either stops ([`FrameLog`]). The
    /// floor is the earliest such lease's, so the replay keeps what the frame logged after the
    /// run began to listen, however late the frame attached. A console lease's release asks for
    /// this pass before it lets `Runtime` go (`log_lease.rs`).
    async fn lease_frame_log(&mut self, id: &SessionId) {
        let since = self
            .sessions
            .get(id)
            .filter(|_| self.lifecycle_sources.accepts(id))
            .and_then(|tab| self.writer.hub().log_leases().console_floor(tab));
        let since = match (since, self.frame_logs.get(id)) {
            (_, None) | (Some(_), Some(FrameLog::On { .. })) | (None, Some(FrameLog::Off)) => {
                return;
            }
            (None, Some(FrameLog::On { .. })) => {
                // Awaited: a console lease's release asks for this pass and lets `Runtime` go
                // only once it answered, so the frame's `Log.disable` must be out by then.
                if let Some(FrameLog::On { lease }) =
                    self.frame_logs.insert(id.clone(), FrameLog::Off)
                {
                    lease.release().await;
                }
                return;
            }
            (Some(since), Some(FrameLog::Off)) => since,
        };
        let session = self
            .browser
            .protocol_session(id.clone())
            .at_connection_epoch();
        // A failed enable leaves it off; the next pass tries again while the frame lives.
        if let Ok(lease) = self
            .writer
            .hub()
            .log_leases()
            .acquire(&session, since)
            .await
        {
            self.frame_logs.insert(id.clone(), FrameLog::On { lease });
        }
    }

    async fn attached(&mut self, parent: Option<&SessionId>, attached: Attached) {
        let Attached {
            session_id,
            target_info,
        } = attached;
        match target_info.kind.as_str() {
            "worker" => {
                let Some(parent) = parent else { return };
                let Some(tab) = self.target_of(parent).await else {
                    return;
                };
                let info = super::worker_table::WorkerInfo {
                    id: target_info.target_id,
                    url: target_info.url,
                };
                self.start_worker(tab, info, session_id, true).await;
            }
            "page" => {
                // An observer has no Page/Runtime domains. Let a foreground core session
                // become the lifecycle source when the script actually acts on this tab.
                if self.root_observers.contains_key(&session_id) {
                    return;
                }
                let target = TargetId(target_info.target_id);
                self.lifecycle_sources.attach(&session_id, &target);
                self.sessions.insert(session_id.clone(), target.clone());
                self.page_sessions.insert(session_id.clone());
                if self.lifecycle_sources.accepts(&session_id) {
                    self.seed_lifecycle(&target, &session_id).await;
                }
            }
            "iframe" => {
                if let Some(parent) = parent
                    && let Some(tab) = self.target_of(parent).await
                {
                    self.lifecycle_sources
                        .attach(&session_id, &TargetId(target_info.target_id.clone()));
                    self.frames.insert(target_info.target_id, tab.clone());
                    self.sessions.insert(session_id.clone(), tab.clone());
                    self.frame_logs.insert(session_id.clone(), FrameLog::Off);
                    if self.lifecycle_sources.accepts(&session_id) {
                        self.seed_lifecycle(&tab, &session_id).await;
                    }
                    self.lease_frame_log(&session_id).await;
                }
            }
            _ => {}
        }
    }

    /// Records a frame's navigation or removal, and marks a tab's main-frame navigation for the
    /// host's URL facts (`EventWriter::main_frame_navigated`). A tab's main frame has the tab's
    /// target id; an out-of-process frame's has its own.
    async fn navigated(&mut self, method: &str, session: Option<&SessionId>, params: Value) {
        let Some(navigation) = navigation_event::parse(method, params) else {
            return;
        };
        let main_of = match (&navigation, session) {
            (PageEvent::Navigation(event), Some(session)) => self
                .target_of(session)
                .await
                .filter(|tab| tab.as_str() == event.frame.as_str()),
            _ => None,
        };
        if let Some(target) = &main_of
            && matches!(&navigation, PageEvent::Navigation(event) if event.new_document)
        {
            for worker in self.writer.hub().workers.list(target) {
                self.close_worker(&worker.id);
            }
        }
        let seq = self.writer.record(navigation);
        if let Some(tab) = main_of {
            self.writer.main_frame_navigated(tab, seq);
        }
    }

    /// The tab a session belongs to: from the attach events, else from core's page registry
    /// (a session attached before the watcher started).
    async fn target_of(&mut self, session: &SessionId) -> Option<TargetId> {
        if let Some(target) = self.sessions.get(session) {
            return Some(target.clone());
        }
        let page = self.view.page_for_session(session).await?;
        let info = self.browser.pages.get_info(page).await?;
        self.sessions
            .insert(session.clone(), info.target_id.clone());
        self.lifecycle_sources.attach(session, &info.target_id);
        self.seed_lifecycle(&info.target_id, session).await;
        Some(info.target_id)
    }

    /// A raw observer auto-attaches workers without enabling Runtime on untouched pages.
    /// Core's full renderer setup is reserved for foreground actions or explicit context
    /// lifecycle listeners. Observer sessions belong to this watcher and detach on drop.
    async fn observe_context(&mut self) {
        if let Ok(Ok(listing)) = time::timeout(Duration::from_secs(1), self.view.list_pages()).await
        {
            let full = self.writer.hub().wants_context_lifecycle();
            for page in listing.pages {
                if self.browser.pages.is_agent_created(&page.page_id).await
                    || self
                        .browser
                        .pages
                        .get_attached_session(page.page_id.clone())
                        .await
                        .is_some()
                {
                    continue;
                }
                if full {
                    let _ =
                        time::timeout(Duration::from_secs(1), self.view.page_session(page.page_id))
                            .await;
                } else if !self
                    .root_observers
                    .values()
                    .any(|observer| observer.target == page.target_id)
                {
                    self.observe_workers(page.target_id).await;
                }
            }
        }
    }

    async fn observe_workers(&mut self, target: TargetId) {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Observed {
            session_id: SessionId,
        }
        let root = ProtocolSession::root(self.browser.connection()).at_connection_epoch();
        let reply = root
            .send_value(
                "Target.attachToTarget",
                json!({"targetId": target.as_str(), "flatten": true}),
            )
            .await;
        let Some(observed) = reply.ok().and_then(parse::<Observed>) else {
            return;
        };
        self.root_observers.insert(
            observed.session_id.clone(),
            RootObserver {
                target: target.clone(),
                root,
            },
        );
        self.sessions.insert(observed.session_id.clone(), target);
        let session = self
            .browser
            .protocol_session(observed.session_id)
            .at_connection_epoch();
        // No Runtime.enable: console reporting on an unrelated page can execute preview
        // getters used to detect inspection. Target auto-attach needs no renderer domains.
        // Include OOPIFs for their workers, but exclude page/prerender and service targets.
        // Nothing is paused: the observer initializes nothing, and Chromium never releases a
        // paused frame whose session it detaches because another session armed auto-attach
        // on the same parent while the frame was still navigating; that page never loads.
        let _ = session
            .send_value(
                "Target.setAutoAttach",
                json!({"autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true,
                    "filter": [{"type":"worker"}, {"type":"iframe"}, {"exclude":true}]}),
            )
            .await;
    }

    /// Sessions can attach to a tab whose documents existed before this watcher. Remember
    /// those frames silently so their first removal still carries a real Frame payload.
    async fn seed_lifecycle(&mut self, target: &TargetId, session: &SessionId) {
        let session = self.browser.protocol_session(session.clone());
        if let Ok(Ok(tree)) = time::timeout(
            Duration::from_secs(1),
            session.send_value("Page.getFrameTree", json!({})),
        )
        .await
        {
            self.lifecycle.seed(target, tree);
        }
    }

    /// Both public callbacks and navigation waits observe the same ordered CDP transition.
    async fn frame_changed(&mut self, method: &str, session: Option<&SessionId>, params: Value) {
        if let Some(session) = session {
            self.target_of(session).await;
            if !self.lifecycle_sources.accepts(session) {
                return;
            }
            self.frame_lifecycle(method, session, &params).await;
        }
        self.navigated(method, session, params).await;
    }

    /// Load notifications from an OOPIF belong to its Frame, never to the page or context.
    async fn page_load(&mut self, method: &str, session: &SessionId) {
        if let Some(target) = self.target_of(session).await
            && self.lifecycle_sources.accepts(session)
            // The chosen root session can predate core's most recently attached session.
            && self.lifecycle_sources.targets.get(session) == Some(&target)
        {
            let kind = if method == "Page.loadEventFired" {
                PageEventKind::Load
            } else {
                PageEventKind::DomContentLoaded
            };
            if self.lifecycle.main(&target).is_none() {
                self.seed_lifecycle(&target, session).await;
            }
            if let Some(frame) = self.lifecycle.main(&target) {
                self.writer.record(PageEvent::Load {
                    target,
                    kind,
                    frame,
                });
            }
        }
    }

    /// The frame log's part of a frame event: an attached or removed child frame, or the main
    /// frame's commit. Same-document navigations and process swaps change no number.
    async fn frame_lifecycle(&mut self, method: &str, session: &SessionId, params: &Value) {
        if let Some(target) = self.target_of(session).await {
            for event in self.lifecycle.record(&target, method, params) {
                self.writer.record(PageEvent::FrameLifecycle(event));
            }
        }
        match method {
            "Page.frameAttached" => {
                if let Some(FrameAttached { frame_id }) = parse(params.clone()) {
                    self.frame_event(session, FrameEvent::Attached(FrameId(frame_id)))
                        .await;
                }
            }
            "Page.frameDetached" => {
                if let Some(detached) = parse::<FrameDetached>(params.clone())
                    && detached.reason != "swap"
                {
                    let removed = FrameEvent::Removed(FrameId(detached.frame_id));
                    self.frame_event(session, removed).await;
                }
            }
            "Page.frameNavigated" => {
                if let Some(FrameNavigated { frame }) = parse(params.clone())
                    && let Some(tab) = self.target_of(session).await
                    // A tab's main frame id is its target id; a child's commit keeps its number.
                    && frame.id == tab.0
                {
                    // Upstream's frame URL: `framePayload.url + (framePayload.urlFragment || '')`.
                    let url = format!("{}{}", frame.url, frame.url_fragment.unwrap_or_default());
                    // A cross-document commit (same-document ones are
                    // `Page.navigatedWithinDocument`): the text the agent last saw is of a
                    // document that is gone.
                    self.baselines.document_changed(&tab);
                    self.attach_log
                        .record(&tab, FrameEvent::MainCommitted { url });
                }
            }
            _ => {}
        }
    }

    /// Logs `event` for the tab `session` belongs to.
    async fn frame_event(&mut self, session: &SessionId, event: FrameEvent) {
        if let Some(tab) = self.target_of(session).await {
            self.attach_log.record(&tab, event);
        }
    }

    /// The tab of `session`, if some run listens for `event` on it.
    /// Whether a console message or exception is the browser's replay of history: a lease
    /// that switched `Runtime` on makes the browser send what it stored before, and a listener
    /// must see only what came after it asked, as it did while `Runtime` stayed on from attach.
    ///
    /// The floor alone cannot tell (ER-006): it is the script's `Date.now()`, whole
    /// milliseconds, while the browser stamps in microseconds, so a report the script saw just
    /// before it asked can be stamped past it; and a wait that rejected an event waits again from
    /// its first floor, under a new lease that replays the rejected event. So a report this
    /// session's log already recorded is a replay, whatever its stamp ([`RecordedReports`]).
    fn replayed(&self, session: &SessionId, key: Option<ReportKey>, params: &Value) -> bool {
        let Some(floor) = self.browser.page_signals.runtime_leases().floor_ms(session) else {
            // Unleased, `Runtime` is off on every session Neo drives (a worker reports only
            // under its `WorkerRuntime` lease, an OOPIF never), so a page session's report is
            // core's console harvest replaying history.
            return self.page_sessions.contains(session);
        };
        params
            .get("timestamp")
            .and_then(Value::as_f64)
            .is_some_and(|stamped| stamped < floor)
            || key.is_some_and(|key| self.recorded.contains(session, key))
    }

    /// A console message or uncaught exception, for the runs listening on its tab, unless it
    /// is history the browser replayed when a lease enabled `Runtime` ([`Self::replayed`]).
    async fn runtime_report(&mut self, method: &str, session: SessionId, params: Value) {
        let key = ReportKey::of(method, &params);
        if self.replayed(&session, key, &params) {
            return;
        }
        let recorded = if method == "Runtime.consoleAPICalled" {
            if let Some(target) = self.wanted(&session, PageEventKind::Console).await
                && let Some(params) = parse::<ConsoleParams>(params)
            {
                self.console(&session, target, params);
                true
            } else {
                false
            }
        } else if let Some(target) = self.wanted(&session, PageEventKind::PageError).await
            && let Some(params) = parse::<ExceptionParams>(params)
        {
            let error = PageError::from_params(target, params);
            self.writer.record(PageEvent::PageError(error));
            true
        } else {
            false
        };
        // Only what the log holds: a report nobody wanted then is not a later waiter's history
        // by identity, and its replay still meets the floor.
        if recorded && let Some(key) = key {
            self.recorded.insert(&session, key);
        }
    }

    /// A browser log entry, as upstream's `_onLogEntryAdded` emits it: a console message of the
    /// entry's level, with no arguments, unless a worker logged it. Only while a console lease
    /// holds `Log` on for the session (`log_lease.rs`), and not the entries `Log.enable` replayed
    /// from before the span's floor. Off the broadcast, so in wire order with the console API's
    /// messages.
    async fn log_entry(&mut self, session: SessionId, params: Value) {
        let Some(floor) = self.writer.hub().log_leases().floor_ms(&session) else {
            // No run asked (`addScriptTag` enables `Log` for its own CSP reports).
            return;
        };
        let key = ReportKey::of("Log.entryAdded", &params);
        let Some(entry) = parse::<LogEntryParams>(params) else {
            return;
        };
        // Upstream releases the entry's argument objects, which it does not pass on.
        let held = entry.argument_objects();
        if !held.is_empty() {
            let objects = self.browser.protocol_session(session.clone());
            tokio::spawn(async move {
                for object in held {
                    let _released = objects
                        .send_value("Runtime.releaseObject", json!({ "objectId": object }))
                        .await;
                }
            });
        }
        // As for `Runtime` reports, the floor is whole milliseconds and the stamp is not: an
        // entry this session's log already recorded is the replay of a new span (ER-006).
        let recorded = key.is_some_and(|key| self.recorded.contains(&session, key));
        if entry.timestamp() < floor
            || recorded
            || !entry.logged_by_page()
            || entry.browser_favicon()
        {
            return;
        }
        // Only under a console lease on the tab. `addScriptTag`'s own `Log` lease is for the CSP
        // reports it reads itself, and hosted mode takes no console lease, so a hosted script
        // hears no log entry (`event_op.rs`, `lease_runtime`).
        if let Some(target) = self.wanted(&session, PageEventKind::Console).await
            && self
                .writer
                .hub()
                .log_leases()
                .console_floor(&target)
                .is_some()
        {
            let message = ConsoleMessage::from_log_entry(target, entry);
            self.writer.record(PageEvent::Console(message));
            if let Some(key) = key {
                self.recorded.insert(&session, key);
            }
        }
    }

    async fn wanted(&mut self, session: &SessionId, event: PageEventKind) -> Option<TargetId> {
        let target = self.target_of(session).await?;
        self.writer.hub().wants(&target, event).then_some(target)
    }

    async fn dialog_opened(&mut self, session: SessionId, params: DialogParams) {
        let target = self.target_of(&session).await;
        // A modal is reported on every attached session. Use the selected native-target
        // source so even an automatically answered alert has one opening/closure pair.
        if !self.lifecycle_sources.accepts(&session) {
            return;
        }
        let kind = DialogKind::parse(&params.kind);
        let hub = self.writer.hub();
        let id = hub.dialogs().allocate();
        if kind == DialogKind::Alert {
            // Core's page signals accept every alert at once, before any listener sees it.
            hub.dialogs().close(id, ClosedBy::Core);
        } else {
            let listening = target
                .as_ref()
                .map(|target| hub.listeners(target, PageEventKind::Dialog))
                .unwrap_or_default();
            hub.dialogs().hold(
                id,
                HeldDialog::new(session.clone(), target.clone(), kind, listening),
            );
            self.graces.push_back(Grace {
                ends: Instant::now() + DIALOG_GRACE,
                dialog: id,
                target: target.clone(),
            });
        }
        if let Some(target) = target {
            let dialog = DialogOpened {
                id,
                target,
                kind,
                message: params.message,
                default_value: params.default_prompt.unwrap_or_default(),
            };
            self.writer.record(PageEvent::Dialog(dialog.clone()));
            self.pending_dialogs.insert(session, dialog);
        }
    }

    /// CDP orders opening before closing on one session. Removing its metadata makes duplicate
    /// notifications and tab teardown idempotent, including alerts core already accepted.
    fn dialog_closed(&mut self, session: &SessionId) {
        self.writer.hub().dialogs().closed_on_session(session);
        if let Some(dialog) = self.pending_dialogs.remove(session) {
            self.writer.record(PageEvent::DialogClosed(dialog));
        }
    }

    /// Ends every grace that is over: a dialog somebody listens for by now stays held for
    /// them, any other is closed by Neo.
    fn end_graces(&mut self) {
        let now = Instant::now();
        while let Some(grace) = self.graces.front()
            && grace.ends <= now
        {
            let Some(Grace { dialog, target, .. }) = self.graces.pop_front() else {
                break;
            };
            let hub = self.writer.hub();
            let listening = target
                .as_ref()
                .map(|target| hub.listeners(target, PageEventKind::Dialog))
                .unwrap_or_default();
            let unanswered = hub.dialogs().end_grace(dialog, listening);
            if let Some(unanswered) = unanswered {
                self.close_unanswered(unanswered);
            }
        }
    }

    /// Answers a dialog nobody else will, off the watcher's loop.
    fn close_unanswered(&self, dialog: Unanswered) {
        let session = self.browser.protocol_session(dialog.session);
        tokio::spawn(async move {
            // It may have closed meanwhile (its tab, the owner); then there is nothing to do.
            let _ = session
                .send_value(
                    "Page.handleJavaScriptDialog",
                    json!({ "accept": dialog.accept }),
                )
                .await;
        });
    }

    /// How a download is going; the table keeps only how it ended.
    fn download_progress(&self, params: Value) {
        if let Some(ProgressParams {
            guid,
            state,
            file_path,
        }) = parse(params)
        {
            self.writer
                .hub()
                .downloads()
                .progress(&guid, &state, file_path);
        }
    }

    fn download(&mut self, download: DownloadParams) {
        let frame_target = TargetId(download.frame_id.clone());
        let page = match self.pages.get(&frame_target) {
            // A tab that opened only to download never gets a document: upstream reports the
            // download on its opener.
            Some(KnownPage {
                opener: Some(opener),
                state: PageState::Pending { .. },
                ..
            }) => Some(opener.clone()),
            Some(_) => Some(frame_target),
            None => self.frames.get(&download.frame_id).cloned(),
        };
        self.writer.hub().downloads().begin(
            download.guid.clone(),
            download.url.clone(),
            download.suggested_filename.clone(),
        );
        self.writer.record(PageEvent::Download(DownloadStarted {
            page,
            frame: FrameId(download.frame_id),
            guid: download.guid,
            url: download.url,
            suggested_filename: download.suggested_filename,
        }));
    }
}

/// Selects one lifecycle source for each native page or OOPIF target. An action's session and
/// context observation's session can coexist, but both receive the same Chromium transitions.
/// Keep the first source until it detaches, then promote a surviving session for that target.
/// Dialog opening shares this selection so one physical modal receives one public identity.
#[derive(Default)]
struct LifecycleSources {
    targets: HashMap<SessionId, TargetId>,
    primary: HashMap<TargetId, SessionId>,
}

impl LifecycleSources {
    fn attach(&mut self, session: &SessionId, target: &TargetId) {
        self.targets.insert(session.clone(), target.clone());
        self.primary
            .entry(target.clone())
            .or_insert_with(|| session.clone());
    }

    fn accepts(&self, session: &SessionId) -> bool {
        self.targets
            .get(session)
            .and_then(|target| self.primary.get(target))
            == Some(session)
    }

    fn detach(&mut self, session: &SessionId) {
        if let Some(target) = self.targets.remove(session)
            && self.primary.get(&target) == Some(session)
        {
            self.primary.remove(&target);
            if let Some((next, _)) = self.targets.iter().find(|(_, known)| **known == target) {
                self.primary.insert(target, next.clone());
            }
        }
    }
}

/// A browser tab, as opposed to an iframe, a worker, a prerendered page or the browser itself.
fn is_tab(info: &TargetInfo) -> bool {
    info.kind == "page" && info.subtype.as_deref() != Some("prerender")
}

/// Whether `target` opened in a normal window. Any failure to tell reads as a tab, which is
/// what hosted mode is for.
async fn window_kind(
    browser: &BrowserSession,
    target: &TargetId,
    window_id: Option<i64>,
) -> WindowKind {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ForTarget {
        window_id: i64,
    }
    #[derive(Deserialize)]
    struct Windows {
        windows: Vec<Window>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Window {
        window_id: i64,
        window_type: String,
    }
    let window_id = if let Some(window_id) = window_id {
        window_id
    } else {
        let reply = browser
            .cdp(
                "Browser.getWindowForTarget",
                json!({ "targetId": target.as_str() }),
                None,
            )
            .await
            .and_then(decode::<ForTarget>);
        let Ok(found) = reply else {
            return WindowKind::Tab;
        };
        found.window_id
    };
    let windows = browser
        .cdp("Browser.getWindows", json!({}), None)
        .await
        .and_then(decode::<Windows>);
    match windows {
        Ok(Windows { windows }) => windows
            .into_iter()
            .find(|window| window.window_id == window_id)
            .filter(|window| window.window_type != "normal")
            .map_or(WindowKind::Tab, |_popup| WindowKind::Window),
        Err(_unknown) => WindowKind::Tab,
    }
}

/// An event's params as `T`; one that does not decode is ignored, like any event the watcher
/// does not need.
fn parse<T: DeserializeOwned>(params: Value) -> Option<T> {
    serde_json::from_value(params).ok()
}

fn decode<T: DeserializeOwned>(reply: Value) -> Result<T, CoreError> {
    serde_json::from_value(reply).map_err(|error| CoreError::Message(error.to_string()))
}

#[cfg(test)]
mod worker_observation_tests {
    use std::error::Error;
    use std::sync::Arc;

    use browseros_cdp::CdpEvent;
    use browseros_core::testing::{TestCall, TestConnection};
    use browseros_core::{
        BrowserSession, BrowserSessionHooks, CdpConnection, SessionId, TabId, TargetId,
    };
    use serde_json::json;
    use tokio::task;

    use super::{Watcher, WatcherHandle};
    use crate::events::worker_table::WorkerInfo;
    use crate::events::{EventHub, PageEvent, PageEventKind, Subscription, WatcherProgress};
    use crate::run_state::RunState;
    use crate::snapshot::{Baseline, SnapshotBaselines, SnapshotForm, TextForm};
    use crate::world::FrameAttachLog;

    /// A tab's snapshot baseline names its main frame's document: a new document or a closed tab
    /// drops it, and nothing else does (a same-document navigation, a child frame's commit,
    /// another tab's events).
    #[tokio::test]
    async fn a_new_main_document_and_a_closed_tab_drop_the_tab_s_snapshot_baseline()
    -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([]);
        let browser = BrowserSession::new(
            Arc::clone(&connection) as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        );
        let (_hub, writer) = EventHub::new();
        let baselines = Arc::new(SnapshotBaselines::default());
        let (mut watcher, _found) = Watcher::new(
            browser,
            writer,
            Arc::new(FrameAttachLog::default()),
            Arc::new(WatcherProgress::default()),
            Arc::clone(&baselines),
        );
        let (tab, other) = (TargetId("tab".to_owned()), TargetId("other".to_owned()));
        let session = SessionId::from("tab-session");
        watcher.sessions.insert(session.clone(), tab.clone());
        watcher.lifecycle_sources.attach(&session, &tab);
        // A snapshot whose run ended with its text given: the stored text it replaces, then it.
        let snapshot = |target: &TargetId, text: &str| {
            let document = baselines.generation(target);
            let previous = baselines.stored(target, document);
            let baseline = Baseline {
                text: text.to_owned(),
                form: SnapshotForm {
                    text: TextForm::LeftOut,
                    urls: false,
                    max_tokens: 8000.0,
                },
            };
            baselines.commit(target, document, baseline);
            previous.map(|baseline| baseline.text)
        };
        snapshot(&tab, "tab text");
        snapshot(&other, "other text");
        let on_tab = |method: &str, params: serde_json::Value| CdpEvent {
            method: method.to_owned(),
            params,
            session_id: Some(session.clone()),
        };

        watcher
            .handle(on_tab(
                "Page.navigatedWithinDocument",
                json!({ "frameId": "tab", "url": "https://a.test/#more" }),
            ))
            .await;
        watcher
            .handle(on_tab(
                "Page.frameNavigated",
                json!({ "frame": { "id": "child", "parentId": "tab", "url": "https://b.test/" } }),
            ))
            .await;
        assert_eq!(
            snapshot(&tab, "tab text").as_deref(),
            Some("tab text"),
            "the main document is still the one the agent saw"
        );

        watcher
            .handle(on_tab(
                "Page.frameNavigated",
                json!({ "frame": { "id": "tab", "url": "https://a.test/next" } }),
            ))
            .await;
        assert_eq!(
            snapshot(&tab, "next text"),
            None,
            "a new main document starts over"
        );

        watcher
            .handle(CdpEvent {
                method: "Target.targetDestroyed".to_owned(),
                params: json!({ "targetId": "other" }),
                session_id: None,
            })
            .await;
        assert_eq!(
            snapshot(&other, "again"),
            None,
            "a closed tab keeps nothing"
        );
        assert_eq!(
            snapshot(&tab, "next text").as_deref(),
            Some("next text"),
            "another tab's close leaves this one's"
        );
        Ok(())
    }

    #[tokio::test]
    async fn observing_an_untouched_page_never_enables_its_runtime() -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([("Target.setAutoAttach", json!({}))]);
        let browser = BrowserSession::new(
            Arc::clone(&connection) as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        );
        let (_hub, writer) = EventHub::new();
        let (mut watcher, _found) = Watcher::new(
            browser,
            writer,
            Arc::new(FrameAttachLog::default()),
            Arc::new(WatcherProgress::default()),
            Arc::new(SnapshotBaselines::default()),
        );
        watcher
            .observe_workers(TargetId("untouched-page".to_owned()))
            .await;
        let calls = connection.calls()?;
        assert_eq!(
            calls.len(),
            2,
            "observation only attaches and arms descendant discovery"
        );
        assert_eq!(calls[0].method, "Target.attachToTarget");
        assert_eq!(calls[1].method, "Target.setAutoAttach");
        assert_eq!(
            calls[1].params["filter"],
            json!([
                {"type":"worker"}, {"type":"iframe"}, {"exclude":true}
            ])
        );
        assert_eq!(
            calls[1].params["waitForDebuggerOnStart"],
            json!(false),
            "nothing the observer attaches may wait for it to resume"
        );
        assert!(
            !calls.iter().any(|call| call.method == "Runtime.enable"),
            "page console reporting must stay off on an untouched tab"
        );
        Ok(())
    }

    /// A watcher over a scripted browser, with the hub it writes.
    fn scripted(connection: &Arc<TestConnection>) -> (Arc<EventHub>, Watcher) {
        let browser = BrowserSession::new(
            Arc::clone(connection) as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        );
        let (hub, writer) = EventHub::new();
        let (watcher, _found) = Watcher::new(
            browser,
            writer,
            Arc::new(FrameAttachLog::default()),
            Arc::new(WatcherProgress::default()),
            Arc::new(SnapshotBaselines::default()),
        );
        (hub, watcher)
    }

    fn worker(id: &str) -> WorkerInfo {
        WorkerInfo {
            id: id.to_owned(),
            url: format!("{id}.js"),
        }
    }

    /// Lets the spawned `Runtime.disable` of a dropped lease reach the connection.
    async fn settle() {
        for _ in 0..20 {
            task::yield_now().await;
        }
    }

    /// The methods of `calls` that went to `session`, in order.
    fn sent_to(calls: &[TestCall], session: &str) -> Vec<String> {
        calls
            .iter()
            .filter(|call| call.session.as_ref().map(SessionId::as_str) == Some(session))
            .map(|call| call.method.clone())
            .collect()
    }

    #[tokio::test]
    async fn a_duplicate_worker_attachment_releases_its_startup_pause() -> Result<(), Box<dyn Error>>
    {
        let connection = TestConnection::new([
            ("Browser.getVersion", json!({"product":"Chrome/143.0"})),
            ("Target.setAutoAttach", json!({})),
            ("Runtime.runIfWaitingForDebugger", json!({})),
            ("Runtime.runIfWaitingForDebugger", json!({})),
        ]);
        let (hub, mut watcher) = scripted(&connection);
        let page = TargetId("page".to_owned());
        let first = SessionId::from("canonical");
        watcher
            .start_worker(page.clone(), worker("worker"), first.clone(), true)
            .await;
        watcher
            .start_worker(
                page.clone(),
                worker("worker"),
                SessionId::from("duplicate"),
                true,
            )
            .await;
        let endpoint = hub.workers.get(&page, "worker").ok_or("worker exists")?;
        assert_eq!(endpoint.session().session_id(), Some(&first));
        let calls = connection.calls()?;
        assert_eq!(
            sent_to(&calls, "canonical"),
            ["Target.setAutoAttach", "Runtime.runIfWaitingForDebugger"],
            "nested workers are armed, and Runtime stays off while nobody listens"
        );
        assert_eq!(
            sent_to(&calls, "duplicate"),
            ["Runtime.runIfWaitingForDebugger"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_worker_reports_runtime_only_while_a_run_listens_on_its_tab()
    -> Result<(), Box<dyn Error>> {
        let connection =
            TestConnection::new([("Browser.getVersion", json!({"product":"Chrome/143.0"}))]);
        for method in [
            "Target.setAutoAttach",
            "Runtime.runIfWaitingForDebugger",
            "Runtime.enable",
            "Runtime.disable",
        ] {
            connection.always(method, json!({}))?;
        }
        let (hub, mut watcher) = scripted(&connection);
        let page = TargetId("page".to_owned());
        let run = Arc::new(RunState::default());
        hub.bind(&run);
        let console = Subscription::Page {
            tab: TabId(1),
            target: page.clone(),
            event: PageEventKind::Console,
        };
        let leases = Arc::clone(watcher.browser.page_signals.runtime_leases());

        // A worker that starts while nobody listens, then a listener, which leases its page
        // as `event_op::listen` does.
        watcher
            .start_worker(
                page.clone(),
                worker("early"),
                SessionId::from("early"),
                true,
            )
            .await;
        run.events.listen(console.clone(), 1_000, 0);
        let page_session = watcher
            .browser
            .protocol_session(SessionId::from("page-session"));
        run.events.hold_runtime(
            &console,
            vec![leases.acquire(&page_session, 1_000.0).await?.into()],
        );
        watcher.lease_worker_runtime().await;
        // A worker that starts while the listener is there.
        watcher
            .start_worker(page.clone(), worker("late"), SessionId::from("late"), true)
            .await;
        let calls = connection.calls()?;
        assert_eq!(
            sent_to(&calls, "early"),
            [
                "Target.setAutoAttach",
                "Runtime.runIfWaitingForDebugger",
                "Runtime.enable"
            ]
        );
        assert_eq!(
            sent_to(&calls, "late"),
            [
                "Target.setAutoAttach",
                "Runtime.enable",
                "Runtime.runIfWaitingForDebugger"
            ],
            "on before the start-up pause ends, so the worker's first console call is reported"
        );
        assert_eq!(
            leases.floor_ms(&SessionId::from("early")),
            Some(1_000.0),
            "replayed history counts from when the listener asked"
        );

        run.events.unlisten(&console.key());
        watcher.lease_worker_runtime().await;
        settle().await;
        let calls = connection.calls()?;
        for session in ["early", "late"] {
            assert_eq!(
                sent_to(&calls, session).last().map(String::as_str),
                Some("Runtime.disable"),
                "{session}: off again once nobody listens"
            );
            assert!(!leases.is_held(&SessionId::from(session)));
        }
        Ok(())
    }

    #[tokio::test]
    async fn a_listener_that_leased_no_page_leaves_workers_quiet_and_a_wait_asks()
    -> Result<(), Box<dyn Error>> {
        let connection =
            TestConnection::new([("Browser.getVersion", json!({"product":"Chrome/143.0"}))]);
        for method in [
            "Target.setAutoAttach",
            "Runtime.runIfWaitingForDebugger",
            "Runtime.enable",
            "Runtime.disable",
        ] {
            connection.always(method, json!({}))?;
        }
        let (hub, mut watcher) = scripted(&connection);
        let page = TargetId("page".to_owned());
        let run = Arc::new(RunState::default());
        hub.bind(&run);
        let errors = Subscription::Page {
            tab: TabId(1),
            target: page.clone(),
            event: PageEventKind::PageError,
        };
        watcher
            .start_worker(page.clone(), worker("w"), SessionId::from("w"), true)
            .await;

        // A listener whose page took no lease (`event_op::lease_runtime` declined it).
        run.events.listen(errors.clone(), 1_000, 0);
        run.events.hold_runtime(&errors, Vec::new());
        watcher.lease_worker_runtime().await;
        let calls = connection.calls()?;
        assert!(
            !sent_to(&calls, "w")
                .iter()
                .any(|method| method == "Runtime.enable"),
            "a listener that asked for nothing leaves the worker quiet"
        );

        // A wait asks, from its own start: what the worker logged before it is history.
        let waiting = run.events.waiting(errors.clone(), 2_000);
        watcher.lease_worker_runtime().await;
        assert_eq!(
            watcher
                .browser
                .page_signals
                .runtime_leases()
                .floor_ms(&SessionId::from("w")),
            Some(2_000.0)
        );
        drop(waiting);
        watcher.lease_worker_runtime().await;
        settle().await;
        assert!(
            !watcher
                .browser
                .page_signals
                .runtime_leases()
                .is_held(&SessionId::from("w"))
        );
        Ok(())
    }

    /// ER-006, the race itself: a wait's floor is the script's `Date.now()`, whole milliseconds,
    /// and the browser stamps with microseconds. Round 3's error, thrown at 1000.6 under a floor
    /// of 1000, is recorded; round 4's wait starts in the same millisecond, its lease opens a new
    /// span, and the browser replays round 3's error, stamped above that floor. It is dropped by
    /// identity, and round 4's own error, in the same millisecond, still arrives. So is the
    /// replay a predicate's re-wait gets under its next lease, however far past the floor.
    #[tokio::test]
    async fn tests_that_a_replay_of_a_recorded_error_is_dropped_whatever_its_stamp()
    -> Result<(), Box<dyn Error>> {
        let connection =
            TestConnection::new([("Browser.getVersion", json!({"product":"Chrome/143.0"}))]);
        for method in ["Runtime.enable", "Runtime.disable"] {
            connection.always(method, json!({}))?;
        }
        let (hub, mut watcher) = scripted(&connection);
        let page = TargetId("page".to_owned());
        let session = SessionId::from("page-session");
        watcher.sessions.insert(session.clone(), page.clone());
        let run = Arc::new(RunState::default());
        hub.bind(&run);
        let errors = Subscription::Page {
            tab: TabId(1),
            target: page,
            event: PageEventKind::PageError,
        };
        let leases = Arc::clone(watcher.browser.page_signals.runtime_leases());
        let protocol = watcher.browser.protocol_session(session.clone());
        let thrown = |id: i64, stamp: f64, exception: serde_json::Value| CdpEvent {
            method: "Runtime.exceptionThrown".to_owned(),
            params: json!({ "timestamp": stamp, "exceptionDetails": {
                "exceptionId": id, "text": "Uncaught", "exception": exception } }),
            session_id: Some(session.clone()),
        };

        let round_three = run.events.waiting(errors.clone(), 1_000);
        let lease = leases.acquire(&protocol, 1_000.0).await?;
        watcher
            .handle(thrown(
                3,
                1_000.6,
                json!({ "type": "number", "value": 0, "description": "0" }),
            ))
            .await;
        drop(lease);
        drop(round_three);
        settle().await;

        let errors_later = errors.clone();
        let _round_four = run.events.waiting(errors, 1_000);
        let round_four_lease = leases.acquire(&protocol, 1_000.0).await?;
        // The new span's replay, then round 4's own error.
        watcher
            .handle(thrown(
                3,
                1_000.6,
                json!({ "type": "number", "value": 0, "description": "0" }),
            ))
            .await;
        watcher
            .handle(thrown(4, 1_000.8, json!({ "type": "string", "value": "" })))
            .await;
        let messages: Vec<String> = hub
            .read_after(0)
            .records
            .into_iter()
            .filter_map(|record| match record.event {
                PageEvent::PageError(error) => Some(error.message),
                _ => None,
            })
            .collect();
        assert_eq!(
            messages,
            ["0", ""],
            "round 3's error once, then round 4's own"
        );

        // A wait whose predicate rejected round 4's error waits again from the same floor, under
        // a new lease: the replay of an error well past the floor is no news either, and a new
        // error that shares its stamp but not its id is.
        let _retry = run.events.waiting(errors_later, 1_000);
        drop(round_four_lease);
        settle().await;
        let _retry_lease = leases.acquire(&protocol, 1_000.0).await?;
        watcher
            .handle(thrown(4, 1_000.8, json!({ "type": "string", "value": "" })))
            .await;
        watcher
            .handle(thrown(
                5,
                1_000.8,
                json!({ "type": "string", "value": "again" }),
            ))
            .await;
        assert_eq!(
            hub.read_after(0).records.len(),
            3,
            "round 4's replay dropped, the new error at its stamp recorded"
        );
        Ok(())
    }
    /// A browser log entry reaches a console listener only while a console lease holds its tab.
    /// An `addScriptTag` or `addStyleTag` insert holds `Log` alone, for the CSP reports it reads
    /// itself, and a hosted console listener takes no lease (`event_op.rs`, `lease_runtime`): an
    /// entry logged then reaches no script, so a hosted one hears none.
    #[tokio::test]
    async fn tests_that_a_log_entry_reaches_a_console_listener_only_under_a_console_lease()
    -> Result<(), Box<dyn Error>> {
        let connection =
            TestConnection::new([("Browser.getVersion", json!({"product":"Chrome/143.0"}))]);
        for method in [
            "Runtime.enable",
            "Runtime.disable",
            "Log.enable",
            "Log.disable",
        ] {
            connection.always(method, json!({}))?;
        }
        let (hub, mut watcher) = scripted(&connection);
        let page = TargetId("page".to_owned());
        let session = SessionId::from("page-session");
        watcher.sessions.insert(session.clone(), page.clone());
        let run = Arc::new(RunState::default());
        hub.bind(&run);
        let console = Subscription::Page {
            tab: TabId(1),
            target: page.clone(),
            event: PageEventKind::Console,
        };
        // A hosted listener: it listens and leases nothing.
        run.events.listen(console, 1_000, 0);
        let protocol = watcher.browser.protocol_session(session.clone());
        let entry = |text: &str| CdpEvent {
            method: "Log.entryAdded".to_owned(),
            params: json!({ "entry": {
                "source": "security", "level": "error", "text": text, "timestamp": 2_000.0 } }),
            session_id: Some(session.clone()),
        };
        let heard = |hub: &EventHub| -> Vec<String> {
            hub.read_after(0)
                .records
                .into_iter()
                .filter_map(|record| match record.event {
                    PageEvent::Console(message) => Some(message.text),
                    _ => None,
                })
                .collect()
        };

        let insert = hub.log_leases().acquire(&protocol, 1_500.0).await?;
        watcher
            .handle(entry("blocked by CSP during the insert"))
            .await;
        assert!(
            heard(&hub).is_empty(),
            "an entry under the insert's own lease is no listener's: {:?}",
            heard(&hub)
        );

        // Outside hosted mode the listener's console lease holds the tab, and the entry is its.
        let runtime = watcher
            .browser
            .page_signals
            .runtime_leases()
            .acquire(&protocol, 1_500.0)
            .await?;
        let console_lease = hub
            .log_leases()
            .acquire_console(runtime, &protocol, page, 1_500.0, WatcherHandle::detached())
            .await
            .map_err(|refused| refused.error)?;
        watcher
            .handle(entry("blocked by CSP under a console lease"))
            .await;
        assert_eq!(heard(&hub), ["blocked by CSP under a console lease"]);
        drop(console_lease);
        insert.release().await;
        Ok(())
    }
}
