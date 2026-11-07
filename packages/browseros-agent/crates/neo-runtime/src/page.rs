//! One handed-out tab as the server side of a browser call sees it (Playwright's `server/page.ts`
//! with `chromium/crPage.ts` and `crInput.ts`): this tab's CDP session,
//! the domains a browser call needs, one watcher task that answers dialogs and tracks
//! navigations, the tab's input turns and devices ([`TabInputs`]), and the `click::Tab`
//! implementation over all of it and the utility world (`world.rs`).
//!
//! This file is mechanism; `click.rs` and `navigate.rs` own policy. Nothing here writes a
//! call-log line or decides a retry, and policy names no CDP types.
//!
//! Lifetime: [`PageSession`] owns this tab's [`Session`], whose `Drop` detaches, so the session
//! detaches when the last `Arc<PageSession>` drops. Every task that writes on the session holds
//! one: the dispatcher's call tasks, the interceptor setups, the input writers, the interceptor
//! stops and the object-group releases, all spawned on the run's `TaskTracker`. So no release
//! can find its session gone, and no end-of-run step has to order a drain before a detach; this
//! also covers a caller that drops the run's future. The watcher holds no `Arc<PageSession>` (it would keep
//! the page alive forever): it holds a CDP client, the session id and a token cancelled when the
//! page drops. A task stuck past the run (a renderer frozen by a dialog the watcher missed) keeps
//! the session attached until its own CDP request times out (browseros-cdp's 60 s), then lets
//! go. A turn at the tab's input ([`InputPermit`]) is held the same way: by every task that
//! still operates a click's input, so it frees only once the last of them is done.
//!
//! [`PageSession::prepare`] enables domains on this tab's first click or beforeunload call.
//! Getter-only runs keep their single live CDP call per getter:
//! - `Page.enable`: `javascriptDialogOpening` (an `alert()` in an `onclick` freezes the renderer,
//!   and the click's mouse acknowledgement never returns until it is answered), the navigation
//!   events behind the pre-check and the post-click barrier, and `Page.enable` itself as that
//!   barrier's flush (`crPage.ts` `inputActionEpilogue`).
//! - `Page.setLifecycleEventsEnabled`: current-document load state, including the replay for
//!   documents that finished loading before this run attached.
//! - `Emulation.setFocusEmulationEnabled {enabled: true}` (untyped: browseros-cdp's surface has
//!   no Emulation domain), as Playwright sends it. It makes a background tab paint, so
//!   `requestAnimationFrame` runs and `checkElementStates`' `stable` resolves (probe P3: 0
//!   frames before, 30 per 500 ms after). The page also sees itself visible and focused while
//!   attached. A failure is logged and ignored: a foreground tab paints anyway.
//! - Not `Runtime.enable` (worlds are found by name, `world.rs`) and not `DOM.enable` (quads,
//!   box models and scrolls by `objectId` need none, probe P2). A run that listens to console
//!   messages or page errors, or reads their history, enables Runtime and Log on its own
//!   ([`PageSession::enable_runtime`]); the watcher then also feeds `page_events.rs`.

use std::collections::HashMap;
use std::future::Future;
use std::iter;
use std::ops::Deref;
use std::sync::Mutex as SyncMutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use browseros_cdp::{CdpClient, CdpError, CdpEvent, SessionId, dom, target};
use futures_util::future;
use neo_runtime_spec::injected::{
    ElementState, ElementStatesOutcome, FrameStyle, FrameStyleStatus, HitTargetAction,
    HitTargetInterceptor, HitTargetOutcome, HitTargetSetup, InjectedCheckElementStatesParams,
    InjectedCleanupDragParams, InjectedDescribeIFrameStyleParams, InjectedDescribeSelectorParams,
    InjectedExpectHitTargetParams, InjectedGetElementBorderWidthParams, InjectedResolveParams,
    InjectedScrollIntoViewParams, InjectedSetupDragListenersParams,
    InjectedSetupHitTargetInterceptorParams, InjectedStopHitTargetInterceptorParams,
    InjectedTitleParams, InjectedViewportSizeParams, Point, ScrollAlignment,
};
use neo_runtime_spec::serve::ResponseChannel;
use neo_runtime_spec::{Element, Handle};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{Mutex, OnceCell, OwnedMutexGuard, broadcast, oneshot, watch};
use tokio::time::{self, Instant};
use tokio_util::sync::{CancellationToken, DropGuard};
use tokio_util::task::TaskTracker;
use tracing::{debug, warn};

use crate::click::{
    Arming, Geometry, NotReady, PageError, PressReport, Resolved, Scroll, SelectorFacts,
    StatesCheck, Stop, Tab, Verdict,
};
use crate::events::{EventHub, Flushes};
use crate::frame_selector::{self, FrameOwner};
use crate::frames::{FrameContext, FrameScope, Frames, Host, local_tree};
use crate::highlight::Highlights;
use crate::input_devices::{
    self, Devices, Effect, EffectSink, InputDevices, InputSender, Owner, Plan,
};
use crate::keyboard;
use crate::locator_handlers::{self, LocatorHandlers};
use crate::navigate::{
    Lifecycle, NavigateReply, Navigation, NavigationState, NavigationTab, Observation,
};
use crate::network::{Happening, Lookup, NetworkLog};
use crate::network_idle::{IdleChange, SharedIdle};
use crate::page_events::PageEvents;
use crate::progress::InputTicket;
use crate::progress::{PlaywrightError, Progress};
use crate::routes::{Answer, ContextRoutes, Pause, Routes};
use crate::run::PageId;
use crate::world::{self, ObjectGroup, World};

/// Detaching is best effort. It runs in the background after a run, so this bound only keeps
/// a hung browser from piling up detach tasks.
const DETACH_TIMEOUT: Duration = Duration::from_secs(1);

/// Neo: how long one frame's drag-listener call may take before a move goes on without that
/// frame. Playwright evaluates there without stalling, so a frame paused on a dialog never holds
/// up the input; Neo bounds the call instead.
const DRAG_FRAME_CALL_WAIT: Duration = Duration::from_secs(1);

/// Neo: how long a move waits for `Input.dragIntercepted` once a frame saw its drag start.
/// Chromium sends it while it handles the move, so this only bounds a browser that never does.
const DRAG_INTERCEPT_WAIT: Duration = Duration::from_secs(5);

/// One run's CDP session on one tab, which owns the attach: dropping it detaches. The
/// `Runtime`'s connection outlives every run, so a session nobody detaches stays attached
/// until the browser exits. Tying the detach to the value covers every way a run ends after
/// the browser answered the attach: the script's end, a failed or stopped setup, an answer
/// that arrives after the run stopped, and a caller that drops the run's future.
pub(crate) struct Session {
    pub(crate) cdp: CdpClient,
    pub(crate) id: SessionId,
    /// browseros-cdp's socket incarnation, read just before the attach is sent: the
    /// incarnation `id` belongs to. Mouse input is written with `send_at_epoch(epoch, …)`, so
    /// a press is never sent on a reconnected socket under a session id that does not exist
    /// there.
    epoch: u64,
    /// Drop is synchronous, so the detach and every cleanup task run on this runtime.
    tokio: TokioHandle,
}

impl Session {
    pub(crate) async fn attach(
        cdp: CdpClient,
        page: PageId,
        tokio: TokioHandle,
    ) -> Result<Self, CdpError> {
        let epoch = cdp.epoch();
        let attached: target::AttachToTargetResult = cdp
            .send_typed(
                "Target.attachToTarget",
                target::AttachToTargetParams {
                    target_id: page.as_str().to_string(),
                    flatten: Some(true),
                },
                None,
            )
            .await?;
        Ok(Self {
            cdp,
            id: SessionId::from(attached.session_id),
            epoch,
            tokio,
        })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let cdp = self.cdp.clone();
        let session = self.id.clone();
        self.tokio.spawn(async move {
            let detach = cdp.send_typed::<_, Value>(
                "Target.detachFromTarget",
                target::DetachFromTargetParams {
                    session_id: Some(session.as_str().to_string()),
                    target_id: None,
                },
                None,
            );
            match time::timeout(DETACH_TIMEOUT, detach).await {
                Ok(Ok(_)) => {}
                // Expected once the tab has closed: the session went with it.
                Ok(Err(error)) => debug!(%session, %error, "detaching a run's session failed"),
                Err(_) => debug!(%session, "detaching a run's session timed out"),
            }
        });
    }
}

/// A handed-out tab's session, attached lazily once per run and shared by its call and writer tasks.
pub(crate) struct PageSession {
    session: Arc<Session>,
    /// The main frame's id, which is the target id.
    main_frame: String,
    /// The run's tracker, for the interceptor-setup, writer, interceptor-stop and
    /// group-release tasks, so `Dispatcher::finish` can wait for them before reading final
    /// deliveries.
    tasks: TaskTracker,
    /// Channels exist before domain setup, so a wait can subscribe before preparation emits
    /// commits. Every watcher attempt publishes into the same channels.
    state: watch::Receiver<PageState>,
    state_updates: watch::Sender<PageState>,
    navigations: broadcast::Sender<Navigation>,
    /// Set after domain setup; dropping the guard stops the watcher.
    prepared: OnceCell<DropGuard>,
    /// Cancelled when the tab closes or the session detaches; every call's `Progress` races
    /// it, so a call on a closed tab ends with Playwright's `TargetClosedError`.
    closed: CancellationToken,
    /// The main frame's utility world, where the glue runs.
    pub(crate) world: Arc<World>,
    pub(crate) frames: Arc<Frames>,
    /// Child frames' navigation, written by this tab's watcher and every OOPIF host's.
    frame_feeds: Arc<FrameFeeds>,
    /// The navigation watcher of each OOPIF host session, started by the first wait on one of
    /// its frames. Dropping a guard (with the page) stops that watcher.
    host_watchers: SyncMutex<HashMap<SessionId, Arc<OnceCell<DropGuard>>>>,
    /// Names this session's object groups apart.
    next_group: AtomicU64,
    /// The tab's input lock and devices, shared with every other run of the `Runtime` on this
    /// tab ([`TabInputs`]). [`Tab::input`] waits for a turn at it.
    input: Arc<TabInput>,
    /// The main frame's navigation requests, fed by the watcher once Network is enabled.
    pub(crate) network: Arc<NetworkLog>,
    /// The run's routed requests on this tab (`routes.rs`).
    pub(crate) routes: Arc<Routes>,
    /// `Network.enable`, sent on this session's first goto and kept for the run.
    network_enabled: OnceCell<()>,
    /// Playwright's `Screenshotter` task queue: one screenshot at a time on this tab, so one's
    /// background override, masks and page preparation never overlap another's. Its cleanup
    /// tasks hold the turn too, until they have undone their part. The turn is the tab's, shared
    /// with every other run of the `Runtime` on it (`screenshot.rs` `TabScreenshots`).
    pub(crate) screenshots: Arc<Mutex<()>>,
    /// The tab's frames' `networkidle`, fed and timed by the watcher.
    idle: Arc<SharedIdle>,
    /// The run's highlights on this tab, hidden when the run ends (`highlight.rs`).
    pub(crate) highlights: Highlights,
    /// What the watcher turns into the run's events, and the tab's console history and dialogs.
    pub(crate) events: PageEvents,
    /// The run's locator handlers on this tab (`locator_handlers.rs`).
    pub(crate) locator_handlers: LocatorHandlers,
    /// `Runtime.enable` and `Log.enable`, sent once a run listens to console messages or page
    /// errors (or reads their history) and kept for the run.
    runtime_enabled: OnceCell<()>,
}

/// One call's frame scope over the tab's shared input, navigation and lifetime.
#[derive(Clone)]
pub(crate) struct FrameTab {
    pub(crate) page: Arc<PageSession>,
    pub(crate) scope: FrameScope,
}

// Input, preparation and top-level navigation always belong to the shared tab.
impl Deref for FrameTab {
    type Target = Arc<PageSession>;

    fn deref(&self) -> &Self::Target {
        &self.page
    }
}

impl FrameTab {
    pub(crate) async fn context(&self) -> Result<Arc<FrameContext>, PageError> {
        match &self.scope {
            FrameScope::Main => Ok(self.frames.main()),
            FrameScope::Frame(id) => self.frames.context(id, None).await,
        }
    }

    pub(crate) fn group(&self, frame: &Arc<FrameContext>, purpose: &str) -> GroupGuard {
        new_group(&self.page, frame, purpose)
    }
}

struct PageUpdates<S> {
    state: watch::Sender<PageState>,
    navigations: broadcast::Sender<Navigation>,
    world: Arc<World>,
    network: Arc<NetworkLog>,
    frames: Option<Arc<Frames>>,
    /// Where the child frames' events go; `None` in tests of the main frame alone.
    feeds: Option<Arc<FrameFeeds>>,
    initial_url: S,
    /// The run's events from this tab, and the hub's flushes; `None` in tests of navigation
    /// alone.
    producer: Option<(PageEvents, Flushes)>,
    /// The tab's `networkidle` bookkeeping; `None` in tests of navigation alone.
    idle: Option<Arc<SharedIdle>>,
    /// The tab's routed requests; `None` in tests of navigation alone.
    routes: Option<Arc<Routes>>,
}

/// What the run's session has reported since `prepare`. Counters only grow, so a caller takes
/// a [`PageMark`] and compares later.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PageState {
    /// Main-frame `Page.frameRequestedNavigation` with `disposition: currentTab`, what
    /// Playwright's `SignalBarrier` waits on.
    pub(crate) navigations_requested: u64,
    /// The URL of the newest requested navigation that has not committed (`frameNavigated`),
    /// finished within the document (`navigatedWithinDocument`) or stopped
    /// (`frameStoppedLoading`): Playwright's `pendingDocument`.
    pub(crate) pending_url: Option<String>,
    /// Main-frame commits, cross-document or within it.
    pub(crate) commits: u64,
    /// The URL of the newest main-frame commit.
    pub(crate) committed_url: Option<String>,
    /// Loader of the current main-frame document. Same-document commits preserve it.
    pub(crate) document: Option<String>,
    pub(crate) dom_content_loaded: bool,
    pub(crate) load: bool,
    pub(crate) dom_content_loaded_count: u64,
    pub(crate) load_count: u64,
    /// `networkidle` fired for the current document (`network_idle.rs`), and how often it fired
    /// across documents.
    pub(crate) network_idle: bool,
    pub(crate) network_idle_count: u64,
    /// Playwright's `Frame.pendingDocument()`: the navigation the frame requested or whose
    /// request started, until a commit takes it or its request fails. Requests are seen only once
    /// `Network.enable` ran.
    pending_document: Option<PendingDocument>,
    /// `Page.javascriptDialogOpening` events seen; each was answered.
    pub(crate) dialogs_opened: u64,
    /// Neo: how many `setContent` writes still expect their reopened document's `init`, each to
    /// clear this frame's lifecycle once, as Playwright's clears it right after each
    /// `document.open()`. Overlapping writes each get their own clear; a new document drops them
    /// (a write still pending then targets the old document, and fails).
    pub(crate) reopens_expected: u32,
    /// Neo: how many of those reopens cleared this frame's lifecycle so far, across documents
    /// ([`Lifecycle::reopens`]).
    pub(crate) reopens: u64,
}

/// The navigation a frame is waiting to commit ([`PageState::pending_document`]).
#[derive(Clone, Debug, PartialEq, Eq)]
enum PendingDocument {
    /// Requested (`Page.frameRequestedNavigation`), with no request known yet.
    Requested,
    /// Its request started (`Network.requestWillBeSent`): the request id, which is also the
    /// document's loader id.
    Started(String),
}

/// A snapshot of the counters in [`PageState`], taken when an interceptor is armed.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PageMark {
    navigations_requested: u64,
    dialogs_opened: u64,
}

impl PageState {
    pub(crate) fn lifecycle(&self) -> Lifecycle {
        Lifecycle {
            dom_content_loaded: self.dom_content_loaded,
            load: self.load,
            dom_content_loaded_count: self.dom_content_loaded_count,
            load_count: self.load_count,
            network_idle: self.network_idle,
            network_idle_count: self.network_idle_count,
            reopens: self.reopens,
        }
    }

    /// `networkidle` added (`true`) or removed for the current document. Whether it changed.
    fn set_network_idle(&mut self, fired: bool) -> bool {
        if self.network_idle == fired {
            return false;
        }
        self.network_idle = fired;
        if fired {
            self.network_idle_count += 1;
        }
        true
    }

    fn did_dom_content_load(&mut self) {
        if !self.dom_content_loaded {
            self.dom_content_loaded = true;
            self.dom_content_loaded_count += 1;
        }
    }

    /// A document that counts as loaded the moment it commits: its lifecycle events, fired.
    fn loaded_at_once(&mut self) -> Fired {
        let before = (self.dom_content_loaded, self.load);
        self.did_dom_content_load();
        self.did_load();
        Fired::between(before, self)
    }

    /// Playwright's `Frame._onClearLifecycle` for a committed document: its `DOMContentLoaded`,
    /// `load` and `networkidle` count again, and no write still expects a reopen of the previous one.
    fn clear_lifecycle(&mut self) {
        self.reopens_expected = 0;
        self.dom_content_loaded = false;
        self.load = false;
        self.network_idle = false;
    }

    /// Playwright's `Frame._onClearLifecycle` right after one `setContent` write's
    /// `document.open()`: that write's clear, and only that one, is spent. Its `networkidle`
    /// clears too, as the tab's idle tracker clears the frame (`Watcher::idle_event`), and the
    /// reopen counts, so the write's own wait tells its document's idleness from the old one's.
    fn reopened(&mut self) {
        self.reopens_expected -= 1;
        self.reopens += 1;
        self.dom_content_loaded = false;
        self.load = false;
        self.network_idle = false;
    }

    fn did_load(&mut self) {
        if !self.load {
            self.load = true;
            self.load_count += 1;
        }
    }
}

/// A child frame's navigation as the watchers report it: the same [`PageState`] the main frame
/// keeps (URL, document, lifecycle flags and counts, through [`apply_frame`]), plus what only a
/// child frame does.
#[derive(Clone, Debug, Default)]
pub(crate) struct FrameState {
    pub(crate) page: PageState,
    /// Removed from its parent (`Page.frameDetached`, reason `remove`), or gone with the main
    /// frame's document.
    pub(crate) detached: bool,
    /// How often its document moved to another process (`Page.frameDetached`, reason `swap`).
    /// Its events then come from another session, which a wait has to find and watch.
    pub(crate) swaps: u64,
    /// The session that reported the last swap. Its late events about this frame would describe
    /// the old document, so they are dropped until that session hosts the frame again.
    swapped_from: Option<SessionId>,
}

/// One child frame's state and commit queue.
struct FrameFeed {
    state: watch::Sender<FrameState>,
    navigations: broadcast::Sender<Navigation>,
}

/// Commits a child frame's queue keeps for a slow wait; a wait that lags behind rechecks the
/// frame's current state instead.
const FRAME_NAVIGATIONS: usize = 256;

/// Every child frame's feed on a tab, by frame id, created by the first event or wait that names
/// the frame. The tab's watcher and every OOPIF host's watcher write here, so a frame keeps one
/// feed, and one set of lifecycle counts, when its document moves between processes. A feed
/// goes when its frame is removed; a wait still holding it sees `detached`.
#[derive(Default)]
pub(crate) struct FrameFeeds {
    frames: SyncMutex<HashMap<String, FrameFeed>>,
}

impl FrameFeeds {
    /// [`PageSession::expect_reopen`] for a child frame (0 if the feeds' lock is poisoned).
    pub(crate) fn expect_reopen(&self, frame: &str, expected: bool) -> u64 {
        let mut reopens = 0;
        if let Ok(mut frames) = self.frames.lock() {
            let feed = frames.entry(frame.to_string()).or_insert_with(new_feed);
            feed.state
                .send_modify(|state| reopens = expect(&mut state.page, expected));
        }
        reopens
    }

    /// `frame`'s current state and its commits from now on.
    pub(crate) fn subscribe(
        &self,
        frame: &str,
    ) -> Result<(watch::Receiver<FrameState>, broadcast::Receiver<Navigation>), PageError> {
        let mut frames = self.frames.lock().map_err(|_| feeds_error())?;
        let feed = frames.entry(frame.to_string()).or_insert_with(new_feed);
        Ok((feed.state.subscribe(), feed.navigations.subscribe()))
    }

    /// Records `url` as `frame`'s URL while no commit has given it one: what the frame tree said
    /// when a wait found the frame, or when its host's watcher started.
    pub(crate) fn seed(&self, frame: &str, url: &str) {
        if let Ok(mut frames) = self.frames.lock() {
            let feed = frames.entry(frame.to_string()).or_insert_with(new_feed);
            feed.state.send_if_modified(|state| {
                if state.page.committed_url.is_some() {
                    return false;
                }
                state.page.committed_url = Some(url.to_string());
                true
            });
        }
    }

    /// Applies one event of `session` to the child frame it names. `main_frame` is the tab's
    /// main frame, whose events the tab's own [`PageState`] takes instead. Whether the event was
    /// the `init` of the frame's document a `setContent` write reopened.
    fn apply(&self, event: &CdpEvent, session: &SessionId, main_frame: Option<&str>) -> bool {
        if event.session_id.as_ref() != Some(session) {
            return false;
        }
        let Some(frame) = event_frame(event) else {
            return false;
        };
        if Some(frame) == main_frame {
            return false;
        }
        let Ok(mut frames) = self.frames.lock() else {
            return false;
        };
        let feed = frames.entry(frame.to_string()).or_insert_with(new_feed);
        let mut navigation = None;
        let mut removed = false;
        let mut reopened = false;
        feed.state.send_if_modified(|state| {
            if event.method == "Page.frameDetached" {
                if event.params["reason"] == "swap" {
                    // The frame's next document lives in another process: forget this one so
                    // that process's lifecycle replay seeds the new document.
                    let page = &mut state.page;
                    page.committed_url = None;
                    page.pending_url = None;
                    page.document = None;
                    page.clear_lifecycle();
                    page.pending_document = None;
                    state.swaps += 1;
                    state.swapped_from = Some(session.clone());
                } else {
                    state.detached = true;
                    removed = true;
                }
                return true;
            }
            if state.swapped_from.as_ref() == Some(session) {
                // A frame coming back to this session announces itself before its document.
                if !matches!(
                    event.method.as_str(),
                    "Page.frameAttached" | "Page.frameNavigated"
                ) {
                    return false;
                }
                state.swapped_from = None;
            }
            let reopens = state.page.reopens;
            let applied = apply_frame(&mut state.page, event, frame);
            navigation = applied.navigation;
            reopened = state.page.reopens > reopens;
            applied.changed
        });
        if let Some(navigation) = navigation {
            let _listeners = feed.navigations.send(navigation);
        }
        if removed {
            // Waits keep their receivers and read `detached`; the frame id is never reused.
            frames.remove(frame);
        }
        reopened
    }
}

