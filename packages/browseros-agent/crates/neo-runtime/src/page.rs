//! One handed-out tab as the server side of a browser call sees it (Playwright's `server/page.ts`
//! with `chromium/crPage.ts` and `crInput.ts`): this tab's CDP session,
//! the domains a browser call needs, one watcher task that answers dialogs and tracks
//! navigations, the tab's input turns ([`TabInputs`]), the pointer writers, and the `click::Tab`
//! implementation over all of it and the utility world (`world.rs`).
//!
//! This file is mechanism; `click.rs` and `navigate.rs` own policy. Nothing here writes a
//! call-log line or decides a retry, and policy names no CDP types.
//!
//! Lifetime: [`PageSession`] owns this tab's [`Session`], whose `Drop` detaches, so the session
//! detaches when the last `Arc<PageSession>` drops. Every task that writes on the session holds
//! one: the dispatcher's call tasks, the interceptor setups, the pointer writers, the interceptor
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

use browseros_cdp::input::DispatchMouseEventParams;
use browseros_cdp::{CdpClient, CdpError, CdpEvent, SessionId, dom, target};
use futures_util::future;
use neo_runtime_spec::injected::{
    ElementState, ElementStatesOutcome, FrameStyle, FrameStyleStatus, HitTargetAction,
    HitTargetInterceptor, HitTargetOutcome, HitTargetSetup, InjectedCheckElementStatesParams,
    InjectedDescribeIFrameStyleParams, InjectedDescribeSelectorParams,
    InjectedExpectHitTargetParams, InjectedGetElementBorderWidthParams, InjectedResolveParams,
    InjectedScrollIntoViewParams, InjectedSetupHitTargetInterceptorParams,
    InjectedStopHitTargetInterceptorParams, InjectedTitleParams, InjectedViewportSizeParams, Point,
    ScrollAlignment,
};
use neo_runtime_spec::serve::ResponseChannel;
use neo_runtime_spec::{Element, Handle};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{Mutex, OnceCell, OwnedMutexGuard, broadcast, oneshot, watch};
use tokio::time;
use tokio_util::sync::{CancellationToken, DropGuard};
use tokio_util::task::TaskTracker;
use tracing::{debug, warn};

use crate::click::{
    Arming, Geometry, MouseButton, MouseClick, NotReady, PageError, PointerInput, PressReport,
    Resolved, Scroll, SelectorFacts, StatesCheck, Tab, Verdict,
};
use crate::events::{EventHub, Flushes};
use crate::frame_selector::{self, FrameOwner};
use crate::frames::{FrameContext, FrameScope, Frames, Host, local_tree};
use crate::keyboard::{self, InputSender};
use crate::navigate::{
    Lifecycle, NavigateReply, Navigation, NavigationState, NavigationTab, Observation,
};
use crate::network::{Lookup, NetworkLog};
use crate::page_events::PageEvents;
use crate::progress::InputTicket;
use crate::progress::PlaywrightError;
use crate::run::PageId;
use crate::world::{self, ObjectGroup, World};

