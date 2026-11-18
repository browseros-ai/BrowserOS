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
//!   before the watcher started is looked up in core's page registry. Each session's frame tree
//!   seeds the frame tracker, read on a task of its own while the tab's later events wait in its
//!   line (`TabLine`), and while a run listens on the context for lifecycle events, core's page
//!   session is attached to each untouched tab the same way (`attach_page`). Nothing on the loop
//!   waits for a renderer there: a busy tab holds only its own tab's events. A tab no session of
//!   core's is on gets the watcher's root observer, its lifecycle source with `Page` on
//!   (`observe_workers`), so such a tab reports its frames' events too; a read that failed is
//!   tried again on later ticks (`ReadRetry`).
//! - **Dialogs.** An `alert()` is already accepted by core's page signals (a listed deviation).
//!   Any other dialog is held for the runs listening for dialogs on its tab, then
//!   [`DIALOG_GRACE`] later held for whoever listens by then, or closed by Neo; the reap tick
//!   closes held dialogs whose runs all ended (`dialog_table.rs`).
//! - **Frames.** `Page.frameAttached`, a removing `Page.frameDetached` and the main frame's
//!   `Page.frameNavigated` go to the `FrameAttachLog`, from one session per native target, so frames are
//!   numbered in the order Playwright numbers them (`world/frame_attach_log.rs`). Each
//!   out-of-process frame's session gets `Page` on as it attaches (`read_tree`), so its
//!   own commits arrive whether or not anything read its frames. That commit
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
use std::sync::Arc;
use std::time::Duration;
use std::{fmt, mem};

use browseros_cdp::CdpEvent;
use browseros_core::{
    BrowserSession, BrowserView, CoreError, FrameId, PageId, ProtocolSession, RuntimeLease,
    SessionId, TabId, TargetId,
};
use futures_util::future;
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
use crate::target_crashes::{RecoveryTicket, TargetCrashes};
use crate::url_scheme;
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

/// How long a synchronize waits for the watcher's answer before the asker goes on without it.
const SYNCHRONIZE_BOUND: Duration = Duration::from_secs(2);

/// How long a session's frame-tree read may take, a frame session's `Page.enable` with it
/// ([`TabLine`]). The tree only seeds the frame tracker with frames that existed before the
/// session reported events, and a read that fails or misses this leaves it unseeded, as a failed
/// read always did; a tab's own session may read again at its next load event (`page_load`). It
/// was 1 s while the whole watcher waited for it; its tab's events wait for it now, and a frame
/// whose renderer is busy with ad script answers in 100 ms to over a second (measured), so the
/// wait is short. A healthy frame answers the enable in about one round trip.
const TREE_READ_WAIT: Duration = Duration::from_millis(200);

/// How long one attach of core's page session to an untouched tab may take for a context-level
/// listener (`attach_page`): core's attach turns the tab's domains on, which its renderer answers.
/// A tab that misses it is attached again on a later pass.
const CONTEXT_ATTACH_WAIT: Duration = Duration::from_secs(1);

/// Bounds passive recovery setup off the event loop. Failure leaves refusal in place and
/// permits the next observation pass to retry; it is not an operation deadline.
const RECOVERY_OBSERVE_WAIT: Duration = Duration::from_secs(1);

/// How long a root observer's `Target.setAutoAttach` may take, on a task of its own
/// (`observe_workers`). Chromium answers it once the tab's renderer acknowledged reporting its
/// child targets (inferred, not measured), which a busy renderer does only when its script lets
/// it. Cut off, the command still reaches the renderer; only the wait for its answer ends.
const OBSERVER_ARM_WAIT: Duration = Duration::from_secs(1);

/// How many times a session's tree is read again after reads that failed or missed
/// [`TREE_READ_WAIT`], one per later tick ([`Watcher::retry_reads`]), before it waits for its
/// tab's next load event or main-frame commit. A read that brings a tree ends the streak.
const READ_RETRIES: u8 = 2;

/// A position in the connection's broadcast event stream: how many events the CDP client had
/// broadcast when it was taken (`BrowserSession::cdp_events_broadcast`), on the numbering
/// [`WatcherProgress`] counts the watcher's work in. A synchronize to a fence waits for exactly
/// the events received before it, never for the ones a busy page keeps sending after.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct EventFence(pub(crate) u64);

impl EventFence {
    /// The fence at the `count`th broadcast event, for a host's tests of its own waiting.
    #[must_use]
    pub const fn at(count: u64) -> Self {
        Self(count)
    }
}

/// The running watcher. Dropping it (with its `Playwright`) stops the task.
#[derive(Debug)]
pub(crate) struct TargetWatcher {
    task: Option<JoinHandle<()>>,
    requests: Requests,
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
    /// A tab the host stopped keeping for itself, and its address now: recorded as opened once
    /// the events before `fence` are, as a tab the owner opens is (`Playwright::hand_back`).
    handed_back: Option<(TargetId, String)>,
    /// The events this request needs handled: those broadcast before it was asked.
    fence: EventFence,
    done: oneshot::Sender<()>,
}

/// The asking side of the watcher's request channel, shared by [`TargetWatcher`] and every
/// [`WatcherHandle`]. It reads fences from the same session the watcher reads events from.
#[derive(Clone)]
struct Requests {
    sender: UnboundedSender<WatcherRequest>,
    /// `None` only for a test handle no watcher answers.
    browser: Option<Arc<BrowserSession>>,
}

// A hand-written `Debug`: the session is not worth printing.
impl fmt::Debug for Requests {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Requests").finish_non_exhaustive()
    }
}

impl Requests {
    /// The fence for "every event received until now".
    fn fence(&self) -> EventFence {
        EventFence(
            self.browser
                .as_ref()
                .map_or(0, |browser| browser.cdp_events_broadcast()),
        )
    }

    /// Asks the watcher to handle the events before `fence`, retire `closed`'s workers and
    /// bring the leases in line, and waits at most [`SYNCHRONIZE_BOUND`]. Whether it answered:
    /// a watcher that is gone answers at once, as there is nothing left to wait for.
    async fn ask(
        &self,
        closed: Option<TargetId>,
        handed_back: Option<(TargetId, String)>,
        fence: EventFence,
    ) -> bool {
        let (done, wait) = oneshot::channel();
        if self
            .sender
            .send(WatcherRequest {
                closed,
                handed_back,
                fence,
                done,
            })
            .is_err()
        {
            return true;
        }
        time::timeout(SYNCHRONIZE_BOUND, wait).await.is_ok()
    }
}

/// A handle on the running watcher for what outlives an op: a console lease's release has it
/// take the frames' `Log` off before `Runtime` goes (`log_lease.rs`).
#[derive(Clone, Debug)]
pub(crate) struct WatcherHandle {
    requests: Requests,
}

impl WatcherHandle {
    /// Waits, at most 2 s, for the watcher to handle the events received so far and bring
    /// every worker's `Runtime` and frame's `Log` in line with who listens now. A watcher that
    /// is gone answers at once.
    pub(crate) async fn synchronize(&self) {
        let fence = self.requests.fence();
        self.requests.ask(None, None, fence).await;
    }

    /// A handle no watcher answers, for tests.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        let (sender, _gone) = mpsc::unbounded_channel();
        Self {
            requests: Requests {
                sender,
                browser: None,
            },
        }
    }
}

impl TargetWatcher {
    /// A handle on this watcher that outlives the op holding it ([`WatcherHandle`]).
    pub(crate) fn handle(&self) -> WatcherHandle {
        WatcherHandle {
            requests: self.requests.clone(),
        }
    }

    /// The fence for "every event received until now", for a caller that takes it before it
    /// queues for its turn to synchronize (`Playwright::event_fence`).
    pub(crate) fn fence(&self) -> EventFence {
        self.requests.fence()
    }

    /// Foreground replies must see every CDP fact received before their command replied:
    /// waits, at most 2 s, until the watcher handled every event before `fence` (later ones are
    /// another reply's) and brought the leases in line. Whether it answered in time.
    /// A confirmed explicit close also retires workers before Chromium finishes tearing
    /// down a renderer with unresolved promises (Playwright issue 43204).
    pub(crate) async fn synchronize(&self, closed: Option<TargetId>, fence: EventFence) -> bool {
        self.requests.ask(closed, None, fence).await
    }