impl FrameFeeds {
    /// A navigation request of child frame `frame` started or failed (`Network.requestWillBeSent`,
    /// `Network.loadingFailed`), which the frame's feed follows as the main frame's state does
    /// ([`apply_navigation_request`]): its pending document, and a failure that aborts it.
    /// Network events name no frame's session, so the tab's watcher routes them here by frame.
    fn apply_network(&self, event: &CdpEvent, frame: &str) {
        let Ok(mut frames) = self.frames.lock() else {
            return;
        };
        let feed = frames.entry(frame.to_string()).or_insert_with(new_feed);
        let mut navigation = None;
        feed.state.send_if_modified(|state| {
            let applied = apply_navigation_request(&mut state.page, event, frame);
            navigation = applied.navigation;
            applied.changed
        });
        if let Some(navigation) = navigation {
            let _listeners = feed.navigations.send(navigation);
        }
    }

    /// `frame`'s `networkidle` fired or was removed (`network_idle.rs`).
    fn set_network_idle(&self, frame: &str, fired: bool) {
        if let Ok(frames) = self.frames.lock()
            && let Some(feed) = frames.get(frame)
        {
            feed.state
                .send_if_modified(|state| state.page.set_network_idle(fired));
        }
    }

    /// Ends every child frame's feed: the main frame committed a new document, and the old
    /// document's frames went with it. Chromium reports `frameDetached` for an OOPIF then, but
    /// nothing for a same-process frame (measured on BrowserOS neo), so its wait would
    /// otherwise run to its timeout.
    fn detach_all(&self) {
        if let Ok(mut frames) = self.frames.lock() {
            for (_, feed) in frames.drain() {
                feed.state.send_modify(|state| state.detached = true);
            }
        }
    }
}

fn new_feed() -> FrameFeed {
    FrameFeed {
        state: watch::channel(FrameState::default()).0,
        navigations: broadcast::channel(FRAME_NAVIGATIONS).0,
    }
}

fn feeds_error() -> PageError {
    PageError::Contract("neo-runtime: frame feeds cannot be accessed".to_string())
}

/// The frame a `Page` event is about: `frame.id` of a commit, `frameId` of the others.
pub(crate) fn event_frame(event: &CdpEvent) -> Option<&str> {
    match event.method.as_str() {
        "Page.frameNavigated" => event.params["frame"]["id"].as_str(),
        "Page.frameRequestedNavigation"
        | "Page.navigatedWithinDocument"
        | "Page.frameStoppedLoading"
        | "Page.lifecycleEvent"
        | "Page.frameAttached"
        | "Page.frameDetached" => event.params["frameId"].as_str(),
        _ => None,
    }
}

/// How the watcher answers a dialog: Playwright's default with no `page.on('dialog')` listener
/// (`client/browserContext.ts`): accept `beforeunload`, dismiss everything else. This applies
/// to the user's real tab while a clicking run is attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogAnswer {
    Accept,
    Dismiss,
}

/// What one event did to the page, for the watcher.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Applied {
    pub(crate) changed: bool,
    pub(crate) dialog: Option<DialogAnswer>,
    pub(crate) closed: bool,
    pub(crate) new_document: bool,
    pub(crate) navigation: Option<Navigation>,
    /// What the main frame's current document fired with this event.
    pub(crate) fired: Fired,
}

/// The lifecycle events a document fired with one CDP event: at most each, once per document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Fired {
    pub(crate) dom_content_loaded: bool,
    pub(crate) load: bool,
}

impl Fired {
    fn between(before: (bool, bool), state: &PageState) -> Self {
        Self {
            dom_content_loaded: !before.0 && state.dom_content_loaded,
            load: !before.1 && state.load,
        }
    }
}

impl PageSession {
    /// `input` is the tab's lock and devices from the `Runtime`'s [`TabInputs`], `screenshots` its
    /// screenshot turn from the `Runtime`'s `TabScreenshots`.
    /// `discovered`: the run's discovery reported the page as new (`Tabs::report_new`), so the
    /// first lifecycle replay is the page's own new life ([`PageEvents::new`]).
    #[expect(
        clippy::too_many_arguments,
        reason = "the tab's turns come from the Runtime, and the rest (its input, closure, event hub \
                  and context routes) from the run; each is one field"
    )]
    pub(crate) fn new(
        session: Session,
        page: &PageId,
        tasks: TaskTracker,
        input: Arc<TabInput>,
        screenshots: Arc<Mutex<()>>,
        closed: CancellationToken,
        hub: EventHub,
        discovered: bool,
        context_routes: Arc<ContextRoutes>,
    ) -> Self {
        let routes = Arc::new(Routes::new(page.as_str(), context_routes));
        let child_routes = Arc::clone(&routes);
        let events = PageEvents::new(
            hub,
            page.as_str(),
            session.cdp.clone(),
            session.id.clone(),
            discovered,
        );
        let child_events = events.clone();
        let network = Arc::new(NetworkLog::new(page.as_str().to_string()));
        let child_network = Arc::clone(&network);
        let world = World::new(
            session.cdp.clone(),
            session.id.clone(),
            page.as_str().to_string(),
        );
        let session = Arc::new(session);
        let world = Arc::new(world);
        let frames = Arc::new(Frames::new(
            Arc::clone(&session),
            page.as_str().to_string(),
            Arc::clone(&world),
            tasks.clone(),
        ));
        // An out-of-process frame's console and requests are this tab's: each turns on when the
        // frame's session is attached, if the run wants the tab's by then.
        frames.on_child_host(Arc::new(move |host| {
            let events = child_events.clone();
            let network = Arc::clone(&child_network);
            let routes = Arc::clone(&child_routes);
            Box::pin(async move {
                events.child_attached(&host).await;
                if network.enabled() {
                    enable_child_network(&network, &host).await;
                    // Playwright gives a new frame session the page's interception before it
                    // runs (`FrameSession._initialize`); Neo's is attached lazily, so its
                    // requests pause from now on.
                    if routes.intercepting() {
                        for (method, params) in interception(true) {
                            if let Err(error) = host.send(method, params).await {
                                debug!(%error, "pausing a frame session's requests failed");
                            }
                        }
                    }
                }
            })
        }));
        let (state_updates, state) = watch::channel(PageState::default());
        let (navigations, _) = broadcast::channel(4096);
        Self {
            session,
            main_frame: page.as_str().to_string(),
            tasks,
            state,
            state_updates,
            navigations,
            prepared: OnceCell::new(),
            closed,
            world,
            frames,
            frame_feeds: Arc::default(),
            host_watchers: SyncMutex::default(),
            next_group: AtomicU64::new(1),
            input,
            network,
            routes,
            network_enabled: OnceCell::new(),
            screenshots,
            idle: Arc::new(SharedIdle::new(page.as_str(), Instant::now())),
            highlights: Highlights::default(),
            events,
            locator_handlers: LocatorHandlers::default(),
            runtime_enabled: OnceCell::new(),
        }
    }

    pub(crate) fn session_id(&self) -> &SessionId {
        &self.session.id
    }

    pub(crate) fn closed(&self) -> CancellationToken {
        self.closed.clone()
    }

    /// Starts the domains and watcher once per page session. Concurrent calls wait for the
    /// same setup. Subscribes `cdp.events()` before `Page.enable`, so no event the
    /// enable triggers (a dialog already open is reported then) is missed. Never
    /// `events_targeted`, which is exclusive per method across the whole shared client.
    ///
    /// The watcher starts before `Page.enable` is sent, because only the watcher can tell the
    /// run that the tab closed: Chromium never answers a command in flight when its target
    /// closes (measured on BrowserOS neo), so a close during the enable would otherwise hang
    /// the call until its timeout instead of ending it with `TargetClosedError`.
    ///
    /// # Errors
    ///
    /// The CDP error of `Page.enable`.
    pub(crate) async fn prepare(&self) -> Result<(), CdpError> {
        self.prepared
            .get_or_try_init(|| async {
                let events = self.session.cdp.events();
                let (seed_ready, ready) = oneshot::channel();
                let cdp = self.session.cdp.clone();
                let session = self.session.id.clone();
                let producer = self.events.clone();
                // The seed's frame tree, read once the lifecycle replay has been applied (see
                // `seed_url`), is where the run's frame and lifecycle events start.
                let initial_url = seed_url(ready, move |method| async move {
                    let tree = cdp.send(method, json!({}), Some(&session)).await;
                    producer.seeded(tree.as_ref().ok());
                    tree
                });
                // Guarded before the spawn: if this setup fails or is dropped before it
                // finishes (the call stopped), the guard stops the watcher it already started.
                let stop = CancellationToken::new();
                let stop_guard = stop.clone().drop_guard();
                self.session.tokio.spawn(watch_events(
                    events,
                    self.session.cdp.clone(),
                    self.session.id.clone(),
                    self.main_frame.clone(),
                    PageUpdates {
                        state: self.state_updates.clone(),
                        navigations: self.navigations.clone(),
                        world: Arc::clone(&self.world),
                        network: Arc::clone(&self.network),
                        frames: Some(Arc::clone(&self.frames)),
                        feeds: Some(Arc::clone(&self.frame_feeds)),
                        initial_url,
                        producer: Some((self.events.clone(), self.events.hub().watcher())),
                        idle: Some(Arc::clone(&self.idle)),
                        routes: Some(Arc::clone(&self.routes)),
                    },
                    self.closed.clone(),
                    stop,
                ));
                self.send("Page.enable", json!({})).await?;
                // Chromium replays the existing document's lifecycle here (navigation_e2e),
                // including pages loaded before this run and restored from bfcache.
                self.send("Page.setLifecycleEventsEnabled", json!({ "enabled": true }))
                    .await?;
                // The replay preceded the reply: once the watcher has applied it, the main
                // frame's lifecycle events are the run's (`PageEvents`). Free when nothing is
                // subscribed.
                self.events.hub().barrier().await;
                self.events.lifecycle_replayed();
                // The watcher reads the frame tree after Page.enable and lifecycle replay.
                // Preparation never waits for that read: clicks must not pay for the seed.
                let _ = seed_ready.send(());
                let focus = self
                    .send(
                        "Emulation.setFocusEmulationEnabled",
                        json!({ "enabled": true }),
                    )
                    .await;
                if let Err(error) = focus {
                    debug!(%error, "focus emulation refused; a background tab may not paint");
                }
                Ok(stop_guard)
            })
            .await
            .map(|_| ())
    }

    /// `Runtime.enable` and `Log.enable`, once per session and after [`Self::prepare`]: what
    /// reports console messages and page errors to the watcher (`page_events.rs`). Neo sends
    /// them only for a run that listens to those events or reads their history.
    ///
    /// The enable replays what V8 kept for the tab's live documents before it replies. The hub's
    /// flush then makes the watcher apply everything that came before the replies, so that
    /// replay lands in the history before the console log goes live.
    ///
    /// # Errors
    ///
    /// The CDP error of `Page.enable` or of a setup command here.
    pub(crate) async fn enable_runtime(&self) -> Result<(), CdpError> {
        self.prepare().await?;
        self.runtime_enabled
            .get_or_try_init(|| async {
                self.events.console.request(self.session_id());
                self.send("Runtime.enable", json!({})).await?;
                self.send("Log.enable", json!({})).await?;
                self.events.hub().replay_barrier().await;
                self.events.console.go_live(self.session_id());
                // The out-of-process frames this run already attached report on their own
                // sessions; later ones turn on when they attach (`PageEvents::child_attached`).
                for host in self.frames.child_hosts() {
                    self.events.enable_child(&host).await;
                }
                Ok::<(), CdpError>(())
            })
            .await
            .copied()
    }

    /// `Network.enable`, once per session and after [`Self::prepare`]: from then on the watcher
    /// records every request of the tab in [`NetworkLog`], and each out-of-process frame session
    /// this run has attached (or attaches later) reports its own requests there too. Sent by the
    /// first goto, history step or navigation wait, a network event listener, or
    /// `page.requests()`, and kept for the run: the session's detach at the run's end disables it.
    ///
    /// # Errors
    ///
    /// The CDP error of `Page.enable` or of `Network.enable` on the tab's session.
    pub(crate) async fn enable_network(&self) -> Result<(), CdpError> {
        self.prepare().await?;
        self.network_enabled
            .get_or_try_init(|| async {
                self.send("Network.enable", json!({})).await?;
                self.network.mark_enabled();
                self.restart_idle().await;
                for host in self.frames.child_hosts() {
                    enable_child_network(&self.network, &host).await;
                }
                Ok::<(), CdpError>(())
            })
            .await
            .copied()
    }

    /// Turns the pausing of every request on or off on the tab's session and on every
    /// out-of-process frame session attached (`routes.rs`); the network log first, as a pause's
    /// route carries its request's record.
    pub(crate) async fn intercept(&self, on: bool) -> Result<(), CdpError> {
        if on {
            self.enable_network().await?;
        }
        for (method, params) in interception(on) {
            self.send(method, params).await?;
        }
        for host in self.frames.child_hosts() {
            for (method, params) in interception(on) {
                if let Err(error) = host.send(method, params).await {
                    debug!(%error, "turning a frame session's request pausing over failed");
                }
            }
        }
        Ok(())
    }

    /// The requests seen from now on are the tab's whole traffic: `networkidle` starts over
    /// (`NetworkIdle::restart`), over the frames this session reports, which a frame-tree read
    /// adds to the ones the watcher saw attach. A failed read leaves the tree as the watcher knows
    /// it.
    async fn restart_idle(&self) {
        let frames = match self.send("Page.getFrameTree", json!({})).await {
            Ok(reply) => local_tree(&reply).unwrap_or_default(),
            Err(error) => {
                debug!(%error, "the frame tree for networkidle could not be read");
                Vec::new()
            }
        };
        let now = Instant::now();
        let changes = self
            .idle
            .with(|idle| {
                for frame in &frames {
                    if let Some(parent) = &frame.parent {
                        idle.attach(&frame.id, parent, now);
                    }
                }
                idle.restart(now)
            })
            .unwrap_or_default();
        apply_idle(
            &self.main_frame,
            &self.state_updates,
            Some(&self.frame_feeds),
            changes,
        );
        self.idle.moved();
    }

    pub(crate) async fn send(&self, method: &str, params: Value) -> Result<Value, CdpError> {
        self.session
            .cdp
            .send(method, params, Some(&self.session.id))
            .await
    }

    /// Sends on `session`, one of this tab's out-of-process frame sessions, or on the tab's own
    /// for `None`: where a request the network log recorded is read (`responses.rs`).
    pub(crate) async fn send_to(
        &self,
        session: Option<&SessionId>,
        method: &str,
        params: Value,
    ) -> Result<Value, CdpError> {
        let session = session.unwrap_or(&self.session.id);
        self.session.cdp.send(method, params, Some(session)).await
    }

    /// Runs a cleanup or writer task on the run's tracker. Spawned on the session's runtime
    /// handle because it may be called from a `Drop`.
    pub(crate) fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        self.tasks.spawn_on(task, &self.session.tokio);
    }

    /// Neo: [`Self::spawn`] for page-side work that matters only while the tab is open (a
    /// capture's restore or reset, an object group's release); browser-side cleanup such as a
    /// PDF stream's close (`pdf.rs`) does not use it. It ends when the tab closes or crashes,
    /// and dropping it drops what it holds (a screenshot turn, an object group): a gone tab has
    /// nothing left to undo, and a crashed one does not answer what is sent to it (BrowserOS
    /// neo, the crashed case of `capture-turn-liveness-review`), so the task would otherwise
    /// hold those, and a host's `Runtime::drain`, until browseros-cdp's request timeout. Only a
    /// prepared session's watcher sees the tab go ([`Self::prepare`]).
    pub(crate) fn spawn_while_open(&self, task: impl Future<Output = ()> + Send + 'static) {
        let closed = self.closed.clone();
        self.spawn(async move {
            tokio::select! {
                biased;
                () = closed.cancelled() => {}
                () = task => {}
            }
        });
    }

    fn state(&self) -> Option<watch::Receiver<PageState>> {
        self.prepared.get().map(|_| self.state.clone())
    }

    /// Resolves when no navigation is pending, with the URL it committed to, if any.
    pub(crate) async fn navigation_settled(&self) -> Result<Option<String>, PageError> {
        let Some(mut state) = self.state() else {
            return Ok(None);
        };
        let commits = state.borrow().commits;
        let settled = state
            .wait_for(|state| state.pending_url.is_none())
            .await
            .map_err(|_| PageError::Closed)?;
        Ok(if settled.commits > commits {
            settled.committed_url.clone()
        } else {
            None
        })
    }

    /// Neo: one `setContent` write expects (or, failing before its `document.open()`, no longer
    /// expects) its reopened document's `init` in `frame`, which clears the frame's lifecycle once,
    /// as Playwright's `_onClearLifecycle` right after `document.open()`, so a later
    /// `waitForLoadState` waits for that write's document. Answers the frame's count of reopens
    /// once this one is in ([`Lifecycle::reopens`]).
    pub(crate) fn expect_reopen(&self, frame: &str, expected: bool) -> u64 {
        if frame == self.main_frame {
            let mut reopens = 0;
            self.state_updates
                .send_modify(|state| reopens = expect(state, expected));
            reopens
        } else {
            self.frame_feeds.expect_reopen(frame, expected)
        }
    }

    pub(crate) fn pending_navigation(&self) -> Option<String> {
        self.state()
            .and_then(|state| state.borrow().pending_url.clone())
    }

    /// Waits can recover from a full commit queue by checking the latest state. An
    /// intermediate URL may have been lost; goto keeps its existing overflow error.
    pub(crate) fn observe_waits(&self) -> NavigationObservation {
        let mut observation = self.observe_navigations();
        observation.recover_lag = true;
        observation
    }

    /// `frame`'s navigation state and its commits from now on.
    pub(crate) fn subscribe_frame(
        &self,
        frame: &str,
    ) -> Result<(watch::Receiver<FrameState>, broadcast::Receiver<Navigation>), PageError> {
        self.frame_feeds.subscribe(frame)
    }

    /// See [`FrameFeeds::seed`].
    pub(crate) fn seed_frame(&self, frame: &str, url: &str) {
        self.frame_feeds.seed(frame, url);
    }

    /// Makes sure `host`'s frames feed [`FrameFeeds`]. The tab's own frames do once the page is
    /// prepared; an OOPIF host gets its own watcher, once per session, and this resolves when
    /// that watcher has applied its seed: `Page.enable`, then `Page.setLifecycleEventsEnabled`,
    /// whose replay reports each of the host's frames' current lifecycle, then the host's frame
    /// tree for their URLs. A setup the call abandons stops its watcher, and the next wait
    /// starts another.
    ///
    /// # Errors
    ///
    /// The host's classified CDP error; `NotConnected` when its session went meanwhile.
    pub(crate) async fn watch_host(&self, host: &Arc<Host>) -> Result<(), PageError> {
        if !host.is_child() {
            return Ok(());
        }
        let watcher = {
            let mut watchers = self.host_watchers.lock().map_err(|_| feeds_error())?;
            Arc::clone(watchers.entry(host.session_id().clone()).or_default())
        };
        watcher
            .get_or_try_init(|| async {
                // Subscribed before any command, so no event the setup causes is missed.
                let events = self.session.cdp.events();
                let (seeded, ready) = oneshot::channel();
                let stop = CancellationToken::new();
                let guard = stop.clone().drop_guard();
                self.session.tokio.spawn(watch_host_frames(
                    events,
                    Arc::clone(host),
                    Arc::clone(&self.frame_feeds),
                    seeded,
                    stop,
                ));
                ready.await.map_err(|_| PageError::NotConnected)??;
                Ok(guard)
            })
            .await
            .map(|_| ())
    }

    fn observe_navigations(&self) -> NavigationObservation {
        NavigationObservation {
            navigations: self.navigations.subscribe(),
            state: self.state.clone(),
            recover_lag: false,
        }
    }

    fn mark(&self) -> PageMark {
        self.state().map_or_else(PageMark::default, |state| {
            let state = state.borrow();
            PageMark {
                navigations_requested: state.navigations_requested,
                dialogs_opened: state.dialogs_opened,
            }
        })
    }
}

/// A commit queue plus current state. Lifecycle counts survive watch coalescing; commits
/// use a queue so two navigations before a CDP reply cannot hide an intermediate URL.
pub(crate) struct NavigationObservation {
    navigations: broadcast::Receiver<Navigation>,
    state: watch::Receiver<PageState>,
    recover_lag: bool,
}

impl Observation for NavigationObservation {
    fn state(&self) -> NavigationState {
        let state = self.state.borrow();
        NavigationState {
            pending_url: state.pending_url.clone(),
            url: state.committed_url.clone(),
            lifecycle: state.lifecycle(),
        }
    }

    async fn next(&mut self) -> Result<Option<Navigation>, PageError> {
        tokio::select! {
            biased;
            navigation = self.navigations.recv() => match navigation {
                Ok(navigation) => Ok(Some(navigation)),
                Err(RecvError::Closed) => Err(PageError::Closed),
                Err(RecvError::Lagged(_)) if self.recover_lag => Ok(None),
                Err(RecvError::Lagged(_)) => Err(PageError::Contract("Navigation event queue overflowed".to_string())),
            },
            changed = self.state.changed() => changed.map(|()| None).map_err(|_| PageError::Closed),
        }
    }
}

impl NavigationTab for FrameTab {
    type Observation = NavigationObservation;

    async fn prepare(&self) -> Result<(), PageError> {
        PageSession::prepare(self)
            .await
            .map_err(|error| world::classify(&error))?;
        // Title's navigation fallback must not hide a detached direct frame as an empty title.
        if matches!(self.scope, FrameScope::Frame(_)) {
            self.context().await?;
        }
        Ok(())
    }

    async fn track_network(&self) -> Result<u64, PageError> {
        self.enable_network()
            .await
            .map_err(|error| match world::classify(&error) {
                PageError::Closed => PageError::Closed,
                _ => PageError::Contract(format!("Network.enable failed: {error}")),
            })?;
        Ok(self.network.lags())
    }

    async fn navigation_response(
        &self,
        document: &str,
        lags: u64,
    ) -> Result<Option<ResponseChannel>, PlaywrightError> {
        let lookup = self
            .network
            .wait(|log| {
                log.lookup(document, lags).map(|lookup| match lookup {
                    Lookup::Pending => None,
                    lookup => Some(lookup),
                })
            })
            .await
            .map_err(|error| PlaywrightError::error(error.to_string()))?;
        Ok(match lookup {
            Lookup::Response(hop) => Some(ResponseChannel {
                guid: hop.response_guid(&self.main_frame),
            }),
            Lookup::NoRequest | Lookup::Failed | Lookup::Pending => None,
        })
    }

    fn observe(&self) -> Result<NavigationObservation, PageError> {
        Ok(self.observe_navigations())
    }