/// Detaching is best effort. It runs in the background after a run, so this bound only keeps
/// a hung browser from piling up detach tasks.
const DETACH_TIMEOUT: Duration = Duration::from_secs(1);

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
    /// The tab's input lock, shared with every other run of the `Runtime` on this tab
    /// ([`TabInputs`]). [`Tab::input`] waits for a turn at it.
    input: Arc<Mutex<()>>,
    /// The main frame's navigation requests, fed by the watcher once Network is enabled.
    pub(crate) network: Arc<NetworkLog>,
    /// `Network.enable`, sent on this session's first goto and kept for the run.
    network_enabled: OnceCell<()>,
    /// What the watcher turns into the run's events, and the tab's console history and dialogs.
    pub(crate) events: PageEvents,
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
    /// Chromium emits no load lifecycle when restoring this document from bfcache.
    restored_from_bfcache: bool,
    /// `Page.javascriptDialogOpening` events seen; each was answered.
    pub(crate) dialogs_opened: u64,
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
        }
    }

    fn did_dom_content_load(&mut self) {
        if !self.dom_content_loaded {
            self.dom_content_loaded = true;
            self.dom_content_loaded_count += 1;
        }
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
    /// main frame, whose events the tab's own [`PageState`] takes instead.
    fn apply(&self, event: &CdpEvent, session: &SessionId, main_frame: Option<&str>) {
        if event.session_id.as_ref() != Some(session) {
            return;
        }
        let Some(frame) = event_frame(event) else {
            return;
        };
        if Some(frame) == main_frame {
            return;
        }
        let Ok(mut frames) = self.frames.lock() else {
            return;
        };
        let feed = frames.entry(frame.to_string()).or_insert_with(new_feed);
        let mut navigation = None;
        let mut removed = false;
        feed.state.send_if_modified(|state| {
            if event.method == "Page.frameDetached" {
                if event.params["reason"] == "swap" {
                    // The frame's next document lives in another process: forget this one so
                    // that process's lifecycle replay seeds the new document.
                    let page = &mut state.page;
                    page.committed_url = None;
                    page.pending_url = None;
                    page.document = None;
                    page.dom_content_loaded = false;
                    page.load = false;
                    page.restored_from_bfcache = false;
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
            let applied = apply_frame(&mut state.page, event, frame);
            navigation = applied.navigation;
            applied.changed
        });
        if let Some(navigation) = navigation {
            let _listeners = feed.navigations.send(navigation);
        }
        if removed {
            // Waits keep their receivers and read `detached`; the frame id is never reused.
            frames.remove(frame);
        }
    }
}

impl FrameFeeds {
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
fn event_frame(event: &CdpEvent) -> Option<&str> {
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
}

impl PageSession {
    /// `input` is the tab's lock from the `Runtime`'s [`TabInputs`].
    pub(crate) fn new(
        session: Session,
        page: &PageId,
        tasks: TaskTracker,
        input: Arc<Mutex<()>>,
        closed: CancellationToken,
        hub: EventHub,
    ) -> Self {
        let events = PageEvents::new(hub, page.as_str(), session.cdp.clone(), session.id.clone());
        let child_events = events.clone();
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
        // An out-of-process frame's console is this tab's: it turns on when the frame's session
        // is attached, if the run wants the tab's console by then.
        frames.on_child_host(Arc::new(move |host| {
            let events = child_events.clone();
            Box::pin(async move { events.child_attached(&host).await })
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
            network: Arc::new(NetworkLog::new(page.as_str().to_string())),
            network_enabled: OnceCell::new(),
            events,
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
                let initial_url = seed_url(ready, move |method| async move {
                    cdp.send(method, json!({}), Some(&session)).await
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
                    },
                    self.closed.clone(),
                    stop,
                ));
                self.send("Page.enable", json!({})).await?;
                // Chromium replays the existing document's lifecycle here (navigation_e2e),
                // including pages loaded before this run and restored from bfcache.
                self.send("Page.setLifecycleEventsEnabled", json!({ "enabled": true }))
                    .await?;
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

    pub(crate) async fn send(&self, method: &str, params: Value) -> Result<Value, CdpError> {
        self.session
            .cdp
            .send(method, params, Some(&self.session.id))
            .await
    }

    /// Runs a cleanup or writer task on the run's tracker. Spawned on the session's runtime
    /// handle because it may be called from a `Drop`.
    pub(crate) fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        self.tasks.spawn_on(task, &self.session.tokio);
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
        self.network_enabled
            .get_or_try_init(|| async { self.send("Network.enable", json!({})).await.map(|_| ()) })
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

/// One `Runtime`'s input locks, one per tab, each shared by every run the `Runtime` makes on
/// that tab. A tab has one mouse, and its document one hit-target slot that every call and every
/// Neo run on it share (`world.rs`): a setup replaces whatever interceptor is armed, and an event
/// is judged by whichever interceptor is armed when it lands. So two clicks whose input overlaps
/// judge each other's events: one click's events are blocked by the other's interceptor while
/// its own, which never saw them, reads `done`. Clicks on a tab therefore take turns
/// ([`InputPermit`]) from their
/// interceptor's setup until its stop has answered, mouse writes in between.
///
/// Entries are weak: a tab's lock lives while a run on the tab, or a task still holding a turn,
/// does. Runs of another `Runtime`, and other CDP clients, are not coordinated.
#[derive(Clone, Default)]
pub(crate) struct TabInputs(Arc<Mutex<HashMap<PageId, Weak<Mutex<()>>>>>);

impl TabInputs {
    /// The input lock of `page`, the one every live run on it shares.
    pub(crate) async fn lock_for(&self, page: &PageId) -> Arc<Mutex<()>> {
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
/// every party that operates a click's input holds a copy, and the tab's input is free for the
/// next click once the last copy drops. The click holds one from before its interceptor's setup
/// to its verdict; the setup's task until the page answers; the armed interceptor until its
/// stop has answered, `Drop`'s stop included; the pointer writer until its last event is answered.
/// So a call that stops early lets go of its own copy only, and the next click on the tab never
/// replaces an interceptor whose events may still land, nor presses into one still armed.
#[derive(Clone)]
pub(crate) struct InputPermit {
    /// Held, never read: dropping the last copy unlocks the tab's input.
    _turn: Arc<OwnedMutexGuard<()>>,
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
    };
    tokio::pin!(initial_url);
    let mut seed_pending = true;
    loop {
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
}

impl Watcher {
    /// Applies one event, or the ring's lag or end. Returns `false` once the session is gone.
    fn on(&mut self, event: Result<CdpEvent, RecvError>) -> bool {
        let session = &self.session;
        match event {
            Ok(event) => {
                // Before the Page events that follow it, so a commit never outruns its request.
                if event.method.starts_with("Network.")
                    && event.session_id.as_ref() == Some(session)
                {
                    self.network.apply(&event.method, &event.params);
                }
                if event.method == "Page.frameNavigated"
                    && event.session_id.as_ref() == Some(session)
                    && let Some(frame) = event.params["frame"]["id"].as_str()
                    && frame != self.main_frame
                    && let Some(frames) = &self.frames
                {
                    frames.committed(frame);
                }
                let mut applied = Applied::default();
                let world = &self.world;
                let main_frame = &self.main_frame;
                self.state.send_if_modified(|state| {
                    applied = apply(state, &event, session, main_frame);
                    if applied.new_document {
                        // Before notifying readers: no in-flight read may return old-document data.
                        world.document_committed();
                    }
                    applied.changed
                });
                if let Some(navigation) = applied.navigation.clone() {
                    let _listeners = self.navigations.send(navigation);
                }
                if let Some(feeds) = &self.feeds {
                    if applied.new_document {
                        feeds.detach_all();
                    }
                    feeds.apply(&event, session, Some(&self.main_frame));
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
/// same-document navigations, stops and lifecycle. The main frame's state and every child
/// frame's feed ([`FrameFeeds`]) both take it, so a child frame's document is followed exactly
/// as the main frame's is. The caller has already matched the event's session.
fn apply_frame(state: &mut PageState, event: &CdpEvent, main_frame: &str) -> Applied {
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
                state.dom_content_loaded = false;
                state.load = false;
                state.restored_from_bfcache = kind.as_deref() == Some("BackForwardCacheRestore");
                state.pending_url = None;
                applied.new_document = true;
                applied.navigation = Some(Navigation {
                    document: state.document.clone(),
                    url: state.committed_url.clone().unwrap_or_default(),
                    lifecycle: state.lifecycle(),
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
                applied.navigation = Some(Navigation {
                    document: None,
                    url,
                    lifecycle: state.lifecycle(),
                });
                applied.changed = true;
            }
        }
        "Page.frameStoppedLoading" => {
            if let Ok(FrameEvent { frame_id, .. }) = serde_json::from_value(params)
                && frame_id == main_frame
            {
                state.pending_url = None;
                // Playwright 26137face ignores stop-loading. Only bfcache needs this fallback:
                // Chromium restores a loaded document without replaying its lifecycle events.
                if state.restored_from_bfcache {
                    state.did_dom_content_load();
                    state.did_load();
                }
                applied.changed = true;
            }
        }
        "Page.lifecycleEvent" => {
            if let Ok(event) = serde_json::from_value::<LifecycleEvent>(params)
                && event.frame_id == main_frame
            {
                applied.changed = apply_lifecycle(state, &event);
            }
        }
        _ => {}
    }
    applied
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
/// object and every handle `getProperties` read out of it (probe P1).
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
        self.page.spawn(async move {
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
        InputPermit {
            _turn: Arc::new(Arc::clone(&self.input).lock_owned().await),
        }
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

    /// Spawns the action's writer ([`write_mouse`] for a move or a click, [`write_tap`] for a
    /// tap) on the run's tracker now, with an `Arc` of this page (so of the session), the ticket
    /// and the armed interceptor, and returns a future that awaits its report while holding a
    /// drop guard on the writer's `cut_short` token: dropping the future (the call stopped) cuts
    /// any remaining `delay` short, and the writer still sends the release and marks the ticket.
    /// The interceptor stays in the writer's task until the last event is acknowledged; if nobody
    /// waits for the report then, it drops there and disarms, after the page has handled the
    /// release. The writer's copy of `input` goes once the last event is answered.
    fn press(
        &self,
        point: Point,
        pointer: PointerInput,
        ticket: InputTicket,
        armed: Option<ArmedInterceptor>,
        input: InputPermit,
    ) -> impl Future<Output = (PressReport, Option<ArmedInterceptor>)> + Send {
        let cut_short = CancellationToken::new();
        let (report, receiver) = oneshot::channel();
        let page = Arc::clone(self);
        let writer_cut_short = cut_short.clone();
        let writer = async move {
            match pointer {
                PointerInput::Click(click) => {
                    let events = mouse_events(&point, &click);
                    write_mouse(page, events, click.delay, ticket, writer_cut_short).await
                }
                PointerInput::Move => {
                    let events = vec![move_event(&point)];
                    write_mouse(page, events, None, ticket, writer_cut_short).await
                }
                PointerInput::Tap => write_tap(page, point, ticket).await,
            }
        };
        self.spawn(async move {
            let written = writer.await;
            drop(input);
            // Nobody may be waiting any more; the ticket already holds the outcome, and the
            // unsent interceptor drops here, disarming.
            let _waiting = report.send((written, armed));
        });
        let cut_short = cut_short.drop_guard();
        async move {
            let pressed = receiver.await.unwrap_or_else(|_| {
                let failed = PressReport::Failed {
                    closed: true,
                    message: "the pointer writer stopped before reporting".to_string(),
                };
                (failed, None)
            });
            drop(cut_short);
            pressed
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
// The pointer writers.
// ---------------------------------------------------------------------------------------------

/// The mouse writer task. It owns every mouse event of one attempt (a hover's move, or a click's
/// move, presses and releases) and the ticket that records them, and holds the sender (for the
/// real tab, the page and so the session) until the last event is acknowledged or failed:
/// cancellation can stop a caller's wait, never split a press from its release.
///
/// Without `delay`: every event is sent at once at the session's socket incarnation and awaited
/// together (`join_all` polls them in order, and browseros-cdp's FIFO send lock writes them in
/// that order: Playwright's pipelined move, press, release). With `delay`: move, then per click
/// press, sleep `delay` (ended early by `cut_short`), release, and the same sleep between
/// clicks; after `cut_short` no further click starts. A release is sent after every press that
/// may have been written, whatever happened in between. Delivery is `Done` only once every
/// planned event was written and acknowledged: a double click cut short after its first click
/// stays `Unknown` (`keyboard::report`).
pub(crate) async fn write_mouse(
    sender: impl InputSender,
    events: Vec<DispatchMouseEventParams>,
    delay: Option<Duration>,
    ticket: InputTicket,
    cut_short: CancellationToken,
) -> PressReport {
    let outcomes = match delay {
        None => future::join_all(events.iter().map(|event| send_mouse(&sender, event))).await,
        Some(delay) => write_with_delay(&sender, &events, delay, &cut_short).await,
    };
    let complete = outcomes.len() == events.len();
    keyboard::report(&outcomes, complete, ticket)
}

/// The touch writer: Playwright's `touchStart` at the point and the `touchEnd` with no points,
/// sent together as the mouse writer's pipelined events are, so a stopped call never leaves a
/// touch down: both are written whoever still waits. `Done` once both are acknowledged.
/// Playwright: `server/chromium/crInput.ts` `RawTouchscreenImpl.tap`: `await progress.race(Promise.all([`
pub(crate) async fn write_tap(
    sender: impl InputSender,
    point: Point,
    ticket: InputTicket,
) -> PressReport {
    let events = tap_events(&point);
    let outcomes = future::join_all(
        events
            .iter()
            .map(|event| sender.send_input("Input.dispatchTouchEvent", event.clone())),
    )
    .await;
    let complete = outcomes.len() == events.len();
    keyboard::report(&outcomes, complete, ticket)
}

/// Playwright's sequential path (`input.ts` `Mouse.click` with `delay`).
async fn write_with_delay(
    sender: &impl InputSender,
    events: &[DispatchMouseEventParams],
    delay: Duration,
    cut_short: &CancellationToken,
) -> Vec<Result<(), CdpError>> {
    let mut outcomes = Vec::with_capacity(events.len());
    let mut events = events.iter();
    if let Some(moved) = events.next() {
        outcomes.push(send_mouse(sender, moved).await);
    }
    while let Some(pressed) = events.next() {
        if cut_short.is_cancelled() || outcomes.iter().any(Result::is_err) {
            break;
        }
        let written = send_mouse(sender, pressed).await;
        let refused = matches!(written, Err(CdpError::ConnectionEpochChanged { .. }));
        outcomes.push(written);
        if refused {
            break;
        }
        pause(delay, cut_short).await;
        if let Some(released) = events.next() {
            outcomes.push(send_mouse(sender, released).await);
        }
        if !events.as_slice().is_empty() {
            pause(delay, cut_short).await;
        }
    }
    outcomes
}

/// A `delay` sleep that the call stopping ends early.
async fn pause(delay: Duration, cut_short: &CancellationToken) {
    tokio::select! {
        () = time::sleep(delay) => {}
        () = cut_short.cancelled() => {}
    }
}

/// One mouse event, serialized as Playwright sends it. Serializing this plain struct cannot
/// fail; if it ever did, the error counts as a failed event, so delivery stays `Unknown` rather
/// than claiming nothing was written.
async fn send_mouse(
    sender: &impl InputSender,
    event: &DispatchMouseEventParams,
) -> Result<(), CdpError> {
    let params = serde_json::to_string(event)?;
    sender.send_input("Input.dispatchMouseEvent", params).await
}

// Keyboard input (`keyboard.rs`) goes through these two.
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

/// Playwright's exact events for a click (`input.ts` `Mouse.click`, `crInput.ts`):
///
/// | type          | button   | buttons | clickCount | force |
/// |---------------|----------|---------|------------|-------|
/// | mouseMoved    | `none`   | 0       | —          | 0     |
/// | mousePressed  | button   | mask    | 1..=n      | 0.5   |
/// | mouseReleased | button   | 0       | 1..=n      | —     |
///
/// all with `x`, `y` and `modifiers: 0`; one press and release pair per click count. Pure.
pub(crate) fn mouse_events(point: &Point, click: &MouseClick) -> Vec<DispatchMouseEventParams> {
    let (name, mask) = match click.button {
        MouseButton::Left => ("left", 1),
        MouseButton::Right => ("right", 2),
        MouseButton::Middle => ("middle", 4),
    };
    let event = |kind: &str, button: &str, buttons: i64| DispatchMouseEventParams {
        r#type: kind.to_string(),
        x: point.x,
        y: point.y,
        modifiers: Some(0),
        button: Some(button.to_string()),
        buttons: Some(buttons),
        ..DispatchMouseEventParams::default()
    };
    let mut events = vec![move_event(point)];
    for count in 1..=click.click_count {
        events.push(DispatchMouseEventParams {
            click_count: Some(i64::from(count)),
            force: Some(0.5),
            ..event("mousePressed", name, mask)
        });
        events.push(DispatchMouseEventParams {
            click_count: Some(i64::from(count)),
            ..event("mouseReleased", name, 0)
        });
    }
    events
}

/// The one move of a hover, and the first event of a click: `crInput.ts` `RawMouseImpl.move`
/// with no button held (`button: 'none'`, `buttons: 0`, `force: 0`) and no modifier. Pure.
/// Playwright: `server/chromium/crInput.ts` `RawMouseImpl.move`: `type: 'mouseMoved',`
pub(crate) fn move_event(point: &Point) -> DispatchMouseEventParams {
    DispatchMouseEventParams {
        r#type: "mouseMoved".to_string(),
        x: point.x,
        y: point.y,
        modifiers: Some(0),
        button: Some("none".to_string()),
        buttons: Some(0),
        force: Some(0.0),
        ..DispatchMouseEventParams::default()
    }
}

/// Playwright's two touch events for a tap (`crInput.ts` `RawTouchscreenImpl.tap`), as the JSON
/// params of `Input.dispatchTouchEvent`: a start with one touch point at `point`, then an end
/// with none, both without modifiers. browseros-cdp has no typed touch params. Pure.
/// Playwright: `server/chromium/crInput.ts` `RawTouchscreenImpl.tap`: `touchPoints: []`
pub(crate) fn tap_events(point: &Point) -> [String; 2] {
    // Playwright: `server/chromium/crInput.ts` `RawTouchscreenImpl.tap`: `type: 'touchStart',`
    let start = json!({
        "type": "touchStart",
        "modifiers": 0,
        "touchPoints": [{ "x": point.x, "y": point.y }],
    });
    // Playwright: `server/chromium/crInput.ts` `RawTouchscreenImpl.tap`: `type: 'touchEnd',`
    let end = json!({ "type": "touchEnd", "modifiers": 0, "touchPoints": [] });
    [start.to_string(), end.to_string()]
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::progress::DeliveryCell;
    use crate::run::Delivery;

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

    /// An event's type, button, buttons mask and click count.
    type EventSummary = (String, Option<String>, Option<i64>, Option<i64>);

    /// One move, then a press and a release per click, every event at the click point.
    #[test]
    fn every_click_count_presses_and_releases_with_its_button_mask() {
        for (button, name, mask) in [
            (MouseButton::Left, "left", 1),
            (MouseButton::Right, "right", 2),
            (MouseButton::Middle, "middle", 4),
        ] {
            let click = MouseClick {
                button,
                click_count: 3,
                delay: None,
            };
            let events = mouse_events(&Point { x: 1.0, y: 2.0 }, &click);
            let points: Vec<(f64, f64)> = events.iter().map(|event| (event.x, event.y)).collect();
            assert_eq!(
                points,
                vec![(1.0, 2.0); events.len()],
                "{name}: every event at the point"
            );
            let summary: Vec<EventSummary> = events
                .into_iter()
                .map(|event| (event.r#type, event.button, event.buttons, event.click_count))
                .collect();
            let pair = |count| {
                [
                    (
                        "mousePressed".to_string(),
                        Some(name.to_string()),
                        Some(mask),
                        Some(count),
                    ),
                    (
                        "mouseReleased".to_string(),
                        Some(name.to_string()),
                        Some(0),
                        Some(count),
                    ),
                ]
            };
            let mut expected = vec![(
                "mouseMoved".to_string(),
                Some("none".to_string()),
                Some(0),
                None,
            )];
            for count in 1..=3 {
                expected.extend(pair(count));
            }
            assert_eq!(summary, expected, "{name}");
        }
    }

    /// A scripted session for the pointer writers, under paused time: each send is recorded
    /// with its event type and answers the next scripted outcome after a round trip; `Hang`
    /// never answers, as a renderer frozen mid-event.
    #[derive(Clone)]
    enum Answer {
        Ok,
        Fail,
        Refuse,
        Hang,
    }

    struct WriterSender {
        sent: Arc<SyncMutex<Vec<String>>>,
        answers: Arc<SyncMutex<VecDeque<Answer>>>,
    }

    impl WriterSender {
        fn new(answers: impl IntoIterator<Item = Answer>) -> (Self, Arc<SyncMutex<Vec<String>>>) {
            let sent = Arc::<SyncMutex<Vec<String>>>::default();
            let sender = Self {
                sent: Arc::clone(&sent),
                answers: Arc::new(SyncMutex::new(answers.into_iter().collect())),
            };
            (sender, sent)
        }
    }

    impl InputSender for WriterSender {
        async fn send_input(&self, _method: &'static str, params: String) -> Result<(), CdpError> {
            let params: Value = serde_json::from_str(&params).expect("params are JSON");
            let kind = params["type"].as_str().unwrap_or_default().to_string();
            self.sent.lock().expect("sent").push(kind);
            let answer = self
                .answers
                .lock()
                .expect("answers")
                .pop_front()
                .unwrap_or(Answer::Ok);
            time::sleep(Duration::from_millis(1)).await;
            match answer {
                Answer::Ok => Ok(()),
                Answer::Fail => Err(CdpError::Timeout {
                    method: "Input.dispatchMouseEvent".to_string(),
                }),
                Answer::Refuse => Err(CdpError::ConnectionEpochChanged {
                    expected: 1,
                    actual: 2,
                }),
                Answer::Hang => future::pending().await,
            }
        }
    }

    fn fresh_ticket() -> (Arc<DeliveryCell>, InputTicket) {
        let cell = Arc::new(DeliveryCell::default());
        let ticket = cell.begin().expect("a fresh cell");
        (cell, ticket)
    }

    fn dblclick(delay: Option<Duration>) -> Vec<DispatchMouseEventParams> {
        let click = MouseClick {
            button: MouseButton::Left,
            click_count: 2,
            delay,
        };
        mouse_events(&Point { x: 5.0, y: 6.0 }, &click)
    }

    /// Design §4.7: a double click the call stopped after its first click is partial input,
    /// `Unknown`, never `Done`; the release its press owes is still written.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_double_click_cut_short_after_its_first_click_reads_unknown() {
        let (sender, sent) = WriterSender::new([]);
        let (cell, ticket) = fresh_ticket();
        let cut_short = CancellationToken::new();
        let delay = Duration::from_millis(50);
        let writer = tokio::spawn(write_mouse(
            sender,
            dblclick(Some(delay)),
            Some(delay),
            ticket,
            cut_short.clone(),
        ));
        time::sleep(Duration::from_millis(10)).await;
        cut_short.cancel();
        let report = writer.await.expect("the writer task");
        assert!(
            matches!(report, PressReport::Failed { closed: false, .. }),
            "{report:?}"
        );
        assert_eq!(cell.delivery(), Delivery::Unknown);
        assert_eq!(
            *sent.lock().expect("sent"),
            ["mouseMoved", "mousePressed", "mouseReleased"],
            "the first press is released; the second click never starts"
        );

        let (sender, sent) = WriterSender::new([]);
        let (cell, ticket) = fresh_ticket();
        let report = write_mouse(
            sender,
            dblclick(Some(delay)),
            Some(delay),
            ticket,
            CancellationToken::new(),
        )
        .await;
        assert!(matches!(report, PressReport::Acked(_)), "{report:?}");
        assert_eq!(cell.delivery(), Delivery::Done);
        assert_eq!(
            sent.lock().expect("sent").len(),
            5,
            "the whole double click"
        );
    }

    /// A tap whose answers are lost still ends its touch (both events are written together),
    /// and reads `Unknown`: the page may have seen them.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_tap_whose_answers_fail_ends_its_touch_and_reads_unknown() {
        let (sender, sent) = WriterSender::new([Answer::Fail, Answer::Fail]);
        let (cell, ticket) = fresh_ticket();
        let report = write_tap(sender, Point { x: 1.0, y: 2.0 }, ticket).await;
        assert!(
            matches!(report, PressReport::Failed { closed: false, .. }),
            "{report:?}"
        );
        assert_eq!(cell.delivery(), Delivery::Unknown);
        assert_eq!(*sent.lock().expect("sent"), ["touchStart", "touchEnd"]);
    }

    /// A tap whose start is never answered has still written its end, and stays `Unknown`
    /// while it waits; a tap whose caller stopped finishes anyway and reads `Done`; a refused
    /// tap wrote nothing.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_tap_survives_a_lost_answer_and_a_stopped_caller() {
        let (sender, sent) = WriterSender::new([Answer::Hang, Answer::Ok]);
        let (cell, ticket) = fresh_ticket();
        let writer = tokio::spawn(write_tap(sender, Point { x: 1.0, y: 2.0 }, ticket));
        time::sleep(Duration::from_secs(5)).await;
        assert_eq!(*sent.lock().expect("sent"), ["touchStart", "touchEnd"]);
        assert_eq!(cell.delivery(), Delivery::Unknown);
        writer.abort();

        let (sender, sent) = WriterSender::new([]);
        let (cell, ticket) = fresh_ticket();
        // Detached, as `Tab::press` starts it: nobody awaits the report.
        drop(tokio::spawn(write_tap(
            sender,
            Point { x: 1.0, y: 2.0 },
            ticket,
        )));
        time::sleep(Duration::from_millis(10)).await;
        assert_eq!(cell.delivery(), Delivery::Done);
        assert_eq!(sent.lock().expect("sent").len(), 2);

        let (sender, _) = WriterSender::new([Answer::Refuse, Answer::Refuse]);
        let (cell, ticket) = fresh_ticket();
        let report = write_tap(sender, Point { x: 1.0, y: 2.0 }, ticket).await;
        assert!(matches!(report, PressReport::NothingWritten), "{report:?}");
        assert_eq!(cell.delivery(), Delivery::NotSent);
    }

    /// A hover's one move is a click's first event, and a tap is Playwright's two touch events:
    /// the start with one touch point at the point, the end with none, both without modifiers.
    #[test]
    fn tests_that_a_hover_moves_like_a_click_and_a_tap_starts_and_ends_one_touch() {
        let point = Point { x: 10.5, y: 20.25 };
        let click = MouseClick {
            button: MouseButton::Left,
            click_count: 1,
            delay: None,
        };
        assert_eq!(move_event(&point), mouse_events(&point, &click)[0]);
        let [start, end] = tap_events(&point)
            .map(|event| serde_json::from_str::<Value>(&event).expect("touch params are JSON"));
        assert_eq!(
            start,
            json!({ "type": "touchStart", "modifiers": 0, "touchPoints": [{ "x": 10.5, "y": 20.25 }] })
        );
        assert_eq!(
            end,
            json!({ "type": "touchEnd", "modifiers": 0, "touchPoints": [] })
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
    fn tests_that_stopped_loading_completes_only_a_bfcache_document() {
        let mut state = PageState::default();
        let stopped = event(
            "Page.frameStoppedLoading",
            Some(SESSION),
            json!({"frameId":MAIN}),
        );
        for (kind, expected) in [("BackForwardCacheRestore", true), ("Navigation", false)] {
            applied(
                &mut state,
                &event(
                    "Page.frameNavigated",
                    Some(SESSION),
                    json!({
                        "frame":{"id":MAIN,"url":"https://a.test/","loaderId":kind},"type":kind
                    }),
                ),
            );
            applied(&mut state, &stopped);
            assert_eq!(state.dom_content_loaded, expected);
            assert_eq!(state.load, expected);
        }
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
            Arc::new(Mutex::new(())),
            CancellationToken::new(),
            EventHub::default(),
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
}