    /// Records `target`'s tab as opened at `url`, after every event received until now: the
    /// host stopped keeping it for itself (`Playwright::hand_back`). Whether the watcher
    /// answered in time.
    pub(crate) async fn hand_back(&self, target: TargetId, url: String) -> bool {
        let fence = self.requests.fence();
        self.requests.ask(None, Some((target, url)), fence).await
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
        crashes: Arc<TargetCrashes>,
    ) -> Self {
        let (sender, pending) = mpsc::unbounded_channel();
        let requests = Requests {
            sender,
            browser: Some(Arc::clone(&browser)),
        };
        let Ok(runtime) = Handle::try_current() else {
            progress.stopped();
            return Self {
                task: None,
                requests,
            };
        };
        let (watcher, found) =
            Watcher::new(browser, writer, attach_log, progress, baselines, crashes);
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
    crashes: Arc<TargetCrashes>,
    /// Proof replies return here; the watcher keeps consuming events through their fences.
    recovering: Option<UnboundedSender<RecoveryTree>>,
    recovery_replies: Vec<RecoveryTree>,
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
    /// Watcher-owned root sessions auto-attach descendants. One on a tab that no session of
    /// core's reports for is the tab's lifecycle source, with `Page` on (`observe_workers`), so
    /// a tab no session of core's is on reports its frames too; crash recovery observes commits on
    /// one, including after reconnect. Page `Runtime` always stays off.
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
    /// Each tab's events in wire order behind its frame-tree reads in flight ([`TabLine`]).
    lines: HashMap<TargetId, TabLine>,
    /// Where those reads send their answers back, so only this task writes the tracker; set by
    /// the run loop. Without it (a test driving `handle` alone) a read is awaited in place.
    seeded: Option<UnboundedSender<SeededTree>>,
    /// The sessions whose last tree read failed or missed [`TREE_READ_WAIT`], to be read again.
    read_retries: HashMap<SessionId, ReadRetry>,
    /// Download begins waiting for the tree reads that were out when they were read, oldest first.
    /// Each is let go only when [`Self::advance_lines`] pops a read (`release_downloads`), so its
    /// release depends on every read in a line being popped, as each is once its answer comes back
    /// within [`TREE_READ_WAIT`]. So no line with a read in it is ever dropped: a closing tab's
    /// reads are ended and popped as failed ones (`end_reads`), or a download waiting for one would
    /// hold every reply's synchronize.
    unowned_downloads: Vec<UnownedDownload>,
    /// Each untouched tab's attach of core's page session for a context-level listener, while
    /// out or until the next pass replaces it ([`Watcher::attach_page`]).
    attaching: HashMap<TargetId, JoinHandle<()>>,
    /// Synchronize requests whose fence covers an event a line still holds: answered once the
    /// log has every event before their fence.
    deferred: Vec<WatcherRequest>,
    /// The session id and target id of each attachment waiting in a tab's line (a worker's),
    /// with that tab: what they report waits behind the attachment, though the loop does not
    /// know them yet. Both are CDP's unique ids, so they share one map.
    held_attachments: HashMap<String, TargetId>,
}

/// One tab's events while a frame-tree read of one of its sessions is out.
///
/// A session's tree is read to seed the frame tracker with frames that existed before the
/// session reported events: an out-of-process frame's session as it attaches, and the tab's own
/// session as it attaches or is first met in an event, and again at a load event while the tab's
/// main frame is unknown (`page_load`). A renderer answers only between its script's tasks (about
/// 100 ms under an ad's busy loop, measured; not at all while a page's script spins), and the
/// loop used to wait for it in place: one busy tab held every tab's events, and every reply's
/// synchronize, behind its reads. Now the read goes out on a task of its own, and the tab's later
/// events wait here, in order, until its answer is applied: the tracker sees the tree and the
/// tab's events in the same order as when the loop waited, while other tabs go on. A read that
/// fails or misses [`TREE_READ_WAIT`] lets the line go on as one that answered does; a line that
/// waited for a read never answered would hold its tab's events, and the synchronize of every
/// reply whose fence covers one, for good.
type TabLine = VecDeque<LineItem>;

enum LineItem {
    /// A session's tree read and how far it got. `started` is how many broadcast events the loop
    /// had read when it went out: an event read at that position or later found it out.
    Read {
        session: SessionId,
        started: u64,
        read: TreeRead,
        answer: TreeAnswer,
    },
    /// An event of the tab read at broadcast `position` and held behind the reads before it.
    Event { position: u64, event: CdpEvent },
    /// The tab's `load` or `DOMContentLoaded`, read at broadcast `position` while its main frame
    /// was unknown, behind the read it waits for: recorded with the main frame that read gave, or
    /// not at all, and never read for again (`page_load`).
    Load { position: u64, kind: PageEventKind },
}

impl LineItem {
    /// The broadcast position of a held event; a read has none.
    fn position(&self) -> Option<u64> {
        match self {
            Self::Event { position, .. } | Self::Load { position, .. } => Some(*position),
            Self::Read { .. } => None,
        }
    }
}

/// A download's begin whose frame no tracker knew when the loop read it, while frame-tree reads
/// were out ([`Watcher::wait_unowned`]).
///
/// `Browser.downloadWillBegin` names only a frame, and a same-process child that existed before
/// its tab's session reported events is known only from a tree read, its tab's or, nested in an
/// out-of-process frame, that frame session's. Handled while such a read is out, the download is
/// recorded with no page, and a script's `download` wait never hears of it. So it waits for every
/// read out when it was read, in any line, and for none started later: a page that keeps adding
/// frames keeps reads out all the time, and a begin that waited for those would hold every reply
/// whose fence covers it. Each of those reads is applied within [`TREE_READ_WAIT`] of going out,
/// tree or none, so it waits about that long at most, and it goes sooner, as soon as a tree names
/// its frame (`release_downloads`). Its position counts in the hold meanwhile.
struct UnownedDownload {
    /// The broadcast position the begin was read at.
    position: u64,
    begin: CdpEvent,
    /// The download's progress read meanwhile, with their positions: the download table drops
    /// progress of a download it has not listed, so it goes on behind the begin.
    progress: Vec<(u64, CdpEvent)>,
}

/// A tree read's state in its line.
enum TreeAnswer {
    /// Still out: the events behind it wait.
    Out,
    /// Back: the tree, or `None` when the read failed or missed [`TREE_READ_WAIT`]. Either way
    /// the events behind it go on; without a tree, as when the read used to fail in place.
    Back(Option<Value>),
}

/// A tree read's answer, for the session it was asked on; `None` when it failed or timed out.
struct SeededTree {
    session: SessionId,
    tree: Option<Value>,
}

/// Whose frame tree a read is of, which decides whether it turns `Page` on too ([`read_tree`]).
#[derive(Clone, Copy)]
enum TreeRead {
    /// A tab's own session: core's attach turned `Page` on there.
    Page,
    /// An out-of-process frame's session, or a tab's root observer: nothing else turns `Page`
    /// on there.
    Frame,
}

/// A session to read again after reads that failed or missed [`TREE_READ_WAIT`] (a renderer
/// busy just then): without it, a tab's same-process frames that existed before its session
/// reported events stay unknown until its next load event or main-frame commit, and a loaded tab
/// that was only briefly busy has neither to come. The retry goes through the tab's line as any
/// read, holding only that tab's events, for up to [`TREE_READ_WAIT`]; a download waiting for
/// earlier reads never waits for it (`UnownedDownload`), as it started later.
struct ReadRetry {
    tab: TargetId,
    read: TreeRead,
    /// Retries this streak of failed reads still allows, from [`READ_RETRIES`].
    left: u8,
    /// When the next goes out, a tick or more after the failure; `None` once none is left.
    due: Option<Instant>,
}

/// A bounded, epoch-pinned read, returned to the event owner without awaiting its own fence.
struct RecoveryTree {
    ticket: RecoveryTicket,
    tree: Option<Value>,
    fence: u64,
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
    recovery: RecoveryObservation,
    /// Set once `observe_recovery` handled this observer, whatever state it found: its tab
    /// crashed while no session of core's was the tab's source, and recovers through it. Main's
    /// recovery observer turned `Page` on there and reported the tab's events from then on, so
    /// this observer's records are written as main's were (`Watcher::records_page_events`). It
    /// stays set for the observer's life: `seed_observer` entering it later does not clear it.
    after_crash: bool,
}

/// Page-only observation survives renderer failure without putting renderer setup on the
/// event owner's loop. Failed enables are retried; dropping the attachment cancels its work.
enum RecoveryObservation {
    Off,
    Enabling(JoinHandle<bool>),
    On,
}

impl Drop for RootObserver {
    fn drop(&mut self) {
        if let RecoveryObservation::Enabling(task) = &self.recovery {
            task.abort();
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // An attach still out would otherwise outlive the watcher by up to its bound.
        for attach in self.attaching.values() {
            attach.abort();
        }
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
    /// The URL the tab's own target events last named (`Target.targetCreated`,
    /// `Target.targetInfoChanged`, discovery's `Target.getTargets`); `None` until one did. The
    /// door check reads it ([`Watcher::door_verdict`]). The tab listing's URL is not used: it
    /// names the tab, and lags a swap of the tab's target, whose new target has none yet.
    url: Option<String>,
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
    frame_id: FrameId,
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
        crashes: Arc<TargetCrashes>,
    ) -> (Self, UnboundedReceiver<OpenedPage>) {
        let (found, found_rx) = mpsc::unbounded_channel();
        let watcher = Self {
            view: BrowserView::unbound(Arc::clone(&browser), None),
            browser,
            writer,
            attach_log,
            baselines,
            progress,
            crashes,
            recovering: None,
            recovery_replies: Vec::new(),
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
            lines: HashMap::new(),
            seeded: None,
            read_retries: HashMap::new(),
            unowned_downloads: Vec::new(),
            attaching: HashMap::new(),
            deferred: Vec::new(),
            held_attachments: HashMap::new(),
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
        let (seeded, mut trees) = mpsc::unbounded_channel();
        self.seeded = Some(seeded);
        let (recovering, mut recovered) = mpsc::unbounded_channel();
        self.recovering = Some(recovering);
        self.progress.start(before, calls_before);
        let mut tick = time::interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            let grace_ends = self.graces.front().map(|grace| grace.ends);
            tokio::select! {
                received = events.recv() => match received {
                    Ok(event) => {
                        if let Some(event) = self.hold(event) {
                            self.handle(event).await;
                        }
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
                    // Every event sent before the call, its context's creation among them: the
                    // broadcast count now covers them (the call was routed before it was read).
                    let fence = EventFence(self.browser.cdp_events_broadcast());
                    self.drain_through(&mut events, fence).await;
                    self.binding_called(call).await;
                    self.progress.call_handled();
                }
                Some(request) = requests.recv() => {
                    // Requests that queued meanwhile share this pass: one drain to the newest
                    // of their fences, one lease pass, every one answered.
                    let mut batch = vec![request];
                    while let Ok(more) = requests.try_recv() {
                        batch.push(more);
                    }
                    // Initial synchronous page facts need attachments even for tabs the
                    // run has never acted on; otherwise their pre-existing workers vanish.
                    if self.epoch.is_none() { self.tick().await; }
                    let fence = batch.iter().map(|request| request.fence).max().unwrap_or_default();
                    self.drain_through(&mut events, fence).await;
                    for request in &mut batch {
                        if let Some(target) = &request.closed {
                            for worker in self.writer.hub().workers.list(target) { self.close_worker(&worker.id); }
                        }
                        if let Some((target, url)) = request.handed_back.take() {
                            self.handed_back(target, url);
                        }
                    }
                    // Every op synchronizes before it replies, so a listener the op added has
                    // its tab's workers and frames reporting before the script's next call.
                    self.lease_worker_runtime().await;
                    self.lease_frame_logs().await;
                    // An event before a fence may still wait in a tab's line; such a request is
                    // answered once the line lets it go (`answer_deferred`).
                    self.deferred.extend(batch);
                    self.answer_deferred();
                }
                Some(opened) = found.recv() => self.found(opened),
                Some(proof) = recovered.recv() => self.recovery_replies.push(proof),
                Some(seeded) = trees.recv() => {
                    self.seeded_tree(seeded);
                    self.advance_lines().await;
                }
                _ = tick.tick() => self.tick().await,
                () = time::sleep_until(grace_ends.unwrap_or_else(Instant::now)), if grace_ends.is_some() => {
                    self.end_graces();
                }
            }
            self.apply_recoveries();
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
        // An out-of-process frame's session going changes its tab's tree (`frame_logs` holds
        // exactly those sessions).
        if self.frame_logs.contains_key(session)
            && let Some(tab) = self.sessions.get(session).cloned()
        {
            self.writer.frames_changed(tab);
        }
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
        self.read_retries.remove(session);
    }

    /// Events were dropped: rediscover, which records what closed meanwhile, and tell the frame
    /// log that commits may be missing, and the snapshot baselines that any tab's document may
    /// have changed unseen.
    fn lost_events(&mut self) {
        self.crashes.invalidate_recoveries();
        self.recovery_replies.clear();
        self.epoch = None;
        self.attach_log.lost_events();
        self.baselines.events_lost();
    }

    /// Handles the queued broadcast events up to `fence`: every event broadcast before whoever
    /// asked took it. It stops there even while more are queued, so a page that keeps sending
    /// events (frames that attach and detach all the time) cannot hold a request past the
    /// events it is owed, and stops sooner if nothing more is queued.
    async fn drain_through(&mut self, events: &mut Receiver<CdpEvent>, fence: EventFence) {
        while self.progress.events_handled() < fence.0 {
            // Counted as the run loop counts, so ops waiting on `progress` see these too.
            match events.try_recv() {
                Ok(event) => {
                    if let Some(event) = self.hold(event) {
                        self.handle(event).await;
                    }
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
            if changed {
                self.crashes.invalidate_recoveries();
                self.recovery_replies.clear();
            }
            if self.discover(changed).await.is_ok() {
                self.epoch = Some(epoch);
                self.connection_epoch = Some(epoch);
            }
        }
        self.observe_context().await;
        self.retry_reads().await;
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
        let epoch = self.browser.connection().connection_epoch();
        let snapshot = self.crashes.snapshot(epoch);
        let root = ProtocolSession::root(self.browser.connection()).at_connection_epoch();
        let targets: Targets = decode(root.send_value("Target.getTargets", json!({})).await?)?;
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
        let listed: Vec<(TargetId, String)> = targets
            .target_infos
            .into_iter()
            .filter(is_tab)
            .map(|info| (TargetId(info.target_id), info.url))
            .collect();
        let live: HashSet<TargetId> = listed.iter().map(|(target, _)| target.clone()).collect();
        self.crashes.reconcile(
            &snapshot,
            &live,
            self.browser.connection().connection_epoch(),
        );
        let gone: Vec<TargetId> = self
            .pages
            .keys()
            .filter(|target| !live.contains(*target))
            .cloned()
            .collect();
        for target in gone {
            self.destroyed(&target);
        }
        for (target, url) in listed {
            let page = self.pages.entry(target).or_insert(KnownPage {
                opener: None,
                tab: None,
                state: PageState::Reported,
                url: None,
            });
            // Only on a page past `Pending`: the URL that admits a tab's seed (`door_verdict`) is
            // always one that took its page out of `Pending` (`created`, `info_changed`), so a
            // seeded tab is a reported one, and a removal records its `Closed`.
            if !url.is_empty() && !matches!(page.state, PageState::Pending { .. }) {
                page.url = Some(url);
            }
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
            self.read_retries.clear();
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
        self.handle_at(
            event,
            EventFence(self.progress.events_handled().saturating_add(1)),
        )
        .await;
        // A load event may have gone into its tab's line (`page_load`): its hold is published
        // before the loop counts it read. An event handled from a line publishes nothing here:
        // `advance_lines` holds that tab's later events aside meanwhile, and publishes once
        // they are back.
        if !self.lines.is_empty() {
            self.publish_hold();
        }
    }

    async fn handle_at(&mut self, event: CdpEvent, ingress: EventFence) {
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
                self.frame_changed(&method, session_id.as_ref(), params, ingress)
                    .await;
            }
            "Target.targetCreated" => {
                if let Some(TargetEvent { target_info }) = parse(params)
                    && self.created(target_info)
                {
                    self.observe_context().await;
                }
            }
            "Target.targetInfoChanged" => {
                if let Some(TargetEvent { target_info }) = parse(params) {
                    self.info_changed(target_info).await;
                }
            }
            "Target.targetDestroyed" => {
                if let Some(Destroyed { target_id }) = parse(params) {
                    let target = TargetId(target_id);
                    // Untagged destruction is not proof of absence on the current connection.
                    // Reconcile on a fresh listing before retiring persistent crash knowledge.
                    self.epoch = None;
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
                    self.page_load(&method, &session, ingress).await;
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

    /// Records a new tab; whether it was one. Only a new tab can need an observer of its own
    /// (`observe_context`): frames and workers attach through their tab's session, and a tab
    /// listing per frame was most of what a page that churns frames cost the watcher.
    fn created(&mut self, info: TargetInfo) -> bool {
        let target = TargetId(info.target_id.clone());
        if !is_tab(&info) || self.pages.contains_key(&target) {
            return false;
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
                url: (!info.url.is_empty()).then(|| info.url.clone()),
            },
        );
        if !info.url.is_empty() {
            self.ready(&target, info.url, info.window_id);
        }
        true
    }

    /// A tab's target changed: its tab id and URL, a pending tab's recording, and the seed of its
    /// root observer once its URL is one the door allows (`seed_observer`): a tab that showed a
    /// refused page, or no URL yet, gets its observer's `Page` and seed on the change that
    /// brings an allowed one, whether or not a session of core's is its source meanwhile.
    async fn info_changed(&mut self, info: TargetInfo) {
        let target = TargetId(info.target_id);
        let Some(page) = self.pages.get_mut(&target) else {
            return;
        };
        page.tab = info.tab_id.or(page.tab);
        if !info.url.is_empty() {
            page.url = Some(info.url.clone());
        }
        let pending = matches!(page.state, PageState::Pending { .. });
        if pending && !info.url.is_empty() {
            self.ready(&target, info.url, info.window_id);
        }
        if let Some(observer) = self.observer_of(&target)
            && self.door_verdict(&target) == Some(false)
        {
            self.seed_observer(&target, &observer).await;
        }
    }

    /// The root observer the watcher attached to `target`, if any.
    fn observer_of(&self, target: &TargetId) -> Option<SessionId> {
        self.root_observers
            .iter()
            .find_map(|(session, observer)| (observer.target == *target).then(|| session.clone()))
    }

    /// Whether the door refuses the page `tab` shows (`url_scheme::refuses_page`), read from the
    /// URL its own target events last named; `None` while none did. The watcher sends nothing
    /// of a page domain to a tab the door refuses: no read, no retry and no observer `Page`
    /// ([`Self::start_seed`]), and it attaches to an untouched tab only once its URL is known
    /// and allowed (`observe_context`). A tab that moves to a refused page after its session's
    /// `Page` went on gets nothing new, and no `Page.disable` either: what still arrives from it
    /// then is what the browser sends on its own on a session with `Page` on (its frames' and
    /// loads' events, its dialogs), never the answer to a command of ours.
    fn door_verdict(&self, tab: &TargetId) -> Option<bool> {
        self.pages
            .get(tab)?
            .url
            .as_deref()
            .map(url_scheme::refuses_page)
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

    /// The host stopped keeping `target`'s tab for itself (the owner browsed in its blank tab):
    /// scripts hear of it as of a tab the owner just opened. A tab not reported yet is recorded
    /// when its URL comes, as any; one already closed is not news.
    fn handed_back(&mut self, target: TargetId, url: String) {
        let Some(page) = self.pages.get(&target) else {
            return;
        };
        if !matches!(page.state, PageState::Reported) {
            return;
        }
        let tab = page.tab;
        self.writer.record(PageEvent::Opened(OpenedPage {
            target,
            opener: None,
            url,
            window: WindowKind::Tab,
            tab,
        }));
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
        // Its line stays: a waiting download is let go only as a read is popped, and the hold
        // counts every item, so a dropped read could park a download and hold every reply's
        // synchronize for good. Its reads end instead, as failed ones, and the next
        // `advance_lines` pops them: no tree goes into the tracker just forgotten, and no retry
        // is due for a session that went with the tab. Events held behind them replay onto a
        // tab already gone.
        if let Some(line) = self.lines.get_mut(target) {
            end_reads(line);
        }
        self.read_retries.retain(|_, retry| retry.tab != *target);
        self.attaching.remove(target);
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
                // A root observer's own attach is set up in `observe_workers`. A session of
                // core's that attaches to a tab whose observer is already its lifecycle source
                // stays a second source-in-waiting (`LifecycleSources`): its copies of the
                // observer's events are dropped, and it takes over once the observer goes.
                if self.root_observers.contains_key(&session_id) {
                    return;
                }
                let target = TargetId(target_info.target_id);
                self.lifecycle_sources.attach(&session_id, &target);
                self.sessions.insert(session_id.clone(), target.clone());
                self.page_sessions.insert(session_id.clone());
                if self.lifecycle_sources.accepts(&session_id) {
                    self.start_seed(&target, &session_id, TreeRead::Page).await;
                } else if self.lifecycle.main(&target).is_none()
                    && let Some(source) = self.lifecycle_sources.primary.get(&target).cloned()
                {
                    // A run is about to act on a tab whose source never brought a tree (its
                    // reads and their retries all failed, and the tab neither loaded nor
                    // committed since): read once more, through the source, so the run's tab
                    // has its main frame. Nothing goes out while a read of the source is in
                    // the tab's line (`start_seed`).
                    let read = if self.root_observers.contains_key(&source) {
                        TreeRead::Frame
                    } else {
                        TreeRead::Page
                    };
                    self.start_seed(&target, &source, read).await;
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
                    // Its subtree is readable only from now on: a tree read before it lacks it.
                    self.writer.frames_changed(tab.clone());
                    self.frame_logs.insert(session_id.clone(), FrameLog::Off);
                    if self.lifecycle_sources.accepts(&session_id) {
                        self.start_seed(&tab, &session_id, TreeRead::Frame).await;
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
    async fn navigated(
        &mut self,
        method: &str,
        session: Option<&SessionId>,
        params: Value,
        ingress: EventFence,
    ) {
        let Some(navigation) = navigation_event::parse(method, params) else {
            return;
        };
        let tab = match session {
            Some(session) => self.target_of(session).await,
            None => None,
        };
        let main_of = match (&navigation, &tab) {
            (PageEvent::Navigation(event), Some(tab)) if tab.as_str() == event.frame.as_str() => {
                Some(tab.clone())
            }
            _ => None,
        };
        if let Some(target) = &main_of
            && matches!(&navigation, PageEvent::Navigation(event) if event.new_document)
        {
            for worker in self.writer.hub().workers.list(target) {
                self.close_worker(&worker.id);
            }
            if let (PageEvent::Navigation(event), Some(session)) = (&navigation, session)
                && let Some(loader) = &event.loader
                && let Some(ticket) = self.crashes.main_document(
                    target,
                    event.frame.clone(),
                    loader.clone(),
                    ingress,
                    self.browser.connection().connection_epoch(),
                )
            {
                self.verify_recovery(ticket, session);
            }
        }
        // Only the record is gated: the workers' closing and the recovery ticket above stand for
        // every tab. The URL hint rides the record's seq, so it goes with it.
        if tab
            .as_ref()
            .is_none_or(|tab| self.records_page_events(tab, session))
        {
            let seq = self.writer.record(navigation);
            if let Some(tab) = main_of {
                self.writer.main_frame_navigated(tab, seq);
            }
        }
    }

    /// Only a fresh matching reply supplies recovery evidence. The completion channel keeps
    /// the event owner free to apply the very events that reply's fence requires.
    fn verify_recovery(&self, ticket: RecoveryTicket, session: &SessionId) {
        let Some(recovering) = self.recovering.clone() else {
            return;
        };
        let protocol = self
            .browser
            .protocol_session(session.clone())
            .at_connection_epoch();
        let browser = Arc::clone(&self.browser);
        drop(tokio::spawn(async move {
            let tree = time::timeout(
                TREE_READ_WAIT,
                protocol.send_value("Page.getFrameTree", json!({})),
            )
            .await
            .ok()
            .and_then(Result::ok);
            let fence = browser.cdp_events_broadcast();
            let _ = recovering.send(RecoveryTree {
                ticket,
                tree,
                fence,
            });
        }));
    }

    fn apply_recoveries(&mut self) {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Tree {
            frame_tree: Node,
        }
        #[derive(Deserialize)]
        struct Node {
            frame: Frame,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Frame {
            id: FrameId,
            loader_id: String,
            parent_id: Option<FrameId>,
        }

        self.crashes
            .apply_replies(self.browser.connection().connection_epoch());

        // Stopping the watcher releases operation waiters with a sentinel; it supplies no
        // recovery evidence. Only events actually applied by this owner satisfy the fence.
        let live = self.progress.events_handled() != u64::MAX;
        let applied = self.progress.applied_through();
        let (ready, pending): (Vec<_>, Vec<_>) = mem::take(&mut self.recovery_replies)
            .into_iter()
            .partition(|proof| live && applied >= proof.fence);
        self.recovery_replies = pending;
        for proof in ready {
            let Some(tree) = proof.tree.and_then(parse::<Tree>) else {
                continue;
            };
            let frame = tree.frame_tree.frame;
            if frame.parent_id.is_none()
                && frame.id == proof.ticket.frame
                && frame.loader_id == proof.ticket.loader
            {
                self.crashes
                    .recovered(proof.ticket, self.browser.connection().connection_epoch());
            }
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
        self.writer.tab_id(info.target_id.clone(), info.tab_id);
        self.sessions
            .insert(session.clone(), info.target_id.clone());
        self.lifecycle_sources.attach(session, &info.target_id);
        // The event that named the session is handled now, before the tree is applied: the read
        // goes out while it is handled, so the tree is newer than it, and the tab's later events
        // wait behind the read.
        self.start_seed(&info.target_id, session, TreeRead::Page)
            .await;
        Some(info.target_id)
    }

    /// A raw observer auto-attaches workers without enabling Runtime on untouched pages.
    /// Core's full renderer setup is reserved for foreground actions or explicit context
    /// lifecycle listeners. Observer sessions belong to this watcher and detach on drop.
    async fn observe_context(&mut self) {
        if let Ok(Ok(listing)) = time::timeout(Duration::from_secs(1), self.view.list_pages()).await
        {
            let full = self.writer.hub().wants_context_lifecycle();
            for page in &listing.pages {
                self.writer.tab_id(page.target_id.clone(), page.tab_id);
            }
            for page in listing.pages {
                // Existing observer sessions keep reporting external main-document commits.
                // A known crash must not make background renderer setup stall the event owner.
                if self.crashes.is_crashed(&page.target_id) {
                    self.observe_recovery(&page.target_id).await;
                    continue;
                }
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
                // One door check covers both attaches of an untouched tab: core's full page
                // session for a context-level listener, and the observer's `Page` and seed
                // (`observe_workers`). A tab on a refused page, or with no URL from its target
                // events yet, gets neither; the pass after it moves to an allowed page attaches
                // it. An observer already there is seeded on the change that brings that URL
                // (`info_changed`), or here, on a pass that finds its tab on an allowed URL some
                // other road brought (discovery after lost events, or core leaving a tab whose
                // observer never seeded); `seed_observer` does nothing once it is entered.
                if full {
                    if self.door_verdict(&page.target_id) == Some(false) {
                        self.attach_page(page.target_id, page.page_id);
                    }
                } else if let Some(observer) = self.observer_of(&page.target_id) {
                    if self.door_verdict(&page.target_id) == Some(false) {
                        self.seed_observer(&page.target_id, &observer).await;
                    }
                } else {
                    self.observe_workers(page.target_id).await;
                }
            }
        }
    }

    /// Attaches core's full page session to the untouched tab `target` for a run's context-level
    /// listener, on a task of its own. Core's attach waits for the tab's renderer to turn its
    /// domains on, which a busy tab answers only when its script lets it; awaited on the loop, it
    /// would hold the loop for [`CONTEXT_ATTACH_WAIT`] on every pass while the tab stays busy, and
    /// with it every tab's events and every reply's synchronize. One attach per tab is out at a
    /// time; a later pass attaches again once it ended without a session.
    /// Nothing comes back to the loop: it learns the session from the attach's own
    /// `Target.attachedToTarget`, in wire order with the tab's other events.
    fn attach_page(&mut self, target: TargetId, page: PageId) {
        if self
            .attaching
            .get(&target)
            .is_some_and(|attach| !attach.is_finished())
        {
            return;
        }
        let view = self.view.clone();
        let attach = tokio::spawn(async move {
            let _ = time::timeout(CONTEXT_ATTACH_WAIT, view.page_session(page)).await;
        });
        self.attaching.insert(target, attach);
    }

    /// Attaches the watcher's own root observer to the untouched tab `target`. It auto-attaches
    /// the tab's workers and out-of-process frames, as on main. Once the tab's URL is one the door
    /// allows ([`Self::door_verdict`]), now or on a later `Target.targetInfoChanged`, it is also
    /// the tab's lifecycle source unless a session of core's came first: `Page` goes on with the
    /// seed read, sent together, through the tab's line (`seed_observer`), so such a tab reports
    /// its frames' commits and its tracker lists them before any run reads its tree. `Runtime`
    /// stays off. Nothing here waits for the tab's renderer: the attach is the browser's to
    /// answer, and the auto-attach and the seed go out on tasks of their own, so a busy tab
    /// holds only its line.
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
        let Some(Observed { session_id }) = reply.ok().and_then(parse::<Observed>) else {
            return;
        };
        self.root_observers.insert(
            session_id.clone(),
            RootObserver {
                target: target.clone(),
                root,
                // On once its seed came back with a tree, which needs its `Page.enable` to have
                // succeeded (`read_tree`); `Page` then stays on across a renderer crash (the
                // browser restores a session's domains on the new renderer), so crash recovery
                // has nothing left to enable on it (`observe_recovery`).
                recovery: RecoveryObservation::Off,
                after_crash: false,
            },
        );
        self.sessions.insert(session_id.clone(), target.clone());
        let session = self
            .browser
            .protocol_session(session_id.clone())
            .at_connection_epoch();
        // No Runtime.enable: console reporting on an unrelated page can execute preview
        // getters used to detect inspection. Include OOPIFs for their workers, but exclude
        // page/prerender and service targets. Nothing is paused: the observer initializes
        // nothing, and Chromium never releases a paused frame whose session it detaches because
        // another session armed auto-attach on the same parent while the frame was still
        // navigating; that page never loads.
        let arm = async move {
            let _ = time::timeout(
                OBSERVER_ARM_WAIT,
                session.send_value(
                    "Target.setAutoAttach",
                    json!({"autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true,
                        "filter": [{"type":"worker"}, {"type":"iframe"}, {"exclude":true}]}),
                ),
            )
            .await;
        };
        drop(tokio::spawn(arm));
        if self.door_verdict(&target) == Some(false) {
            self.seed_observer(&target, &session_id).await;
        }
    }

    /// Enters `observer` among its tab's lifecycle sessions, once, together with its `Page.enable`
    /// and seed read, sent through the tab's line (`start_seed`): its tab's source, or the source
    /// in waiting behind a session of core's that came first. Never one without the other: a
    /// mute observer entered as a source would drop the events of a core session that attaches
    /// behind it, and one entered behind core's session would be promoted with `Page` off when core
    /// detaches (core does detach its sessions), the tab silent from then on. A waiting observer's
    /// events are dropped as any waiting session's are; its seed still goes into the tracker.
    async fn seed_observer(&mut self, target: &TargetId, observer: &SessionId) {
        if self.lifecycle_sources.entered(observer) {
            return;
        }
        self.lifecycle_sources.attach(observer, target);
        self.start_seed(target, observer, TreeRead::Frame).await;
    }

    /// Reconnect loses every session, but keeps crash facts. Reattach a minimal observer so
    /// an external reload can supply a later main-document commit. This never enables Runtime,
    /// installs a world, or waits for Page.enable on the event owner's loop.
    async fn observe_recovery(&mut self, target: &TargetId) {
        if self
            .lifecycle_sources
            .primary
            .get(target)
            .is_some_and(|session| !self.root_observers.contains_key(session))
        {
            // A surviving foreground Page session already observes external recovery.
            return;
        }
        let existing = self
            .root_observers
            .iter()
            .find_map(|(id, observer)| (observer.target == *target).then(|| id.clone()));
        let session = if let Some(session) = existing {
            session
        } else {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Observed {
                session_id: SessionId,
            }
            let root = ProtocolSession::root(self.browser.connection()).at_connection_epoch();
            let reply = time::timeout(
                RECOVERY_OBSERVE_WAIT,
                root.send_value(
                    "Target.attachToTarget",
                    json!({"targetId": target.as_str(), "flatten": true}),
                ),
            )
            .await;
            let Some(observed) = reply.ok().and_then(Result::ok).and_then(parse::<Observed>) else {
                return;
            };
            self.root_observers.insert(
                observed.session_id.clone(),
                RootObserver {
                    target: target.clone(),
                    root,
                    recovery: RecoveryObservation::Off,
                    after_crash: false,
                },
            );
            self.sessions
                .insert(observed.session_id.clone(), target.clone());
            observed.session_id
        };
        let refused = self.door_verdict(target) == Some(true);
        let Some(observer) = self.root_observers.get_mut(&session) else {
            return;
        };
        // Usually seeded long before its tab crashed, with `Page` on already: what makes its
        // records main's is the recovery, not which call turned `Page` on.
        observer.after_crash = true;
        if let RecoveryObservation::Enabling(task) = &mut observer.recovery {
            if !task.is_finished() {
                return;
            }
            // A finished task has no renderer wait left. Failure permits a later retry.
            let RecoveryObservation::Enabling(task) =
                mem::replace(&mut observer.recovery, RecoveryObservation::Off)
            else {
                return;
            };
            if task.await.unwrap_or(false) {
                observer.recovery = RecoveryObservation::On;
            }
        }
        if matches!(observer.recovery, RecoveryObservation::Off) && !refused {
            self.lifecycle_sources.attach(&session, target);
            let protocol = self.browser.protocol_session(session).at_connection_epoch();
            observer.recovery = RecoveryObservation::Enabling(tokio::spawn(async move {
                matches!(
                    time::timeout(
                        RECOVERY_OBSERVE_WAIT,
                        protocol.send_value("Page.enable", json!({}))
                    )
                    .await,
                    Ok(Ok(_))
                )
            }));
        }
    }

    /// Reads `session`'s frame tree into the tracker, so the tab's frames that existed before the
    /// session reported events still carry a real Frame payload when they go. The read goes out
    /// on a task of its own, and the tab's events read from now on wait in its line until the
    /// answer is applied ([`TabLine`]). A session has one read in its tab's line at most: while
    /// one is there, this sends none. Whether a read of `session` is in the line now.
    async fn start_seed(&mut self, tab: &TargetId, session: &SessionId, read: TreeRead) -> bool {
        let in_line =
            self.lines.values().flatten().any(
                |item| matches!(item, LineItem::Read { session: asked, .. } if asked == session),
            );
        if in_line {
            return true;
        }
        if self.door_verdict(tab) == Some(true) {
            return false;
        }
        let protocol = self.browser.protocol_session(session.clone());
        let Some(seeded) = self.seeded.clone() else {
            if let Some(tree) = read_tree(&protocol, read).await {
                self.lifecycle.seed(tab, tree);
            }
            return false;
        };
        self.lines
            .entry(tab.clone())
            .or_default()
            .push_back(LineItem::Read {
                session: session.clone(),
                started: self.progress.events_handled(),
                read,
                answer: TreeAnswer::Out,
            });
        let session = session.clone();
        drop(tokio::spawn(async move {
            let tree = read_tree(&protocol, read).await;
            let _ = seeded.send(SeededTree { session, tree });
        }));
        true
    }

    /// Schedules one more read of `session` on a later tick after a read that failed or missed
    /// [`TREE_READ_WAIT`], while its streak allows ([`ReadRetry`]).
    fn retry_later(&mut self, tab: &TargetId, session: SessionId, read: TreeRead) {
        let retry = self
            .read_retries
            .entry(session)
            .or_insert_with(|| ReadRetry {
                tab: tab.clone(),
                read,
                left: READ_RETRIES,
                due: None,
            });
        retry.due = (retry.left > 0).then(|| Instant::now() + TICK);
        retry.left = retry.left.saturating_sub(1);
    }

    /// Sends each retry that is due ([`ReadRetry`]) while its session is still its target's
    /// lifecycle source, or is the tab's root observer waiting behind a session of core's: its
    /// read turns its `Page` on, which it needs before core's detach promotes it
    /// (`seed_observer`). `start_seed` sends none while a read of that session is in its line,
    /// nor any to a tab the door refuses.
    async fn retry_reads(&mut self) {
        let now = Instant::now();
        let due: Vec<(SessionId, TargetId, TreeRead)> = self
            .read_retries
            .iter_mut()
            .filter_map(|(session, retry)| {
                let at = retry.due?;
                (at <= now).then(|| {
                    retry.due = None;
                    (session.clone(), retry.tab.clone(), retry.read)
                })
            })
            .collect();
        for (session, tab, read) in due {
            let reads = self.lifecycle_sources.accepts(&session)
                || self.root_observers.contains_key(&session);
            if reads && self.sessions.contains_key(&session) {
                self.start_seed(&tab, &session, read).await;
            }
        }
    }

    /// Puts an event just read from the broadcast in its tab's line if that tab has a frame-tree
    /// read out ([`TabLine`]), or gives it back to be handled now. A line's own events are
    /// handled straight from it, in order, when the read before them is back.
    fn hold(&mut self, event: CdpEvent) -> Option<CdpEvent> {
        if event.method == "Target.targetCrashed"
            && let Some(target) = event.params.get("targetId").and_then(Value::as_str)
        {
            self.crashes.crashed_event(
                TargetId(target.to_owned()),
                EventFence(self.progress.events_handled().saturating_add(1)),
            );
        }
        let Some(tab) = self.line_for(&event) else {
            return self.wait_unowned(event);
        };
        let position = self.progress.events_handled();
        if event.method == "Target.attachedToTarget" {
            let session = event.params.get("sessionId");
            let target = event
                .params
                .get("targetInfo")
                .and_then(|info| info.get("targetId"));
            for key in [session, target]
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                self.held_attachments.insert(key.to_owned(), tab.clone());
            }
        }
        if let Some(line) = self.lines.get_mut(&tab) {
            line.push_back(LineItem::Event { position, event });
        }
        self.publish_hold();
        None
    }

    /// Keeps a download's begin that no line takes while its frame is unknown and tree reads are
    /// out, with its progress behind it ([`UnownedDownload`]), or gives the event back to be
    /// handled now: a begin whose frame a tracker knows, or read while no read is out.
    fn wait_unowned(&mut self, event: CdpEvent) -> Option<CdpEvent> {
        let position = self.progress.events_handled();
        match event.method.as_str() {
            "Browser.downloadProgress" => {
                let guid = event.params.get("guid").and_then(Value::as_str);
                let Some(waiting) = self.unowned_downloads.iter_mut().find(|waiting| {
                    guid.is_some()
                        && waiting.begin.params.get("guid").and_then(Value::as_str) == guid
                }) else {
                    return Some(event);
                };
                waiting.progress.push((position, event));
            }
            "Browser.downloadWillBegin" => {
                let unowned = event
                    .params
                    .get("frameId")
                    .and_then(Value::as_str)
                    .is_some_and(|frame| self.frame_owner(&FrameId(frame.to_owned())).is_none());
                let reading = self
                    .lines
                    .values()
                    .flatten()
                    .any(|item| matches!(item, LineItem::Read { .. }));
                if !(unowned && reading) {
                    return Some(event);
                }
                self.unowned_downloads.push(UnownedDownload {
                    position,
                    begin: event,
                    progress: Vec::new(),
                });
            }
            _ => return Some(event),
        }
        self.publish_hold();
        None
    }

    /// Lets go of each waiting download that is ready ([`UnownedDownload`]): as soon as a tree
    /// names its frame, or once every read out at its begin is applied, tree or none. One whose tab
    /// is known goes into that tab's line at its place ([`place_by_position`]), its progress after
    /// it, when the tab holds a line (the read that named it was in that line), and is handled
    /// now otherwise; one no tree named is handled now, with no page. It must not wait on for the
    /// other reads once a tree named its frame: its tab's line would meanwhile replay the frame's
    /// removal, and the download would find no owner.
    async fn release_downloads(&mut self) {
        for download in mem::take(&mut self.unowned_downloads) {
            let owner = download
                .begin
                .params
                .get("frameId")
                .and_then(Value::as_str)
                .and_then(|frame| self.frame_owner(&FrameId(frame.to_owned())))
                .cloned();
            let reading = self.lines.values().flatten().any(|item| {
                matches!(item, LineItem::Read { started, .. } if *started <= download.position)
            });
            let release = owner.is_some() || !reading;
            if !release {
                self.unowned_downloads.push(download);
                continue;
            }
            let UnownedDownload {
                position,
                begin,
                progress,
            } = download;
            if let Some(line) = owner.and_then(|owner| self.lines.get_mut(&owner)) {
                place_by_position(
                    line,
                    position,
                    LineItem::Event {
                        position,
                        event: begin,
                    },
                );
                for (position, event) in progress {
                    place_by_position(line, position, LineItem::Event { position, event });
                }
                continue;
            }
            self.handle_at(begin, EventFence(position.saturating_add(1)))
                .await;
            for (position, event) in progress {
                self.handle_at(event, EventFence(position.saturating_add(1)))
                    .await;
            }
        }
    }

    /// The tab whose line `event` must wait in: while a tab has a frame-tree read out, every
    /// later event of that tab goes behind it, so the log and the worker table change in the
    /// tab's wire order. Two kinds go on at once, as neither publishes anything:
    /// - a new frame session's attachment, which only adds a read of its own to the line
    ///   (held, the reads of a page that keeps adding frames would go out one at a time);
    /// - a session's execution-context bookkeeping, which a binding call, arriving on a
    ///   channel of its own, needs in arrival order to name its frame.
    ///
    /// A target's `Target.targetCreated` and `Target.targetInfoChanged` never wait, whatever
    /// session carries them: they are the browser's facts about a target, which Chromium sends
    /// on the browser's session, never a tab's (so on Chromium this changes nothing). Held behind
    /// a read, the change that gives a tab its URL would leave it `Pending` meanwhile, and a
    /// discovery that removed it then would record no `Closed` for a tab already open.
    fn line_for(&self, event: &CdpEvent) -> Option<TargetId> {
        if self.lines.is_empty() {
            return None;
        }
        if matches!(
            event.method.as_str(),
            "Target.targetCreated" | "Target.targetInfoChanged"
        ) {
            return None;
        }
        let named = |key: Option<&str>| key.and_then(|key| self.held_attachments.get(key));
        // What a session or target whose attachment waits in a line reports waits behind it,
        // bookkeeping included: a worker's readiness or its end must not pass its attachment.
        let held = named(event.session_id.as_ref().map(SessionId::as_str))
            .or_else(|| named(event.params.get("sessionId").and_then(Value::as_str)))
            .or_else(|| named(event.params.get("targetId").and_then(Value::as_str)));
        if let Some(tab) = held {
            return self.lines.contains_key(tab).then(|| tab.clone());
        }
        let tab = match event.method.as_str() {
            "Browser.downloadWillBegin" => {
                let frame = FrameId(event.params.get("frameId")?.as_str()?.to_owned());
                // Browser events have no page session. A held attachment already identifies
                // its tab, but must reach the tracker before the download queries ownership.
                self.frame_owner(&frame)
                    .or_else(|| {
                        self.lines.iter().find_map(|(tab, line)| {
                            line.iter()
                                .any(|item| {
                                    let LineItem::Event { event, .. } = item else {
                                        return false;
                                    };
                                    let id = match event.method.as_str() {
                                        "Page.frameAttached" | "Page.navigatedWithinDocument" => {
                                            event.params.get("frameId")
                                        }
                                        "Page.frameNavigated" => event
                                            .params
                                            .get("frame")
                                            .and_then(|frame| frame.get("id")),
                                        _ => None,
                                    };
                                    id.and_then(Value::as_str) == Some(frame.0.as_str())
                                })
                                .then_some(tab)
                        })
                    })?
                    .clone()
            }
            "Browser.downloadProgress" => {
                let guid = event.params.get("guid")?.as_str()?;
                // Completion for an unknown GUID is discarded. It must follow a held begin
                // through the same line so DownloadTable registration always happens first.
                self.lines
                    .iter()
                    .find_map(|(tab, line)| {
                        line.iter()
                            .any(|item| {
                                matches!(item, LineItem::Event { event, .. }
                                    if event.method == "Browser.downloadWillBegin"
                                        && event.params.get("guid").and_then(Value::as_str)
                                            == Some(guid))
                            })
                            .then_some(tab)
                    })?
                    .clone()
            }
            "Target.attachedToTarget"
                if event
                    .params
                    .get("targetInfo")
                    .and_then(|info| info.get("type"))
                    .and_then(Value::as_str)
                    == Some("iframe") =>
            {
                return None;
            }
            "Runtime.executionContextCreated"
            | "Runtime.executionContextDestroyed"
            | "Runtime.executionContextsCleared"
            | "Inspector.workerScriptLoaded" => return None,
            // A frame or worker session going: named in the params, not by the event's session.
            "Target.detachedFromTarget" => self
                .sessions
                .get(&SessionId::from(event.params.get("sessionId")?.as_str()?))?
                .clone(),
            "Target.targetDestroyed" | "Target.targetCrashed" => {
                TargetId(event.params.get("targetId")?.as_str()?.to_owned())
            }
            _ => self.sessions.get(event.session_id.as_ref()?)?.clone(),
        };
        self.lines.contains_key(&tab).then_some(tab)
    }

    /// A session's tree came back: its read in the line is answered, to be applied in order by
    /// [`Self::advance_lines`]. A read its tab's end already answered (`end_reads`) is no longer
    /// out, so the tree of a gone tab is dropped here.
    fn seeded_tree(&mut self, SeededTree { session, tree }: SeededTree) {
        let read = self
            .lines
            .values_mut()
            .flat_map(|line| line.iter_mut())
            .find_map(|item| match item {
                LineItem::Read {
                    session: asked,
                    answer: answer @ TreeAnswer::Out,
                    ..
                } if *asked == session => Some(answer),
                _ => None,
            });
        if let Some(answer) = read {
            *answer = TreeAnswer::Back(tree);
        }
    }

    /// Applies, in each tab's order, every answered read and the events behind it, up to the
    /// next read still out; then answers the synchronize requests the log now covers.
    async fn advance_lines(&mut self) {
        loop {
            let next = self.lines.iter_mut().find_map(|(tab, line)| {
                let ready = match line.front() {
                    Some(LineItem::Read { answer, .. }) => matches!(answer, TreeAnswer::Back(_)),
                    Some(LineItem::Event { .. } | LineItem::Load { .. }) => true,
                    None => false,
                };
                if ready {
                    line.pop_front().map(|item| (tab.clone(), item))
                } else {
                    None
                }
            });
            let Some((tab, item)) = next else {
                break;
            };
            match item {
                LineItem::Read {
                    session,
                    read,
                    answer,
                    ..
                } => {
                    if let TreeAnswer::Back(Some(tree)) = answer {
                        self.lifecycle.seed(&tab, tree);
                        self.read_retries.remove(&session);
                        if let Some(observer) = self.root_observers.get_mut(&session) {
                            observer.recovery = RecoveryObservation::On;
                        }
                    } else if self.sessions.contains_key(&session) {
                        // A session gone with its tab, or detached, has nothing to read again.
                        self.retry_later(&tab, session, read);
                    }
                    self.release_downloads().await;
                }
                LineItem::Load { kind, .. } => self.record_load(tab, kind),
                LineItem::Event { event, position } => {
                    // What the event queues for its own tab (a load event's new read and the load
                    // behind it, `page_load`) goes ahead of the tab's events read after it, as
                    // when an event is handled as it is read: those come back behind it.
                    // The tab's own `Target.targetDestroyed` ends the reads behind it, as
                    // `destroyed` ends the reads of the line it finds.
                    let destroys = event.method == "Target.targetDestroyed"
                        && event.params.get("targetId").and_then(Value::as_str)
                            == Some(tab.as_str());
                    let later = self.lines.remove(&tab);
                    self.handle_at(event, EventFence(position.saturating_add(1)))
                        .await;
                    if let Some(mut later) = later {
                        if destroys {
                            end_reads(&mut later);
                        }
                        self.lines.entry(tab).or_default().extend(later);
                    }
                }
            }
        }
        self.lines.retain(|_, line| !line.is_empty());
        // Every held attachment was handled with its line: the loop knows those sessions now.
        let lines = &self.lines;
        self.held_attachments
            .retain(|_, tab| lines.contains_key(tab));
        self.publish_hold();
        self.answer_deferred();
    }

    /// Tells [`WatcherProgress`] the oldest event the watcher holds, in a line or as a waiting
    /// download, so no fence past it counts as in the log. Every item counts, not only each line's
    /// first, so the hold stays right whatever order a line holds its items in. It equals the
    /// first-item hold while lines stay sorted, as [`place_by_position`] and the replay keep them,
    /// so no row pins it; it stays in case a later change breaks that order.
    fn publish_hold(&self) {
        let oldest = self
            .lines
            .values()
            .flatten()
            .filter_map(LineItem::position)
            .chain(
                self.unowned_downloads
                    .iter()
                    .map(|download| download.position),
            )
            .min();
        self.progress.hold_from(oldest);
    }

    /// Answers the deferred synchronize requests whose fence the log now covers.
    fn answer_deferred(&mut self) {
        if self.deferred.is_empty() {
            return;
        }
        // Publish crash facts before waking operations whose event prefix is now applied.
        self.crashes
            .apply_replies(self.browser.connection().connection_epoch());
        let applied = self.progress.applied_through();
        let (ready, waiting): (Vec<_>, Vec<_>) = mem::take(&mut self.deferred)
            .into_iter()
            .partition(|request| request.fence.0 <= applied);
        self.deferred = waiting;
        for request in ready {
            let _ = request.done.send(());
        }
    }

    /// Both public callbacks and navigation waits observe the same ordered CDP transition.
    async fn frame_changed(
        &mut self,
        method: &str,
        session: Option<&SessionId>,
        params: Value,
        ingress: EventFence,
    ) {
        if let Some(session) = session {
            self.target_of(session).await;
            if !self.lifecycle_sources.accepts(session) {
                return;
            }
            self.frame_lifecycle(method, session, &params).await;
        }
        self.navigated(method, session, params, ingress).await;
    }

    /// A tab's `load` or `DOMContentLoaded`, recorded with its main frame. Load notifications
    /// from an OOPIF belong to its Frame, never to the page or context.
    ///
    /// Only a tree read names the main frame of a tab first met with its document already
    /// committed. While it is unknown, the event waits in the tab's line behind its session's
    /// read: the one in the line, or else a new one. So a tab whose read failed or missed
    /// [`TREE_READ_WAIT`] (a popup or an adopted tab whose renderer was busy) is read again at
    /// its next load event, and its `load` is not lost until its next main-frame commit; a read
    /// that came back with a tree named the main frame, so none follows it. Its readers are a script's page and context `load` and
    /// `domcontentloaded` listeners and an awaited document's readiness.
    async fn page_load(&mut self, method: &str, session: &SessionId, ingress: EventFence) {
        let Some(target) = self.target_of(session).await else {
            return;
        };
        if !self.lifecycle_sources.accepts(session)
            // The chosen root session can predate core's most recently attached session.
            || self.lifecycle_sources.targets.get(session) != Some(&target)
        {
            return;
        }
        let kind = if method == "Page.loadEventFired" {
            PageEventKind::Load
        } else {
            PageEventKind::DomContentLoaded
        };
        if self.lifecycle.main(&target).is_none()
            && self.start_seed(&target, session, TreeRead::Page).await
        {
            self.lines
                .entry(target)
                .or_default()
                .push_back(LineItem::Load {
                    position: ingress.0.saturating_sub(1),
                    kind,
                });
            return;
        }
        self.record_load(target, kind);
    }

    /// Records `target`'s load event of `kind` with its main frame, if the tracker knows it and the
    /// tab's source writes records ([`Self::records_page_events`]). A load that waited in the
    /// tab's line carries no session: it is the tab's source as the load is written that decides,
    /// so a session of core's that attached meanwhile has it written.
    fn record_load(&mut self, target: TargetId, kind: PageEventKind) {
        let source = self.lifecycle_sources.primary.get(&target).cloned();
        if !self.records_page_events(&target, source.as_ref()) {
            return;
        }
        if let Some(frame) = self.lifecycle.main(&target) {
            self.writer.record(PageEvent::Load {
                target,
                kind,
                frame,
            });
        }
    }

    /// The frame log's part of a frame event: an attached or removed child frame, or the main
    /// frame's commit. Same-document navigations and process swaps change no number.
    async fn frame_lifecycle(&mut self, method: &str, session: &SessionId, params: &Value) {
        if let Some(target) = self.target_of(session).await {
            let record = self.records_page_events(&target, Some(session));
            let mut changed = false;
            for event in self.lifecycle.record(&target, method, params) {
                if record {
                    self.writer.record(PageEvent::FrameLifecycle(event));
                }
                changed = true;
            }
            // A child frame attached, left or committed, or the main frame did: the tab's tree
            // is no longer the one a run last read (`EventWriter::frames_changed`).
            if changed || method == "Page.frameNavigated" {
                self.writer.frames_changed(target);
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

    /// A JavaScript dialog opened: held for the runs that listen for it, recorded, and closed
    /// after [`DIALOG_GRACE`] when nobody does. That is Neo's only on a tab a session of core's is
    /// on when the dialog opens: one a run acted on (agent-created or adopted), or, while a run
    /// listens on the context for lifecycle events, any tab core attached for that listener
    /// (`attach_page`), as on main. A dialog that only a root observer reports, on a tab no
    /// session of core's is on, is left alone: no hold, no grace, no close and no record, so the
    /// user answers it; closed after the grace, a `confirm()`, `prompt()` or `beforeunload` on
    /// the user's tab would be dismissed by Neo within 250 ms. Core's page signals accept alerts
    /// only on core's own sessions, so they leave such a tab's alerts alone too. Once a session
    /// of core's is on the tab, its dialogs are Neo's, whichever session is its lifecycle source.
    async fn dialog_opened(&mut self, session: SessionId, params: DialogParams) {
        let target = self.target_of(&session).await;
        // A modal is reported on every attached session. Use the selected native-target
        // source so even an automatically answered alert has one opening/closure pair.
        if !self.lifecycle_sources.accepts(&session) {
            return;
        }
        if target
            .as_ref()
            .is_some_and(|target| !self.core_attached(target))
        {
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

    /// Whether a page-domain event of `tab`'s that came on `session`, applied now, gets a record in
    /// the shared event log (a frame's attach, removal or commit, a navigation, a load), so that
    /// the log holds what it held on main. It does not for an event of the tab's root observer
    /// while no session of core's is on the tab ([`Self::core_attached`], the dialog rule's
    /// predicate): on main that observer had `Page` off and reported nothing. The log is one ring
    /// for every run, 1,024 records long (`LOG_CAPACITY`), which an untouched tab churning frames
    /// would fill for nobody: every reader of these records has a session of core's on the tab
    /// first (a page listener is admitted on the tab, a context listener attaches core to every
    /// tab). An out-of-process frame's session records as on main, and so does an observer whose
    /// tab went through crash recovery (`RootObserver::after_crash`). The tracker and the tab's
    /// frame version follow every event either way, and write nothing to the ring
    /// (`EventWriter::frames_changed`). Asked as the event is applied, in its line: from core's
    /// attach on, the tab's events are recorded, in order, whichever session is its source;
    /// events its line still held from before the attach replay after it and are recorded too.
    /// `session` is the one the event came on, or for a load the tab's source as it is written;
    /// with none, the event is recorded.
    fn records_page_events(&self, tab: &TargetId, session: Option<&SessionId>) -> bool {
        if self.core_attached(tab) {
            return true;
        }
        let Some(observer) = session.and_then(|session| self.root_observers.get(session)) else {
            return true;
        };
        observer.after_crash
    }

    /// Whether a session of core's is on `target`'s tab: one of its own page sessions, from an
    /// attach the watcher saw or core's registry, as opposed to the watcher's root observer.
    fn core_attached(&self, target: &TargetId) -> bool {
        self.lifecycle_sources
            .targets
            .iter()
            .any(|(session, known)| known == target && !self.root_observers.contains_key(session))
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

    /// Raw frame ownership, before download-only popups are attributed to their opener.
    /// OOPIF targets remain a fallback while their lifecycle tree read is outstanding.
    fn frame_owner(&self, frame: &FrameId) -> Option<&TargetId> {
        self.pages
            .get_key_value(&TargetId(frame.0.clone()))
            .map(|(target, _)| target)
            .or_else(|| self.frames.get(&frame.0))
            .or_else(|| self.lifecycle.owner(frame))
    }

    fn download(&mut self, download: DownloadParams) {
        let frame_target = TargetId(download.frame_id.0.clone());
        let page = match self.pages.get(&frame_target) {
            // A tab that opened only to download never gets a document: upstream reports the
            // download on its opener.
            Some(KnownPage {
                opener: Some(opener),
                state: PageState::Pending { .. },
                ..
            }) => Some(opener.clone()),
            Some(_) => Some(frame_target),
            None => self.frame_owner(&download.frame_id).cloned(),
        };
        self.writer.hub().downloads().begin(
            download.guid.clone(),
            download.url.clone(),
            download.suggested_filename.clone(),
        );
        self.writer.record(PageEvent::Download(DownloadStarted {
            page,
            frame: download.frame_id,
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

    /// Whether `session` is among its tab's lifecycle sessions, its source or one in waiting.
    fn entered(&self, session: &SessionId) -> bool {
        self.targets.contains_key(session)
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

/// Ends every read in a gone tab's line ([`Watcher::destroyed`]): each counts as answered with no
/// tree, a tree already back included, so popping it seeds nothing; a read's own answer, landing
/// later, finds no read out and is dropped (`seeded_tree`).
fn end_reads(line: &mut TabLine) {
    for item in line {
        if let LineItem::Read { answer, .. } = item {
            *answer = TreeAnswer::Back(None);
        }
    }
}

/// Inserts `item`, an event read at `position`, into `line` right after the last item that must
/// come before it: an event read earlier, or a read that was out when it was read (index 0 when
/// none is). Not before the first read started later: a read a replayed load queued can stand at
/// the line's front with a late start, and the tab's older events behind it still come first.
/// A released download goes there, so it replays before every newer event of its tab, its
/// frame's removal among them.
fn place_by_position(line: &mut TabLine, position: u64, item: LineItem) {
    let at = line
        .iter()
        .rposition(|queued| match queued {
            LineItem::Event {
                position: queued, ..
            }
            | LineItem::Load {
                position: queued, ..
            } => *queued < position,
            LineItem::Read { started, .. } => *started <= position,
        })
        .map_or(0, |last| last + 1);
    line.insert(at, item);
}

/// One frame-tree read for the tracker ([`Watcher::start_seed`]), within [`TREE_READ_WAIT`];
/// `None` when it failed or ran out.
async fn read_tree(protocol: &ProtocolSession, read: TreeRead) -> Option<Value> {
    let tree = match read {
        TreeRead::Page => {
            time::timeout(
                TREE_READ_WAIT,
                protocol.send_value("Page.getFrameTree", json!({})),
            )
            .await
        }
        // A frame's own commits reach the watcher only from its own session, and only once `Page`
        // is on there. Core turns it on only on tabs it created, for its file chooser
        // interception; on an adopted tab (the owner's, another session's) only a frames read
        // would (`frame_tree::oopif_tree`). Both commands go out together, `Page.enable` first:
        // `join` polls them in order and the client writes through a fair lock, so the frame's
        // renderer gets the enable first and answers in that order. A commit made before `Page`
        // was on is in the tree (`FrameLifecycleTracker::seed` gives it to the frame), and one
        // made after sends its own event, which waits in the tab's line until this read is
        // applied. Awaiting the enable before sending the read measured worse: on an adopted
        // tab, whose frames all attach at once, one frame's read missed the deadline in each of
        // two runs. Sent together, the read keeps its place in the renderer's queue, likely
        // ahead of core's own setup of the frame (inferred). The read counts only when the enable
        // succeeded: a tree that came with a failed enable is of a session that reports nothing,
        // and seeding from it would end the retries while the session's events never come.
        TreeRead::Frame => time::timeout(
            TREE_READ_WAIT,
            future::join(
                protocol.send_value("Page.enable", json!({})),
                protocol.send_value("Page.getFrameTree", json!({})),
            ),
        )
        .await
        .map(|(enabled, tree)| enabled.and(tree)),
    };
    tree.ok().and_then(Result::ok)
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

    use std::time::Duration;

    use browseros_cdp::CdpEvent;
    use browseros_core::testing::{TestCall, TestConnection};
    use browseros_core::{
        BrowserSession, BrowserSessionHooks, CdpConnection, SessionId, TabId, TargetId,
    };
    use serde_json::json;
    use tokio::sync::{broadcast, mpsc, oneshot};
    use tokio::{task, time};

    use std::collections::VecDeque;

    use super::{EventFence, SeededTree, Watcher, WatcherHandle, WatcherRequest};
    use crate::events::worker_table::WorkerInfo;
    use crate::events::{EventHub, PageEvent, PageEventKind, Subscription, WatcherProgress};
    use crate::run_state::RunState;
    use crate::snapshot::{Baseline, SnapshotBaselines, SnapshotForm, TextForm};
    use crate::world::FrameAttachLog;

    /// A request is owed only the events broadcast before its fence: the watcher stops there
    /// with later ones still queued, so a page that keeps sending (frames that attach and
    /// detach all the time) cannot hold a reply past the events it is owed.
    #[tokio::test]
    async fn a_drain_stops_at_its_fence_while_later_events_wait() -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([]);
        let browser = BrowserSession::new(
            Arc::clone(&connection) as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        );
        let (_hub, writer) = EventHub::new();
        let progress = Arc::new(WatcherProgress::default());
        let (mut watcher, _found) = Watcher::new(
            browser,
            writer,
            Arc::new(FrameAttachLog::default()),
            Arc::clone(&progress),
            Arc::new(SnapshotBaselines::default()),
            Arc::default(),
        );
        let (sender, mut events) = broadcast::channel(16);
        progress.start(0, 0);
        for n in 0..10 {
            sender
                .send(CdpEvent {
                    method: "Page.lifecycleEvent".to_owned(),
                    params: json!({ "n": n }),
                    session_id: None,
                })
                .map_err(|error| error.to_string())?;
        }
        watcher.drain_through(&mut events, EventFence(3)).await;
        assert_eq!(progress.events_handled(), 3, "every event before the fence");
        assert_eq!(events.len(), 7, "the later ones wait for the run loop");
        watcher.drain_through(&mut events, EventFence(100)).await;
        assert_eq!(
            progress.events_handled(),
            10,
            "short of its fence, a drain ends when nothing more is queued"
        );
        Ok(())
    }

    /// Handles `event` as the run loop does: held in its tab's line or handled, then counted
    /// as read.
    async fn read(watcher: &mut Watcher, event: CdpEvent) {
        if let Some(event) = watcher.hold(event) {
            watcher.handle(event).await;
        }
        watcher.progress.handled(1);
    }

    /// A new out-of-process frame's session is asked to turn `Page` on before its tree is read,
    /// on that session: the frame's renderer answers in order, so a commit it makes in between
    /// is either in the tree or reported as its own `Page.frameNavigated`, never lost.
    #[tokio::test]
    async fn a_frame_session_turns_page_on_before_its_tree_is_read() -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([]);
        connection.always("Page.enable", json!({}))?;
        connection.always(
            "Page.getFrameTree",
            json!({ "frameTree": { "frame": { "id": "ad", "url": "https://ad.test/" } } }),
        )?;
        let browser = BrowserSession::new(
            Arc::clone(&connection) as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        );
        let (_hub, writer) = EventHub::new();
        let progress = Arc::new(WatcherProgress::default());
        let (mut watcher, _found) = Watcher::new(
            browser,
            writer,
            Arc::new(FrameAttachLog::default()),
            Arc::clone(&progress),
            Arc::new(SnapshotBaselines::default()),
            Arc::default(),
        );
        progress.start(0, 0);
        let (seeded, mut answers) = mpsc::unbounded_channel();
        watcher.seeded = Some(seeded);
        let (tab, page) = (TargetId("tab".to_owned()), SessionId::from("page"));
        watcher.sessions.insert(page.clone(), tab.clone());
        watcher.lifecycle_sources.attach(&page, &tab);
        read(
            &mut watcher,
            CdpEvent {
                method: "Target.attachedToTarget".to_owned(),
                params: json!({ "sessionId": "ad",
                    "targetInfo": { "targetId": "ad", "type": "iframe", "url": "https://ad.test/" } }),
                session_id: Some(page),
            },
        )
        .await;
        let answer = time::timeout(Duration::from_secs(30), answers.recv())
            .await?
            .ok_or("the frame's tree read answers")?;
        assert!(answer.tree.is_some(), "the frame's tree was read");
        let ad = Some(SessionId::from("ad"));
        let asked: Vec<String> = connection
            .calls()?
            .into_iter()
            .filter(|call: &TestCall| call.session == ad)
            .map(|call| call.method)
            .filter(|method| method.starts_with("Page."))
            .collect();
        assert_eq!(
            asked,
            ["Page.enable", "Page.getFrameTree"],
            "Page on first, then the tree, on the frame's own session"
        );
        Ok(())
    }

    /// While a new frame session's tree read is out, the tab's later events wait behind it and
    /// run after its answer, in the order the loop read them: a child known only to that tree
    /// and removed before the answer lands is still published removed, with its payload, and a
    /// synchronize whose fence covers a held event is answered only once it is in the log.
    #[tokio::test]
    async fn a_tab_s_events_wait_behind_its_frame_tree_read() -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([]);
        let browser = BrowserSession::new(
            Arc::clone(&connection) as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        );
        let (hub, writer) = EventHub::new();
        let progress = Arc::new(WatcherProgress::default());
        let (mut watcher, _found) = Watcher::new(
            browser,
            writer,
            Arc::new(FrameAttachLog::default()),
            Arc::clone(&progress),
            Arc::new(SnapshotBaselines::default()),
            Arc::default(),
        );
        progress.start(0, 0);
        let (seeded, _answers) = mpsc::unbounded_channel();
        watcher.seeded = Some(seeded);
        let (tab, page, ad) = (
            TargetId("tab".to_owned()),
            SessionId::from("page"),
            SessionId::from("ad"),
        );
        watcher.sessions.insert(page.clone(), tab.clone());
        watcher.lifecycle_sources.attach(&page, &tab);
        // The ad frame's own session attaches: its tree read goes out.
        read(
            &mut watcher,
            CdpEvent {
                method: "Target.attachedToTarget".to_owned(),
                params: json!({ "sessionId": "ad",
                    "targetInfo": { "targetId": "ad", "type": "iframe", "url": "https://ad.test/" } }),
                session_id: Some(page.clone()),
            },
        )
        .await;
        // A child the session never announced goes before the answer comes back.
        read(
            &mut watcher,
            CdpEvent {
                method: "Page.frameDetached".to_owned(),
                params: json!({ "frameId": "child", "reason": "remove" }),
                session_id: Some(ad.clone()),
            },
        )
        .await;
        assert_eq!(progress.events_handled(), 2, "both read");
        assert_eq!(progress.applied_through(), 1, "the removal is held");
        let (done, mut answered) = oneshot::channel();
        watcher.deferred.push(WatcherRequest {
            closed: None,
            handed_back: None,
            fence: EventFence(2),
            done,
        });
        watcher.answer_deferred();
        assert!(
            answered.try_recv().is_err(),
            "its fence covers the held removal"
        );

        watcher.seeded_tree(SeededTree {
            session: ad,
            tree: Some(json!({ "frameTree": {
                "frame": { "id": "ad", "url": "https://ad.test/" },
                "childFrames": [{ "frame": { "id": "child", "parentId": "ad",
                    "url": "https://ad.test/child" } }]
            } })),
        });
        watcher.advance_lines().await;
        let removed: Vec<String> = hub
            .read_after(0)
            .records
            .into_iter()
            .filter_map(|record| match record.event {
                PageEvent::FrameLifecycle(event) if event.kind == PageEventKind::FrameDetached => {
                    Some(event.frame.url)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            removed,
            ["https://ad.test/child"],
            "removed, with the tree's payload"
        );
        assert!(watcher.lines.is_empty());
        assert_eq!(progress.applied_through(), 2);
        assert!(answered.try_recv().is_ok(), "answered once the log has it");
        Ok(())
    }

    /// A watcher whose tab `tab` (sessions `page` and `ad`) has a frame-tree read out; another
    /// tab has session `elsewhere`.
    fn watcher_with_a_line() -> Watcher {
        let connection = TestConnection::new([]);
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
            Arc::default(),
        );
        let tab = TargetId("tab".to_owned());
        watcher
            .sessions
            .insert(SessionId::from("page"), tab.clone());
        watcher.sessions.insert(SessionId::from("ad"), tab.clone());
        watcher
            .sessions
            .insert(SessionId::from("elsewhere"), TargetId("other".to_owned()));
        watcher.lines.insert(tab, VecDeque::new());
        watcher
    }

    fn cdp(method: &str, session: &str, params: serde_json::Value) -> CdpEvent {
        CdpEvent {
            method: method.to_owned(),
            params,
            session_id: Some(SessionId::from(session)),
        }
    }

    fn worker_attached() -> CdpEvent {
        cdp(
            "Target.attachedToTarget",
            "page",
            json!({ "sessionId": "w", "targetInfo": { "targetId": "w", "type": "worker" } }),
        )
    }

    /// What waits in a tab's line while its frame-tree read is out: everything of that tab that
    /// publishes or changes the worker table (a worker's attachment, a console report, a frame
    /// event, a frame session's detach, the tab's end), but not a new frame session's attachment
    /// (it only adds a read) nor context bookkeeping; and nothing of another tab.
    #[tokio::test]
    async fn a_line_holds_what_publishes_and_lets_bookkeeping_through() -> Result<(), Box<dyn Error>>
    {
        let watcher = watcher_with_a_line();
        let tab = Some(TargetId("tab".to_owned()));
        let held = [
            worker_attached(),
            cdp(
                "Runtime.consoleAPICalled",
                "page",
                json!({ "type": "log", "args": [] }),
            ),
            cdp(
                "Page.frameNavigated",
                "page",
                json!({ "frame": { "id": "tab", "url": "a" } }),
            ),
            cdp(
                "Target.detachedFromTarget",
                "page",
                json!({ "sessionId": "ad" }),
            ),
            cdp(
                "Target.targetDestroyed",
                "browser",
                json!({ "targetId": "tab" }),
            ),
        ];
        for event in &held {
            assert_eq!(watcher.line_for(event), tab, "{} waits", event.method);
        }
        let through = [
            cdp(
                "Target.attachedToTarget",
                "page",
                json!({ "sessionId": "f", "targetInfo": { "targetId": "f", "type": "iframe" } }),
            ),
            cdp(
                "Runtime.executionContextCreated",
                "page",
                json!({ "context": {} }),
            ),
            cdp(
                "Page.frameNavigated",
                "elsewhere",
                json!({ "frame": { "id": "o", "url": "b" } }),
            ),
        ];
        for event in &through {
            assert_eq!(watcher.line_for(event), None, "{} goes on", event.method);
        }
        Ok(())
    }

    /// A worker whose attachment waits in a line: what its session and target report waits
    /// behind it, its readiness and its end included, though the loop does not know them yet.
    #[tokio::test]
    async fn a_held_worker_s_reports_wait_behind_its_attachment() -> Result<(), Box<dyn Error>> {
        let mut watcher = watcher_with_a_line();
        let tab = Some(TargetId("tab".to_owned()));
        assert!(
            watcher.hold(worker_attached()).is_none(),
            "the attachment waits"
        );
        let worker = [
            cdp("Inspector.workerScriptLoaded", "w", json!({})),
            cdp(
                "Runtime.executionContextCreated",
                "w",
                json!({ "context": {} }),
            ),
            cdp(
                "Target.detachedFromTarget",
                "page",
                json!({ "sessionId": "w" }),
            ),
            cdp(
                "Target.targetDestroyed",
                "browser",
                json!({ "targetId": "w" }),
            ),
        ];
        for event in &worker {
            assert_eq!(
                watcher.line_for(event),
                tab,
                "the worker's {}",
                event.method
            );
        }
        Ok(())
    }

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
            Arc::default(),
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
                    inert: false,
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
            Arc::default(),
        );
        watcher
            .observe_workers(TargetId("untouched-page".to_owned()))
            .await;
        // The auto-attach goes out on a task of its own.
        settle().await;
        let calls = connection.calls()?;
        assert_eq!(
            calls.first().map(|call| call.method.as_str()),
            Some("Target.attachToTarget")
        );
        let armed = calls
            .iter()
            .find(|call| call.method == "Target.setAutoAttach")
            .ok_or("the observer arms descendant discovery")?;
        assert_eq!(
            armed.params["filter"],
            json!([
                {"type":"worker"}, {"type":"iframe"}, {"exclude":true}
            ])
        );
        assert_eq!(
            armed.params["waitForDebuggerOnStart"],
            json!(false),
            "nothing the observer attaches may wait for it to resume"
        );
        assert!(
            !calls.iter().any(|call| call.method.starts_with("Runtime.")),
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
            Arc::default(),
        );
        (hub, watcher)
    }

    /// A same-process child created after the initial tree has no target attachment of its
    /// own. Its download must still reach listeners on the page whose session attached it.
    #[tokio::test]
    async fn a_same_process_iframe_download_is_reported_on_its_page() -> Result<(), Box<dyn Error>>
    {
        let connection = TestConnection::new([
            (
                "Target.getTargets",
                json!({ "targetInfos": [
                    { "targetId": "page", "type": "page", "url": "https://example.test/" }
                ] }),
            ),
            ("Target.setDiscoverTargets", json!({})),
            ("Browser.setDownloadBehavior", json!({})),
            (
                "Page.getFrameTree",
                json!({ "frameTree": { "frame": {
                    "id": "page", "url": "https://example.test/"
                } } }),
            ),
        ]);
        let (hub, mut watcher) = scripted(&connection);
        watcher.discover(false).await?;
        watcher
            .handle(CdpEvent {
                method: "Target.attachedToTarget".to_owned(),
                params: json!({ "sessionId": "page-session", "targetInfo": {
                    "targetId": "page", "type": "page", "url": "https://example.test/"
                } }),
                session_id: None,
            })
            .await;
        watcher
            .handle(CdpEvent {
                method: "Page.frameAttached".to_owned(),
                params: json!({ "frameId": "child", "parentFrameId": "page" }),
                session_id: Some(SessionId::from("page-session")),
            })
            .await;
        assert!(
            hub.read_after(0).records.into_iter().any(|record| matches!(
                record.event,
                PageEvent::FrameLifecycle(event)
                    if event.kind == PageEventKind::FrameAttached
                        && event.target.as_str() == "page"
                        && event.frame.id.0 == "child"
            )),
            "the page session must publish its child's attachment before the download"
        );
        watcher
            .handle(CdpEvent {
                method: "Browser.downloadWillBegin".to_owned(),
                params: json!({ "frameId": "child", "guid": "download",
                    "url": "https://example.test/attachment.txt",
                    "suggestedFilename": "iframe-attachment.txt" }),
                session_id: None,
            })
            .await;
        let download = hub
            .read_after(0)
            .records
            .into_iter()
            .find_map(|record| match record.event {
                PageEvent::Download(download) => Some(download),
                _ => None,
            })
            .ok_or("the browser's download must be published")?;
        assert_eq!(
            download.page,
            Some(TargetId("page".to_owned())),
            "a child without its own target still downloads on its parent page"
        );
        Ok(())
    }

    /// Root download events must share the tab's ordering barrier: otherwise the begin can
    /// pass its frame attachment, or completion can pass a held begin and be discarded.
    #[tokio::test]
    async fn a_same_process_iframe_download_waits_for_its_attachment_and_keeps_completion()
    -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([]);
        let (hub, mut watcher) = scripted(&connection);
        watcher.progress.start(0, 0);
        let page = TargetId("page".to_owned());
        let page_session = SessionId::from("page-session");
        let frame_session = SessionId::from("remote-session");
        watcher.sessions.insert(page_session.clone(), page.clone());
        watcher.lifecycle_sources.attach(&page_session, &page);
        let (seeded, _answers) = mpsc::unbounded_channel();
        watcher.seeded = Some(seeded);

        // An OOPIF's tree read holds later events from every session of its tab, including
        // the page session that announces this new same-process child.
        read(
            &mut watcher,
            cdp(
                "Target.attachedToTarget",
                "page-session",
                json!({ "sessionId": "remote-session", "targetInfo": {
                    "targetId": "remote", "type": "iframe", "url": "https://example.test/frame"
                } }),
            ),
        )
        .await;
        read(
            &mut watcher,
            cdp(
                "Page.frameAttached",
                "page-session",
                json!({ "frameId": "child", "parentFrameId": "page" }),
            ),
        )
        .await;
        for (method, params) in [
            (
                "Browser.downloadWillBegin",
                json!({ "frameId": "child", "guid": "download",
                    "url": "https://example.test/attachment.txt",
                    "suggestedFilename": "iframe-attachment.txt" }),
            ),
            (
                "Browser.downloadProgress",
                json!({ "guid": "download", "state": "completed",
                    "filePath": "/downloads/iframe-attachment.txt" }),
            ),
        ] {
            read(
                &mut watcher,
                CdpEvent {
                    method: method.to_owned(),
                    params,
                    session_id: None,
                },
            )
            .await;
        }
        let published_before_tree = hub
            .read_after(0)
            .records
            .iter()
            .any(|record| matches!(record.event, PageEvent::Download(_)));

        watcher.seeded_tree(SeededTree {
            session: frame_session,
            tree: Some(json!({ "frameTree": { "frame": {
                "id": "remote", "parentId": "page", "url": "https://example.test/frame"
            } } })),
        });
        watcher.advance_lines().await;
        assert!(
            !published_before_tree,
            "the download must wait until its held frame attachment is published"
        );
        check_child_download(&hub, &page)
    }

    /// Checks the published order and final file together: a page event without its later
    /// completion would still leave the host waiting for bytes that the browser saved.
    fn check_child_download(hub: &EventHub, page: &TargetId) -> Result<(), Box<dyn Error>> {
        use std::path::PathBuf;

        use crate::DownloadState;

        let records = hub.read_after(0).records;
        let attached = records
            .iter()
            .position(|record| {
                matches!(
                    &record.event,
                    PageEvent::FrameLifecycle(event)
                        if event.kind == PageEventKind::FrameAttached
                            && &event.target == page
                            && event.frame.id.0 == "child"
                )
            })
            .ok_or("the held child attachment must be published")?;
        let (began, download) = records
            .iter()
            .enumerate()
            .find_map(|(position, record)| match &record.event {
                PageEvent::Download(download) => Some((position, download)),
                _ => None,
            })
            .ok_or("the browser's download must be published")?;
        assert!(
            attached < began,
            "the download must follow its held frame attachment"
        );
        assert_eq!(
            download.page.as_ref(),
            Some(page),
            "the same-process child's download belongs to its page"
        );
        let tracked = hub
            .downloads()
            .get("download")
            .ok_or("the download is listed")?;
        assert_eq!(
            tracked.facts().state,
            DownloadState::Completed(PathBuf::from("/downloads/iframe-attachment.txt")),
            "completed progress must follow its begin through the same ordering barrier"
        );
        Ok(())
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

#[cfg(test)]
mod crash_tests {
    use std::error::Error;
    use std::sync::Arc;
    use std::time::Duration;

    use browseros_cdp::CdpEvent;
    use browseros_core::testing::TestConnection;
    use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection, SessionId, TargetId};
    use serde_json::{Value, json};
    use tokio::sync::mpsc;
    use tokio::{task, time};

    use super::{LineItem, TreeAnswer, TreeRead, Watcher};
    use crate::events::{EventHub, PageEvent, WatcherProgress};
    use crate::snapshot::SnapshotBaselines;
    use crate::target_crashes::TargetCrashes;

    /// Task scheduling is progress, not a renderer deadline; tolerate a loaded test runner.
    const RECOVERY_REPLY_WATCHDOG: Duration = Duration::from_secs(30);

    fn watcher(connection: Arc<TestConnection>) -> Watcher {
        let browser = BrowserSession::new(
            connection as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        );
        let (_, writer) = EventHub::new();
        let progress = Arc::new(WatcherProgress::default());
        progress.start(0, 0);
        Watcher::new(
            browser,
            writer,
            Arc::default(),
            progress,
            Arc::new(SnapshotBaselines::default()),
            Arc::new(TargetCrashes::default()),
        )
        .0
    }

    fn event(method: &str, params: Value, session: Option<&SessionId>) -> CdpEvent {
        CdpEvent {
            method: method.to_owned(),
            params,
            session_id: session.cloned(),
        }
    }

    async fn read(watcher: &mut Watcher, event: CdpEvent) {
        if let Some(event) = watcher.hold(event) {
            watcher.handle(event).await;
        }
        watcher.progress.handled(1);
        watcher.apply_recoveries();
    }

    #[tokio::test]
    async fn target_crashed_reconnect_observes_an_external_reload_without_runtime_setup()
    -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([]);
        for (method, reply) in [
            (
                "Target.getTargets",
                json!({"targetInfos":[{"targetId":"page", "type":"page", "url":"https://a.test/"}]}),
            ),
            ("Target.setDiscoverTargets", json!({})),
            ("Browser.setDownloadBehavior", json!({})),
            (
                "Browser.getTabs",
                json!({"tabs":[{"tabId":7, "targetId":"page", "windowId":1, "url":"https://a.test/", "title":"A", "isActive":true, "isLoading":false, "loadProgress":1.0, "isPinned":false, "isHidden":false}]}),
            ),
            ("Page.enable", json!({})),
            (
                "Page.getFrameTree",
                json!({"frameTree":{"frame":{"id":"page", "loaderId":"recovered", "url":"https://a.test/"}}}),
            ),
        ] {
            connection.always(method, reply)?;
        }
        let mut watcher = watcher(Arc::clone(&connection));
        let target = TargetId("page".to_owned());
        let old = SessionId::from("old-session");
        watcher.sessions.insert(old.clone(), target.clone());
        watcher.lifecycle_sources.attach(&old, &target);
        read(
            &mut watcher,
            event("Target.targetCrashed", json!({"targetId":"page"}), None),
        )
        .await;
        watcher.discover(true).await?;
        assert!(
            !watcher.sessions.contains_key(&old),
            "reconnect retires old sessions"
        );
        let listing = watcher.view.list_pages().await?;
        assert_eq!(
            listing.pages.len(),
            1,
            "the reconnect fixture supplies a valid native tab"
        );
        watcher.observe_context().await;
        let observer = watcher
            .root_observers
            .keys()
            .next()
            .cloned()
            .ok_or("recovery observation reattaches the crashed target")?;
        time::timeout(RECOVERY_REPLY_WATCHDOG, async {
            loop {
                if connection
                    .calls()?
                    .iter()
                    .any(|call| call.method == "Page.enable")
                {
                    return Ok::<(), Box<dyn Error>>(());
                }
                task::yield_now().await;
            }
        })
        .await??;
        let (sender, mut replies) = mpsc::unbounded_channel();
        watcher.recovering = Some(sender);
        read(
            &mut watcher,
            event(
                "Page.frameNavigated",
                json!({"frame":{"id":"page", "loaderId":"recovered", "url":"https://a.test/"}}),
                Some(&observer),
            ),
        )
        .await;
        watcher.recovery_replies.push(
            time::timeout(RECOVERY_REPLY_WATCHDOG, replies.recv())
                .await?
                .ok_or("fresh recovery proof")?,
        );
        watcher.apply_recoveries();
        assert!(
            !watcher.crashes.is_crashed(&target),
            "an external reload can clear remembered refusal after reconnect"
        );
        assert!(
            !connection
                .calls()?
                .iter()
                .any(|call| call.method.starts_with("Runtime.")
                    || call.method == "Page.createIsolatedWorld"),
            "the observer installs no runtime or world"
        );
        Ok(())
    }

    #[tokio::test]
    async fn target_crashed_is_known_before_opening_and_before_a_tree_hold_releases() {
        let mut watcher = watcher(TestConnection::new([]));
        let target = TargetId("pending".to_owned());
        watcher
            .lines
            .entry(target.clone())
            .or_default()
            .push_back(LineItem::Read {
                session: SessionId::from("pending-session"),
                started: 0,
                read: TreeRead::Page,
                answer: TreeAnswer::Out,
            });
        read(
            &mut watcher,
            event("Target.targetCrashed", json!({"targetId":"pending"}), None),
        )
        .await;
        assert!(
            watcher.crashes.is_crashed(&target),
            "admission cannot wait behind renderer work or public opening"
        );
        assert_eq!(
            watcher.progress.applied_through(),
            0,
            "presentation still respects the held event"
        );
        for _ in 0..1100 {
            watcher
                .writer
                .record(PageEvent::Crashed(TargetId("other".to_owned())));
        }
        assert!(
            watcher.crashes.is_crashed(&target),
            "journal eviction cannot forget admission facts"
        );
        watcher.destroyed(&target);
        assert!(
            watcher.crashes.is_crashed(&target),
            "untagged destroy alone cannot establish closure"
        );
        watcher.lost_events();
        assert!(
            watcher.crashes.is_crashed(&target),
            "event lag invalidates proofs, not crash evidence"
        );
    }

    #[tokio::test]
    #[expect(
        clippy::too_many_lines,
        reason = "one target lifecycle checks the accepted proof, failed proof, and a newer crash against the same event and reply fences"
    )]
    async fn target_crashed_recovery_requires_a_fresh_matching_main_tree_and_applied_reply_fence()
    -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([(
            "Page.getFrameTree",
            json!({"frameTree":{"frame":{
            "id":"page", "loaderId":"new", "url":"https://a.test/new"}}}),
        )]);
        let mut watcher = watcher(Arc::clone(&connection));
        let target = TargetId("page".to_owned());
        let session = SessionId::from("page-session");
        watcher.sessions.insert(session.clone(), target.clone());
        watcher.lifecycle_sources.attach(&session, &target);
        let (sender, mut replies) = mpsc::unbounded_channel();
        watcher.recovering = Some(sender);
        read(
            &mut watcher,
            event("Target.targetCrashed", json!({"targetId":"page"}), None),
        )
        .await;
        for (method, params) in [
            (
                "Page.navigatedWithinDocument",
                json!({"frameId":"page", "url":"https://a.test/#same"}),
            ),
            (
                "Page.frameNavigated",
                json!({"frame":{"id":"child", "parentId":"page", "loaderId":"child", "url":"https://a.test/child"}}),
            ),
        ] {
            read(&mut watcher, event(method, params, Some(&session))).await;
        }
        assert!(
            replies.try_recv().is_err(),
            "same-document and child commits do not request proof"
        );
        read(
            &mut watcher,
            event(
                "Page.frameNavigated",
                json!({"frame":{
            "id":"page", "loaderId":"new", "url":"https://a.test/new"}}),
                Some(&session),
            ),
        )
        .await;
        let mut proof = time::timeout(RECOVERY_REPLY_WATCHDOG, replies.recv())
            .await?
            .ok_or("fresh proof reply")?;
        assert_eq!(
            connection
                .calls()?
                .iter()
                .filter(|call| call.method == "Page.getFrameTree")
                .count(),
            1
        );
        // Model an event received before that reply but still waiting in a tab's line.
        proof.fence = watcher.progress.events_handled() + 1;
        watcher.recovery_replies.push(proof);
        watcher.apply_recoveries();
        assert!(
            watcher.crashes.is_crashed(&target),
            "received events are not yet applied events"
        );
        read(&mut watcher, event("Unknown.event", json!({}), None)).await;
        assert!(
            !watcher.crashes.is_crashed(&target),
            "fresh matching main document and its fence prove recovery"
        );
        read(
            &mut watcher,
            event("Target.targetCrashed", json!({"targetId":"page"}), None),
        )
        .await;
        read(
            &mut watcher,
            event(
                "Page.frameNavigated",
                json!({"frame":{
            "id":"page", "loaderId":"new", "url":"https://a.test/new"}}),
                Some(&session),
            ),
        )
        .await;
        assert!(
            replies.try_recv().is_err(),
            "the known previous loader is not a replacement document"
        );

        connection.always(
            "Page.getFrameTree",
            json!({"frameTree":{"frame":{
            "id":"page", "loaderId":"different", "url":"https://a.test/new"}}}),
        )?;
        read(
            &mut watcher,
            event(
                "Page.frameNavigated",
                json!({"frame":{
            "id":"page", "loaderId":"attempt", "url":"https://a.test/new"}}),
                Some(&session),
            ),
        )
        .await;
        let proof = time::timeout(RECOVERY_REPLY_WATCHDOG, replies.recv())
            .await?
            .ok_or("mismatched proof reply")?;
        watcher.recovery_replies.push(proof);
        watcher.apply_recoveries();
        assert!(
            watcher.crashes.is_crashed(&target),
            "a fresh reply must match the candidate loader exactly"
        );

        connection.always(
            "Page.getFrameTree",
            json!({"frameTree":{"frame":{
            "id":"page", "loaderId":"next", "url":"https://a.test/new"}}}),
        )?;
        read(
            &mut watcher,
            event(
                "Page.frameNavigated",
                json!({"frame":{
            "id":"page", "loaderId":"next", "url":"https://a.test/new"}}),
                Some(&session),
            ),
        )
        .await;
        let mut proof = time::timeout(RECOVERY_REPLY_WATCHDOG, replies.recv())
            .await?
            .ok_or("later proof reply")?;
        proof.fence = watcher.progress.events_handled() + 1;
        watcher.recovery_replies.push(proof);
        read(
            &mut watcher,
            event("Target.targetCrashed", json!({"targetId":"page"}), None),
        )
        .await;
        assert!(
            watcher.crashes.is_crashed(&target),
            "new crash evidence defeats a proof waiting for its reply fence"
        );
        Ok(())
    }
}

/// The frame-tree reads that seed the tracker, and a context listener's page attach, as the run
/// loop drives them: none holds the loop, a tab's events keep their order behind its read, and a
/// read that fails or runs out lets them go.
#[cfg(test)]
mod seed_tests {
    use std::error::Error;
    use std::sync::Arc;
    use std::time::Duration;

    use browseros_cdp::CdpEvent;
    use browseros_core::testing::{TestConnection, cdp_error, cdp_pending};
    use browseros_core::{
        BrowserSession, BrowserSessionHooks, CdpConnection, SessionId, TabId, TargetId,
    };
    use serde_json::{Value, json};
    use tokio::sync::mpsc::{self, UnboundedReceiver};
    use tokio::sync::oneshot;
    use tokio::{task, time};

    use super::{
        CONTEXT_ATTACH_WAIT, DIALOG_GRACE, EventFence, FrameId, PageState, REAP, SeededTree, TICK,
        TREE_READ_WAIT, Watcher, WatcherRequest,
    };
    use crate::DownloadState;
    use crate::events::{EventHub, PageEvent, PageEventKind, Subscription, WatcherProgress};
    use crate::run_state::RunState;
    use crate::snapshot::SnapshotBaselines;
    use crate::world::FrameAttachLog;

    type TestResult = Result<(), Box<dyn Error>>;

    /// A watcher over `connection` whose tree reads answer on the returned channel, as the run
    /// loop's do.
    fn watcher(
        connection: &Arc<TestConnection>,
    ) -> (Arc<EventHub>, Watcher, UnboundedReceiver<SeededTree>) {
        let browser = BrowserSession::new(
            Arc::clone(connection) as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        );
        let (hub, writer) = EventHub::new();
        let progress = Arc::new(WatcherProgress::default());
        progress.start(0, 0);
        let (mut watcher, _found) = Watcher::new(
            browser,
            writer,
            Arc::new(FrameAttachLog::default()),
            progress,
            Arc::new(SnapshotBaselines::default()),
            Arc::default(),
        );
        let (seeded, answers) = mpsc::unbounded_channel();
        watcher.seeded = Some(seeded);
        (hub, watcher, answers)
    }

    /// Handles `event` as the run loop does: held in its tab's line or handled, then counted
    /// as read.
    async fn read(watcher: &mut Watcher, event: CdpEvent) {
        if let Some(event) = watcher.hold(event) {
            watcher.handle(event).await;
        }
        watcher.progress.handled(1);
    }

    /// Applies the next tree read's answer as the run loop does.
    async fn answer_next(
        watcher: &mut Watcher,
        answers: &mut UnboundedReceiver<SeededTree>,
    ) -> TestResult {
        let answer = time::timeout(Duration::from_secs(30), answers.recv())
            .await?
            .ok_or("a tree read answers")?;
        watcher.seeded_tree(answer);
        watcher.advance_lines().await;
        Ok(())
    }

    /// Session `session`, tab `tab`'s own, attaches.
    fn page_attached(session: &str, tab: &str) -> CdpEvent {
        CdpEvent {
            method: "Target.attachedToTarget".to_owned(),
            params: json!({ "sessionId": session,
                "targetInfo": { "targetId": tab, "type": "page", "url": "https://a.test/" } }),
            session_id: None,
        }
    }

    fn on(session: &str, method: &str, params: Value) -> CdpEvent {
        CdpEvent {
            method: method.to_owned(),
            params,
            session_id: Some(SessionId::from(session)),
        }
    }

    /// Tab `tab` appears, its own target event naming `url`.
    fn target_created(tab: &str, url: &str) -> CdpEvent {
        CdpEvent {
            method: "Target.targetCreated".to_owned(),
            params: json!({ "targetInfo": { "targetId": tab, "type": "page", "url": url } }),
            session_id: None,
        }
    }

    /// Tab `tab`'s target now shows `url`.
    fn info_changed(tab: &str, url: &str) -> CdpEvent {
        CdpEvent {
            method: "Target.targetInfoChanged".to_owned(),
            params: json!({ "targetInfo": { "targetId": tab, "type": "page", "url": url } }),
            session_id: None,
        }
    }

    /// Tab `tab`'s tree, its main frame at `url`.
    fn tree(tab: &str, url: &str) -> Value {
        json!({ "frameTree": { "frame": { "id": tab, "loaderId": "seeded", "url": url } } })
    }

    /// The log's load and frame records, in order: each load with its main frame's URL, each frame
    /// transition with its frame's URL, or its id while it has none.
    fn records(hub: &EventHub) -> Vec<String> {
        hub.read_after(0)
            .records
            .into_iter()
            .filter_map(|record| match record.event {
                PageEvent::Load { kind, frame, .. } => {
                    Some(format!("{} {}", kind.name(), frame.url))
                }
                PageEvent::FrameLifecycle(event) => {
                    let frame = if event.frame.url.is_empty() {
                        event.frame.id.0
                    } else {
                        event.frame.url
                    };
                    Some(format!("{} {frame}", event.kind.name()))
                }
                _ => None,
            })
            .collect()
    }

    /// The log's tab records, in order: each tab opened or closed, by its target.
    fn tab_records(hub: &EventHub) -> Vec<String> {
        hub.read_after(0)
            .records
            .into_iter()
            .filter_map(|record| match record.event {
                PageEvent::Opened(page) => Some(format!("opened {}", page.target.as_str())),
                PageEvent::Closed(target) => Some(format!("closed {}", target.as_str())),
                _ => None,
            })
            .collect()
    }

    /// Answers discovery's calls, `Target.getTargets` listing no tab.
    fn discovery_lists_nothing(connection: &TestConnection) -> TestResult {
        for (method, answer) in [
            ("Target.getTargets", json!({ "targetInfos": [] })),
            ("Target.setDiscoverTargets", json!({})),
            ("Browser.setDownloadBehavior", json!({})),
        ] {
            connection.always(method, answer)?;
        }
        Ok(())
    }

    /// How many tree reads went to `session`.
    fn tree_reads(connection: &TestConnection, session: &str) -> Result<usize, Box<dyn Error>> {
        Ok(connection
            .calls()?
            .iter()
            .filter(|call| {
                call.method == "Page.getFrameTree"
                    && call.session.as_ref().map(SessionId::as_str) == Some(session)
            })
            .count())
    }

    /// Lets spawned reads and attaches reach the connection.
    async fn settle() {
        for _ in 0..20 {
            task::yield_now().await;
        }
    }

    /// The first invariant of a tab's line: the tab's events are recorded in the order the tab
    /// sent them, whatever reads they wait behind. A `DOMContentLoaded` held behind the tab's
    /// failed first read reads the tab again when its turn comes, and the commit and the `load`
    /// the tab sent after it stay behind it until that read is applied.
    #[tokio::test]
    async fn a_tab_s_events_keep_their_order_through_its_line() -> TestResult {
        let connection = TestConnection::new([
            ("Page.getFrameTree", cdp_error("busy")),
            ("Page.getFrameTree", tree("tab", "https://a.test/")),
        ]);
        let (hub, mut watcher, mut answers) = watcher(&connection);
        read(&mut watcher, page_attached("page", "tab")).await;
        for (method, params) in [
            ("Page.domContentEventFired", json!({ "timestamp": 1.0 })),
            (
                "Page.frameNavigated",
                json!({ "frame": { "id": "tab", "loaderId": "next", "url": "https://a.test/next" } }),
            ),
            ("Page.loadEventFired", json!({ "timestamp": 2.0 })),
        ] {
            read(&mut watcher, on("page", method, params)).await;
        }

        answer_next(&mut watcher, &mut answers).await?;
        assert_eq!(
            records(&hub),
            Vec::<String>::new(),
            "the DOMContentLoaded waits for the second read, and the rest behind it"
        );
        assert_eq!(watcher.progress.applied_through(), 1);

        answer_next(&mut watcher, &mut answers).await?;
        assert_eq!(
            records(&hub),
            [
                "domcontentloaded https://a.test/",
                "framenavigated https://a.test/next",
                "load https://a.test/next"
            ],
            "in the tab's own order"
        );
        assert_eq!(watcher.progress.applied_through(), 4);
        assert!(watcher.lines.is_empty());
        Ok(())
    }

    /// The second invariant of a tab's line: a read that fails, or that the renderer never
    /// answers, lets the tab's events go once it gives up, as one that answered does, and with
    /// them the reply whose fence covers one.
    #[tokio::test(start_paused = true)]
    async fn a_read_that_fails_or_times_out_releases_its_line() -> TestResult {
        let connection = TestConnection::new([
            ("Page.getFrameTree", cdp_error("No frame tree")),
            ("Page.getFrameTree", cdp_pending()),
        ]);
        let (hub, mut watcher, mut answers) = watcher(&connection);
        for (tab, session) in [("failed", "failed-page"), ("silent", "silent-page")] {
            read(&mut watcher, page_attached(session, tab)).await;
            let child = format!("{tab}-child");
            read(
                &mut watcher,
                on(
                    session,
                    "Page.frameAttached",
                    json!({ "frameId": child, "parentFrameId": tab }),
                ),
            )
            .await;
            let read_so_far = watcher.progress.events_handled();
            let (done, mut answered) = oneshot::channel();
            watcher.deferred.push(WatcherRequest {
                closed: None,
                handed_back: None,
                fence: EventFence(read_so_far),
                done,
            });
            watcher.answer_deferred();
            assert!(
                answered.try_recv().is_err(),
                "{tab}: the reply waits for the held attach"
            );

            answer_next(&mut watcher, &mut answers).await?;
            assert!(watcher.lines.is_empty(), "{tab}: the line is gone");
            assert_eq!(watcher.progress.applied_through(), read_so_far, "{tab}");
            assert!(answered.try_recv().is_ok(), "{tab}: the reply goes on");
            assert!(
                records(&hub).contains(&format!("frameattached {child}")),
                "{tab}: {:?}",
                records(&hub)
            );
        }
        Ok(())
    }

    /// A tab whose renderer never answers its tree read costs the loop nothing: its attach is
    /// handled at once, its own later events wait in its line, and another tab's events apply.
    #[tokio::test(start_paused = true)]
    async fn a_page_attach_whose_tree_read_never_answers_does_not_stop_the_loop() -> TestResult {
        let connection = TestConnection::new([]);
        connection.always("Page.getFrameTree", cdp_pending())?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        let (healthy, healthy_page) = (
            TargetId("healthy".to_owned()),
            SessionId::from("healthy-page"),
        );
        watcher
            .sessions
            .insert(healthy_page.clone(), healthy.clone());
        watcher.lifecycle_sources.attach(&healthy_page, &healthy);
        // No time passes on the loop's clock: a read awaited in place would run into this.
        time::timeout(TREE_READ_WAIT / 4, async {
            read(&mut watcher, page_attached("busy-page", "busy")).await;
            for (session, tab) in [("busy-page", "busy"), ("healthy-page", "healthy")] {
                let frame = json!({ "frameId": format!("{tab}-child"), "parentFrameId": tab });
                read(&mut watcher, on(session, "Page.frameAttached", frame)).await;
            }
        })
        .await
        .map_err(|elapsed| format!("the loop waited for the busy tab's tree: {elapsed}"))?;
        assert_eq!(watcher.progress.events_handled(), 3, "every event was read");
        assert_eq!(
            records(&hub),
            ["frameattached healthy-child"],
            "the other tab's event is in the log; the busy tab's waits for its read"
        );
        assert_eq!(
            watcher.progress.applied_through(),
            1,
            "only the busy tab's own event is held"
        );
        Ok(())
    }

    /// A load event of a tab whose main frame is unknown, because its first read failed, reads
    /// the tab again, and is recorded once that read is applied, with the main frame it names:
    /// never before it.
    #[tokio::test]
    async fn a_load_event_of_a_tab_whose_main_frame_is_unknown_is_recorded_with_the_frame_its_read_names()
    -> TestResult {
        let connection = TestConnection::new([
            ("Page.getFrameTree", cdp_error("busy")),
            ("Page.getFrameTree", tree("tab", "https://a.test/")),
        ]);
        let (hub, mut watcher, mut answers) = watcher(&connection);
        read(&mut watcher, page_attached("page", "tab")).await;
        answer_next(&mut watcher, &mut answers).await?;
        read(
            &mut watcher,
            on("page", "Page.loadEventFired", json!({ "timestamp": 1.0 })),
        )
        .await;
        assert_eq!(
            records(&hub),
            Vec::<String>::new(),
            "it waits for the read it started"
        );
        assert_eq!(
            watcher.progress.applied_through(),
            1,
            "and keeps its place in the log"
        );

        answer_next(&mut watcher, &mut answers).await?;
        assert_eq!(records(&hub), ["load https://a.test/"]);
        assert_eq!(watcher.progress.applied_through(), 2);
        assert_eq!(
            tree_reads(&connection, "page")?,
            2,
            "the failed read was followed by another"
        );
        Ok(())
    }

    /// A session is read once while its read is in the line, however many load events wait
    /// behind it, and never again once a read came back with its tab's main frame.
    #[tokio::test]
    async fn a_session_is_not_read_again_while_its_read_is_out_or_after_a_tree() -> TestResult {
        let connection =
            TestConnection::new([("Page.getFrameTree", tree("tab", "https://a.test/"))]);
        let (hub, mut watcher, mut answers) = watcher(&connection);
        read(&mut watcher, page_attached("page", "tab")).await;
        for (method, stamp) in [
            ("Page.domContentEventFired", 1.0),
            ("Page.loadEventFired", 2.0),
        ] {
            read(
                &mut watcher,
                on("page", method, json!({ "timestamp": stamp })),
            )
            .await;
        }
        answer_next(&mut watcher, &mut answers).await?;
        read(
            &mut watcher,
            on("page", "Page.loadEventFired", json!({ "timestamp": 3.0 })),
        )
        .await;
        settle().await;
        assert_eq!(
            records(&hub),
            [
                "domcontentloaded https://a.test/",
                "load https://a.test/",
                "load https://a.test/"
            ]
        );
        assert_eq!(tree_reads(&connection, "page")?, 1);
        assert!(watcher.lines.is_empty());
        Ok(())
    }

    /// A download's begin and its completion, from the browser's own session.
    fn download(frame: &str) -> [CdpEvent; 2] {
        let browser = |method: &str, params: Value| CdpEvent {
            method: method.to_owned(),
            params,
            session_id: None,
        };
        [
            browser(
                "Browser.downloadWillBegin",
                json!({ "frameId": frame, "guid": "download", "url": "https://a.test/file.txt",
                    "suggestedFilename": "file.txt" }),
            ),
            browser(
                "Browser.downloadProgress",
                json!({ "guid": "download", "state": "completed",
                    "filePath": "/downloads/file.txt" }),
            ),
        ]
    }

    /// The tab each recorded download was given, in order.
    fn download_pages(hub: &EventHub) -> Vec<Option<String>> {
        hub.read_after(0)
            .records
            .into_iter()
            .filter_map(|record| match record.event {
                PageEvent::Download(download) => Some(download.page.map(|page| page.0)),
                _ => None,
            })
            .collect()
    }

    /// Whether the download's completion reached the table after its begin.
    fn completed(hub: &EventHub) -> bool {
        hub.downloads().get("download").is_some_and(|tracked| {
            tracked.facts().state == DownloadState::Completed("/downloads/file.txt".into())
        })
    }

    /// Tab `tab`'s tree, with a same-process child `child` when one is given.
    fn tree_with(tab: &str, child: Option<&str>) -> Value {
        let children: Vec<Value> = child
            .into_iter()
            .map(|child| {
                json!({ "frame": { "id": child, "parentId": tab, "url": "https://a.test/c" } })
            })
            .collect();
        json!({ "frameTree": {
            "frame": { "id": tab, "loaderId": "seeded", "url": "https://a.test/" },
            "childFrames": children } })
    }

    /// Applies `session`'s tree read with `tree`, as the run loop applies an answer.
    async fn answer(watcher: &mut Watcher, session: &str, tree: Value) {
        watcher.seeded_tree(SeededTree {
            session: SessionId::from(session),
            tree: Some(tree),
        });
        watcher.advance_lines().await;
    }

    /// A download begun by a same-process child that existed before its tab's session reported
    /// events, while that session's first tree read is out: no event names the child, only the
    /// tree. The download waits, holding its place in the log, and is recorded with its tab
    /// once the tree is applied, its completion after it.
    #[tokio::test]
    async fn a_download_of_a_child_only_a_read_out_names_is_recorded_with_its_tab() -> TestResult {
        let connection = TestConnection::new([]);
        connection.always("Page.getFrameTree", cdp_pending())?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        read(&mut watcher, page_attached("page", "tab")).await;
        for event in download("child") {
            read(&mut watcher, event).await;
        }
        assert_eq!(
            download_pages(&hub),
            Vec::<Option<String>>::new(),
            "it waits"
        );
        assert_eq!(
            watcher.progress.applied_through(),
            1,
            "its place in the log is held"
        );

        answer(&mut watcher, "page", tree_with("tab", Some("child"))).await;
        assert_eq!(download_pages(&hub), [Some("tab".to_owned())]);
        assert!(completed(&hub), "its completion followed it");
        assert_eq!(watcher.progress.applied_through(), 3);
        Ok(())
    }

    /// When the reads out at a download's begin end without naming its frame, it is recorded with
    /// no page, as when no read was out: neither lost nor kept.
    #[tokio::test]
    async fn a_download_whose_frame_no_read_names_is_recorded_without_a_page() -> TestResult {
        let connection = TestConnection::new([]);
        connection.always("Page.getFrameTree", cdp_pending())?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        read(&mut watcher, page_attached("page", "tab")).await;
        for event in download("elsewhere") {
            read(&mut watcher, event).await;
        }
        answer(&mut watcher, "page", tree_with("tab", None)).await;
        assert_eq!(download_pages(&hub), [None]);
        assert!(completed(&hub), "its completion followed it");
        assert!(watcher.unowned_downloads.is_empty() && watcher.lines.is_empty());
        assert_eq!(watcher.progress.applied_through(), 3);
        Ok(())
    }

    /// A download waits only for the reads out when it began: one that went out after it, which
    /// a page that keeps adding frames always has, does not keep it waiting.
    #[tokio::test]
    async fn a_read_started_after_a_download_does_not_keep_it_waiting() -> TestResult {
        let connection = TestConnection::new([]);
        connection.always("Page.getFrameTree", cdp_pending())?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        read(&mut watcher, page_attached("a-page", "a")).await;
        for event in download("b-child") {
            read(&mut watcher, event).await;
        }
        read(&mut watcher, page_attached("b-page", "b")).await;
        answer(&mut watcher, "a-page", tree_with("a", None)).await;
        assert_eq!(
            download_pages(&hub),
            [None],
            "recorded while the later read is still out"
        );
        assert!(completed(&hub));
        assert!(
            watcher.lines.contains_key(&TargetId("b".to_owned())),
            "the later read is still out"
        );
        Ok(())
    }

    /// A download begun while no tree read is out is recorded at once, with no page when no
    /// tracker knows its frame.
    #[tokio::test]
    async fn a_download_begun_while_no_read_is_out_is_recorded_at_once() -> TestResult {
        let connection = TestConnection::new([]);
        let (hub, mut watcher, _answers) = watcher(&connection);
        for event in download("child") {
            read(&mut watcher, event).await;
        }
        assert_eq!(download_pages(&hub), [None]);
        assert!(completed(&hub));
        assert_eq!(watcher.progress.applied_through(), 2);
        Ok(())
    }

    /// A child its tab's tree names begins a download while that read is out, and the tab then
    /// removes it, the removal held behind the read: the download replays before the removal, so
    /// it is recorded with its tab.
    #[tokio::test]
    async fn a_download_whose_child_its_tab_then_removes_keeps_its_tab() -> TestResult {
        let connection = TestConnection::new([]);
        connection.always("Page.getFrameTree", cdp_pending())?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        read(&mut watcher, page_attached("t-page", "t")).await;
        for event in download("child") {
            read(&mut watcher, event).await;
        }
        let removed = json!({ "frameId": "child", "reason": "remove" });
        read(&mut watcher, on("t-page", "Page.frameDetached", removed)).await;
        answer(&mut watcher, "t-page", tree_with("t", Some("child"))).await;
        assert_eq!(download_pages(&hub), [Some("t".to_owned())]);
        assert!(completed(&hub), "its completion followed it");
        Ok(())
    }

    /// A download is let go as soon as a tree names its frame, before the other reads out at its
    /// begin answer: waiting on, its tab's line would replay the child's removal first.
    #[tokio::test]
    async fn a_download_is_let_go_as_soon_as_a_tree_names_its_frame() -> TestResult {
        let connection = TestConnection::new([]);
        connection.always("Page.getFrameTree", cdp_pending())?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        read(&mut watcher, page_attached("u-page", "u")).await;
        read(&mut watcher, page_attached("t-page", "t")).await;
        for event in download("child") {
            read(&mut watcher, event).await;
        }
        let removed = json!({ "frameId": "child", "reason": "remove" });
        read(&mut watcher, on("t-page", "Page.frameDetached", removed)).await;
        answer(&mut watcher, "t-page", tree_with("t", Some("child"))).await;
        assert_eq!(
            download_pages(&hub),
            [Some("t".to_owned())],
            "at once, while u's read is out"
        );
        assert!(
            watcher.lines.contains_key(&TargetId("u".to_owned())),
            "u's read is still out"
        );
        Ok(())
    }

    /// A released download rests in its tab's line at its place, behind the tab's older held
    /// event, and is recorded after it, with its tab.
    #[tokio::test]
    async fn a_released_download_waits_behind_its_tab_s_older_events() -> TestResult {
        let connection = TestConnection::new([]);
        connection.always("Page.getFrameTree", cdp_pending())?;
        connection.always("Page.enable", cdp_pending())?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        read(&mut watcher, page_attached("t-page", "t")).await;
        // An out-of-process frame of t attaches: its read holds t's later events.
        let frame = json!({ "sessionId": "tf",
            "targetInfo": { "targetId": "tf", "type": "iframe", "url": "https://tf.test/" } });
        read(&mut watcher, on("t-page", "Target.attachedToTarget", frame)).await;
        let older = json!({ "frameId": "older", "parentFrameId": "t" });
        read(&mut watcher, on("t-page", "Page.frameAttached", older)).await;
        for event in download("child") {
            read(&mut watcher, event).await;
        }
        answer(&mut watcher, "t-page", tree_with("t", Some("child"))).await;
        assert_eq!(
            download_pages(&hub),
            Vec::<Option<String>>::new(),
            "it waits behind the older event"
        );
        assert_eq!(
            watcher.progress.applied_through(),
            2,
            "the older event is held"
        );

        let tf_tree = json!({ "frameTree": { "frame": { "id": "tf", "parentId": "t", "url": "https://tf.test/" } } });
        answer(&mut watcher, "tf", tf_tree).await;
        let order: Vec<String> = hub
            .read_after(0)
            .records
            .into_iter()
            .filter_map(|record| match record.event {
                PageEvent::FrameLifecycle(event) => Some(event.frame.id.0),
                PageEvent::Download(download) => {
                    Some(format!("download {:?}", download.page.map(|page| page.0)))
                }
                _ => None,
            })
            .collect();
        assert_eq!(order, ["older", "download Some(\"t\")"], "in t's own order");
        assert!(completed(&hub));
        Ok(())
    }

    /// A read of the download's own tab that started after the download began does not keep it:
    /// once the tab's earlier read names its frame, the download goes in ahead of that later
    /// read, at its place, and is recorded at once with its tab.
    #[tokio::test]
    async fn a_later_read_in_the_download_s_own_tab_does_not_keep_it() -> TestResult {
        let connection = TestConnection::new([]);
        connection.always("Page.getFrameTree", cdp_pending())?;
        connection.always("Page.enable", cdp_pending())?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        read(&mut watcher, page_attached("t-page", "t")).await;
        for event in download("child") {
            read(&mut watcher, event).await;
        }
        // An out-of-process frame of t attaches after the begin: its read stands in t's line and
        // never answers.
        let frame = json!({ "sessionId": "tf",
            "targetInfo": { "targetId": "tf", "type": "iframe", "url": "https://tf.test/" } });
        read(&mut watcher, on("t-page", "Target.attachedToTarget", frame)).await;
        answer(&mut watcher, "t-page", tree_with("t", Some("child"))).await;
        assert_eq!(
            download_pages(&hub),
            [Some("t".to_owned())],
            "at once, ahead of the frame's later read"
        );
        assert!(completed(&hub), "its completion followed it");
        assert!(
            watcher.lines.contains_key(&TargetId("t".to_owned())),
            "the frame's read is still out"
        );
        assert_eq!(
            watcher.progress.applied_through(),
            4,
            "nothing of t is held"
        );
        Ok(())
    }

    /// The watcher's root observer on a tab no session of core's is on.
    fn observer(watcher: &Watcher) -> Result<SessionId, Box<dyn Error>> {
        Ok(watcher
            .root_observers
            .keys()
            .next()
            .cloned()
            .ok_or("the tab is observed")?)
    }

    /// A tab no session of core's is on: its root observer is its lifecycle source. `Page` goes
    /// on with the seed read, on the observer's own session, so the frames that existed are known
    /// and the frames' events reach the tracker; `Runtime` stays off.
    #[tokio::test]
    async fn an_untouched_tab_s_observer_is_its_lifecycle_source() -> TestResult {
        let connection = TestConnection::new([]);
        for method in ["Target.setAutoAttach", "Page.enable"] {
            connection.always(method, json!({}))?;
        }
        connection.always("Page.getFrameTree", tree_with("tab", Some("child")))?;
        let (_hub, mut watcher, mut answers) = watcher(&connection);
        read(&mut watcher, target_created("tab", "https://a.test/")).await;
        watcher.observe_workers(TargetId("tab".to_owned())).await;
        let observer = observer(&watcher)?;
        answer_next(&mut watcher, &mut answers).await?;
        let child = FrameId("child".to_owned());
        assert_eq!(
            watcher.lifecycle.owner(&child),
            Some(&TargetId("tab".to_owned())),
            "its seed named the existing child"
        );
        let removed = json!({ "frameId": "child", "reason": "remove" });
        read(
            &mut watcher,
            on(observer.as_str(), "Page.frameDetached", removed),
        )
        .await;
        assert!(
            watcher.lifecycle.owner(&child).is_none(),
            "its removal reached the tracker"
        );
        settle().await;
        let calls: Vec<String> = connection
            .calls()?
            .into_iter()
            .filter(|call| call.session.as_ref() == Some(&observer))
            .map(|call| call.method)
            .collect();
        let page: Vec<&String> = calls
            .iter()
            .filter(|method| method.starts_with("Page."))
            .collect();
        assert_eq!(
            page,
            ["Page.enable", "Page.getFrameTree"],
            "on its own session, in order"
        );
        assert!(calls.iter().any(|method| method == "Target.setAutoAttach"));
        assert!(!calls.iter().any(|method| method.starts_with("Runtime.")));
        Ok(())
    }

    /// An untouched tab whose renderer answers nothing (its auto-attach, `Page.enable` and tree
    /// read all pending) costs the loop nothing: observing it returns at once, and another tab's
    /// events are applied.
    #[tokio::test(start_paused = true)]
    async fn an_observer_whose_renderer_never_answers_costs_the_loop_nothing() -> TestResult {
        let connection = TestConnection::new([]);
        for method in ["Target.setAutoAttach", "Page.enable", "Page.getFrameTree"] {
            connection.always(method, cdp_pending())?;
        }
        let (hub, mut watcher, _answers) = watcher(&connection);
        let (healthy, healthy_page) = (
            TargetId("healthy".to_owned()),
            SessionId::from("healthy-page"),
        );
        watcher
            .sessions
            .insert(healthy_page.clone(), healthy.clone());
        watcher.lifecycle_sources.attach(&healthy_page, &healthy);
        read(&mut watcher, target_created("busy", "https://busy.test/")).await;
        // No time passes on the loop's clock: a command awaited in place would run into this.
        time::timeout(TREE_READ_WAIT / 4, async {
            watcher.observe_workers(TargetId("busy".to_owned())).await;
            let frame = json!({ "frameId": "healthy-child", "parentFrameId": "healthy" });
            read(
                &mut watcher,
                on("healthy-page", "Page.frameAttached", frame),
            )
            .await;
        })
        .await
        .map_err(|elapsed| format!("the loop waited for the busy tab: {elapsed}"))?;
        assert_eq!(records(&hub), ["frameattached healthy-child"]);
        Ok(())
    }

    /// A tree read that failed is read again on a later tick, and again once more, but no
    /// further; a read that brings a tree ends the retries.
    #[tokio::test(start_paused = true)]
    async fn a_failed_read_is_read_again_on_a_later_tick_at_most_twice() -> TestResult {
        let connection = TestConnection::new([]);
        connection.always("Page.getFrameTree", cdp_pending())?;
        let (_hub, mut watcher, _answers) = watcher(&connection);
        let failed = |session: &str| SeededTree {
            session: SessionId::from(session),
            tree: None,
        };
        read(&mut watcher, page_attached("page", "tab")).await;
        for reads in [2, 3] {
            watcher.seeded_tree(failed("page"));
            watcher.advance_lines().await;
            watcher.retry_reads().await;
            settle().await;
            assert_eq!(
                tree_reads(&connection, "page")?,
                reads - 1,
                "not before a later tick"
            );
            time::advance(TICK).await;
            watcher.retry_reads().await;
            settle().await;
            assert_eq!(
                tree_reads(&connection, "page")?,
                reads,
                "read again a tick later"
            );
        }
        watcher.seeded_tree(failed("page"));
        watcher.advance_lines().await;
        time::advance(TICK * 4).await;
        watcher.retry_reads().await;
        settle().await;
        assert_eq!(tree_reads(&connection, "page")?, 3, "two retries at most");

        read(&mut watcher, page_attached("other-page", "other")).await;
        watcher.seeded_tree(failed("other-page"));
        watcher.advance_lines().await;
        time::advance(TICK).await;
        watcher.retry_reads().await;
        settle().await;
        answer(&mut watcher, "other-page", tree_with("other", None)).await;
        time::advance(TICK * 4).await;
        watcher.retry_reads().await;
        settle().await;
        assert_eq!(
            tree_reads(&connection, "other-page")?,
            2,
            "none after a read that brought a tree"
        );
        Ok(())
    }

    /// Core's page session attaching to a tab whose observer is its lifecycle source takes no
    /// frame event twice: its copy of a commit is dropped while the observer reports, and it
    /// reports the tab's next commit once the observer goes, so none is lost.
    #[tokio::test]
    async fn core_s_session_attaching_to_an_observed_tab_doubles_no_frame_event() -> TestResult {
        let connection = TestConnection::new([]);
        for method in ["Target.setAutoAttach", "Page.enable"] {
            connection.always(method, json!({}))?;
        }
        connection.always("Page.getFrameTree", tree_with("tab", Some("child")))?;
        let (hub, mut watcher, mut answers) = watcher(&connection);
        read(&mut watcher, target_created("tab", "https://a.test/")).await;
        watcher.observe_workers(TargetId("tab".to_owned())).await;
        let observer = observer(&watcher)?;
        answer_next(&mut watcher, &mut answers).await?;
        read(&mut watcher, page_attached("core", "tab")).await;
        let commit = |url: &str| json!({ "frame": { "id": "child", "parentId": "tab", "loaderId": url, "url": url } });
        for session in [observer.as_str(), "core"] {
            let moved = commit("https://a.test/moved");
            read(&mut watcher, on(session, "Page.frameNavigated", moved)).await;
        }
        assert_eq!(
            records(&hub),
            ["framenavigated https://a.test/moved"],
            "once"
        );
        let gone = CdpEvent {
            method: "Target.detachedFromTarget".to_owned(),
            params: json!({ "sessionId": observer.as_str() }),
            session_id: None,
        };
        read(&mut watcher, gone).await;
        let again = commit("https://a.test/again");
        read(&mut watcher, on("core", "Page.frameNavigated", again)).await;
        assert_eq!(
            records(&hub),
            [
                "framenavigated https://a.test/moved",
                "framenavigated https://a.test/again"
            ],
            "core's session reports once the observer went"
        );
        Ok(())
    }

    /// A watcher with a seeded root observer on the untouched tab `tab`, which answers a dialog
    /// command, and the observer's session.
    async fn observed_tab(
        connection: &Arc<TestConnection>,
    ) -> Result<(Arc<EventHub>, Watcher, SessionId), Box<dyn Error>> {
        for method in [
            "Target.setAutoAttach",
            "Page.enable",
            "Page.handleJavaScriptDialog",
        ] {
            connection.always(method, json!({}))?;
        }
        connection.always("Page.getFrameTree", tree_with("tab", None))?;
        let (hub, mut watcher, mut answers) = watcher(connection);
        read(&mut watcher, target_created("tab", "https://a.test/")).await;
        watcher.observe_workers(TargetId("tab".to_owned())).await;
        let observer = observer(&watcher)?;
        answer_next(&mut watcher, &mut answers).await?;
        Ok((hub, watcher, observer))
    }

    fn dialog_recorded(hub: &EventHub) -> bool {
        hub.read_after(0)
            .records
            .iter()
            .any(|record| matches!(record.event, PageEvent::Dialog(_)))
    }

    fn dialog_answered(connection: &TestConnection) -> Result<bool, Box<dyn Error>> {
        Ok(connection
            .calls()?
            .iter()
            .any(|call| call.method == "Page.handleJavaScriptDialog"))
    }

    /// A `confirm()` on a tab no session of core's is on, which only its root observer reports, is
    /// the user's to answer: Neo records nothing and answers nothing, past its grace and a reap.
    #[tokio::test(start_paused = true)]
    async fn a_dialog_on_an_untouched_tab_is_never_answered_by_neo() -> TestResult {
        let connection = TestConnection::new([]);
        let (hub, mut watcher, observer) = observed_tab(&connection).await?;
        let confirm = json!({ "type": "confirm", "message": "Leave this page?" });
        read(
            &mut watcher,
            on(observer.as_str(), "Page.javascriptDialogOpening", confirm),
        )
        .await;
        time::advance(DIALOG_GRACE + REAP).await;
        watcher.end_graces();
        watcher.tick().await;
        settle().await;
        assert!(
            !dialog_recorded(&hub),
            "Neo records no dialog of the user's tab"
        );
        assert!(!dialog_answered(&connection)?, "and never answers it");
        Ok(())
    }

    /// The same `confirm()` on a tab a session of core's is on is Neo's, as before: recorded, and
    /// answered once its grace ends with nobody listening, though the root observer that came
    /// first stays the tab's lifecycle source.
    #[tokio::test(start_paused = true)]
    async fn a_dialog_on_a_tab_with_a_core_session_is_held_and_closed() -> TestResult {
        let connection = TestConnection::new([]);
        let (hub, mut watcher, observer) = observed_tab(&connection).await?;
        read(&mut watcher, page_attached("core", "tab")).await;
        let confirm = json!({ "type": "confirm", "message": "Leave this page?" });
        read(
            &mut watcher,
            on(observer.as_str(), "Page.javascriptDialogOpening", confirm),
        )
        .await;
        assert!(dialog_recorded(&hub), "recorded");
        time::advance(DIALOG_GRACE * 2).await;
        watcher.end_graces();
        settle().await;
        assert!(
            dialog_answered(&connection)?,
            "closed after its grace with nobody listening"
        );
        Ok(())
    }

    /// A frame read whose `Page.enable` failed is a failed read, whatever tree came with it: the
    /// session would report nothing, so nothing is seeded from that tree, and it is read again.
    #[tokio::test(start_paused = true)]
    async fn a_read_whose_enable_failed_is_a_failed_read() -> TestResult {
        let connection = TestConnection::new([]);
        connection.always("Target.setAutoAttach", json!({}))?;
        connection.always("Page.enable", cdp_error("Page domain unavailable"))?;
        connection.always("Page.getFrameTree", tree_with("tab", Some("child")))?;
        let (_hub, mut watcher, mut answers) = watcher(&connection);
        read(&mut watcher, target_created("tab", "https://a.test/")).await;
        watcher.observe_workers(TargetId("tab".to_owned())).await;
        let observer = observer(&watcher)?;
        answer_next(&mut watcher, &mut answers).await?;
        assert!(
            watcher
                .lifecycle
                .owner(&FrameId("child".to_owned()))
                .is_none(),
            "nothing was seeded from the tree that came with the failed enable"
        );
        time::advance(TICK).await;
        watcher.retry_reads().await;
        settle().await;
        assert_eq!(
            tree_reads(&connection, observer.as_str())?,
            2,
            "read again a tick later"
        );
        Ok(())
    }

    /// A run about to act on a tab whose observer never brought a tree (its read and both retries
    /// failed, and the tab neither loaded nor committed) reads it once more as core's session
    /// attaches, through the observer, the tab's source, so the tab gets its main frame.
    #[tokio::test(start_paused = true)]
    async fn core_s_attach_reads_a_tab_whose_reads_all_failed_once_more() -> TestResult {
        let connection = TestConnection::new([]);
        for method in ["Target.setAutoAttach", "Page.enable"] {
            connection.always(method, json!({}))?;
        }
        connection.always("Page.getFrameTree", cdp_pending())?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        read(&mut watcher, target_created("tab", "https://a.test/")).await;
        watcher.observe_workers(TargetId("tab".to_owned())).await;
        let observer = observer(&watcher)?;
        for _ in 0..3 {
            watcher.seeded_tree(SeededTree {
                session: observer.clone(),
                tree: None,
            });
            watcher.advance_lines().await;
            time::advance(TICK).await;
            watcher.retry_reads().await;
            settle().await;
        }
        assert_eq!(
            tree_reads(&connection, observer.as_str())?,
            3,
            "its read and two retries"
        );
        read(&mut watcher, page_attached("core", "tab")).await;
        settle().await;
        assert_eq!(
            tree_reads(&connection, observer.as_str())?,
            4,
            "one more read, through the source"
        );
        answer(&mut watcher, observer.as_str(), tree_with("tab", None)).await;
        let load = json!({ "timestamp": 1.0 });
        read(
            &mut watcher,
            on(observer.as_str(), "Page.loadEventFired", load),
        )
        .await;
        assert_eq!(
            records(&hub),
            ["load https://a.test/"],
            "the tab has its main frame"
        );
        Ok(())
    }

    /// The calls of a page domain that went to `session`, in order.
    fn page_domain_calls(
        connection: &TestConnection,
        session: &SessionId,
    ) -> Result<Vec<String>, Box<dyn Error>> {
        Ok(connection
            .calls()?
            .into_iter()
            .filter(|call| call.session.as_ref() == Some(session))
            .map(|call| call.method)
            .filter(|method| {
                [
                    "Page.",
                    "Runtime.",
                    "DOM.",
                    "Input.",
                    "Emulation.",
                    "Accessibility.",
                ]
                .iter()
                .any(|domain| method.starts_with(domain))
            })
            .collect())
    }

    /// An untouched tab on a `file:` page, which the door refuses, gets nothing of a page domain
    /// from its observer, which is no lifecycle source meanwhile; once it moves to a page the door
    /// allows, the observer's `Page.enable` and seed read go out, and it is the tab's source.
    #[tokio::test]
    async fn an_untouched_tab_on_a_file_page_is_seeded_only_once_it_moves_off_it() -> TestResult {
        let connection = TestConnection::new([]);
        for method in ["Target.setAutoAttach", "Page.enable"] {
            connection.always(method, json!({}))?;
        }
        connection.always("Page.getFrameTree", tree_with("tab", None))?;
        let (_hub, mut watcher, _answers) = watcher(&connection);
        read(&mut watcher, target_created("tab", "file:///etc/hosts")).await;
        watcher.observe_workers(TargetId("tab".to_owned())).await;
        let observer = observer(&watcher)?;
        settle().await;
        assert_eq!(
            page_domain_calls(&connection, &observer)?,
            Vec::<String>::new(),
            "nothing of a page domain while it shows a file page"
        );
        assert!(
            !watcher.lifecycle_sources.accepts(&observer),
            "a mute observer is no lifecycle source"
        );

        read(&mut watcher, info_changed("tab", "https://a.test/")).await;
        settle().await;
        assert_eq!(
            page_domain_calls(&connection, &observer)?,
            ["Page.enable", "Page.getFrameTree"],
            "its seed starts on the change that brings an allowed page"
        );
        assert!(watcher.lifecycle_sources.accepts(&observer));
        Ok(())
    }

    /// A tab's own target events never wait in its line, whatever session carries them: the
    /// change that gives a `Pending` tab its URL is applied while a read of core's session is out,
    /// so a discovery that removes the tab then finds it reported and records its `Closed`.
    /// Chromium sends the change on the browser's session; here it comes on the tab's own.
    #[tokio::test]
    async fn a_tab_s_target_events_never_wait_in_its_line() -> TestResult {
        let connection = TestConnection::new([]);
        discovery_lists_nothing(&connection)?;
        connection.always("Page.getFrameTree", cdp_pending())?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        read(&mut watcher, target_created("tab", "")).await;
        read(&mut watcher, page_attached("page", "tab")).await;
        assert!(
            watcher.lines.contains_key(&TargetId("tab".to_owned())),
            "core's read of the tab is out"
        );
        let mut change = info_changed("tab", "https://a.test/");
        change.session_id = Some(SessionId::from("page"));
        read(&mut watcher, change).await;
        assert_eq!(
            tab_records(&hub),
            ["opened tab"],
            "its URL change was applied at once"
        );

        watcher.discover(false).await?;
        assert_eq!(tab_records(&hub), ["opened tab", "closed tab"]);
        Ok(())
    }

    /// A tab that discovery removes while a read of it is out keeps its line, the read ending as
    /// a failed one: the read's tree, landing later, puts nothing in the gone tab's tracker, no
    /// retry is due for its session, and a download waiting for that read is let go, with the
    /// hold behind it.
    #[tokio::test]
    async fn a_tab_gone_with_a_read_out_ends_the_read_as_a_failed_one() -> TestResult {
        let connection = TestConnection::new([]);
        discovery_lists_nothing(&connection)?;
        connection.always("Page.getFrameTree", cdp_pending())?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        read(&mut watcher, target_created("tab", "https://a.test/")).await;
        read(&mut watcher, page_attached("page", "tab")).await;
        for event in download("child") {
            read(&mut watcher, event).await;
        }
        assert_eq!(
            download_pages(&hub),
            Vec::<Option<String>>::new(),
            "it waits for the read"
        );
        assert_eq!(watcher.progress.applied_through(), 2, "its place is held");

        watcher.discover(false).await?;
        assert_eq!(tab_records(&hub), ["opened tab", "closed tab"]);
        answer(&mut watcher, "page", tree_with("tab", Some("child"))).await;
        assert!(
            watcher
                .lifecycle
                .main(&TargetId("tab".to_owned()))
                .is_none(),
            "the gone tab's tracker stays empty"
        );
        assert!(
            watcher.read_retries.is_empty(),
            "no retry for a session gone with its tab"
        );
        assert_eq!(
            download_pages(&hub),
            [None],
            "the download was let go, with no page"
        );
        assert!(completed(&hub), "its completion followed it");
        assert!(watcher.unowned_downloads.is_empty() && watcher.lines.is_empty());
        assert_eq!(watcher.progress.applied_through(), 4, "the hold moved on");
        Ok(())
    }

    /// Discovery fills a tab's URL only on a page past `Pending`: a pending tab it lists with a URL
    /// stays pending, and its observer, which the door admits only on a known allowed URL, sends
    /// nothing of a page domain and is no lifecycle source. So the URL that lets a seed out is
    /// always one that reported its tab.
    #[tokio::test]
    async fn discovery_gives_a_pending_tab_no_url() -> TestResult {
        let connection = TestConnection::new([]);
        for (method, answer) in [
            (
                "Target.getTargets",
                json!({ "targetInfos": [
                    { "targetId": "tab", "type": "page", "url": "https://a.test/" } ] }),
            ),
            ("Target.setDiscoverTargets", json!({})),
            ("Browser.setDownloadBehavior", json!({})),
            ("Target.setAutoAttach", json!({})),
            ("Page.enable", json!({})),
        ] {
            connection.always(method, answer)?;
        }
        connection.always("Page.getFrameTree", tree_with("tab", None))?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        read(&mut watcher, target_created("tab", "")).await;
        watcher.discover(false).await?;
        watcher.observe_workers(TargetId("tab".to_owned())).await;
        let observer = observer(&watcher)?;
        settle().await;
        assert_eq!(
            page_domain_calls(&connection, &observer)?,
            Vec::<String>::new(),
            "no seed and no Page.enable on discovery's URL"
        );
        assert!(!watcher.lifecycle_sources.accepts(&observer));
        assert!(
            matches!(
                watcher
                    .pages
                    .get(&TargetId("tab".to_owned()))
                    .map(|page| &page.state),
                Some(PageState::Pending { .. })
            ),
            "the fill did not report it"
        );
        assert_eq!(tab_records(&hub), Vec::<String>::new());
        Ok(())
    }

    /// A tab's own `Target.targetDestroyed`, replayed from its line, ends the reads that went into
    /// the line behind it, as `destroyed` ends those of a line it finds: an out-of-process frame's
    /// read that went out after the destroy was read puts nothing in the gone tab's tracker when
    /// its tree lands, and leaves no retry.
    #[tokio::test]
    async fn a_tab_s_replayed_destroy_ends_the_reads_behind_it() -> TestResult {
        let connection = TestConnection::new([]);
        connection.always("Page.getFrameTree", cdp_pending())?;
        connection.always("Page.enable", cdp_pending())?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        read(&mut watcher, target_created("t", "https://t.test/")).await;
        read(&mut watcher, page_attached("t-page", "t")).await;
        let destroyed = CdpEvent {
            method: "Target.targetDestroyed".to_owned(),
            params: json!({ "targetId": "t" }),
            session_id: None,
        };
        read(&mut watcher, destroyed).await;
        let frame = json!({ "sessionId": "tf",
            "targetInfo": { "targetId": "tf", "type": "iframe", "url": "https://tf.test/" } });
        read(&mut watcher, on("t-page", "Target.attachedToTarget", frame)).await;
        assert_eq!(
            tab_records(&hub),
            ["opened t"],
            "the destroy waits behind the tab's read"
        );

        answer(&mut watcher, "t-page", tree_with("t", None)).await;
        assert_eq!(tab_records(&hub), ["opened t", "closed t"]);
        let tf_tree = json!({ "frameTree": {
            "frame": { "id": "tf", "parentId": "t", "url": "https://tf.test/" },
            "childFrames": [
                { "frame": { "id": "deep", "parentId": "tf", "url": "https://tf.test/deep" } }
            ] } });
        answer(&mut watcher, "tf", tf_tree).await;
        assert!(
            watcher
                .lifecycle
                .owner(&FrameId("deep".to_owned()))
                .is_none(),
            "the gone tab's tracker learns no frame from the read behind its destroy"
        );
        assert!(
            watcher.read_retries.is_empty(),
            "no retry for a session gone with its tab"
        );
        assert!(watcher.lines.is_empty());
        Ok(())
    }

    /// Core's session came first on a tab whose observer never seeded (no URL then): when the
    /// tab's URL comes, the observer is entered behind core's session with its `Page.enable` and
    /// seed, so the observer that core's detach promotes reports the tab's commits to the run whose
    /// session attaches next. Entered mute, it would be promoted with `Page` off, that session
    /// would wait behind it with its events dropped, and the tab would go silent.
    #[tokio::test]
    async fn an_observer_behind_core_s_session_turns_page_on_before_it_is_promoted() -> TestResult {
        let connection = TestConnection::new([]);
        for method in ["Target.setAutoAttach", "Page.enable"] {
            connection.always(method, json!({}))?;
        }
        connection.always("Page.getFrameTree", tree_with("tab", Some("child")))?;
        let (hub, mut watcher, mut answers) = watcher(&connection);
        read(&mut watcher, target_created("tab", "")).await;
        watcher.observe_workers(TargetId("tab".to_owned())).await;
        let observer = observer(&watcher)?;
        read(&mut watcher, page_attached("core", "tab")).await;
        answer_next(&mut watcher, &mut answers).await?;
        assert!(
            watcher.lifecycle_sources.accepts(&SessionId::from("core")),
            "core's session is the tab's source"
        );

        read(&mut watcher, info_changed("tab", "https://a.test/")).await;
        settle().await;
        assert_eq!(
            page_domain_calls(&connection, &observer)?,
            ["Page.enable", "Page.getFrameTree"],
            "its seed goes out though core's session is the source"
        );
        answer_next(&mut watcher, &mut answers).await?;
        let gone = CdpEvent {
            method: "Target.detachedFromTarget".to_owned(),
            params: json!({ "sessionId": "core" }),
            session_id: None,
        };
        read(&mut watcher, gone).await;
        assert!(watcher.lifecycle_sources.accepts(&observer), "promoted");
        // A run's next session of core's waits behind the promoted observer, its copies dropped.
        read(&mut watcher, page_attached("core-next", "tab")).await;
        let commit = json!({ "frame": { "id": "child", "parentId": "tab", "loaderId": "next",
            "url": "https://a.test/next" } });
        read(
            &mut watcher,
            on(observer.as_str(), "Page.frameNavigated", commit),
        )
        .await;
        assert_eq!(
            records(&hub),
            ["framenavigated https://a.test/next"],
            "the promoted observer reports the tab's commit"
        );
        Ok(())
    }

    /// An observer waiting behind core's session whose seed's `Page.enable` fails is read again
    /// on a later tick, as a source is: once its enable succeeds, core's detach promotes an
    /// observer with `Page` on, and with core's next session on the tab its commits are recorded.
    #[tokio::test(start_paused = true)]
    async fn a_waiting_observer_whose_enable_failed_is_read_again() -> TestResult {
        let connection =
            TestConnection::new([("Page.enable", cdp_error("Page domain unavailable"))]);
        for method in ["Target.setAutoAttach", "Page.enable"] {
            connection.always(method, json!({}))?;
        }
        connection.always("Page.getFrameTree", tree_with("tab", Some("child")))?;
        let (hub, mut watcher, mut answers) = watcher(&connection);
        read(&mut watcher, target_created("tab", "")).await;
        watcher.observe_workers(TargetId("tab".to_owned())).await;
        let observer = observer(&watcher)?;
        read(&mut watcher, page_attached("core", "tab")).await;
        answer_next(&mut watcher, &mut answers).await?;
        read(&mut watcher, info_changed("tab", "https://a.test/")).await;
        answer_next(&mut watcher, &mut answers).await?;
        assert!(
            !watcher.lifecycle_sources.accepts(&observer),
            "it waits behind core's session"
        );

        time::advance(TICK).await;
        watcher.retry_reads().await;
        settle().await;
        assert_eq!(
            page_domain_calls(&connection, &observer)?,
            [
                "Page.enable",
                "Page.getFrameTree",
                "Page.enable",
                "Page.getFrameTree"
            ],
            "read again on a later tick while it waits"
        );
        answer_next(&mut watcher, &mut answers).await?;
        assert!(watcher.read_retries.is_empty(), "its enable succeeded");
        let gone = CdpEvent {
            method: "Target.detachedFromTarget".to_owned(),
            params: json!({ "sessionId": "core" }),
            session_id: None,
        };
        read(&mut watcher, gone).await;
        read(&mut watcher, page_attached("core-next", "tab")).await;
        let commit = json!({ "frame": { "id": "child", "parentId": "tab", "loaderId": "next",
            "url": "https://a.test/next" } });
        read(
            &mut watcher,
            on(observer.as_str(), "Page.frameNavigated", commit),
        )
        .await;
        assert_eq!(
            records(&hub),
            ["framenavigated https://a.test/next"],
            "the promoted observer reports the tab's commit"
        );
        Ok(())
    }

    /// An untouched tab that churns frames writes nothing to the shared event log while no
    /// session of core's is on it: its observer applies each frame's attach, commit and removal,
    /// and its load, to the tracker, and the tab's frame version moves, but no record is written.
    #[tokio::test]
    async fn an_untouched_tab_churning_frames_writes_no_record() -> TestResult {
        let connection = TestConnection::new([]);
        let (hub, mut watcher, observer) = observed_tab(&connection).await?;
        watcher.writer.tab_id(TargetId("tab".to_owned()), TabId(7));
        let version = |hub: &EventHub| {
            hub.frame_versions()
                .into_iter()
                .find_map(|(tab, version)| (tab == TabId(7)).then_some(version))
        };
        let before = version(&hub);
        for round in 0..3 {
            let child = format!("churn-{round}");
            let events = [
                (
                    "Page.frameAttached",
                    json!({ "frameId": child, "parentFrameId": "tab" }),
                ),
                (
                    "Page.frameNavigated",
                    json!({ "frame": { "id": child, "parentId": "tab", "loaderId": child,
                        "url": "https://ads.test/" } }),
                ),
                (
                    "Page.frameDetached",
                    json!({ "frameId": child, "reason": "remove" }),
                ),
            ];
            for (method, params) in events {
                read(&mut watcher, on(observer.as_str(), method, params)).await;
            }
        }
        let load = json!({ "timestamp": 1.0 });
        read(
            &mut watcher,
            on(observer.as_str(), "Page.loadEventFired", load),
        )
        .await;
        let written: Vec<String> = hub
            .read_after(0)
            .records
            .into_iter()
            .filter(|record| !matches!(record.event, PageEvent::Opened(_)))
            .map(|record| format!("{:?}", record.event))
            .collect();
        assert_eq!(written, Vec::<String>::new(), "no record but its opening");
        let after = version(&hub);
        assert!(
            after.is_some() && after > before,
            "its frame version moved: {before:?} to {after:?}"
        );
        Ok(())
    }

    /// From the moment a session of core's is on a tab whose observer stays its source (core's
    /// copies dropped), the tab's frame and load records are written from the observer's events,
    /// in order; before it, none.
    #[tokio::test]
    async fn a_tab_core_is_on_records_its_frames_from_core_s_attach_on() -> TestResult {
        let connection = TestConnection::new([]);
        let (hub, mut watcher, observer) = observed_tab(&connection).await?;
        let attached = |frame: &str| json!({ "frameId": frame, "parentFrameId": "tab" });
        read(
            &mut watcher,
            on(observer.as_str(), "Page.frameAttached", attached("before")),
        )
        .await;
        read(&mut watcher, page_attached("core", "tab")).await;
        assert!(
            watcher.lifecycle_sources.accepts(&observer),
            "the observer stays the source"
        );
        read(
            &mut watcher,
            on(observer.as_str(), "Page.frameAttached", attached("after")),
        )
        .await;
        read(
            &mut watcher,
            on("core", "Page.frameAttached", attached("core-copy")),
        )
        .await;
        let load = json!({ "timestamp": 2.0 });
        read(
            &mut watcher,
            on(observer.as_str(), "Page.loadEventFired", load),
        )
        .await;
        assert_eq!(
            records(&hub),
            ["frameattached after", "load https://a.test/"]
        );
        Ok(())
    }

    /// An untouched tab's out-of-process frame records its frames' events, as on main (its
    /// session is the frame's own, whose `Page` main turned on too); the tab's observer's own
    /// frame events write nothing.
    #[tokio::test]
    async fn an_untouched_tab_s_out_of_process_frame_records_as_on_main() -> TestResult {
        let connection = TestConnection::new([]);
        let (hub, mut watcher, observer) = observed_tab(&connection).await?;
        let frame = json!({ "sessionId": "tf",
            "targetInfo": { "targetId": "tf", "type": "iframe", "url": "https://tf.test/" } });
        read(
            &mut watcher,
            on(observer.as_str(), "Target.attachedToTarget", frame),
        )
        .await;
        let tf_tree = json!({ "frameTree": {
            "frame": { "id": "tf", "parentId": "tab", "url": "https://tf.test/" } } });
        answer(&mut watcher, "tf", tf_tree).await;
        let attached = |id: &str, parent: &str| json!({ "frameId": id, "parentFrameId": parent });
        read(
            &mut watcher,
            on(
                observer.as_str(),
                "Page.frameAttached",
                attached("local", "tab"),
            ),
        )
        .await;
        read(
            &mut watcher,
            on("tf", "Page.frameAttached", attached("deep", "tf")),
        )
        .await;
        assert_eq!(records(&hub), ["frameattached deep"]);
        Ok(())
    }

    /// A crashed untouched tab recovers through its observer, seeded long before the crash: once
    /// a pass handled it (`observe_recovery`, which finds `Page` on already), the reload's
    /// main-frame commit is written with its navigation record and moves the host's URL hint, as
    /// main's recovery observer wrote it. Before the crash, its commits wrote nothing.
    #[tokio::test]
    async fn a_crashed_untouched_tab_s_reload_is_recorded_through_its_observer() -> TestResult {
        let connection = TestConnection::new([]);
        let (hub, mut watcher, observer) = observed_tab(&connection).await?;
        let tab = TargetId("tab".to_owned());
        let commit = |loader: &str| {
            json!({ "frame": { "id": "tab", "loaderId": loader,
            "url": "https://a.test/" } })
        };
        let navigations = |hub: &EventHub| {
            hub.read_after(0)
                .records
                .into_iter()
                .filter(|record| matches!(&record.event, PageEvent::Navigation(_)))
                .count()
        };
        read(
            &mut watcher,
            on(observer.as_str(), "Page.frameNavigated", commit("before")),
        )
        .await;
        assert_eq!(navigations(&hub), 0, "no record before the crash");
        assert_eq!(hub.navigated_at(&tab), None);

        let crashed = CdpEvent {
            method: "Target.targetCrashed".to_owned(),
            params: json!({ "targetId": "tab", "status": "crashed", "errorCode": 1 }),
            session_id: None,
        };
        read(&mut watcher, crashed).await;
        connection.always(
            "Browser.getTabs",
            json!({ "tabs": [{ "tabId": 7, "targetId": "tab", "windowId": 1,
                "url": "https://a.test/", "title": "A", "isActive": false,
                "isLoading": false, "loadProgress": 1.0, "isPinned": false,
                "isHidden": false }] }),
        )?;
        watcher.observe_context().await;
        read(
            &mut watcher,
            on(observer.as_str(), "Page.frameNavigated", commit("reload")),
        )
        .await;
        assert_eq!(navigations(&hub), 1, "the reload's commit is recorded");
        assert!(hub.navigated_at(&tab).is_some(), "the URL hint moved");
        Ok(())
    }

    /// An untouched tab whose observer was never entered, now on a URL the door allows that no
    /// target event of its brought (discovery filled it), is seeded on the next pass of
    /// `observe_context`: the move to an allowed page is seen whichever road brings its URL.
    #[tokio::test]
    async fn a_pass_seeds_an_observer_whose_tab_s_url_discovery_filled() -> TestResult {
        let connection = TestConnection::new([]);
        for (method, answer) in [
            (
                "Target.getTargets",
                json!({ "targetInfos": [
                    { "targetId": "tab", "type": "page", "url": "https://a.test/" } ] }),
            ),
            ("Target.setDiscoverTargets", json!({})),
            ("Browser.setDownloadBehavior", json!({})),
            ("Target.setAutoAttach", json!({})),
            ("Page.enable", json!({})),
            (
                "Browser.getTabs",
                json!({ "tabs": [{ "tabId": 7, "targetId": "tab", "windowId": 1,
                    "url": "https://a.test/", "title": "A", "isActive": false,
                    "isLoading": false, "loadProgress": 1.0, "isPinned": false,
                    "isHidden": false }] }),
            ),
        ] {
            connection.always(method, answer)?;
        }
        connection.always("Page.getFrameTree", tree_with("tab", None))?;
        let (_hub, mut watcher, _answers) = watcher(&connection);
        // Its creation's pass attaches its observer, which the door keeps mute on a file page.
        read(&mut watcher, target_created("tab", "file:///etc/hosts")).await;
        let observer = observer(&watcher)?;
        settle().await;
        assert_eq!(
            page_domain_calls(&connection, &observer)?,
            Vec::<String>::new()
        );

        watcher.discover(false).await?;
        watcher.observe_context().await;
        settle().await;
        assert_eq!(
            page_domain_calls(&connection, &observer)?,
            ["Page.enable", "Page.getFrameTree"],
            "the pass seeds it"
        );
        assert!(watcher.lifecycle_sources.accepts(&observer));
        Ok(())
    }

    /// While a run listens on the context for lifecycle events, an untouched tab on a `file:` page
    /// gets no attach of core's page session, and the pass after it moves to an allowed page
    /// attaches it.
    #[tokio::test(start_paused = true)]
    async fn a_context_listener_attaches_no_tab_on_a_file_page() -> TestResult {
        let connection = TestConnection::new([]);
        connection.always(
            "Browser.getTabs",
            json!({ "tabs": [{ "tabId": 7, "targetId": "tab", "windowId": 1,
                "url": "file:///etc/hosts", "title": "hosts", "isActive": false,
                "isLoading": false, "loadProgress": 1.0, "isPinned": false, "isHidden": false }] }),
        )?;
        connection.always("Target.attachToTarget", cdp_pending())?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        let run = Arc::new(RunState::default());
        hub.bind(&run);
        run.events
            .listen(Subscription::Context(PageEventKind::Dialog), 1_000, 0);
        let attaches = || -> Result<usize, Box<dyn Error>> {
            Ok(connection
                .calls()?
                .iter()
                .filter(|call| call.method == "Target.attachToTarget")
                .count())
        };
        read(&mut watcher, target_created("tab", "file:///etc/hosts")).await;
        watcher.observe_context().await;
        settle().await;
        assert_eq!(attaches()?, 0, "no attach while it shows a file page");

        read(&mut watcher, info_changed("tab", "https://a.test/")).await;
        watcher.observe_context().await;
        settle().await;
        assert_eq!(
            attaches()?,
            1,
            "the pass after it moved to an allowed page attaches it"
        );
        Ok(())
    }

    /// While a run listens on the context for lifecycle events, an untouched tab whose renderer
    /// never answers core's attach costs the loop nothing, and only one attach of it is out at a
    /// time; once that one ends without a session, a later pass attaches it again.
    #[tokio::test(start_paused = true)]
    async fn a_context_listener_s_page_attach_never_holds_the_loop() -> TestResult {
        let connection = TestConnection::new([]);
        connection.always(
            "Browser.getTabs",
            json!({ "tabs": [{ "tabId": 7, "targetId": "busy", "windowId": 1,
                "url": "https://busy.test/", "title": "Busy", "isActive": false,
                "isLoading": false, "loadProgress": 1.0, "isPinned": false, "isHidden": false }] }),
        )?;
        connection.always("Target.attachToTarget", cdp_pending())?;
        let (hub, mut watcher, _answers) = watcher(&connection);
        let run = Arc::new(RunState::default());
        hub.bind(&run);
        run.events
            .listen(Subscription::Context(PageEventKind::Dialog), 1_000, 0);
        // Its URL from its own target event: the door allows it, so the pass this event runs
        // attaches it.
        read(&mut watcher, target_created("busy", "https://busy.test/")).await;
        let attaches = || -> Result<usize, Box<dyn Error>> {
            Ok(connection
                .calls()?
                .iter()
                .filter(|call| call.method == "Target.attachToTarget")
                .count())
        };

        for pass in 0..2 {
            time::timeout(TREE_READ_WAIT / 4, watcher.observe_context())
                .await
                .map_err(|elapsed| {
                    format!("pass {pass} waited for the busy tab's attach: {elapsed}")
                })?;
        }
        settle().await;
        assert_eq!(attaches()?, 1, "one attach of the tab is out at a time");

        time::sleep(CONTEXT_ATTACH_WAIT * 2).await;
        settle().await;
        time::timeout(TREE_READ_WAIT / 4, watcher.observe_context()).await?;
        settle().await;
        assert_eq!(
            attaches()?,
            2,
            "an attach that ended without a session is tried again"
        );
        Ok(())
    }
}