    async fn navigate(
        &self,
        url: &str,
        referer: Option<&str>,
    ) -> Result<NavigateReply, PlaywrightError> {
        let mut params =
            json!({"url": url, "frameId": self.main_frame, "referrerPolicy": "unsafeUrl"});
        if let Some(referer) = referer {
            params["referrer"] = json!(referer);
        }
        let reply = self
            .send("Page.navigate", params)
            .await
            .map_err(|error| match error {
                CdpError::Protocol { message, .. } => {
                    PlaywrightError::error(format!("Protocol error (Page.navigate): {message}"))
                }
                error if world::classify(&error) == PageError::Closed => {
                    PlaywrightError::target_closed()
                }
                error => PlaywrightError::error(error.to_string()),
            })?;
        serde_json::from_value(reply)
            .map_err(|error| PlaywrightError::error(format!("Page.navigate answered: {error}")))
    }

    async fn read_title(&self) -> Result<String, PageError> {
        self.context()
            .await?
            .call(None, &InjectedTitleParams {})
            .await
            .map(|result| result.value)
    }
}

/// A fresh object group of `page`'s session, released when the guard drops.
fn new_group(page: &Arc<PageSession>, frame: &Arc<FrameContext>, purpose: &str) -> GroupGuard {
    let index = page.next_group.fetch_add(1, Ordering::Relaxed);
    GroupGuard {
        page: Arc::clone(page),
        frame: Arc::clone(frame),
        group: ObjectGroup(format!("neo-{purpose}-{index}")),
    }
}

// ---------------------------------------------------------------------------------------------
// The tab's input, one click at a time.
// ---------------------------------------------------------------------------------------------

/// A tab's input: its turn lock, and inside it the tab's keyboard and mouse
/// (`input_devices.rs`), so only a call holding the turn reaches them.
#[derive(Default)]
pub(crate) struct TabInput {
    /// The turn, granted in arrival order.
    turn: Arc<Mutex<()>>,
    /// The tab's devices: reached through a held turn ([`InputPermit`]), and by
    /// [`TabInput::mark_ended`] alone without one.
    devices: SyncMutex<InputDevices>,
}

impl TabInput {
    /// Neo: records that `owner`'s run ended (`Effect::Retire`) without waiting for the turn, so
    /// whatever it holds, now or once a write it started answers, counts as held by no live run
    /// however its release goes: a release that is refused, fails, or never gets the turn
    /// before the run's deadline. It writes nothing, and a held turn's planning sees it at its
    /// next snapshot.
    pub(crate) fn mark_ended(&self, owner: &Owner) {
        // A poisoned lock means a panic mid-update; Neo never reaches inside one.
        if let Ok(mut devices) = self.devices.lock() {
            devices.mark_ended(owner);
        }
    }
}

/// One `Runtime`'s input locks, one per tab, each shared by every run the `Runtime` makes on
/// that tab. A tab has one keyboard and one mouse, whose state lives in its lock, and its
/// document one hit-target slot that every call and every Neo run on it share (`world.rs`): a
/// setup replaces whatever interceptor is armed, and an event is judged by whichever interceptor
/// is armed when it lands. So two clicks whose input overlaps judge each other's events: one
/// click's events are blocked by the other's interceptor while its own, which never saw them,
/// reads `done`. Input to a tab therefore takes turns ([`InputPermit`]): a click from its
/// interceptor's setup until its stop has answered, mouse writes in between; a key or device
/// call from its first event to its last answer.
///
/// Entries are weak: a tab's lock, and the devices in it, live while a run on the tab, or a
/// task still holding a turn, does; the next run then starts from Playwright's new page. Runs of
/// another `Runtime`, and other CDP clients, are not coordinated.
#[derive(Clone, Default)]
pub(crate) struct TabInputs(Arc<Mutex<HashMap<PageId, Weak<TabInput>>>>);

impl TabInputs {
    /// The input lock of `page`, the one every live run on it shares.
    pub(crate) async fn lock_for(&self, page: &PageId) -> Arc<TabInput> {
        let mut locks = self.0.lock().await;
        if let Some(lock) = locks.get(page).and_then(Weak::upgrade) {
            return lock;
        }
        // Tabs whose runs and turns are all gone go too, so the map holds only live tabs.
        locks.retain(|_, lock| lock.strong_count() > 0);
        let lock = Arc::default();
        locks.insert(page.clone(), Arc::downgrade(&lock));
        lock
    }
}

/// A turn at a tab's input ([`TabInputs`]). Cheap to clone, and held the way the session is:
/// every party that operates a call's input holds a copy, and the tab's input is free for the
/// next call once the last copy drops. A click holds one from before its interceptor's setup
/// to its verdict; the setup's task until the page answers; the armed interceptor until its
/// stop has answered, `Drop`'s stop included; the input writer until its last event is
/// answered. So a call that stops early lets go of its own copy only, and the next click on the
/// tab never replaces an interceptor whose events may still land, nor presses into one still
/// armed. Its holder reaches the tab's devices through it ([`Devices`]), and the writer records
/// into them through it.
#[derive(Clone)]
pub(crate) struct InputPermit {
    turn: Arc<HeldTurn>,
}

/// The turn itself, shared by every copy of its [`InputPermit`]: whatever still writes for its
/// run holds a copy. Once the last goes, nothing of the turn writes any more, so an ended run
/// that lists no hold is forgotten before the next turn starts, and never within a turn.
struct HeldTurn {
    _guard: OwnedMutexGuard<()>,
    tab: Arc<TabInput>,
    /// The run whose session took the turn.
    owner: Owner,
}

impl Drop for HeldTurn {
    fn drop(&mut self) {
        // The guard drops after this body, so the next turn finds the ended runs forgotten.
        if let Ok(mut devices) = self.tab.devices.lock() {
            devices.forget_ended();
        }
    }
}

impl InputPermit {
    /// A turn at `tab`'s input, granted in arrival order. tokio's lock is cancel-safe: a wait
    /// dropped before its turn holds nothing.
    async fn take(tab: &Arc<TabInput>, owner: Owner) -> Self {
        let guard = Arc::clone(&tab.turn).lock_owned().await;
        Self {
            turn: Arc::new(HeldTurn {
                _guard: guard,
                tab: Arc::clone(tab),
                owner,
            }),
        }
    }
}

impl EffectSink for InputPermit {
    fn apply(&self, effect: &Effect, answered: bool) {
        // A poisoned lock means a panic mid-update; Neo never reaches inside one.
        if let Ok(mut devices) = self.turn.tab.devices.lock() {
            devices.apply(effect, answered);
        }
    }

    fn devices(&self) -> InputDevices {
        self.turn
            .tab
            .devices
            .lock()
            .map(|devices| devices.clone())
            .unwrap_or_default()
    }

    fn run_ended(&self) -> bool {
        self.turn
            .tab
            .devices
            .lock()
            .is_ok_and(|devices| devices.ended.contains(&self.turn.owner))
    }
}

impl Devices for InputPermit {
    fn snapshot(&self) -> Result<InputDevices, PageError> {
        self.turn
            .tab
            .devices
            .lock()
            .map(|devices| devices.clone())
            .map_err(|_| {
                PageError::Contract("neo-runtime: the tab's input state cannot be read".to_string())
            })
    }

    fn owner(&self) -> Owner {
        self.turn.owner.clone()
    }
}

// ---------------------------------------------------------------------------------------------
// The watcher.
// ---------------------------------------------------------------------------------------------

/// Playwright's attach source (`chromium/crPage.ts:500-502,689-694`): the same main-frame URL
/// and fragment as a commit event, including chrome-error URLs. An abandoned setup never
/// queries the page or completes its seed; its stop guard disposes the watcher instead.
async fn seed_url<F, Fut>(ready: oneshot::Receiver<()>, read: F) -> Result<Option<String>, CdpError>
where
    F: FnOnce(&'static str) -> Fut,
    Fut: Future<Output = Result<Value, CdpError>>,
{
    if ready.await.is_err() {
        return future::pending().await;
    }
    let reply = read("Page.getFrameTree").await?;
    // frameTree and Page.frameNavigated carry the same frame payload; child frames are ignored.
    let tree: FrameNavigated = serde_json::from_value(reply["frameTree"].clone())?;
    let frame = tree.frame;
    if frame.url.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!(
        "{}{}",
        frame.url,
        frame.url_fragment.as_deref().unwrap_or("")
    )))
}

/// The watcher task: one `cdp.events()` receiver for the run, filtered to this session (and to
/// `Target.detachedFromTarget` naming it). Answers dialogs with `Page.handleJavaScriptDialog`
/// on its own task, so no click await can block the answer; applies every event to `state`;
/// cancels `closed` when the session goes. On `RecvError::Lagged` it warns, forgets any pending
/// navigation (whose end it may have missed, and which would otherwise hold every later click
/// until its timeout), and sends one `Page.handleJavaScriptDialog {accept: false}` whose "No
/// dialog is showing" error is ignored, so a dialog event lost to the 4096-slot ring cannot
/// hang the run.
/// Lost commits and lifecycle events are not reconstructed; a navigation can time out after
/// overflow. A frame-tree resync would also need ordered lifecycle replay.
async fn watch_events(
    mut events: broadcast::Receiver<CdpEvent>,
    cdp: CdpClient,
    session: SessionId,
    main_frame: String,
    updates: PageUpdates<impl Future<Output = Result<Option<String>, CdpError>>>,
    closed: CancellationToken,
    stop: CancellationToken,
) {
    let PageUpdates {
        state,
        navigations,
        world,
        network,
        frames,
        feeds,
        initial_url,
        producer,
        idle,
        routes,
    } = updates;
    let (producer, mut flushes) = producer.unzip();
    let mut watcher = Watcher {
        cdp,
        session,
        main_frame,
        state,
        navigations,
        world,
        network,
        frames,
        feeds,
        producer,
        closed,
        idle,
        routes,
    };
    tokio::pin!(initial_url);
    let mut seed_pending = true;
    loop {
        let deadline = watcher
            .idle
            .as_ref()
            .and_then(|idle| idle.with(|idle| idle.deadline()).flatten());
        let event = tokio::select! {
            // The CDP reader publishes events and replies in socket order. Drain events
            // before applying the frame-tree reply, even if both became ready in one poll.
            biased;
            () = stop.cancelled() => return,
            event = events.recv() => event,
            // A reply barrier (`events.rs`): every event already in the ring preceded the reply
            // that asked for it, so apply them all before answering. Selected after the events,
            // so it never jumps ahead of one that was ready.
            Some(ack) = next_flush(&mut flushes) => {
                loop {
                    let drained = match events.try_recv() {
                        Ok(event) => Ok(event),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Lagged(skipped)) => Err(RecvError::Lagged(skipped)),
                        Err(TryRecvError::Closed) => Err(RecvError::Closed),
                    };
                    if !watcher.on(drained) {
                        return;
                    }
                }
                let _waiter = ack.send(());
                continue;
            }
            // A frame's quiet period ended: Playwright's network idle timer.
            () = sleep_until(deadline) => {
                watcher.idle_tick();
                continue;
            }
            () = moved(watcher.idle.as_deref()) => continue,
            seed = &mut initial_url, if seed_pending => {
                seed_pending = false;
                match seed {
                    Ok(Some(url)) => {
                        watcher.state.send_if_modified(|state| {
                            if state.committed_url.is_some() {
                                return false;
                            }
                            state.committed_url = Some(url);
                            true
                        });
                    }
                    Ok(None) => debug!(session = %watcher.session, "initial page URL empty; waiting for a commit"),
                    Err(error) => debug!(session = %watcher.session, %error, "initial page URL unavailable; waiting for a commit"),
                }
                continue;
            }
        };
        if !watcher.on(event) {
            return;
        }
    }
}

/// Sleeps until `deadline`; forever without one.
async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => time::sleep_until(deadline).await,
        None => future::pending().await,
    }
}

/// Resolves when the idle tracker's deadline moved from outside the watcher; never without one.
async fn moved(idle: Option<&SharedIdle>) {
    match idle {
        Some(idle) => idle.wait_moved().await,
        None => future::pending().await,
    }
}

/// Writes `networkidle` changes into the main frame's state and the child frames' feeds.
fn apply_idle(
    main_frame: &str,
    state: &watch::Sender<PageState>,
    feeds: Option<&FrameFeeds>,
    changes: Vec<IdleChange>,
) {
    for change in changes {
        let (frame, fired) = match &change {
            IdleChange::Fired(frame) => (frame, true),
            IdleChange::Lost(frame) => (frame, false),
        };
        if frame == main_frame {
            state.send_if_modified(|state| state.set_network_idle(fired));
        } else if let Some(feeds) = feeds {
            feeds.set_network_idle(frame, fired);
        }
    }
}

/// The hub's next flush for a watcher that has one; never for one that has none.
async fn next_flush(flushes: &mut Option<Flushes>) -> Option<oneshot::Sender<()>> {
    match flushes {
        Some(flushes) => flushes.recv().await,
        None => future::pending().await,
    }
}

/// What [`watch_events`] applies each event to.
struct Watcher {
    cdp: CdpClient,
    session: SessionId,
    main_frame: String,
    state: watch::Sender<PageState>,
    navigations: broadcast::Sender<Navigation>,
    world: Arc<World>,
    network: Arc<NetworkLog>,
    frames: Option<Arc<Frames>>,
    feeds: Option<Arc<FrameFeeds>>,
    producer: Option<PageEvents>,
    closed: CancellationToken,
    idle: Option<Arc<SharedIdle>>,
    routes: Option<Arc<Routes>>,
}

impl Watcher {
    /// Applies what `networkidle` makes of one event of the tab's session: its requests (the
    /// network log's `happenings`) and its frames' attach, detach, commit and lifecycle.
    /// `reopened`: the event was the `init` of a frame's document a `setContent` write reopened,
    /// which clears the frame here in socket order, before any request the written document
    /// starts.
    fn idle_event(&self, event: &CdpEvent, happenings: &[Happening], reopened: bool) {
        let Some(idle) = &self.idle else {
            return;
        };
        let now = Instant::now();
        let params = &event.params;
        let changes = idle
            .with(|idle| {
                for happening in happenings {
                    match happening {
                        Happening::Request(hop) => {
                            if let Some((frame, event_stream)) = self.network.idle_facts(hop) {
                                idle.started(hop.clone(), &frame, event_stream);
                            }
                        }
                        Happening::Finished(hop)
                        | Happening::Failed(hop)
                        | Happening::Forgotten(hop) => idle.ended(hop, now),
                        Happening::Response(_) => {}
                    }
                }
                match event.method.as_str() {
                    "Page.frameAttached" => {
                        if let (Some(frame), Some(parent)) =
                            (params["frameId"].as_str(), params["parentFrameId"].as_str())
                        {
                            idle.attach(frame, parent, now);
                        }
                        Vec::new()
                    }
                    "Page.frameDetached" => params["frameId"]
                        .as_str()
                        .map(|frame| idle.detach(frame))
                        .unwrap_or_default(),
                    "Page.frameNavigated" => {
                        let frame = &params["frame"];
                        match (frame["id"].as_str(), frame["loaderId"].as_str()) {
                            (Some(id), Some(loader)) => {
                                if let Some(parent) = frame["parentId"].as_str() {
                                    idle.attach(id, parent, now);
                                }
                                idle.commit(id, loader, now)
                            }
                            _ => Vec::new(),
                        }
                    }
                    "Page.lifecycleEvent" => {
                        let name = match params["name"].as_str() {
                            Some("DOMContentLoaded") => Some("domcontentloaded"),
                            Some("load") => Some("load"),
                            _ => None,
                        };
                        match (params["frameId"].as_str(), name) {
                            (Some(frame), Some(name)) => idle.lifecycle_event(frame, name),
                            // Playwright: `server/frames.ts` `Frame.setContent`: `this._onClearLifecycle();`
                            (Some(frame), None) if reopened && params["name"] == "init" => idle
                                .clear_lifecycle(
                                    frame,
                                    params["loaderId"].as_str().unwrap_or_default(),
                                    now,
                                ),
                            _ => Vec::new(),
                        }
                    }
                    _ => Vec::new(),
                }
            })
            .unwrap_or_default();
        self.apply_idle(changes);
    }

    /// The tab's routes' share of one event (`routes.rs`): a pause of the tab's session or of an
    /// attached frame session's; a `requestWillBeSent` a parked pause waited for, its `request`
    /// event just gone, so its `route` event follows it; a frame session gone with its pauses.
    fn route_event(&self, event: &CdpEvent) {
        let Some(routes) = &self.routes else {
            return;
        };
        let from = || match event.session_id.as_ref() {
            Some(from) if from == &self.session => Some(None),
            Some(from) if self.network.has_child(from) => Some(Some(from)),
            _ => None,
        };
        match event.method.as_str() {
            "Fetch.requestPaused" => {
                if let Some(from) = from() {
                    let pause = routes.paused(from, &event.params, &self.network);
                    self.apply_pause(pause);
                }
            }
            "Network.requestWillBeSent" => {
                if from().is_some()
                    && let Some(request) = event.params["requestId"].as_str()
                    && let Some(pause) = routes.unpark(request, &self.network)
                {
                    self.apply_pause(pause);
                }
            }
            "Target.detachedFromTarget" => {
                if let Some(child) = event.params["sessionId"].as_str() {
                    routes.forget_session(&SessionId::from(child));
                }
            }
            // A fetch waits on its request: a failure on the network ends it.
            "Network.loadingFailed" => {
                if from().is_some()
                    && let Some(request) = event.params["requestId"].as_str()
                {
                    let error = event.params["errorText"].as_str().unwrap_or_default();
                    routes.fetch_failed(request, error);
                }
            }
            _ => {}
        }
    }

    /// Carries out what the tab's routes made of a pause: an answer at once, nothing yet, or the
    /// run's `route` event, whose pause goes on untouched when nobody takes it.
    fn apply_pause(&self, pause: Pause) {
        match pause {
            Pause::Answer(answer) => send_answer(&self.cdp, &self.session, answer),
            Pause::Parked => {}
            Pause::Route {
                index,
                channel,
                params,
            } => {
                let Some(routes) = &self.routes else {
                    return;
                };
                if self
                    .producer
                    .as_ref()
                    .is_some_and(|producer| producer.route(channel, &params))
                {
                    routes.emitted(index);
                } else if let Some(answer) = routes.release(index) {
                    send_answer(&self.cdp, &self.session, answer);
                }
            }
        }
    }

    /// `Frame._failInflightRequests` for a frame the tab's session reports removed
    /// (`Page.frameDetached`, reason `remove`), and for the requests of an out-of-process frame
    /// session that detached (`Target.detachedFromTarget`). A frame whose document moved to
    /// another process (reason `swap`) is noted for its later removal.
    fn detached_requests(&self, event: &CdpEvent) -> Vec<Happening> {
        match event.method.as_str() {
            "Page.frameDetached" if event.session_id.as_ref() == Some(&self.session) => {
                let Some(frame) = event.params["frameId"].as_str() else {
                    return Vec::new();
                };
                match event.params["reason"].as_str() {
                    Some("remove") => self.network.detach_frame(frame),
                    Some("swap") => {
                        self.network.frame_moved(frame);
                        Vec::new()
                    }
                    _ => Vec::new(),
                }
            }
            "Target.detachedFromTarget" => event.params["sessionId"]
                .as_str()
                .map(SessionId::from)
                .filter(|child| self.network.has_child(child))
                .map(|child| self.network.detach_session(&child))
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// A child frame's navigation request starting or failing, routed to that frame's feed: the
    /// start names its frame, the failure only its request, whose frame the network log knows.
    /// `recorded`: the event came from a session the network log records.
    fn child_navigation_request(&self, feeds: &FrameFeeds, event: &CdpEvent, recorded: bool) {
        if !recorded {
            return;
        }
        let frame = match event.method.as_str() {
            "Network.requestWillBeSent" => event.params["frameId"].as_str().map(str::to_string),
            "Network.loadingFailed" => event.params["requestId"]
                .as_str()
                .and_then(|request| self.network.frame_of(request)),
            _ => None,
        };
        if let Some(frame) = frame.filter(|frame| *frame != self.main_frame) {
            feeds.apply_network(event, &frame);
        }
    }

    /// A network idle timer fired.
    fn idle_tick(&self) {
        let changes = self
            .idle
            .as_ref()
            .and_then(|idle| idle.with(|idle| idle.tick(Instant::now())))
            .unwrap_or_default();
        self.apply_idle(changes);
    }

    fn apply_idle(&self, changes: Vec<IdleChange>) {
        apply_idle(
            &self.main_frame,
            &self.state,
            self.feeds.as_deref(),
            changes,
        );
    }

    /// Applies one event of the tab's session to the main frame's state: what it did, and
    /// whether it reopened the main frame's document for a `setContent` write.
    fn apply_state(&self, event: &CdpEvent) -> (Applied, bool) {
        let mut applied = Applied::default();
        let mut reopened = false;
        let world = &self.world;
        let main_frame = &self.main_frame;
        let session = &self.session;
        self.state.send_if_modified(|state| {
            let reopens = state.reopens;
            applied = apply(state, event, session, main_frame);
            reopened = state.reopens > reopens;
            if applied.new_document {
                // Before notifying readers: no in-flight read may return old-document data.
                world.document_committed();
            }
            applied.changed
        });
        (applied, reopened)
    }

    /// Applies one event, or the ring's lag or end. Returns `false` once the session is gone.
    fn on(&mut self, event: Result<CdpEvent, RecvError>) -> bool {
        let session = &self.session;
        match event {
            Ok(event) => {
                // Before the Page events that follow it, so a commit never outruns its request.
                // An out-of-process frame session's requests are the tab's once the page enabled
                // its Network domain (`enable_child_network`).
                let mut happenings = Vec::new();
                let mut recorded = false;
                if event.method.starts_with("Network.") {
                    let from = match event.session_id.as_ref() {
                        Some(from) if from == session => Some(None),
                        Some(from) if self.network.has_child(from) => Some(Some(from)),
                        _ => None,
                    };
                    if let Some(from) = from {
                        recorded = true;
                        happenings = self.network.apply(from, &event.method, &event.params);
                        if let Some(producer) = &self.producer {
                            producer.network(&self.network, &happenings);
                        }
                    }
                }
                self.route_event(&event);
                // A frame removed from the page, or an out-of-process frame session gone, ends
                // its requests still loading: no later event of theirs can arrive.
                let detached = self.detached_requests(&event);
                if !detached.is_empty() {
                    if let Some(producer) = &self.producer {
                        producer.network(&self.network, &detached);
                    }
                    happenings.extend(detached);
                }
                if event.method == "Page.frameNavigated"
                    && event.session_id.as_ref() == Some(session)
                    && let Some(frame) = event.params["frame"]["id"].as_str()
                    && frame != self.main_frame
                    && let Some(frames) = &self.frames
                {
                    frames.committed(frame);
                }
                let (applied, mut reopened) = self.apply_state(&event);
                if let Some(navigation) = applied.navigation.clone() {
                    let _listeners = self.navigations.send(navigation);
                }
                if let Some(feeds) = &self.feeds {
                    if applied.new_document {
                        feeds.detach_all();
                    }
                    reopened |= feeds.apply(&event, session, Some(&self.main_frame));
                    self.child_navigation_request(feeds, &event, recorded);
                }
                // After the frames' state, so a commit's cleared lifecycle comes first. Only the
                // tab's own session reports the frames `networkidle` follows (`network_idle.rs`).
                if event.session_id.as_ref() == Some(session) {
                    self.idle_event(&event, &happenings, reopened);
                }
                // The run's events from this one, after the page state they may describe. A
                // dialog a listener owns is the script's to answer.
                let answer = match &self.producer {
                    Some(producer) => producer.apply(&event, &applied),
                    None => applied.dialog,
                };
                if let Some(answer) = answer {
                    answer_dialog(&self.cdp, session, answer);
                }
                if applied.closed {
                    if let Some(producer) = &self.producer {
                        producer.closing(Some(&event));
                    }
                    self.closed.cancel();
                    return false;
                }
            }
            Err(RecvError::Lagged(skipped)) => {
                warn!(%session, skipped, "the run's event watcher lagged; answering any dialog");
                self.network.lagged();
                // A lost end would hold `networkidle` forever: the requests seen so far are
                // forgotten, and every frame's quiet period starts again.
                let changes = self
                    .idle
                    .as_ref()
                    .and_then(|idle| idle.with(|idle| idle.forget_requests(Instant::now())))
                    .unwrap_or_default();
                self.apply_idle(changes);
                self.state
                    .send_if_modified(|state| state.pending_url.take().is_some());
                if let Some(producer) = &self.producer {
                    producer.lagged();
                }
                answer_dialog(&self.cdp, session, DialogAnswer::Dismiss);
            }
            Err(RecvError::Closed) => {
                if let Some(producer) = &self.producer {
                    producer.closing(None);
                }
                self.closed.cancel();
                return false;
            }
        }
        true
    }
}

/// Neo: how long an out-of-process frame's first call waits for that frame's `Network.enable`. A
/// frame frozen by its page's dialog may never answer; the call then goes on without it.
const CHILD_NETWORK_WAIT: Duration = Duration::from_secs(5);

/// `Network.enable` on an out-of-process frame's session, whose requests are the tab's from then
/// on: the session is registered first, so no event the enable triggers is dropped. Bounded by
/// [`CHILD_NETWORK_WAIT`]; a failure is logged, and the frame's requests stay unseen.
async fn enable_child_network(network: &NetworkLog, host: &Arc<Host>) {
    let session = host.session_id().clone();
    if network.has_child(&session) {
        return;
    }
    network.add_child(&session);
    match time::timeout(CHILD_NETWORK_WAIT, host.send("Network.enable", json!({}))).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => debug!(%session, %error, "a frame's network did not turn on"),
        Err(_) => debug!(%session, "a frame's network did not turn on in time"),
    }
}

/// Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._updateProtocolRequestInterceptionForSession`: `fetchPromise = info.session.send('Fetch.enable', { handleAuthRequests: true, patterns: [{ urlPattern: '*', requestStage: 'Request' }] });`
///
/// Deviation: `server/chromium/crNetworkManager.ts` `CRNetworkManager._onAuthRequired`: `authChallengeResponse: { response, username, password },`
/// Neo pauses requests without taking their auth challenges (`handleAuthRequests: false`):
/// with no credentials Playwright answers `Default`, which is Chromium's own handling.
fn interception(on: bool) -> [(&'static str, Value); 2] {
    let fetch = if on {
        (
            "Fetch.enable",
            json!({ "handleAuthRequests": false, "patterns": [{ "urlPattern": "*", "requestStage": "Request" }] }),
        )
    } else {
        ("Fetch.disable", json!({}))
    };
    [
        // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._updateProtocolRequestInterceptionForSession`: `const cachePromise = info.session.send('Network.setCacheDisabled', { cacheDisabled: enabled });`
        ("Network.setCacheDisabled", json!({ "cacheDisabled": on })),
        fetch,
    ]
}

/// Sends a pause's answer without waiting for it (`_sendMayFail`): the request may already be
/// gone, which only the page sees.
fn send_answer(cdp: &CdpClient, tab: &SessionId, answer: Answer) {
    let cdp = cdp.clone();
    let session = answer.session.unwrap_or_else(|| tab.clone());
    tokio::spawn(async move {
        if let Err(error) = cdp.send(answer.method, answer.params, Some(&session)).await {
            debug!(%session, %error, "answering a paused request failed");
        }
    });
}

/// Sends a dialog answer without waiting for it: the renderer is blocked until it lands, and
/// the watcher must keep reading events meanwhile.
fn answer_dialog(cdp: &CdpClient, session: &SessionId, answer: DialogAnswer) {
    let cdp = cdp.clone();
    let session = session.clone();
    tokio::spawn(async move {
        let accept = answer == DialogAnswer::Accept;
        let answered = cdp
            .send(
                "Page.handleJavaScriptDialog",
                json!({ "accept": accept }),
                Some(&session),
            )
            .await;
        if let Err(error) = answered {
            debug!(%session, %error, "answering a dialog failed");
        }
    });
}

/// An OOPIF host's navigation watcher: one `cdp.events()` receiver filtered to the host's
/// session, every frame event applied to `feeds`, until the session detaches or the page goes
/// (`stop`). Its setup (`Page.enable`, lifecycle events, frame tree) runs inside the same loop,
/// with events drained first, so the URL seed lands after every event that preceded its reply
/// and never overwrites a commit. Dialogs are the tab's: Chromium reports an OOPIF's dialog on
/// the tab session only (spike Q12). Holds the host, so its session stays attached while it
/// runs; holds no page, so the page's drop stops it.
async fn watch_host_frames(
    mut events: broadcast::Receiver<CdpEvent>,
    host: Arc<Host>,
    feeds: Arc<FrameFeeds>,
    seeded: oneshot::Sender<Result<(), PageError>>,
    stop: CancellationToken,
) {
    let session = host.session_id().clone();
    let setup = async {
        host.send("Page.enable", json!({})).await?;
        host.send("Page.setLifecycleEventsEnabled", json!({ "enabled": true }))
            .await?;
        host.send("Page.getFrameTree", json!({})).await
    };
    tokio::pin!(setup);
    let mut seeded = Some(seeded);
    loop {
        tokio::select! {
            biased;
            () = stop.cancelled() => return,
            event = events.recv() => match event {
                Ok(event) => {
                    if event.method == "Target.detachedFromTarget"
                        && event.params["sessionId"].as_str() == Some(session.as_str())
                    {
                        return;
                    }
                    feeds.apply(&event, &session, None);
                }
                Err(RecvError::Lagged(skipped)) => {
                    warn!(%session, skipped, "a frame host's event watcher lagged");
                }
                Err(RecvError::Closed) => return,
            },
            tree = &mut setup, if seeded.is_some() => {
                let seed = tree
                    .map_err(|error| host.classify(&error))
                    .and_then(|tree| local_tree(&tree));
                let result = seed.map(|frames| {
                    for frame in frames {
                        feeds.seed(&frame.id, &frame.url);
                    }
                });
                let failed = result.is_err();
                if let Some(seeded) = seeded.take() {
                    let _waiter = seeded.send(result);
                }
                if failed {
                    return;
                }
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameRequestedNavigation {
    frame_id: String,
    url: String,
    disposition: String,
}

#[derive(Deserialize)]
struct FrameNavigated {
    frame: NavigatedFrame,
    #[serde(rename = "type", default)]
    kind: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NavigatedFrame {
    id: String,
    url: String,
    #[serde(default)]
    url_fragment: Option<String>,
    #[serde(default)]
    loader_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifecycleEvent {
    frame_id: String,
    loader_id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameEvent {
    frame_id: String,
    #[serde(default)]
    url: Option<String>,
}

/// The fields of `Network.requestWillBeSent` that tell a navigation request.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NavigationRequestStarted {
    request_id: String,
    loader_id: String,
    #[serde(default)]
    frame_id: Option<String>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    request: StartedRequest,
}

#[derive(Deserialize)]
struct StartedRequest {
    url: String,
}

/// The fields of `Network.loadingFailed` a navigation's failure needs.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NavigationRequestFailed {
    request_id: String,
    #[serde(default)]
    error_text: Option<String>,
    #[serde(default)]
    blocked_reason: Option<String>,
    #[serde(default)]
    canceled: bool,
}

impl NavigationRequestFailed {
    /// The text a navigation wait rejects with.
    ///
    /// Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._onLoadingFailed`: `request.request._setFailureText(event.errorText || event.blockedReason || '');`
    /// Playwright: `server/frames.ts` `FrameManager.requestFailed`: `errorText += '; maybe frame was detached?';`
    fn error_text(&self) -> String {
        let mut text = [&self.error_text, &self.blocked_reason]
            .into_iter()
            .flatten()
            .find(|text| !text.is_empty())
            .cloned()
            .unwrap_or_default();
        if self.canceled {
            text.push_str("; maybe frame was detached?");
        }
        text
    }
}

#[derive(Deserialize)]
struct DialogOpening {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DetachedFromTarget {
    #[serde(default)]
    session_id: Option<String>,
}

/// Applies one event to the page state. Pure. Event shapes are local serde structs:
/// browseros-cdp's typed surface lacks most of them, and adding them would change a shared
/// crate that core also compiles.
pub(crate) fn apply(
    state: &mut PageState,
    event: &CdpEvent,
    session: &SessionId,
    main_frame: &str,
) -> Applied {
    let mut applied = Applied::default();
    if event.method == "Target.detachedFromTarget" {
        let detached = serde_json::from_value::<DetachedFromTarget>(event.params.clone());
        applied.closed =
            detached.is_ok_and(|detached| detached.session_id.as_deref() == Some(session.as_str()));
        return applied;
    }
    if event.session_id.as_ref() != Some(session) {
        return applied;
    }
    match event.method.as_str() {
        "Page.javascriptDialogOpening" => {
            let beforeunload = serde_json::from_value::<DialogOpening>(event.params.clone())
                .is_ok_and(|dialog| dialog.kind == "beforeunload");
            state.dialogs_opened += 1;
            applied.changed = true;
            applied.dialog = Some(if beforeunload {
                DialogAnswer::Accept
            } else {
                DialogAnswer::Dismiss
            });
        }
        "Inspector.detached" | "Inspector.targetCrashed" => applied.closed = true,
        _ => applied = apply_frame(state, event, main_frame),
    }
    applied
}

/// The part of [`apply`] that follows one frame, `frame`: its requested navigations, commits,
/// same-document navigations, stops and lifecycle, and, for the main frame, its navigation
/// requests' start and failure (Network events name no frame, so a child frame's feed never
/// receives them). The main frame's state and every child frame's feed ([`FrameFeeds`]) both
/// take it, so a child frame's document is followed exactly as the main frame's is. The caller
/// has already matched the event's session.
fn apply_frame(state: &mut PageState, event: &CdpEvent, main_frame: &str) -> Applied {
    if matches!(
        event.method.as_str(),
        "Network.requestWillBeSent" | "Network.loadingFailed"
    ) {
        return apply_navigation_request(state, event, main_frame);
    }
    let mut applied = Applied::default();
    let params = event.params.clone();
    match event.method.as_str() {
        "Page.frameRequestedNavigation" => {
            if let Ok(requested) = serde_json::from_value::<FrameRequestedNavigation>(params)
                && requested.frame_id == main_frame
                && requested.disposition == "currentTab"
            {
                state.navigations_requested += 1;
                state.pending_url = Some(requested.url);
                // server/frames.ts frameRequestedNavigation: a navigation whose request is not
                // known yet replaces any pending one.
                state.pending_document = Some(PendingDocument::Requested);
                applied.changed = true;
            }
        }
        "Page.frameNavigated" => {
            if let Ok(FrameNavigated { frame, kind }) = serde_json::from_value(params)
                && frame.id == main_frame
            {
                state.commits += 1;
                let fragment = frame.url_fragment.as_deref().unwrap_or("");
                state.committed_url = Some(format!("{}{fragment}", frame.url));
                state.document = frame.loader_id;
                // server/frames.ts frameCommittedNewDocumentNavigation: the pending document is
                // this one, with its request, or, with no request known, assumed to be; a later
                // navigation whose request already started stays pending.
                let request = match state.pending_document.take() {
                    Some(PendingDocument::Started(pending))
                        if state.document.as_ref() == Some(&pending) =>
                    {
                        true
                    }
                    Some(PendingDocument::Started(later)) => {
                        state.pending_document = Some(PendingDocument::Started(later));
                        false
                    }
                    Some(PendingDocument::Requested) | None => false,
                };
                state.clear_lifecycle();
                if kind.as_deref() == Some("BackForwardCacheRestore") {
                    // Deviation: `server/chromium/chromiumSwitches.ts` `chromiumSwitches`: `'--disable-back-forward-cache',`
                    // Playwright's Chromium never restores a cached document, and one it met
                    // would never fire `load`, so its history navigation would time out. Neo
                    // runs in the user's browser, where Chromium caches only a document that
                    // finished loading and restores it without lifecycle events, reporting
                    // `frameStoppedLoading` before this commit (measured on BrowserOS neo): it
                    // counts as loaded at once.
                    applied.fired = state.loaded_at_once();
                }
                state.pending_url = None;
                applied.new_document = true;
                applied.navigation = Some(Navigation {
                    document: state.document.clone(),
                    url: state.committed_url.clone().unwrap_or_default(),
                    lifecycle: state.lifecycle(),
                    request,
                    error: None,
                });
                applied.changed = true;
            }
        }
        "Page.navigatedWithinDocument" => {
            if let Ok(FrameEvent {
                frame_id,
                url: Some(url),
            }) = serde_json::from_value(params)
                && frame_id == main_frame
            {
                state.commits += 1;
                state.committed_url = Some(url.clone());
                state.pending_url = None;
                // server/frames.ts frameCommittedSameDocumentNavigation.
                if state.pending_document == Some(PendingDocument::Requested) {
                    state.pending_document = None;
                }
                applied.navigation = Some(Navigation {
                    document: None,
                    url,
                    lifecycle: state.lifecycle(),
                    request: false,
                    error: None,
                });
                applied.changed = true;
            }
        }
        "Page.frameStoppedLoading" => {
            if let Ok(FrameEvent { frame_id, .. }) = serde_json::from_value(params)
                && frame_id == main_frame
            {
                state.pending_url = None;
                applied.changed = true;
            }
        }
        "Page.lifecycleEvent" => {
            if let Ok(event) = serde_json::from_value::<LifecycleEvent>(params)
                && event.frame_id == main_frame
            {
                let before = (state.dom_content_loaded, state.load);
                applied.changed = apply_lifecycle(state, &event);
                applied.fired = Fired::between(before, state);
            }
        }
        _ => {}
    }
    applied
}

/// The main frame's navigation requests, for [`apply_frame`]: one starting is the pending
/// document; the pending document's request failing aborts its navigation, as a navigation event
/// with its error.
fn apply_navigation_request(state: &mut PageState, event: &CdpEvent, main_frame: &str) -> Applied {
    let mut applied = Applied::default();
    // Read in place: every subresource request passes through here.
    let params = &event.params;
    if event.method == "Network.requestWillBeSent" {
        // server/frames.ts requestStarted, for crNetworkManager.ts's navigation requests:
        // `requestId === loaderId`, type `Document`, and never a `data:` URL.
        if let Ok(started) = NavigationRequestStarted::deserialize(params)
            && started.frame_id.as_deref() == Some(main_frame)
            && started.request_id == started.loader_id
            && started.kind.as_deref() == Some("Document")
            && !started.request.url.starts_with("data:")
        {
            state.pending_document = Some(PendingDocument::Started(started.request_id));
            applied.changed = true;
        }
        return applied;
    }
    // server/frames.ts requestFailed: only the pending document's request aborts the navigation;
    // an older one that a newer navigation replaced fails silently.
    if let Ok(failed) = NavigationRequestFailed::deserialize(params)
        && state.pending_document == Some(PendingDocument::Started(failed.request_id.clone()))
    {
        state.pending_document = None;
        applied.navigation = Some(Navigation {
            url: state.committed_url.clone().unwrap_or_default(),
            lifecycle: state.lifecycle(),
            request: false,
            error: Some(failed.error_text()),
            document: Some(failed.request_id),
        });
        applied.changed = true;
    }
    applied
}

/// Neo: one write's expected reopen, counted in or given back. Answers the count of reopens
/// ([`PageState::reopens`]) once that write's is in, the expected ones going in order.
fn expect(state: &mut PageState, expected: bool) -> u64 {
    state.reopens_expected = if expected {
        state.reopens_expected.saturating_add(1)
    } else {
        state.reopens_expected.saturating_sub(1)
    };
    state.reopens + u64::from(state.reopens_expected)
}

fn apply_lifecycle(state: &mut PageState, event: &LifecycleEvent) -> bool {
    let seeded = state.document.is_none();
    if seeded {
        state.document = Some(event.loader_id.clone());
    }
    if state.document.as_deref() != Some(&event.loader_id) {
        return seeded;
    }
    match event.name.as_str() {
        "DOMContentLoaded" => state.did_dom_content_load(),
        "load" => state.did_load(),
        // Playwright: `server/frames.ts` `Frame.setContent`: `this._onClearLifecycle();`
        "init" if state.reopens_expected > 0 => state.reopened(),
        _ => return seeded,
    }
    true
}

// ---------------------------------------------------------------------------------------------
// Handles that clean up after themselves.
// ---------------------------------------------------------------------------------------------

/// An object group this session holds. Dropping it releases the group
/// (`Runtime.releaseObjectGroup`) on a tracked task holding the page, fire and forget: a
/// renderer stuck on a dialog must not hold the dropping call. One release frees the result
/// object and every handle `getProperties` read out of it (probe P1). A tab that closed or
/// crashed took its groups with it, so no release waits on it
/// ([`PageSession::spawn_while_open`]).
pub(crate) struct GroupGuard {
    page: Arc<PageSession>,
    frame: Arc<FrameContext>,
    group: ObjectGroup,
}

impl GroupGuard {
    pub(crate) fn group(&self) -> &ObjectGroup {
        &self.group
    }
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        let frame = Arc::clone(&self.frame);
        let ObjectGroup(group) = self.group.clone();
        self.page.spawn_while_open(async move {
            let released = frame
                .host
                .send(
                    "Runtime.releaseObjectGroup",
                    json!({ "objectGroup": group }),
                )
                .await;
            if let Err(error) = released {
                debug!(%error, "releasing an object group failed");
            }
        });
    }
}

impl Drop for PageSession {
    fn drop(&mut self) {
        // A run whose future is dropped never reaches its run end: its end is recorded here,
        // once nothing of the run writes any more, so what it holds is held by no live run.
        self.mark_input_ended();
    }
}

/// `click::Tab::Element` for the real tab: one element and the group of the resolution that
/// produced it. The handle dies with the group.
#[derive(Clone)]
pub(crate) struct ResolvedElement {
    handle: Handle<Element>,
    _group: Arc<GroupGuard>,
    pub(crate) frame: Arc<FrameContext>,
    pub(crate) frame_visible: bool,
    owners: Vec<FrameOwner>,
}

impl ResolvedElement {
    /// A typed handle; this element's shared group keeps it alive through a detached page step.
    pub(crate) fn handle(&self) -> &Handle<Element> {
        &self.handle
    }
}

/// `click::Tab::Armed` for the real tab. The interceptor handle lives in its own object group,
/// separate from the element's, so its stop never races the element group's release: the
/// interceptor's JS closure keeps its own reference to the element.
///
/// Releasing the group does not disarm: the listener stays in the document's injected script,
/// shared with the user and with later runs, until a stop names it or another setup replaces
/// it. So this value is the only way to stop it, and it exists from the setup's answer on,
/// whether or not the click still waits (`arm_interceptor` on the run's tracker). Its owner
/// follows the input (`click::Tab::Armed`), and its `Drop` sends the stop.
pub(crate) struct ArmedInterceptor {
    handle: Handle<HitTargetInterceptor>,
    group: Option<GroupGuard>,
    page: Arc<PageSession>,
    frame: Arc<FrameContext>,
    /// The counters when it was armed, for the verdict's dialog and navigation race.
    mark: PageMark,
    /// Set once `stopHitTargetInterceptor` has answered, so `Drop` does not send it again.
    stopped: bool,
    /// The click's turn at the tab's input, held until the stop has answered: until then the
    /// slot may still hold this interceptor, and a forced click, which arms nothing of its own,
    /// would press into it.
    input: Option<InputPermit>,
}

impl Drop for ArmedInterceptor {
    fn drop(&mut self) {
        let group = self.group.take();
        let input = self.input.take();
        if self.stopped {
            return;
        }
        // Playwright's `finally` sends the same fire-and-forget stop. The stop goes before the
        // group release, in one task, so the handle is still alive when the stop names it.
        let frame = Arc::clone(&self.frame);
        let interceptor = self.handle.clone();
        self.page.spawn(async move {
            let stopped = frame
                .call(
                    None,
                    &InjectedStopHitTargetInterceptorParams { interceptor },
                )
                .await;
            if let Err(error) = stopped {
                debug!(?error, "disarming a hit-target interceptor failed");
            }
            // The slot no longer holds it (or its document is gone), so the next click may
            // take its turn.
            drop(input);
            drop(group);
        });
    }
}

/// Navigation events of this session, from a receiver subscribed before a click's input.
pub(crate) struct NavigationWatch {
    events: broadcast::Receiver<CdpEvent>,
}

/// What a click's own navigations came to while draining a [`NavigationWatch`]: the same
/// [`apply`] the watcher runs, on a state of its own that starts at the input.
#[derive(Default)]
struct Barrier {
    state: PageState,
    /// Requested main-frame navigations not yet settled: Playwright's barrier retains once per
    /// request, and one navigation event releases every waiter at once.
    waiting: usize,
    /// One URL per waiter a commit released, as each Playwright waiter logs its own line.
    committed: Vec<String>,
}

impl Barrier {
    fn apply(&mut self, event: &CdpEvent, session: &SessionId, main_frame: &str) {
        let before = (self.state.navigations_requested, self.state.commits);
        let was_pending = self.state.pending_url.is_some();
        if !apply(&mut self.state, event, session, main_frame).changed {
            return;
        }
        if self.state.navigations_requested > before.0 {
            self.waiting += 1;
        } else if self.state.commits > before.1 {
            let url = self.state.committed_url.clone().unwrap_or_default();
            self.committed.extend(iter::repeat_n(url, self.waiting));
            self.waiting = 0;
        } else if was_pending && self.state.pending_url.is_none() {
            // `frameStoppedLoading`: a request that never commits (a 204, a download).
            self.waiting = 0;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The tab, for click.
// ---------------------------------------------------------------------------------------------

impl Tab for FrameTab {
    type Element = ResolvedElement;
    type Armed = ArmedInterceptor;
    type Input = InputPermit;
    type Navigations = NavigationWatch;

    async fn prepare(&self) -> Result<(), PageError> {
        PageSession::prepare(self)
            .await
            .map_err(|error| world::classify(&error))
    }

    async fn describe(&self, selector: &str) -> Result<SelectorFacts, PageError> {
        let invalid = frame_selector::split(selector).err();
        if let Some(message) = &invalid
            && message.starts_with("Neo ")
        {
            return Err(PageError::Threw(message.clone()));
        }
        let frame = self.context().await?;
        let described = frame
            .call(
                None,
                &InjectedDescribeSelectorParams {
                    selector: selector.to_string(),
                },
            )
            .await?;
        Ok(SelectorFacts {
            locator: described.locator,
            invalid: invalid.or(described.invalid_selector),
        })
    }

    async fn resolve(
        &self,
        selector: &str,
        strict: bool,
    ) -> Result<Option<Resolved<ResolvedElement>>, PageError> {
        let Some(located) = frame_selector::locate(self, selector, strict).await? else {
            return Ok(None);
        };
        let group = self.group(&located.frame, "resolve");
        let resolved = located
            .frame
            .call(
                Some(group.group()),
                &InjectedResolveParams {
                    selector: located.selector,
                    strict,
                },
            )
            .await?;
        Ok(resolved.element.map(|handle| Resolved {
            element: ResolvedElement {
                handle,
                _group: Arc::new(group),
                frame: located.frame,
                frame_visible: located.frame_visible,
                owners: located.owners,
            },
            count: u32::try_from(resolved.count).unwrap_or(u32::MAX),
            preview: resolved.preview.unwrap_or_default(),
        }))
    }

    async fn check_states(
        &self,
        element: &ResolvedElement,
        states: &[ElementState],
        scroll: bool,
    ) -> Result<StatesCheck, PageError> {
        // An offscreen OOPIF can throttle animation frames indefinitely. Bring its owners
        // into view before a stability check waits on that frame's animation callbacks, as
        // Playwright's best-effort scroll of an element in a child frame does; never under
        // `scroll: 'none'`, where Playwright scrolls nothing and the frame stays offscreen.
        // Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `if (scroll === 'none')`
        if scroll && states.contains(&ElementState::Stable) {
            for owner in &element.owners {
                owner
                    .frame
                    .host
                    .send(
                        "DOM.scrollIntoViewIfNeeded",
                        json!({"objectId":owner.element.id().object_id()}),
                    )
                    .await
                    .map_err(|error| owner.frame.host.classify(&error))?;
            }
        }
        let checked = element
            .frame
            .call(
                None,
                &InjectedCheckElementStatesParams {
                    element: element.handle.clone(),
                    states: states.to_vec(),
                    frame_visible: element.frame_visible,
                },
            )
            .await?;
        Ok(match checked.outcome {
            None => StatesCheck::Ready,
            Some(ElementStatesOutcome::NotConnected(_)) => StatesCheck::NotConnected,
            Some(ElementStatesOutcome::Missing(missing)) => {
                StatesCheck::Missing(missing.missing_state)
            }
        })
    }

    /// `Protocol`: `DOM.scrollIntoViewIfNeeded {objectId, rect?}`; "Node does not have a layout
    /// object" is `NotVisible` and "Node is detached from document" `NotConnected`, the one
    /// place Playwright itself reads CDP error text (`crPage.ts`
    /// `_scrollRectIntoViewIfNeeded`). Other modes: the `scrollIntoView` glue with `block` and
    /// `inline` both the mode.
    async fn scroll(
        &self,
        element: &ResolvedElement,
        scroll: Scroll,
        position: Option<&Point>,
    ) -> Result<Option<NotReady>, PageError> {
        let alignment = match scroll {
            Scroll::Protocol => {
                let params = dom::ScrollIntoViewIfNeededParams {
                    object_id: Some(element.handle.id().object_id().to_string()),
                    rect: position.map(|position| dom::Rect {
                        x: position.x,
                        y: position.y,
                        width: 0.0,
                        height: 0.0,
                    }),
                    ..dom::ScrollIntoViewIfNeededParams::default()
                };
                let scrolled = element
                    .frame
                    .host
                    .send(
                        "DOM.scrollIntoViewIfNeeded",
                        serde_json::to_value(params)
                            .map_err(|error| PageError::Contract(error.to_string()))?,
                    )
                    .await;
                return match scrolled {
                    Ok(_) => Ok(None),
                    Err(CdpError::Protocol { message, .. })
                        if message.contains("Node does not have a layout object") =>
                    {
                        Ok(Some(NotReady::NotVisible))
                    }
                    Err(CdpError::Protocol { message, .. })
                        if message.contains("Node is detached from document") =>
                    {
                        Ok(Some(NotReady::NotConnected))
                    }
                    Err(error) => Err(element.frame.host.classify(&error)),
                };
            }
            Scroll::End => ScrollAlignment::End,
            Scroll::Center => ScrollAlignment::Center,
            Scroll::Start => ScrollAlignment::Start,
        };
        element
            .frame
            .call(
                None,
                &InjectedScrollIntoViewParams {
                    element: element.handle.clone(),
                    block: alignment,
                    inline: alignment,
                },
            )
            .await?;
        Ok(None)
    }

    /// `DOM.getContentQuads {objectId}` together with the `viewportSize` glue (Playwright's
    /// `Promise.all`), or `DOM.getBoxModel {objectId}` with `getElementBorderWidth` for
    /// `position`. A failed quads, box or border read is `Geometry::NoQuads`, as Playwright's
    /// `_sendMayFail` and `.catch` make it, unless the session is gone.
    async fn geometry(
        &self,
        element: &ResolvedElement,
        position: Option<&Point>,
        force: bool,
    ) -> Result<Geometry, PageError> {
        // Scrolling a remote descendant also scrolls its owners. Its own local rect can
        // stay still while those parent documents are moving; wait for those rects before
        // combining coordinates from separate renderer sessions.
        // Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `if (!force) {`
        if !force && !element.owners.is_empty() {
            for owner in &element.owners {
                let states = owner
                    .frame
                    .call(
                        None,
                        &InjectedCheckElementStatesParams {
                            element: owner.element.clone(),
                            states: vec![ElementState::Stable],
                            frame_visible: true,
                        },
                    )
                    .await?;
                if states.outcome.is_some() {
                    return Err(PageError::NotConnected);
                }
            }
            let states = element
                .frame
                .call(
                    None,
                    &InjectedCheckElementStatesParams {
                        element: element.handle.clone(),
                        states: vec![ElementState::Stable],
                        frame_visible: element.frame_visible,
                    },
                )
                .await?;
            if states.outcome.is_some() {
                return Err(PageError::NotConnected);
            }
        }
        let params = json!({"objectId":element.handle.id().object_id()});
        if position.is_some() {
            let model = element.frame.host.send("DOM.getBoxModel", params);
            let border_params = InjectedGetElementBorderWidthParams {
                element: element.handle.clone(),
            };
            let border = element.frame.call(None, &border_params);
            let (model, border) = tokio::join!(model, border);
            let model = match model {
                Ok(model) => model,
                Err(error) => return no_geometry(&element.frame.host.classify(&error)),
            };
            let border = match border {
                Ok(border) => border,
                Err(error) => return no_geometry(&error),
            };
            let model: dom::GetBoxModelResult = serde_json::from_value(model)
                .map_err(|error| PageError::Contract(error.to_string()))?;
            let offset = self
                .frames
                .offset(&element.frame.host)
                .await?
                .unwrap_or_default();
            return Ok(Geometry::Offset {
                border_box: translate(quad(model.model.border)?, offset),
                border: (border.left, border.top),
            });
        }
        let quads = element.frame.host.send("DOM.getContentQuads", params);
        let viewport_frame = self.frames.main();
        let viewport = viewport_frame.call(None, &InjectedViewportSizeParams {});
        let (quads, viewport) = tokio::join!(quads, viewport);
        let viewport = viewport?;
        let quads = match quads {
            Ok(quads) => quads,
            Err(error) => return no_geometry(&element.frame.host.classify(&error)),
        };
        let quads: dom::GetContentQuadsResult = serde_json::from_value(quads)
            .map_err(|error| PageError::Contract(error.to_string()))?;
        let offset = self
            .frames
            .offset(&element.frame.host)
            .await?
            .unwrap_or_default();
        Ok(Geometry::Quads {
            quads: quads
                .quads
                .into_iter()
                .map(|points| quad(points).map(|quad| translate(quad, offset)))
                .collect::<Result<_, _>>()?,
            viewport: (pixels(viewport.width), pixels(viewport.height)),
        })
    }

    /// A turn at the tab's input lock, granted in arrival order. tokio's lock is cancel-safe: a
    /// wait dropped before its turn holds nothing.
    async fn input(&self) -> InputPermit {
        InputPermit::take(&self.input, self.session.id.clone()).await
    }

    /// [`arm_interceptor`] on the run's tracker, spawned on the first poll, and a wait for its
    /// answer. The page installs the interceptor before it answers, so the answer must be read
    /// even when the call stops first: then nobody takes the `ArmedInterceptor`, and dropping it
    /// in the task sends the stop. Spawning on the first poll, not eagerly, means a race that
    /// had already lost never sends the setup at all. The task holds `input` until the answer,
    /// and an armed interceptor keeps it until its stop has answered.
    fn arm(
        &self,
        element: &ResolvedElement,
        hit_point: Point,
        action: HitTargetAction,
        input: InputPermit,
    ) -> impl Future<Output = Result<Arming<ArmedInterceptor>, PageError>> + Send {
        let page = Arc::clone(self);
        let element = element.clone();
        async move {
            // These reads install nothing: keep them in Progress's cancellable future. Only
            // interceptor setup must outlive the waiter so a late handle is always disarmed.
            let hit_point = match frame_hit_target(&page, &element, hit_point).await? {
                FrameHitTarget::Point(point) => point,
                FrameHitTarget::Blocked(description) => {
                    return Ok(Arming::Intercepted(description));
                }
            };
            let (answer, armed) = oneshot::channel();
            let setup = arm_interceptor(Arc::clone(&page), element, hit_point, action, input);
            page.spawn(async move {
                // Nobody may be waiting any more; the unsent answer drops here, disarming.
                let _waiting = answer.send(setup.await);
            });
            armed.await.unwrap_or(Err(PageError::Closed))
        }
    }

    /// Starts the input writer of `plan` (`input_devices::start_writer`) on the run's tracker
    /// now, with an `Arc` of this page (so of the session), the ticket, the turn and the armed
    /// interceptor, and returns a future of its report: dropping the future (the call stopped)
    /// cuts any remaining `delay` short, and the writer still sends every owed release and
    /// marks the ticket. The interceptor stays in the writer's task until the last event is
    /// acknowledged; if nobody waits for the report then, it drops there and disarms, after the
    /// page has handled the release. The writer's copy of `input` goes once the last event is
    /// answered.
    fn press(
        &self,
        plan: Plan,
        ticket: InputTicket,
        armed: Option<ArmedInterceptor>,
        input: InputPermit,
    ) -> impl Future<Output = (PressReport, Option<ArmedInterceptor>)> + Send {
        let written = input_devices::start_writer(
            |task| self.spawn(task),
            Arc::clone(&self.page),
            plan,
            ticket,
            input,
            armed,
        );
        async move {
            let (report, armed) = written.await;
            (report, armed.flatten())
        }
    }

    /// `stopHitTargetInterceptor`, raced against `PageState` moving past the mark taken when it
    /// was armed (a dialog opened or a navigation requested): Playwright's
    /// `raceAgainstEvaluationStallingEvents`, which gives up on evaluating while either is in
    /// the way. If the race is lost, the stop still goes out in the background
    /// (`ArmedInterceptor`'s `Drop`), as Playwright's `finally` sends it.
    async fn verdict(&self, mut armed: ArmedInterceptor) -> Verdict {
        let mark = armed.mark;
        let mut state = self.state();
        let stalled = async {
            match state.as_mut() {
                Some(state) => {
                    // A closed watcher (the tab went) reads as stalled too.
                    let _moved: bool = state
                        .wait_for(|state| {
                            state.dialogs_opened > mark.dialogs_opened
                                || state.navigations_requested > mark.navigations_requested
                        })
                        .await
                        .is_ok();
                }
                None => future::pending::<()>().await,
            }
        };
        let params = InjectedStopHitTargetInterceptorParams {
            interceptor: armed.handle.clone(),
        };
        let stop = armed.frame.call(None, &params);
        tokio::select! {
            biased;
            () = stalled => Verdict::Done,
            stopped = stop => match stopped {
                Ok(stopped) => {
                    armed.stopped = true;
                    match stopped.outcome {
                        HitTargetOutcome::Done(_) => Verdict::Done,
                        HitTargetOutcome::Blocked(blocked) => {
                            Verdict::Blocked(blocked.hit_target_description)
                        }
                    }
                }
                Err(_) => Verdict::Done,
            },
        }
    }

    fn pending_navigation(&self) -> Option<String> {
        PageSession::pending_navigation(self)
    }

    async fn locator_handlers_checkpoint(&self, progress: &Progress) -> Result<(), Stop> {
        locator_handlers::checkpoint(self, progress)
            .await
            .map_err(Stop::End)
    }

    async fn navigation_settled(&self) -> Result<Option<String>, PageError> {
        PageSession::navigation_settled(self).await
    }

    fn watch_navigations(&self) -> NavigationWatch {
        NavigationWatch {
            events: self.session.cdp.events(),
        }
    }

    /// Sends `Page.enable` again as a flush: Chromium replies after every
    /// `frameRequestedNavigation` the input caused (`crPage.ts` `inputActionEpilogue`).
    /// browseros-cdp's single reader broadcasts events and routes replies in socket order
    /// (`client.rs` `handle_text_message`), so once the reply is back every such event is
    /// already queued in `watched`, which was subscribed before the input: draining it finds
    /// exactly the navigations the click requested. Each is then waited out until it commits,
    /// finishes in the document, or stops. On `Lagged`, it falls back to the watcher's
    /// `pending_url`. The caller races this against the call's deadline.
    async fn settle_after_input(
        &self,
        mut watched: NavigationWatch,
    ) -> Result<Vec<String>, PageError> {
        if let Err(error) = self.send("Page.enable", json!({})).await
            && world::classify(&error) == PageError::Closed
        {
            return Err(PageError::Closed);
        }
        let mut barrier = Barrier::default();
        let session = &self.session.id;
        loop {
            match watched.events.try_recv() {
                Ok(event) => barrier.apply(&event, session, &self.main_frame),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Lagged(_)) => return settled_after_lag(self).await,
                Err(TryRecvError::Closed) => return Err(PageError::Closed),
            }
        }
        while barrier.waiting > 0 {
            match watched.events.recv().await {
                Ok(event) => barrier.apply(&event, session, &self.main_frame),
                Err(RecvError::Lagged(_)) => return settled_after_lag(self).await,
                Err(RecvError::Closed) => return Err(PageError::Closed),
            }
        }
        Ok(barrier.committed)
    }
}

enum FrameHitTarget {
    Point(Option<Point>),
    Blocked(String),
}

/// Playwright's `_checkFrameIsHitTarget`: owner boxes are top-level, but each hit check runs
/// in its parent's document. Gather all boundaries before checking, because any transformed
/// ancestor disables preliminary hit checks for the whole path, leaving the event interceptor.
async fn frame_hit_target(
    page: &PageSession,
    element: &ResolvedElement,
    point: Point,
) -> Result<FrameHitTarget, PageError> {
    let mut boundaries = Vec::new();
    for owner in &element.owners {
        let model = owner
            .frame
            .host
            .send(
                "DOM.getBoxModel",
                json!({"objectId":owner.element.id().object_id()}),
            )
            .await
            .map_err(|error| match owner.frame.host.classify(&error) {
                PageError::Closed => PageError::Closed,
                _ => PageError::NotConnected,
            })?;
        let style = owner
            .frame
            .call(
                None,
                &InjectedDescribeIFrameStyleParams {
                    element: owner.element.clone(),
                },
            )
            .await?;
        let insets = match style.style {
            FrameStyle::Status(FrameStyleStatus::ErrorNotconnected) => {
                return Err(PageError::NotConnected);
            }
            FrameStyle::Status(FrameStyleStatus::Transformed) => {
                return Ok(FrameHitTarget::Point(None));
            }
            FrameStyle::Insets(insets) => insets,
        };
        let model: dom::GetBoxModelResult = serde_json::from_value(model)
            .map_err(|error| PageError::Contract(error.to_string()))?;
        let offset = page
            .frames
            .offset(&owner.frame.host)
            .await?
            .unwrap_or_default();
        let border = translate(quad(model.model.border)?, offset);
        let x = border
            .iter()
            .step_by(2)
            .copied()
            .fold(f64::INFINITY, f64::min);
        let y = border
            .iter()
            .skip(1)
            .step_by(2)
            .copied()
            .fold(f64::INFINITY, f64::min);
        boundaries.push((
            owner,
            Point {
                x: point.x - x - insets.left,
                y: point.y - y - insets.top,
            },
        ));
    }
    let mut point_in_parent = point;
    for (owner, point_in_frame) in boundaries {
        let checked = owner
            .frame
            .call(
                None,
                &InjectedExpectHitTargetParams {
                    element: owner.element.clone(),
                    point: point_in_parent,
                },
            )
            .await?;
        if let HitTargetOutcome::Blocked(blocked) = checked.outcome {
            return Ok(FrameHitTarget::Blocked(blocked.hit_target_description));
        }
        point_in_parent = point_in_frame;
    }
    Ok(FrameHitTarget::Point(Some(point_in_parent)))
}

/// `setupHitTargetInterceptor` into a fresh group: `error:notconnected` is `NotConnected`, any
/// other description `Intercepted`, the handle `Armed` with the counters taken just before the
/// setup was sent and the click's `input` turn. Runs on the run's tracker ([`Tab::arm`] for
/// `FrameTab`), so it reads the answer to the end whoever still waits for it.
async fn arm_interceptor(
    page: Arc<PageSession>,
    element: ResolvedElement,
    hit_point: Option<Point>,
    action: HitTargetAction,
    input: InputPermit,
) -> Result<Arming<ArmedInterceptor>, PageError> {
    let mark = page.mark();
    let group = new_group(&page, &element.frame, "interceptor");
    let frame = Arc::clone(&element.frame);
    let (armed, current) = frame
        .call_retaining_reply(
            Some(group.group()),
            &InjectedSetupHitTargetInterceptorParams {
                element: element.handle.clone(),
                action,
                hit_point,
                // Only Playwright's trial blocks every event, and Neo's trial sends none.
                block_all_events: false,
            },
        )
        .await?;
    let owned = match armed.outcome {
        HitTargetSetup::Description(description) if description == "error:notconnected" => {
            Arming::NotConnected
        }
        HitTargetSetup::Description(description) => Arming::Intercepted(description),
        HitTargetSetup::Armed(handle) => Arming::Armed(ArmedInterceptor {
            handle,
            group: Some(group),
            page,
            frame: Arc::clone(&frame),
            mark,
            stopped: false,
            input: Some(input),
        }),
    };
    if current {
        Ok(owned)
    } else {
        // The page acted before replying. Drop the owner (and send stop) even if the caller
        // timed out or a commit made the old element unusable; then loop A may resolve again.
        drop(owned);
        Ok(Arming::NotConnected)
    }
}

/// For a barrier whose own receiver lagged: waits for the watcher to see no pending navigation.
async fn settled_after_lag(page: &PageSession) -> Result<Vec<String>, PageError> {
    Ok(page.navigation_settled().await?.into_iter().collect())
}

/// A geometry read that failed: `NoQuads` (Playwright reads it as not visible), unless the
/// session is gone.
fn no_geometry(error: &PageError) -> Result<Geometry, PageError> {
    match error {
        PageError::Closed => Err(PageError::Closed),
        _ => Ok(Geometry::NoQuads),
    }
}

/// A CDP `DOM.Quad`, which is always eight numbers.
fn translate(mut quad: [f64; 8], (x, y): (f64, f64)) -> [f64; 8] {
    for point in quad.as_chunks_mut::<2>().0 {
        point[0] += x;
        point[1] += y;
    }
    quad
}

fn quad(points: Vec<f64>) -> Result<[f64; 8], PageError> {
    <[f64; 8]>::try_from(points)
        .map_err(|points| PageError::Contract(format!("a DOM quad of {} numbers", points.len())))
}

/// A viewport dimension in CSS pixels.
fn pixels(value: i64) -> f64 {
    i32::try_from(value).map_or(f64::MAX, f64::from)
}

// ---------------------------------------------------------------------------------------------
// Input, written by `input_devices.rs`.
// ---------------------------------------------------------------------------------------------

/// Every input writer sends through the session at its socket incarnation.
impl InputSender for Arc<PageSession> {
    async fn send_input(&self, method: &'static str, params: String) -> Result<(), CdpError> {
        self.send_input_json(method, &params).await
    }

    /// Playwright: `server/chromium/crDragDrop.ts` `DragManager.interceptDragCausedByMove`: `await progress.race(client.send('Input.setInterceptDrags', { enabled: true }));`
    ///
    /// The listeners go into every frame's utility world, interception is on for the move
    /// alone and always turned off again, and a frame that saw a drag start hands Neo the
    /// drag Chromium intercepted.
    async fn move_intercepting_drag(&self, params: String) -> Result<Option<Value>, CdpError> {
        let frames = self.drag_frames().await;
        future::join_all(frames.iter().map(|frame| {
            time::timeout(
                DRAG_FRAME_CALL_WAIT,
                frame.call(None, &InjectedSetupDragListenersParams {}),
            )
        }))
        .await;
        // Subscribed before interception is on, so its event cannot slip past.
        let mut events = self.session.cdp.events();
        if let Err(error) = self
            .send_input_json("Input.setInterceptDrags", r#"{"enabled":true}"#)
            .await
        {
            // Playwright: `server/chromium/crDragDrop.ts` `DragManager.interceptDragCausedByMove`: `this._crPage._page.safeNonStallingEvaluateInAllFrames('window.__cleanupDrag?.()', 'utility').catch(() => {});`
            // The listeners go on every exit, this one included.
            PageSession::cleanup_drag(&frames).await;
            return Err(error);
        }
        let moved = self
            .send_input_json("Input.dispatchMouseEvent", &params)
            .await;
        let started = PageSession::cleanup_drag(&frames).await;
        let disabled = self
            .send_input_json("Input.setInterceptDrags", r#"{"enabled":false}"#)
            .await;
        moved?;
        disabled?;
        if !started {
            return Ok(None);
        }
        let intercepted = async {
            loop {
                match events.recv().await {
                    Ok(event)
                        if event.method == "Input.dragIntercepted"
                            && event.session_id.as_ref() == Some(&self.session.id) =>
                    {
                        return Ok(event.params["data"].clone());
                    }
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return Err(CdpError::SessionGone),
                }
            }
        };
        time::timeout(DRAG_INTERCEPT_WAIT, intercepted)
            .await
            .map_err(|_| CdpError::Timeout {
                method: "Input.dragIntercepted".to_string(),
            })?
            .map(Some)
    }
}

// The frames a drag's listeners go to.
impl PageSession {
    /// Neo: `cleanupDrag` in every frame, each call bounded: removes the drag listeners and
    /// answers whether any frame saw the move start a drag the page did not prevent.
    async fn cleanup_drag(frames: &[Arc<FrameContext>]) -> bool {
        future::join_all(frames.iter().map(|frame| {
            time::timeout(
                DRAG_FRAME_CALL_WAIT,
                frame.call(None, &InjectedCleanupDragParams {}),
            )
        }))
        .await
        .into_iter()
        .any(|cleaned| cleaned.is_ok_and(|result| result.is_ok_and(|cleaned| cleaned.started)))
    }

    /// Neo: every frame's context, the main one first, then the frame tree in pre-order. A
    /// frame whose context cannot be reached is left out, as Playwright's evaluation in all
    /// frames skips one it cannot reach.
    async fn drag_frames(&self) -> Vec<Arc<FrameContext>> {
        let main = self.frames.main();
        let Ok(tree) = self.frames.tree().await else {
            return vec![main];
        };
        let mut contexts: HashMap<String, Arc<FrameContext>> = HashMap::new();
        contexts.insert(main.id.clone(), Arc::clone(&main));
        let mut frames = vec![main];
        for info in tree {
            if contexts.contains_key(&info.id) {
                continue;
            }
            let Some(parent) = info
                .parent
                .as_ref()
                .and_then(|parent| contexts.get(parent))
                .map(|parent| Arc::clone(&parent.host))
            else {
                continue;
            };
            if let Ok(context) = self.frames.context(&info.id, Some(&parent)).await {
                contexts.insert(info.id.clone(), Arc::clone(&context));
                frames.push(context);
            }
        }
        frames
    }
}

// The run's end, for its input.
impl PageSession {
    /// Records that this session's run ended (`TabInput::mark_ended`): no wait, no write.
    /// A run's end records it on every tab before any of its cleanup waits
    /// (`Tabs::mark_input_ended`).
    pub(crate) fn mark_input_ended(&self) {
        self.input.mark_ended(&self.session.id);
    }

    /// Releases every key and button this run's session left held on the tab (design section
    /// 4.6): a keyUp per held key, newest first, then a mouseUp per held button at the
    /// pointer's last position, under a turn so no other call's input lands in between. The
    /// run is recorded as ended first, before any wait or write ([`TabInput::mark_ended`]), so
    /// a release that is refused or fails, or whose future is dropped, leaves what it holds to
    /// the next run's end, held by no live run. `Tabs::release_input` runs it on a task of its
    /// own, which no budget cuts off. A closed tab writes nothing.
    pub(crate) async fn release_run_input(self: &Arc<Self>) {
        self.mark_input_ended();
        if self.closed.is_cancelled() {
            return;
        }
        let permit = InputPermit::take(&self.input, self.session.id.clone()).await;
        write_run_releases(self, &permit).await;
    }
}

/// Neo: a run end's releases under `permit`'s turn: whatever its run holds, and every dead
/// hold (an ended run's), except what another live run still holds. A failed release stays
/// held, for the next run's end.
async fn write_run_releases(sender: &impl InputSender, permit: &InputPermit) {
    let Ok(mut simulated) = permit.snapshot() else {
        return;
    };
    let owner = permit.owner();
    let mut plan = Plan::default();
    keyboard::plan_key_releases(&mut plan, &mut simulated, &owner);
    input_devices::plan_button_releases(&mut plan, &mut simulated, &owner);
    let written = input_devices::write_plan(sender, permit, &plan, &CancellationToken::new()).await;
    if let Some(Err(error)) = written.outcomes.iter().find(|outcome| outcome.is_err()) {
        debug!(%error, "releasing the run's held input failed");
    }
}

// Input commands and the browser's platform go through these two.
impl PageSession {
    /// One input command on the session's own socket incarnation, its params already JSON and
    /// written unchanged (key events keep Playwright's field order on the wire). browseros-cdp
    /// refuses it with `ConnectionEpochChanged` before writing once the socket was replaced, so
    /// no input goes out under a session id the new socket does not know, and a writer can
    /// prove it wrote nothing.
    pub(crate) async fn send_input_json(&self, method: &str, params: &str) -> Result<(), CdpError> {
        self.session
            .cdp
            .send_raw_json_at_epoch(self.session.epoch, method, params, Some(&self.session.id))
            .await
            .map(|_| ())
    }

    /// The browser's userAgent: `Browser.getVersion`, a browser-wide command sent without the
    /// session, as Playwright reads it once per browser (`crBrowser.ts`).
    pub(crate) async fn user_agent(&self) -> Result<String, CdpError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Version {
            user_agent: String,
        }
        let version: Version = self
            .session
            .cdp
            .send_typed("Browser.getVersion", json!({}), None)
            .await?;
        Ok(version.user_agent)
    }
}

#[cfg(test)]
mod tests {
    use std::mem;
    use std::sync::atomic::AtomicBool;

    use tokio::sync::Notify;
    use tokio::task::JoinHandle;

    use super::*;

    const SESSION: &str = "S1";
    const MAIN: &str = "MAIN";

    fn event(method: &str, session: Option<&str>, params: Value) -> CdpEvent {
        CdpEvent {
            method: method.to_string(),
            params,
            session_id: session.map(SessionId::from),
        }
    }

    fn applied(state: &mut PageState, event: &CdpEvent) -> Applied {
        apply(state, event, &SessionId::from(SESSION), MAIN)
    }

    const CHILD: &str = "CHILD";
    const OOPIF_SESSION: &str = "S2";

    fn lifecycle(session: &str, frame: &str, loader: &str, name: &str) -> CdpEvent {
        event(
            "Page.lifecycleEvent",
            Some(session),
            json!({ "frameId": frame, "loaderId": loader, "name": name }),
        )
    }

    fn navigated(session: &str, frame: &str, url: &str, loader: &str) -> CdpEvent {
        event(
            "Page.frameNavigated",
            Some(session),
            json!({ "frame": { "id": frame, "url": url, "loaderId": loader } }),
        )
    }

    fn detached(session: &str, frame: &str, reason: &str) -> CdpEvent {
        event(
            "Page.frameDetached",
            Some(session),
            json!({ "frameId": frame, "reason": reason }),
        )
    }

    #[test]
    fn tests_that_a_child_frames_events_feed_only_its_frame() {
        let feeds = FrameFeeds::default();
        let tab = SessionId::from(SESSION);
        let (state, mut commits) = feeds.subscribe(CHILD).expect("feed");
        for event in [
            navigated(SESSION, CHILD, "https://b.test/pay", "doc-1"),
            lifecycle(SESSION, CHILD, "doc-1", "DOMContentLoaded"),
            lifecycle(SESSION, CHILD, "doc-1", "load"),
            // Another session's and the main frame's events are not this frame's.
            lifecycle(OOPIF_SESSION, CHILD, "doc-9", "load"),
            navigated(SESSION, MAIN, "https://a.test/", "main-2"),
        ] {
            feeds.apply(&event, &tab, Some(MAIN));
        }
        let child = state.borrow().clone();
        assert_eq!(
            child.page.committed_url.as_deref(),
            Some("https://b.test/pay")
        );
        assert_eq!(
            (
                child.page.load,
                child.page.load_count,
                child.page.document.as_deref()
            ),
            (true, 1, Some("doc-1"))
        );
        assert_eq!(
            commits.try_recv().expect("the commit is queued").url,
            "https://b.test/pay"
        );
        assert!(commits.try_recv().is_err(), "one commit only");

        let mut main = PageState::default();
        let event = navigated(SESSION, CHILD, "https://b.test/pay", "doc-1");
        assert_eq!(
            applied(&mut main, &event),
            Applied::default(),
            "the main frame's state ignores a child's commit"
        );
    }

    #[test]
    fn tests_that_a_swap_starts_a_new_document_and_ignores_the_old_process() {
        let feeds = FrameFeeds::default();
        let tab = SessionId::from(SESSION);
        let oopif = SessionId::from(OOPIF_SESSION);
        let (state, _commits) = feeds.subscribe(CHILD).expect("feed");
        feeds.apply(
            &navigated(SESSION, CHILD, "https://a.test/pay", "old"),
            &tab,
            Some(MAIN),
        );
        feeds.apply(&lifecycle(SESSION, CHILD, "old", "load"), &tab, Some(MAIN));
        feeds.apply(&detached(SESSION, CHILD, "swap"), &tab, Some(MAIN));
        // A late event of the old document from the old process must not seed the new one.
        feeds.apply(&lifecycle(SESSION, CHILD, "old", "load"), &tab, Some(MAIN));
        let swapped = state.borrow().clone();
        assert_eq!(swapped.swaps, 1);
        assert!(!swapped.detached);
        assert_eq!(
            (
                swapped.page.committed_url,
                swapped.page.document,
                swapped.page.load
            ),
            (None, None, false)
        );
        // The new process's replay seeds the new document; the counts keep growing.
        feeds.seed(CHILD, "https://c.test/pay");
        feeds.apply(
            &lifecycle(OOPIF_SESSION, CHILD, "new", "commit"),
            &oopif,
            None,
        );
        feeds.apply(
            &lifecycle(OOPIF_SESSION, CHILD, "new", "load"),
            &oopif,
            None,
        );
        let replayed = state.borrow().clone();
        assert_eq!(
            replayed.page.committed_url.as_deref(),
            Some("https://c.test/pay")
        );
        assert_eq!(
            (
                replayed.page.document.as_deref(),
                replayed.page.load,
                replayed.page.load_count
            ),
            (Some("new"), true, 2)
        );
        // Back in the page's process: its attach and commit are followed again.
        feeds.apply(
            &event(
                "Page.frameAttached",
                Some(SESSION),
                json!({ "frameId": CHILD }),
            ),
            &tab,
            Some(MAIN),
        );
        feeds.apply(
            &navigated(SESSION, CHILD, "https://a.test/back", "back"),
            &tab,
            Some(MAIN),
        );
        assert_eq!(
            state.borrow().page.committed_url.as_deref(),
            Some("https://a.test/back")
        );
    }

    #[tokio::test]
    async fn tests_that_a_removed_frame_or_a_page_commit_ends_its_feed() {
        let feeds = FrameFeeds::default();
        let tab = SessionId::from(SESSION);
        let (mut removed, _commits) = feeds.subscribe(CHILD).expect("feed");
        feeds.apply(&detached(SESSION, CHILD, "remove"), &tab, Some(MAIN));
        removed.changed().await.expect("the removal is published");
        assert!(removed.borrow_and_update().detached);
        assert!(
            removed.changed().await.is_err(),
            "the feed is gone with its frame"
        );

        let (mut other, _other_commits) = feeds.subscribe("OTHER").expect("feed");
        feeds.detach_all();
        other.changed().await.expect("the page commit is published");
        assert!(other.borrow().detached);
    }

    #[test]
    fn tests_that_a_seed_never_replaces_a_commit() {
        let feeds = FrameFeeds::default();
        let tab = SessionId::from(SESSION);
        let (state, _commits) = feeds.subscribe(CHILD).expect("feed");
        feeds.apply(
            &navigated(SESSION, CHILD, "https://a.test/new", "doc"),
            &tab,
            Some(MAIN),
        );
        feeds.seed(CHILD, "https://a.test/old");
        assert_eq!(
            state.borrow().page.committed_url.as_deref(),
            Some("https://a.test/new")
        );
    }

    /// A run's end records the run as ended before it waits for the tab's turn, so a release
    /// that never gets the turn before the run's deadline still leaves what the run held to the
    /// next run's end.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_run_is_marked_ended_without_waiting_for_the_turn() {
        use std::slice;

        use crate::keyboard::SHIFT;

        let tab = Arc::new(TabInput::default());
        let a = SessionId("A".to_string());
        let held = InputPermit::take(&tab, a.clone()).await;
        held.apply(
            &Effect::KeyDown {
                code: "ShiftLeft",
                modifier: SHIFT,
                owner: a.clone(),
            },
            true,
        );
        // Another call holds the turn: the mark goes in at once all the same.
        tab.mark_ended(&a);
        assert_eq!(held.devices().ended, slice::from_ref(&a));
        drop(held);
        let next = InputPermit::take(&tab, SessionId("B".to_string())).await;
        let devices = next.snapshot().expect("devices");
        assert!(
            !devices.live_other(&a, &SessionId("B".to_string())),
            "A is not live"
        );
        assert!(
            devices.releases(&a, &SessionId("B".to_string())),
            "B's end releases A's keys"
        );
    }

    #[test]
    fn requested_main_frame_navigations_in_this_tab_are_pending_until_they_commit() {
        let mut state = PageState::default();
        let requested = |frame: &str, disposition: &str| {
            event(
                "Page.frameRequestedNavigation",
                Some(SESSION),
                json!({ "frameId": frame, "reason": "anchorClick", "url": "https://a.test/next",
                        "disposition": disposition }),
            )
        };
        assert!(!applied(&mut state, &requested("CHILD", "currentTab")).changed);
        assert!(!applied(&mut state, &requested(MAIN, "newTab")).changed);
        let other_session = event(
            "Page.frameRequestedNavigation",
            Some("S2"),
            json!({ "frameId": MAIN, "url": "x", "disposition": "currentTab" }),
        );
        assert!(!applied(&mut state, &other_session).changed);
        assert!(applied(&mut state, &requested(MAIN, "currentTab")).changed);
        assert_eq!(state.pending_url.as_deref(), Some("https://a.test/next"));
        assert_eq!(state.navigations_requested, 1);

        let navigated = event(
            "Page.frameNavigated",
            Some(SESSION),
            json!({ "frame": { "id": MAIN, "loaderId": "L", "url": "https://a.test/next",
                               "urlFragment": "#top", "securityOrigin": "", "mimeType": "" },
                    "type": "Navigation" }),
        );
        assert!(applied(&mut state, &navigated).changed);
        assert_eq!(
            (
                state.pending_url.as_deref(),
                state.committed_url.as_deref(),
                state.commits
            ),
            (None, Some("https://a.test/next#top"), 1)
        );
    }

    /// The events a `location.hash` handler causes: a fragment navigation raises no request,
    /// only a commit within the document.
    #[test]
    fn a_fragment_navigation_commits_within_the_document() {
        let mut state = PageState::default();
        for (method, params) in [
            (
                "Page.frameScheduledNavigation",
                json!({ "frameId": MAIN, "delay": 0,
                "reason": "scriptInitiated", "url": "http://127.0.0.1/ok.html#done" }),
            ),
            ("Page.frameStartedLoading", json!({ "frameId": MAIN })),
            (
                "Page.navigatedWithinDocument",
                json!({ "frameId": MAIN,
                "url": "http://127.0.0.1/ok.html#done", "navigationType": "fragment" }),
            ),
            ("Page.frameStoppedLoading", json!({ "frameId": MAIN })),
            (
                "Page.frameClearedScheduledNavigation",
                json!({ "frameId": MAIN }),
            ),
        ] {
            applied(&mut state, &event(method, Some(SESSION), params));
        }
        assert_eq!(state.navigations_requested, 0);
        assert_eq!(state.commits, 1);
        assert_eq!(
            state.committed_url.as_deref(),
            Some("http://127.0.0.1/ok.html#done")
        );
    }

    #[test]
    fn a_stopped_load_ends_a_pending_navigation() {
        let mut state = PageState {
            pending_url: Some("https://a.test/204".to_string()),
            ..PageState::default()
        };
        let stopped = event(
            "Page.frameStoppedLoading",
            Some(SESSION),
            json!({ "frameId": MAIN }),
        );
        assert!(applied(&mut state, &stopped).changed);
        assert_eq!(state.pending_url, None);
        assert_eq!(state.commits, 0);
    }

    #[test]
    fn tests_that_lifecycle_belongs_to_the_current_main_frame_document() {
        let mut state = PageState::default();
        let commit = |loader| {
            event(
                "Page.frameNavigated",
                Some(SESSION),
                json!({
                    "frame": {"id": MAIN, "url": "https://a.test/", "loaderId": loader}
                }),
            )
        };
        let lifecycle = |frame, loader, name| {
            event(
                "Page.lifecycleEvent",
                Some(SESSION),
                json!({
                    "frameId": frame, "loaderId": loader, "name": name
                }),
            )
        };
        applied(&mut state, &commit("one"));
        assert_eq!(state.document.as_deref(), Some("one"));
        applied(&mut state, &lifecycle("CHILD", "one", "load"));
        applied(&mut state, &lifecycle(MAIN, "stale", "load"));
        assert!(!state.load);
        applied(&mut state, &lifecycle(MAIN, "one", "DOMContentLoaded"));
        assert!(state.dom_content_loaded);
        assert!(!state.load);
        applied(&mut state, &lifecycle(MAIN, "one", "load"));
        assert!(state.load);
        applied(
            &mut state,
            &event(
                "Page.navigatedWithinDocument",
                Some(SESSION),
                json!({
                    "frameId": MAIN, "url": "https://a.test/#hash"
                }),
            ),
        );
        assert!(state.load);
        applied(&mut state, &commit("two"));
        assert!(!state.load);
        assert!(!state.dom_content_loaded);
    }

    #[test]
    fn tests_that_stopped_loading_does_not_complete_an_ordinary_document() {
        let mut state = PageState::default();
        applied(
            &mut state,
            &event(
                "Page.frameStoppedLoading",
                Some(SESSION),
                json!({"frameId": MAIN}),
            ),
        );
        assert!(!state.dom_content_loaded);
        assert!(!state.load);
    }

    #[test]
    fn tests_that_a_bfcache_restore_commits_a_loaded_document_without_a_request() {
        // BrowserOS neo reports a restore's stop before its commit, then no lifecycle at all.
        let stopped = event(
            "Page.frameStoppedLoading",
            Some(SESSION),
            json!({"frameId":MAIN}),
        );
        for (kind, loaded) in [("BackForwardCacheRestore", true), ("Navigation", false)] {
            let mut state = PageState::default();
            applied(&mut state, &stopped);
            let commit = applied(
                &mut state,
                &event(
                    "Page.frameNavigated",
                    Some(SESSION),
                    json!({
                        "frame":{"id":MAIN,"url":"https://a.test/","loaderId":kind},"type":kind
                    }),
                ),
            );
            assert_eq!((state.dom_content_loaded, state.load), (loaded, loaded));
            let navigation = commit.navigation.expect("a commit");
            assert_eq!(navigation.lifecycle.load, loaded, "{kind}");
            assert!(!navigation.request, "no request started for {kind}");
        }
    }

    fn request_started(id: &str, frame: &str, url: &str, kind: &str) -> CdpEvent {
        event(
            "Network.requestWillBeSent",
            Some(SESSION),
            json!({
                "requestId": id, "loaderId": id, "frameId": frame, "type": kind,
                "request": {"url": url, "method": "GET", "headers": {}}
            }),
        )
    }

    fn request_failed(id: &str, error: &str, canceled: bool) -> CdpEvent {
        event(
            "Network.loadingFailed",
            Some(SESSION),
            json!({"requestId": id, "errorText": error, "canceled": canceled, "type": "Document"}),
        )
    }

    fn committed(loader: &str) -> CdpEvent {
        event(
            "Page.frameNavigated",
            Some(SESSION),
            json!({"frame": {"id": MAIN, "url": format!("https://a.test/{loader}"), "loaderId": loader}}),
        )
    }

    #[test]
    fn tests_that_a_commit_carries_its_request_only_when_its_navigation_request_started() {
        let mut state = PageState::default();
        applied(
            &mut state,
            &request_started("D", MAIN, "https://a.test/D", "Document"),
        );
        let navigation = applied(&mut state, &committed("D"))
            .navigation
            .expect("commit");
        assert!(navigation.request && navigation.error.is_none());
        // A navigation requested from the page, whose request Neo never saw start.
        applied(
            &mut state,
            &event(
                "Page.frameRequestedNavigation",
                Some(SESSION),
                json!({"frameId": MAIN, "url": "https://a.test/E", "disposition": "currentTab"}),
            ),
        );
        let navigation = applied(&mut state, &committed("E"))
            .navigation
            .expect("commit");
        assert!(!navigation.request);
        // Subresources, child frames and `data:` documents are never a navigation's request.
        for (id, frame, url, kind) in [
            ("S", MAIN, "https://a.test/s.js", "Script"),
            ("C", "child", "https://a.test/c", "Document"),
            ("U", MAIN, "data:text/html,x", "Document"),
        ] {
            applied(&mut state, &request_started(id, frame, url, kind));
            assert_eq!(state.pending_document, None, "{id}");
        }
    }

    #[test]
    fn tests_that_only_the_pending_documents_failure_aborts_its_navigation() {
        let mut state = PageState::default();
        applied(&mut state, &committed("A"));
        applied(
            &mut state,
            &request_started("B", MAIN, "https://a.test/B", "Document"),
        );
        let aborted = applied(&mut state, &request_failed("B", "net::ERR_ABORTED", true))
            .navigation
            .expect("the pending navigation failed");
        assert_eq!(
            aborted.error.as_deref(),
            Some("net::ERR_ABORTED; maybe frame was detached?")
        );
        assert_eq!(
            (aborted.document.as_deref(), aborted.url.as_str()),
            (Some("B"), "https://a.test/A"),
            "the failed navigation, at the URL the frame still shows"
        );
        assert_eq!(state.pending_document, None);
        // A navigation the page replaced before its request failed: Playwright's pending
        // document is the newer one by then, so nothing aborts.
        applied(
            &mut state,
            &request_started("C", MAIN, "https://a.test/C", "Document"),
        );
        applied(
            &mut state,
            &event(
                "Page.frameRequestedNavigation",
                Some(SESSION),
                json!({"frameId": MAIN, "url": "https://a.test/D", "disposition": "currentTab"}),
            ),
        );
        assert_eq!(
            applied(&mut state, &request_failed("C", "net::ERR_ABORTED", true)).navigation,
            None
        );
        // An error page commits under the failed request's loader after the abort, without it.
        applied(
            &mut state,
            &request_started("E", MAIN, "https://a.test/E", "Document"),
        );
        let failed = applied(
            &mut state,
            &request_failed("E", "net::ERR_EMPTY_RESPONSE", false),
        );
        assert_eq!(
            failed.navigation.and_then(|navigation| navigation.error),
            Some("net::ERR_EMPTY_RESPONSE".to_string())
        );
        let error_page = applied(&mut state, &committed("E"))
            .navigation
            .expect("commit");
        assert!(!error_page.request);
    }

    #[test]
    fn tests_that_a_later_navigation_stays_pending_through_an_earlier_commit() {
        let mut state = PageState::default();
        applied(
            &mut state,
            &request_started("NEW", MAIN, "https://a.test/new", "Document"),
        );
        // An error page of an earlier navigation commits after the new request started.
        let navigation = applied(&mut state, &committed("OLD"))
            .navigation
            .expect("commit");
        assert!(!navigation.request);
        assert_eq!(
            state.pending_document,
            Some(PendingDocument::Started("NEW".to_string()))
        );
        assert!(
            applied(&mut state, &request_failed("NEW", "net::ERR_FAILED", false))
                .navigation
                .is_some_and(|navigation| navigation.error.is_some())
        );
    }

    #[test]
    fn tests_that_replayed_lifecycle_seeds_an_existing_document() {
        let mut state = PageState::default();
        for name in ["init", "DOMContentLoaded", "load"] {
            applied(
                &mut state,
                &event(
                    "Page.lifecycleEvent",
                    Some(SESSION),
                    json!({
                        "frameId": MAIN, "loaderId": "existing", "name": name
                    }),
                ),
            );
        }
        assert_eq!(state.document.as_deref(), Some("existing"));
        assert!(state.dom_content_loaded && state.load);
    }

    #[test]
    fn tests_that_lifecycle_counts_survive_resets_and_ignore_duplicate_events() {
        let mut state = PageState::default();
        for document in ["one", "two"] {
            let committed = applied(
                &mut state,
                &event(
                    "Page.frameNavigated",
                    Some(SESSION),
                    json!({
                        "frame": {"id": MAIN, "url": "https://a.test/", "loaderId": document}
                    }),
                ),
            );
            let snapshot = committed.navigation.expect("commit").lifecycle;
            assert!(!snapshot.load && !snapshot.dom_content_loaded);
            assert_eq!(snapshot.load_count, state.load_count);
            for name in ["DOMContentLoaded", "load"] {
                for _ in 0..2 {
                    applied(
                        &mut state,
                        &event(
                            "Page.lifecycleEvent",
                            Some(SESSION),
                            json!({
                                "frameId": MAIN, "loaderId": document, "name": name
                            }),
                        ),
                    );
                }
            }
        }
        assert_eq!(state.dom_content_loaded_count, 2);
        assert_eq!(state.load_count, 2);
        let hash = applied(
            &mut state,
            &event(
                "Page.navigatedWithinDocument",
                Some(SESSION),
                json!({
                    "frameId": MAIN, "url": "https://a.test/#hash"
                }),
            ),
        );
        let snapshot = hash.navigation.expect("same-document commit").lifecycle;
        assert!(snapshot.load && snapshot.dom_content_loaded);
        assert_eq!(snapshot.load_count, 2);
    }

    #[test]
    fn tests_that_a_reopen_clears_networkidle_and_counts_for_the_writes_own_wait() {
        let lifecycle = |state: &mut PageState, name: &str| {
            applied(
                state,
                &event(
                    "Page.lifecycleEvent",
                    Some(SESSION),
                    json!({ "frameId": MAIN, "loaderId": "one", "name": name }),
                ),
            )
        };
        let mut state = PageState::default();
        applied(
            &mut state,
            &event(
                "Page.frameNavigated",
                Some(SESSION),
                json!({ "frame": {"id": MAIN, "url": "https://a.test/", "loaderId": "one"} }),
            ),
        );
        assert!(state.set_network_idle(true));
        // A page's own document.open(), unarmed: Playwright keeps the frame's networkidle.
        lifecycle(&mut state, "init");
        assert!(state.network_idle && state.lifecycle().reopens == 0);
        // Two overlapping writes: each learns the reopen count its own reopen reaches.
        assert_eq!(expect(&mut state, true), 1);
        assert_eq!(expect(&mut state, true), 2);
        lifecycle(&mut state, "init");
        assert!(
            !state.network_idle,
            "the first reopen clears the old document's idleness"
        );
        assert_eq!(state.lifecycle().reopens, 1);
        assert!(
            state.set_network_idle(true),
            "the first written document went idle"
        );
        lifecycle(&mut state, "init");
        assert!(
            !state.network_idle,
            "the second write's reopen clears it again"
        );
        assert_eq!(
            (state.lifecycle().reopens, state.network_idle_count),
            (2, 2)
        );
        lifecycle(&mut state, "init");
        assert_eq!(
            state.lifecycle().reopens,
            2,
            "no write expects another reopen"
        );
        // A write that failed before document.open() gives its place back.
        assert_eq!(expect(&mut state, true), 3);
        assert_eq!(expect(&mut state, false), 2);
    }

    #[test]
    fn tests_that_a_reopened_document_clears_its_lifecycle_only_when_set_content_armed_it() {
        let lifecycle = |state: &mut PageState, name: &str| {
            applied(
                state,
                &event(
                    "Page.lifecycleEvent",
                    Some(SESSION),
                    json!({ "frameId": MAIN, "loaderId": "one", "name": name }),
                ),
            )
        };
        let mut state = PageState::default();
        applied(
            &mut state,
            &event(
                "Page.frameNavigated",
                Some(SESSION),
                json!({ "frame": {"id": MAIN, "url": "https://a.test/", "loaderId": "one"} }),
            ),
        );
        for name in ["DOMContentLoaded", "load"] {
            lifecycle(&mut state, name);
        }
        // A page's own document.open(), unarmed: Playwright keeps the frame loaded.
        lifecycle(&mut state, "init");
        assert!(state.load && state.dom_content_loaded);
        // setContent's: the reopened document's events count again.
        expect(&mut state, true);
        assert!(lifecycle(&mut state, "init").changed);
        assert!(!state.load && !state.dom_content_loaded && state.reopens_expected == 0);
        for name in ["DOMContentLoaded", "load"] {
            lifecycle(&mut state, name);
        }
        assert!(state.load && state.dom_content_loaded);
        assert_eq!((state.dom_content_loaded_count, state.load_count), (2, 2));
        // Two overlapping writes: each init spends its own clear, so the later document's
        // unfired load is not the earlier one's.
        expect(&mut state, true);
        expect(&mut state, true);
        lifecycle(&mut state, "init");
        for name in ["DOMContentLoaded", "load"] {
            lifecycle(&mut state, name);
        }
        lifecycle(&mut state, "init");
        lifecycle(&mut state, "DOMContentLoaded");
        assert!(state.dom_content_loaded && !state.load && state.reopens_expected == 0);
        // A write that failed before document.open() gives its clear back.
        expect(&mut state, true);
        expect(&mut state, false);
        lifecycle(&mut state, "init");
        assert!(state.dom_content_loaded);
        // A new document drops the clears nothing consumed.
        expect(&mut state, true);
        applied(
            &mut state,
            &event(
                "Page.frameNavigated",
                Some(SESSION),
                json!({ "frame": {"id": MAIN, "url": "https://a.test/b", "loaderId": "two"} }),
            ),
        );
        assert_eq!(state.reopens_expected, 0);
    }

    #[test]
    fn tests_that_bfcache_counts_each_lifecycle_once() {
        let mut state = PageState::default();
        applied(
            &mut state,
            &event(
                "Page.frameNavigated",
                Some(SESSION),
                json!({
                    "frame": {"id": MAIN, "url": "https://a.test/", "loaderId": "restored"},
                    "type": "BackForwardCacheRestore"
                }),
            ),
        );
        for _ in 0..2 {
            applied(
                &mut state,
                &event(
                    "Page.frameStoppedLoading",
                    Some(SESSION),
                    json!({"frameId": MAIN}),
                ),
            );
        }
        for name in ["DOMContentLoaded", "load"] {
            applied(
                &mut state,
                &event(
                    "Page.lifecycleEvent",
                    Some(SESSION),
                    json!({
                        "frameId": MAIN, "loaderId": "restored", "name": name
                    }),
                ),
            );
        }
        assert_eq!(state.dom_content_loaded_count, 1);
        assert_eq!(state.load_count, 1);
    }

    #[tokio::test]
    async fn tests_that_the_url_seed_follows_queued_events_and_preserves_a_commit() {
        use crate::fake_browser::{FakeBrowser, Replies};

        let browser = FakeBrowser::start(Replies::default()).await;
        let cdp = browser.cdp().await;
        for committed in [false, true] {
            let (events, receiver) = broadcast::channel(16);
            let (state, mut current) = watch::channel(PageState::default());
            let (navigations, _) = broadcast::channel(16);
            let stop = CancellationToken::new();
            if committed {
                events
                    .send(event(
                        "Page.frameNavigated",
                        Some(SESSION),
                        json!({
                            "frame": {"id": MAIN, "url": "https://a.test/new", "loaderId": "new"}
                        }),
                    ))
                    .expect("queued commit");
            }
            events
                .send(event(
                    "Page.lifecycleEvent",
                    Some(SESSION),
                    json!({
                        "frameId": MAIN, "loaderId": "new", "name": "load"
                    }),
                ))
                .expect("queued load");
            let before_seed = current.clone();
            let (seeded, consumed) = oneshot::channel();
            let initial_url = async move {
                assert!(
                    before_seed.borrow().load,
                    "events before the reply apply first"
                );
                let _ = seeded.send(());
                Ok(Some("https://a.test/old".to_string()))
            };
            let updates = PageUpdates {
                state,
                navigations,
                world: Arc::new(World::new(
                    cdp.clone(),
                    SessionId::from(SESSION),
                    MAIN.to_string(),
                )),
                network: Arc::new(NetworkLog::new(MAIN.to_string())),
                frames: None,
                feeds: None,
                initial_url,
                producer: None,
                idle: None,
                routes: None,
            };
            let watcher = watch_events(
                receiver,
                cdp.clone(),
                SessionId::from(SESSION),
                MAIN.to_string(),
                updates,
                CancellationToken::new(),
                stop.clone(),
            );
            let verify = async {
                consumed.await.expect("seed consumed");
                let expected = if committed {
                    "https://a.test/new"
                } else {
                    "https://a.test/old"
                };
                current
                    .wait_for(|state| state.committed_url.as_deref() == Some(expected))
                    .await
                    .expect("seed published");
                stop.cancel();
            };
            tokio::join!(watcher, verify);
            assert_eq!(current.borrow().load_count, 1);
        }
    }

    #[tokio::test]
    async fn tests_that_observers_created_before_preparation_receive_watcher_updates() {
        use crate::fake_browser::{FakeBrowser, PAGE, Replies};
        use crate::waits::WaitTab;

        let browser = FakeBrowser::start(Replies::default()).await;
        let id = PageId::new(PAGE.to_string());
        let session = Session::attach(browser.cdp().await, id.clone(), TokioHandle::current())
            .await
            .expect("attach");
        let page = Arc::new(PageSession::new(
            session,
            &id,
            TaskTracker::new(),
            Arc::default(),
            Arc::new(Mutex::new(())),
            CancellationToken::new(),
            EventHub::default(),
            false,
            Arc::default(),
        ));
        let tab = FrameTab {
            page: Arc::clone(&page),
            scope: FrameScope::Main,
        };
        let mut events = WaitTab::observe(&tab).expect("wait observer before prepare");
        let mut navigation =
            NavigationTab::observe(&tab).expect("navigation observer before prepare");
        PageSession::prepare(&page).await.expect("prepare");
        time::timeout(Duration::from_secs(1), async {
            while !events.state().lifecycle.load {
                events.next().await.expect("watcher update");
            }
            while !navigation.state().lifecycle.load {
                navigation.next().await.expect("watcher update");
            }
        })
        .await
        .expect("both observers see preparation's lifecycle replay");
        assert!(events.state().lifecycle.dom_content_loaded);
        assert_eq!(events.state().lifecycle.load_count, 1);
        drop(events);
        drop(navigation);
        assert_eq!(page.navigations.receiver_count(), 0);
        assert_eq!(page.state_updates.receiver_count(), 1);
    }

    /// A session on the fake's tab with its own tracker and closed token.
    async fn tracked_page(
        cdp: CdpClient,
        tasks: &TaskTracker,
        closed: &CancellationToken,
    ) -> Arc<PageSession> {
        use crate::fake_browser::PAGE;

        let id = PageId::new(PAGE.to_string());
        let session = Session::attach(cdp, id.clone(), TokioHandle::current())
            .await
            .expect("attach");
        Arc::new(PageSession::new(
            session,
            &id,
            tasks.clone(),
            Arc::default(),
            Arc::new(Mutex::new(())),
            closed.clone(),
            EventHub::default(),
            false,
            Arc::default(),
        ))
    }

    /// A task waiting for an answer its gone tab will never send ends when the tab goes, and
    /// what it held (here a sender standing in for the tab's screenshot turn) goes with it, so
    /// the run's tracker drains.
    #[tokio::test]
    async fn a_background_task_ends_when_its_tab_goes() {
        use std::future;

        use crate::fake_browser::{FakeBrowser, Replies};

        let browser = FakeBrowser::start(Replies::default()).await;
        let tasks = TaskTracker::new();
        let closed = CancellationToken::new();
        let page = tracked_page(browser.cdp().await, &tasks, &closed).await;
        let (held, released) = oneshot::channel::<()>();
        page.spawn_while_open(async move {
            future::pending::<()>().await;
            drop(held);
        });
        closed.cancel();
        assert!(
            time::timeout(Duration::from_secs(5), released)
                .await
                .is_ok(),
            "the task's hold went with the tab"
        );
        tasks.close();
        assert!(
            time::timeout(Duration::from_secs(5), tasks.wait())
                .await
                .is_ok()
        );
    }

    /// While the tab stays, the task runs to its end.
    #[tokio::test]
    async fn a_background_task_runs_to_its_end_while_its_tab_stays() {
        use crate::fake_browser::{FakeBrowser, Replies};

        let browser = FakeBrowser::start(Replies::default()).await;
        let tasks = TaskTracker::new();
        let closed = CancellationToken::new();
        let page = tracked_page(browser.cdp().await, &tasks, &closed).await;
        let (done, finished) = oneshot::channel();
        page.spawn_while_open(async move {
            time::sleep(Duration::from_millis(20)).await;
            let _sent = done.send("ran");
        });
        assert_eq!(
            time::timeout(Duration::from_secs(5), finished)
                .await
                .ok()
                .and_then(Result::ok),
            Some("ran")
        );
    }

    /// A dropped group is released while its tab is open, and not once the tab went: the tab
    /// took the group with it, and a crashed one would never answer the release.
    #[tokio::test]
    async fn a_group_is_released_only_while_its_tab_is_open() {
        use crate::fake_browser::{FakeBrowser, Replies};

        fn released(browser: &FakeBrowser) -> Vec<String> {
            browser
                .exchanges()
                .iter()
                .filter(|exchange| exchange.method() == "Runtime.releaseObjectGroup")
                .filter_map(|exchange| {
                    exchange.command["params"]["objectGroup"]
                        .as_str()
                        .map(str::to_string)
                })
                .collect()
        }

        let browser = FakeBrowser::start(Replies::default()).await;
        let tasks = TaskTracker::new();
        let closed = CancellationToken::new();
        let page = tracked_page(browser.cdp().await, &tasks, &closed).await;
        let tab = FrameTab {
            page: Arc::clone(&page),
            scope: FrameScope::Main,
        };
        let frame = tab.context().await.expect("the main frame");
        drop(tab.group(&frame, "open"));
        time::timeout(Duration::from_secs(5), async {
            while released(&browser).is_empty() {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the open tab's group is released");
        closed.cancel();
        drop(tab.group(&frame, "gone"));
        tasks.close();
        time::timeout(Duration::from_secs(5), tasks.wait())
            .await
            .expect("the tracker drains");
        let groups = released(&browser);
        assert_eq!(groups.len(), 1, "{groups:?}");
        assert!(groups[0].starts_with("neo-open-"), "{groups:?}");
    }

    async fn assert_seed_keeps_watcher_open(
        initial_url: impl Future<Output = Result<Option<String>, CdpError>>,
    ) {
        use crate::fake_browser::{FakeBrowser, Replies};

        let browser = FakeBrowser::start(Replies::default()).await;
        let cdp = browser.cdp().await;
        let (events, receiver) = broadcast::channel(16);
        let (state, mut current) = watch::channel(PageState::default());
        let (navigations, _) = broadcast::channel(16);
        let closed = CancellationToken::new();
        let stop = CancellationToken::new();
        let watcher = watch_events(
            receiver,
            cdp.clone(),
            SessionId::from(SESSION),
            MAIN.to_string(),
            PageUpdates {
                state,
                navigations,
                world: Arc::new(World::new(cdp, SessionId::from(SESSION), MAIN.to_string())),
                network: Arc::new(NetworkLog::new(MAIN.to_string())),
                frames: None,
                feeds: None,
                initial_url,
                producer: None,
                idle: None,
                routes: None,
            },
            closed.clone(),
            stop.clone(),
        );
        time::timeout(Duration::from_secs(1), async {
            tokio::join!(watcher, async {
                time::sleep(Duration::from_millis(10)).await;
                assert!(!closed.is_cancelled(), "a seed cannot close an open tab");
                assert!(current.borrow().committed_url.is_none());
                events
                    .send(event(
                        "Page.frameNavigated",
                        Some(SESSION),
                        json!({
                            "frame": {"id": MAIN, "url": "https://a.test/later", "loaderId": "new"}
                        }),
                    ))
                    .expect("watcher still subscribed");
                current
                    .wait_for(|state| {
                        state.committed_url.as_deref() == Some("https://a.test/later")
                    })
                    .await
                    .expect("watcher still applies commits");
                assert!(!closed.is_cancelled());
                stop.cancel();
            });
        })
        .await
        .expect("seed paths and stop must be bounded");
    }

    #[tokio::test]
    async fn tests_that_failed_or_empty_seeds_do_not_close_the_tab() {
        for result in [Ok(None), Err(CdpError::Json("seed failed".to_string()))] {
            assert_seed_keeps_watcher_open(future::ready(result)).await;
        }
    }

    #[tokio::test]
    async fn tests_that_the_seed_uses_frame_tree_urls_including_error_pages_and_fragments() {
        for (url, fragment, expected) in [
            ("https://a.test/", "#part", Some("https://a.test/#part")),
            (
                "chrome-error://chromewebdata/",
                "",
                Some("chrome-error://chromewebdata/"),
            ),
            ("", "", None),
        ] {
            let (enabled, ready) = oneshot::channel();
            enabled.send(()).expect("enable completed");
            let result = time::timeout(
                Duration::from_secs(1),
                seed_url(ready, |method| {
                    assert_eq!(method, "Page.getFrameTree");
                    future::ready(Ok(json!({"frameTree":{"frame":{
                        "id": MAIN, "url": url, "urlFragment": fragment,
                        "unreachableUrl": "http://127.0.0.1:9/unreachable"
                    }}})))
                }),
            )
            .await
            .expect("bounded seed")
            .expect("frame tree");
            assert_eq!(result.as_deref(), expected);
        }
    }

    #[tokio::test]
    async fn tests_that_abandoning_prepare_never_closes_the_tab_or_queries_a_url() {
        use std::sync::atomic::AtomicBool;

        let (enabled, ready) = oneshot::channel::<()>();
        let queried = AtomicBool::new(false);
        drop(enabled);
        let seed = seed_url(ready, |_| {
            queried.store(true, Ordering::Relaxed);
            future::ready(Ok(json!({})))
        });
        assert_seed_keeps_watcher_open(seed).await;
        assert!(!queried.load(Ordering::Relaxed));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_wait_observations_drain_commits_before_state_and_recover_lag() {
        let (state, receiver) = watch::channel(PageState::default());
        let (commits, queue) = broadcast::channel(2);
        let mut events = NavigationObservation {
            navigations: queue,
            state: receiver,
            recover_lag: true,
        };
        for index in 0..3 {
            let url = format!("https://a.test/{index}");
            state.send_modify(|state| state.committed_url = Some(url.clone()));
            commits
                .send(Navigation {
                    document: Some(url.clone()),
                    url,
                    lifecycle: Lifecycle::default(),
                    request: true,
                    error: None,
                })
                .expect("subscribed");
        }
        assert_eq!(
            events.next().await,
            Ok(None),
            "lag asks the wait to recheck current state"
        );
        assert_eq!(events.state().url.as_deref(), Some("https://a.test/2"));
        assert_eq!(
            events.next().await.expect("commit").expect("queued").url,
            "https://a.test/1"
        );
        assert_eq!(
            events.next().await.expect("commit").expect("queued").url,
            "https://a.test/2"
        );
        assert_eq!(
            events.next().await,
            Ok(None),
            "coalesced state follows the queue"
        );
        drop(events);
        assert_eq!(commits.receiver_count(), 0);
        assert_eq!(state.receiver_count(), 0);
    }

    #[test]
    fn dialogs_are_answered_as_playwright_does_without_a_listener() {
        let mut state = PageState::default();
        let opening = |kind: &str| {
            event(
                "Page.javascriptDialogOpening",
                Some(SESSION),
                json!({ "url": "", "message": "x", "type": kind, "hasBrowserHandler": false }),
            )
        };
        assert_eq!(
            applied(&mut state, &opening("alert")).dialog,
            Some(DialogAnswer::Dismiss)
        );
        assert_eq!(
            applied(&mut state, &opening("beforeunload")).dialog,
            Some(DialogAnswer::Accept)
        );
        assert_eq!(state.dialogs_opened, 2);
    }

    #[test]
    fn the_session_closes_on_its_own_detach_only() {
        let mut state = PageState::default();
        let detached = |session: &str| {
            event(
                "Target.detachedFromTarget",
                None,
                json!({ "sessionId": session, "targetId": MAIN }),
            )
        };
        assert!(!applied(&mut state, &detached("S2")).closed);
        assert!(applied(&mut state, &detached(SESSION)).closed);
        let crashed = event("Inspector.targetCrashed", Some(SESSION), json!({}));
        assert!(applied(&mut state, &crashed).closed);
    }

    #[test]
    fn the_barrier_releases_every_waiter_on_one_commit() {
        let session = SessionId::from(SESSION);
        let mut barrier = Barrier::default();
        let requested = event(
            "Page.frameRequestedNavigation",
            Some(SESSION),
            json!({ "frameId": MAIN, "url": "https://a.test/b", "disposition": "currentTab" }),
        );
        barrier.apply(&requested, &session, MAIN);
        barrier.apply(&requested, &session, MAIN);
        assert_eq!(barrier.waiting, 2);
        let navigated = event(
            "Page.frameNavigated",
            Some(SESSION),
            json!({ "frame": { "id": MAIN, "url": "https://a.test/b" } }),
        );
        barrier.apply(&navigated, &session, MAIN);
        assert_eq!(barrier.waiting, 0);
        assert_eq!(barrier.committed, ["https://a.test/b", "https://a.test/b"]);

        barrier.apply(&requested, &session, MAIN);
        let stopped = event(
            "Page.frameStoppedLoading",
            Some(SESSION),
            json!({ "frameId": MAIN }),
        );
        barrier.apply(&stopped, &session, MAIN);
        assert_eq!(
            barrier.waiting, 0,
            "a request that stopped without committing"
        );
    }

    #[derive(Clone)]
    struct ReviewDelayedInput {
        events: Arc<SyncMutex<Vec<String>>>,
        first: Arc<AtomicBool>,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }

    impl ReviewDelayedInput {
        fn new(gated: bool) -> Self {
            Self {
                events: Arc::default(),
                first: Arc::new(AtomicBool::new(gated)),
                entered: Arc::default(),
                release: Arc::default(),
            }
        }

        fn events(&self) -> Vec<String> {
            self.events.lock().expect("events").clone()
        }
    }

    impl InputSender for ReviewDelayedInput {
        async fn send_input(&self, _method: &'static str, params: String) -> Result<(), CdpError> {
            let value: Value = serde_json::from_str(&params).expect("input params");
            self.events.lock().expect("events").push(
                value["type"]
                    .as_str()
                    .expect("input event type")
                    .to_string(),
            );
            if self.first.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            Ok(())
        }
    }

    // The actual input turn and writer, with just one CDP acknowledgement held until
    // after the run's cleanup has marked its owner and exhausted its turn-wait budget.
    async fn review_end_before_late_input(restore: bool) {
        use crate::keyboard::{Platform, SHIFT};

        let tab = Arc::new(TabInput::default());
        let owner_a = SessionId("review-A".to_string());
        let owner_b = SessionId("review-B".to_string());
        let a = InputPermit::take(&tab, owner_a.clone()).await;
        if restore {
            a.apply(
                &Effect::KeyDown {
                    code: "ShiftLeft",
                    modifier: SHIFT,
                    owner: owner_a.clone(),
                },
                true,
            );
        }
        let mut simulated = a.snapshot().expect("initial devices");
        let mut plan = Plan::default();
        if restore {
            let changes = keyboard::plan_ensure_modifiers(
                &mut plan,
                &mut simulated,
                0,
                Platform::Linux,
                &owner_a,
            );
            keyboard::plan_restore_modifiers(
                &mut plan,
                &mut simulated,
                &changes,
                Platform::Linux,
                &owner_a,
            );
        } else {
            keyboard::plan_key_down(
                &mut plan,
                &mut simulated,
                "Shift",
                Platform::Linux,
                &owner_a,
            )
            .expect("Shift");
        }
        let sender = ReviewDelayedInput::new(true);
        let writer_sender = sender.clone();
        let stopped = CancellationToken::new();
        let writer_stop = stopped.clone();
        let writer = tokio::spawn(async move {
            let result = input_devices::write_plan(&writer_sender, &a, &plan, &writer_stop).await;
            drop(a);
            result
        });
        sender.entered.notified().await;

        // release_run_input's mark happens before its wait. The writer still owns
        // the turn, so the bounded release never gets to build a cleanup plan.
        tab.mark_ended(&owner_a);
        stopped.cancel();
        assert!(
            time::timeout(Duration::from_millis(1), InputPermit::take(&tab, owner_a),)
                .await
                .is_err()
        );
        sender.release.notify_one();
        let written = writer.await.expect("writer task");
        assert!(written.outcomes.iter().all(Result::is_ok));
        assert_eq!(
            sender.events(),
            if restore {
                vec!["keyUp", "rawKeyDown"]
            } else {
                vec!["rawKeyDown"]
            },
            "the already-sent input, and any owed restore, were acknowledged after the mark",
        );

        review_b_presses_shift_and_ends(&tab, owner_b).await;
    }

    // The second half of `review_end_before_late_input`, unchanged: B presses Shift and ends.
    async fn review_b_presses_shift_and_ends(tab: &Arc<TabInput>, owner_b: SessionId) {
        use crate::input_devices::KeyboardState;
        use crate::keyboard::Platform;

        let b = InputPermit::take(tab, owner_b.clone()).await;
        let mut simulated = b.snapshot().expect("devices after late input");
        let mut down_b = Plan::default();
        keyboard::plan_key_down(
            &mut down_b,
            &mut simulated,
            "Shift",
            Platform::Linux,
            &owner_b,
        )
        .expect("B's Shift");
        let immediate = ReviewDelayedInput::new(false);
        let _down =
            input_devices::write_plan(&immediate, &b, &down_b, &CancellationToken::new()).await;
        let mut simulated = b.snapshot().expect("B's devices");
        let mut end_b = Plan::default();
        keyboard::plan_key_releases(&mut end_b, &mut simulated, &owner_b);
        input_devices::plan_button_releases(&mut end_b, &mut simulated, &owner_b);
        let releasing = ReviewDelayedInput::new(false);
        let _end =
            input_devices::write_plan(&releasing, &b, &end_b, &CancellationToken::new()).await;
        assert_eq!(
            releasing.events(),
            ["keyUp"],
            "A ended before its final input was acknowledged; B must not preserve it as a live holder: {:?}",
            b.devices(),
        );
        assert_eq!(b.devices().keyboard, KeyboardState::default());
    }

    #[tokio::test(start_paused = true)]
    async fn review_recheck3_late_down_keeps_its_finished_owner_retired() {
        review_end_before_late_input(false).await;
    }

    #[tokio::test(start_paused = true)]
    async fn review_recheck3_owed_restore_keeps_its_finished_owner_retired() {
        review_end_before_late_input(true).await;
    }

    /// A run whose end is recorded writes nothing new, only what it owes: a call that gets the
    /// turn after its run's end, though not cut short, sends no press, and the run's releases
    /// still go out.
    #[tokio::test(start_paused = true)]
    async fn tests_that_an_ended_runs_writer_sends_only_what_it_owes() {
        use crate::input_devices::KeyboardState;
        use crate::keyboard::{Platform, SHIFT};

        let tab = Arc::new(TabInput::default());
        let a = SessionId("A".to_string());
        let permit = InputPermit::take(&tab, a.clone()).await;
        permit.apply(
            &Effect::KeyDown {
                code: "ShiftLeft",
                modifier: SHIFT,
                owner: a.clone(),
            },
            true,
        );
        tab.mark_ended(&a);
        let mut simulated = permit.snapshot().expect("devices");
        let mut press = Plan::default();
        keyboard::plan_key_down(&mut press, &mut simulated, "a", Platform::Linux, &a).expect("a");
        let sender = ReviewDelayedInput::new(false);
        let never = CancellationToken::new();
        let _pressed = input_devices::write_plan(&sender, &permit, &press, &never).await;
        assert!(sender.events().is_empty(), "an ended run presses nothing");
        let mut simulated = permit.snapshot().expect("devices");
        let mut end = Plan::default();
        keyboard::plan_key_releases(&mut end, &mut simulated, &a);
        let _released = input_devices::write_plan(&sender, &permit, &end, &never).await;
        assert_eq!(sender.events(), ["keyUp"], "its release still goes out");
        assert_eq!(permit.devices().keyboard, KeyboardState::default());
    }

    // Seeded random interleavings of two runs' input and ends (A2 re-check 3): the real turn,
    // writer and run-end releases, with a browser that receives, holds back, fails or refuses
    // each write.

    /// `SplitMix64`, so each case replays one schedule.
    struct Seeded(u64);

    impl Seeded {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }

        fn index(&mut self, len: usize) -> usize {
            usize::try_from(self.below(len as u64)).expect("index")
        }
    }

    /// How the browser answers one write.
    #[derive(Clone, Copy)]
    enum Answer {
        Now,
        /// Received at once, answered when the schedule lets it go.
        HeldBack,
        /// Answered with an error, received or not.
        Fails {
            received: bool,
        },
        /// The session's socket is gone: never received.
        Refused,
    }

    /// The browser's side of one case.
    struct FuzzBrowser {
        /// Each key code or button that is down, and the runs whose writes pressed it since it
        /// last went up.
        down: HashMap<String, Vec<Owner>>,
        /// The runs whose end has not come.
        live: Vec<Owner>,
        /// Every release a run's end wrote while another live run had it pressed.
        stolen: Vec<String>,
        /// What the current call released, and who had it pressed: a press of it in the same
        /// call (a modifier's restore) is theirs again.
        released: HashMap<String, Vec<Owner>>,
        held_back: Vec<oneshot::Sender<()>>,
        /// Whether an intercepted HTML5 drag runs: a move with the left button down may start
        /// one; a drop or a cancel ends it.
        dragging: bool,
        /// Once set, every write is answered at once.
        settled: bool,
        answers: Seeded,
    }

    impl FuzzBrowser {
        fn answer(&mut self) -> Answer {
            if self.settled {
                return Answer::Now;
            }
            match self.answers.below(10) {
                0..=4 => Answer::Now,
                5 | 6 => Answer::HeldBack,
                7 => Answer::Fails { received: true },
                8 => Answer::Fails { received: false },
                _ => Answer::Refused,
            }
        }

        /// One write: what the browser does with it if `received`, and what this call released
        /// whether received or not, so its restore goes back to whoever had the key.
        fn write(
            &mut self,
            method: &str,
            event: &Value,
            writer: &Owner,
            run_end: bool,
            received: bool,
        ) {
            if method == "Input.dispatchDragEvent" {
                let ends = matches!(event["type"].as_str(), Some("drop" | "dragCancel"));
                if received && ends {
                    self.dragging = false;
                    // The drag held the left button, and a drop stands in for whichever
                    // button's release made it (Playwright's `RawMouseImpl.up`): after it the
                    // events carry the tab's own mask, so no button stays down.
                    let pressers = self.down.remove("left").unwrap_or_default();
                    self.down.remove("right");
                    let live = pressers
                        .iter()
                        .find(|presser| *presser != writer && self.live.contains(presser));
                    if let Some(live) = live
                        && run_end
                    {
                        self.stolen
                            .push(format!("{}'s end ended a drag under {}", writer.0, live.0));
                    }
                }
                return;
            }
            let what = event["code"]
                .as_str()
                .or_else(|| event["button"].as_str())
                .unwrap_or_default()
                .to_string();
            match event["type"].as_str().unwrap_or_default() {
                "keyDown" | "rawKeyDown" | "mousePressed" => {
                    // A press of what this call released is its restore, for whoever had it.
                    let restored = self.released.remove(&what);
                    if received {
                        let pressers = self.down.entry(what).or_default();
                        for presser in restored.unwrap_or_else(|| vec![writer.clone()]) {
                            if !pressers.contains(&presser) {
                                pressers.push(presser);
                            }
                        }
                    }
                }
                "keyUp" | "mouseReleased" => {
                    let pressers = self.down.get(&what).cloned().unwrap_or_default();
                    if !run_end {
                        self.released.insert(what.clone(), pressers.clone());
                    }
                    if received {
                        self.down.remove(&what);
                        let live = pressers
                            .iter()
                            .find(|presser| *presser != writer && self.live.contains(presser));
                        if let Some(live) = live
                            && run_end
                        {
                            self.stolen.push(format!(
                                "{}'s end released {what} under {}",
                                writer.0, live.0
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// One writer's way to the browser: whose writes, and whether they are a run end's.
    struct FuzzSender {
        browser: Arc<SyncMutex<FuzzBrowser>>,
        writer: Owner,
        run_end: bool,
        /// Set once a write of this sender failed or was refused.
        failed: Arc<AtomicBool>,
    }

    impl InputSender for FuzzSender {
        async fn send_input(&self, method: &'static str, params: String) -> Result<(), CdpError> {
            let event: Value = serde_json::from_str(&params).expect("input params");
            let (answer, held_back) = {
                let mut browser = self.browser.lock().expect("browser");
                let answer = browser.answer();
                let received =
                    !matches!(answer, Answer::Refused | Answer::Fails { received: false });
                if matches!(answer, Answer::Refused | Answer::Fails { .. }) {
                    self.failed.store(true, Ordering::SeqCst);
                }
                browser.write(method, &event, &self.writer, self.run_end, received);
                let held_back = matches!(answer, Answer::HeldBack).then(|| {
                    let (release, wait) = oneshot::channel();
                    browser.held_back.push(release);
                    wait
                });
                (answer, held_back)
            };
            if let Some(wait) = held_back {
                let _released = wait.await;
            }
            match answer {
                Answer::Now | Answer::HeldBack => Ok(()),
                Answer::Fails { .. } => Err(CdpError::Timeout {
                    method: method.to_string(),
                }),
                Answer::Refused => Err(CdpError::ConnectionEpochChanged {
                    expected: 1,
                    actual: 2,
                }),
            }
        }

        /// The move, and then, with the left button down and no drag running, a drag the page
        /// starts three times in four, which Chromium hands over (`Input.dragIntercepted`).
        async fn move_intercepting_drag(&self, params: String) -> Result<Option<Value>, CdpError> {
            self.send_input("Input.dispatchMouseEvent", params).await?;
            let mut browser = self.browser.lock().expect("browser");
            let left_down = browser
                .down
                .get("left")
                .is_some_and(|pressers| !pressers.is_empty());
            if left_down && !browser.dragging && browser.answers.below(4) != 0 {
                browser.dragging = true;
                return Ok(Some(json!({ "items": [], "dragOperationsMask": 1 })));
            }
            Ok(None)
        }
    }

    /// One call of a run's script.
    #[derive(Clone, Copy, Debug)]
    enum FuzzCall {
        Down(&'static str),
        Up(&'static str),
        /// A click's modifiers with no click: Playwright's `_ensureModifiers`, then the restore.
        Modifiers(u8),
        /// A click with modifiers: the change, a left press and its release, the restore.
        Click(u8),
        /// `mouse.down` and `mouse.up` with the left button, or the right one.
        Press(bool),
        Release(bool),
        /// A move that is not a click's, through the drag manager.
        Move((f64, f64), u32),
    }

    impl FuzzCall {
        fn random(rng: &mut Seeded) -> Self {
            use crate::keyboard::{CONTROL, SHIFT};
            const KEYS: [&str; 4] = ["Shift", "Control", "a", "Escape"];
            const MASKS: [u8; 4] = [0, SHIFT, CONTROL, SHIFT | CONTROL];
            const POINTS: [(f64, f64); 3] = [(10.0, 10.0), (60.0, 40.0), (250.0, 60.0)];
            // Presses and moves weigh more, so that drags start and meet the other calls.
            match rng.below(10) {
                0 => Self::Down(KEYS[rng.index(KEYS.len())]),
                1 => Self::Up(KEYS[rng.index(KEYS.len())]),
                2 => Self::Modifiers(MASKS[rng.index(MASKS.len())]),
                3 => Self::Click(MASKS[rng.index(MASKS.len())]),
                4 | 5 => Self::Press(rng.below(3) == 0),
                6 => Self::Release(rng.below(3) == 0),
                _ => Self::Move(
                    POINTS[rng.index(POINTS.len())],
                    u32::try_from(1 + rng.below(3)).expect("steps"),
                ),
            }
        }

        fn plan(self, plan: &mut Plan, simulated: &mut InputDevices, owner: &Owner) {
            use crate::click::MouseButton;
            use crate::keyboard::Platform;
            let linux = Platform::Linux;
            match self {
                Self::Down(key) => {
                    keyboard::plan_key_down(plan, simulated, key, linux, owner).expect("key");
                }
                Self::Up(key) => {
                    keyboard::plan_key_up(plan, simulated, key, linux, None).expect("key");
                }
                Self::Modifiers(target) => {
                    let changes =
                        keyboard::plan_ensure_modifiers(plan, simulated, target, linux, owner);
                    keyboard::plan_restore_modifiers(plan, simulated, &changes, linux, owner);
                }
                Self::Click(target) => {
                    let changes =
                        keyboard::plan_ensure_modifiers(plan, simulated, target, linux, owner);
                    let down =
                        input_devices::plan_down(plan, simulated, MouseButton::Left, 1, owner);
                    input_devices::plan_up(plan, simulated, MouseButton::Left, 1, Some(down));
                    keyboard::plan_restore_modifiers(plan, simulated, &changes, linux, owner);
                }
                Self::Press(right) => {
                    let button = if right {
                        MouseButton::Right
                    } else {
                        MouseButton::Left
                    };
                    input_devices::plan_down(plan, simulated, button, 1, owner);
                }
                Self::Release(right) => {
                    let button = if right {
                        MouseButton::Right
                    } else {
                        MouseButton::Left
                    };
                    input_devices::plan_up(plan, simulated, button, 1, None);
                }
                Self::Move(to, steps) => input_devices::plan_free_move(plan, simulated, to, steps),
            }
        }
    }

    /// One case: two runs and an observer (`case.tab`, which keeps the tab's devices) on one
    /// tab.
    struct FuzzCase {
        tab: Arc<TabInput>,
        browser: Arc<SyncMutex<FuzzBrowser>>,
        /// Each run's calls are cut short at its end, as `RunScope::ended` does.
        stops: [CancellationToken; 2],
        ended: [bool; 2],
        tasks: Vec<JoinHandle<()>>,
    }

    fn fuzz_owner(run: usize) -> Owner {
        SessionId(["fuzz-A", "fuzz-B", "fuzz-observer"][run].to_string())
    }

    impl FuzzCase {
        fn new(seed: u64) -> Self {
            Self {
                tab: Arc::new(TabInput::default()),
                browser: Arc::new(SyncMutex::new(FuzzBrowser {
                    down: HashMap::new(),
                    live: (0..3).map(fuzz_owner).collect(),
                    stolen: Vec::new(),
                    released: HashMap::new(),
                    held_back: Vec::new(),
                    dragging: false,
                    settled: false,
                    answers: Seeded(seed ^ 0x5EED_5EED),
                })),
                stops: [CancellationToken::new(), CancellationToken::new()],
                ended: [false; 2],
                tasks: Vec::new(),
            }
        }

        fn sender(&self, run: usize, run_end: bool) -> FuzzSender {
            FuzzSender {
                browser: Arc::clone(&self.browser),
                writer: fuzz_owner(run),
                run_end,
                failed: Arc::default(),
            }
        }

        /// A call: a turn, a plan from the tab's devices, the writer.
        fn call(&mut self, run: usize, call: FuzzCall) {
            let (tab, owner, stop) = (
                Arc::clone(&self.tab),
                fuzz_owner(run),
                self.stops[run].clone(),
            );
            let sender = self.sender(run, false);
            self.tasks.push(tokio::spawn(async move {
                let permit = InputPermit::take(&tab, owner.clone()).await;
                let mut simulated = permit.snapshot().expect("devices");
                let mut plan = Plan::default();
                call.plan(&mut plan, &mut simulated, &owner);
                let written = input_devices::write_plan(&sender, &permit, &plan, &stop).await;
                let mut browser = sender.browser.lock().expect("browser");
                browser.released.clear();
                // A call whose every write was answered resolves, a drag's skipped press
                // included, unless its run ended under it.
                if !written.complete
                    && !sender.failed.load(Ordering::SeqCst)
                    && !stop.is_cancelled()
                {
                    browser.stolen.push(format!(
                        "{}'s {call:?} read incomplete, every write answered",
                        owner.0
                    ));
                }
            }));
        }

        /// A run's end as `Dispatcher::finish` makes it: the end recorded on the tab, the run's
        /// calls cut short, its releases under a turn (`write_run_releases`), the wait for the
        /// turn and the writes bounded by `budget`.
        fn end(&mut self, run: usize, budget: Option<Duration>) {
            let owner = fuzz_owner(run);
            self.browser
                .lock()
                .expect("browser")
                .live
                .retain(|live| *live != owner);
            self.tab.mark_ended(&owner);
            if let Some(stop) = self.stops.get(run) {
                stop.cancel();
                self.ended[run] = true;
            }
            let (tab, sender) = (Arc::clone(&self.tab), self.sender(run, true));
            self.tasks.push(tokio::spawn(async move {
                let release = async {
                    let permit = InputPermit::take(&tab, owner).await;
                    write_run_releases(&sender, &permit).await;
                };
                match budget {
                    Some(budget) => {
                        let _released = time::timeout(budget, release).await;
                    }
                    None => release.await,
                }
            }));
        }

        /// A run whose future is dropped: no run end, only `PageSession`'s drop, which records
        /// the end once the run's calls are gone.
        fn drop_run(&mut self, run: usize) {
            let owner = fuzz_owner(run);
            self.browser
                .lock()
                .expect("browser")
                .live
                .retain(|live| *live != owner);
            self.stops[run].cancel();
            self.ended[run] = true;
            self.tab.mark_ended(&owner);
        }

        /// Lets one held-back answer go.
        fn let_go(&mut self, rng: &mut Seeded) {
            let mut browser = self.browser.lock().expect("browser");
            if !browser.held_back.is_empty() {
                let at = rng.index(browser.held_back.len());
                let _answered = browser.held_back.swap_remove(at).send(());
            }
        }

        /// Answers everything at once from now on, until every task is done; then the runs
        /// still live end, and the observer last, nothing bounded.
        async fn settle(&mut self) {
            self.browser.lock().expect("browser").settled = true;
            for _ in 0..10_000 {
                let held_back = mem::take(&mut self.browser.lock().expect("browser").held_back);
                for release in held_back {
                    let _answered = release.send(());
                }
                if self.tasks.iter().all(JoinHandle::is_finished) {
                    break;
                }
                time::sleep(Duration::from_millis(1)).await;
            }
            for task in mem::take(&mut self.tasks) {
                task.await.expect("a call or a release panicked");
            }
            for run in 0..2 {
                if !self.ended[run] {
                    self.end(run, None);
                }
            }
            self.end(2, None);
            for task in mem::take(&mut self.tasks) {
                task.await.expect("a release panicked");
            }
        }
    }

    async fn fuzz_case(seed: u64) {
        let mut rng = Seeded(seed);
        let mut case = FuzzCase::new(seed);
        for _ in 0..4 + rng.below(20) {
            let run = rng.index(2);
            match rng.below(10) {
                0..=5 if !case.ended[run] => case.call(run, FuzzCall::random(&mut rng)),
                6 if !case.ended[run] => match rng.below(4) {
                    0 => case.drop_run(run),
                    1 => case.end(run, None),
                    _ => case.end(run, Some(Duration::from_millis(rng.below(3)))),
                },
                7 | 8 => case.let_go(&mut rng),
                _ => {}
            }
            time::sleep(Duration::from_millis(rng.below(3))).await;
        }
        case.settle().await;
        let browser = case.browser.lock().expect("browser");
        assert!(
            browser.stolen.is_empty(),
            "seed {seed}: {:?}",
            browser.stolen
        );
        assert!(
            browser.down.is_empty(),
            "seed {seed}: still down {:?}",
            browser.down
        );
        assert!(!browser.dragging, "seed {seed}: a drag still runs");
        let devices = case.tab.devices.lock().expect("devices").clone();
        assert_eq!(
            devices,
            InputDevices {
                mouse: devices.mouse.clone(),
                ..InputDevices::default()
            },
            "seed {seed}"
        );
        assert!(devices.mouse.buttons.is_empty(), "seed {seed}: {devices:?}");
    }

    /// Two runs and an observer on one tab, 3000 seeded schedules of key presses and releases
    /// (Escape included), modifier changes, clicks with modifiers, button presses, free moves
    /// that may start an intercepted HTML5 drag, run ends (some bounded, cut off waiting for
    /// the turn or mid-write, some dropped with no release at all), with each write answered at
    /// once, held back, failed (received or not) or refused. A call whose every write was
    /// answered reads complete, a drag's skipped press included; no run's end ends a drag
    /// another live run has the left button down for. Once every answer is in and every run has ended, nothing
    /// is down in the browser or held in the tab's devices, and no run's end ever released what
    /// a live run had pressed.
    #[tokio::test(start_paused = true)]
    async fn tests_that_random_interleavings_of_input_and_run_ends_leave_nothing_held() {
        for seed in 0..3000 {
            fuzz_case(seed).await;
        }
    }
}
