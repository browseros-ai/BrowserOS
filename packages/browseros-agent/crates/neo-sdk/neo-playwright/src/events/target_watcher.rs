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
//!   `Page.frameNavigated` go to the `FrameAttachLog`, on any session of the tab, so frames are
//!   numbered in the order Playwright numbers them (`world/frame_attach_log.rs`).
//! - **Console and page errors** are recorded only while some run listens for them on that tab,
//!   so a chatty page cannot push popups and dialogs out of the ring. BrowserOS in hosted mode
//!   sends neither (measured: no `Runtime.consoleAPICalled` and no `Runtime.exceptionThrown`
//!   with `BROWSERCLAW_HOSTED_MODE=1`), so there those events never fire.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use browseros_cdp::CdpEvent;
use browseros_core::{BrowserSession, BrowserView, CoreError, FrameId, SessionId, TargetId};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::{self, Instant, MissedTickBehavior};

use crate::events::dialog_table::{ClosedBy, HeldDialog, Unanswered};
use crate::events::event_hub::EventWriter;
use crate::events::frame_lifecycle::FrameLifecycleTracker;
use crate::events::page_event::{
    ConsoleMessage, ConsoleParams, DialogId, DialogKind, DialogOpened, DownloadStarted,
    ExceptionParams, FileChooserOpened, OpenedPage, PageError, PageEvent, WindowKind,
};
use crate::events::{PageEventKind, WatcherProgress, navigation_event};
use crate::world::{BackendNodeId, FrameAttachLog, FrameEvent};

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
}

impl Drop for TargetWatcher {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl TargetWatcher {
    /// Starts watching `browser`, recording page events through `writer` and frame lifecycles
    /// into `attach_log`, and counting what it handled into `progress`. Outside a tokio runtime
    /// there is nothing to run it on, so nothing is recorded and no one waits for it.
    pub(crate) fn spawn(
        browser: Arc<BrowserSession>,
        writer: EventWriter,
        attach_log: Arc<FrameAttachLog>,
        progress: Arc<WatcherProgress>,
    ) -> Self {
        let Ok(runtime) = Handle::try_current() else {
            progress.stopped();
            return Self { task: None };
        };
        let (watcher, found) = Watcher::new(browser, writer, attach_log, progress);
        Self {
            task: Some(runtime.spawn(watcher.run(found))),
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
    progress: Arc<WatcherProgress>,
    lifecycle: FrameLifecycleTracker,
    /// The connection epoch discovery ran for; `None` until it succeeds, and after a lag.
    epoch: Option<u64>,
    /// The tabs the watcher knows.
    pages: HashMap<TargetId, KnownPage>,
    /// Popups whose window lookup was in flight when they closed.
    closed_early: HashSet<TargetId>,
    /// Which tab each page or OOPIF session belongs to.
    sessions: HashMap<SessionId, TargetId>,
    /// Core may attach concurrently. One source per native target prevents each CDP session
    /// from publishing the same lifecycle transition; OOPIFs have their own native target.
    lifecycle_sources: LifecycleSources,
    /// Which tab each out-of-process frame belongs to (an OOPIF's frame id is its target id).
    frames: HashMap<String, TargetId>,
    /// Dialogs in their grace, oldest first; every grace is as long, so this is by deadline.
    graces: VecDeque<Grace>,
    next_reap: Instant,
    /// Window lookups report their popup back on this channel, so only this task writes the
    /// log. The receiving end is a local of the task's loop.
    found: UnboundedSender<OpenedPage>,
}

struct KnownPage {
    opener: Option<TargetId>,
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
    url: String,
    #[serde(default)]
    opener_id: Option<String>,
    #[serde(default)]
    subtype: Option<String>,
    /// BrowserOS's: the window that holds the tab.
    #[serde(default)]
    window_id: Option<i64>,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Destroyed {
    target_id: String,
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
    ) -> (Self, UnboundedReceiver<OpenedPage>) {
        let (found, found_rx) = mpsc::unbounded_channel();
        let watcher = Self {
            view: BrowserView::unbound(Arc::clone(&browser), None),
            browser,
            writer,
            attach_log,
            progress,
            lifecycle: FrameLifecycleTracker::default(),
            epoch: None,
            pages: HashMap::new(),
            closed_early: HashSet::new(),
            sessions: HashMap::new(),
            lifecycle_sources: LifecycleSources::default(),
            frames: HashMap::new(),
            graces: VecDeque::new(),
            next_reap: Instant::now() + REAP,
            found,
        };
        (watcher, found_rx)
    }

    async fn run(mut self, mut found: UnboundedReceiver<OpenedPage>) {
        // Subscribed before discovery, so no target is created unseen in between. The count
        // comes with the subscription, so it holds exactly the events this receiver never gets.
        let (mut events, before) = self.browser.cdp_events_counted();
        self.progress.start(before);
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
                    // Events were dropped: rediscover, which records what closed meanwhile, and
                    // tell the frame log that commits may be missing.
                    Err(RecvError::Lagged(skipped)) => {
                        self.epoch = None;
                        self.attach_log.lost_events();
                        self.progress.handled(skipped);
                    }
                    Err(RecvError::Closed) => {
                        self.progress.stopped();
                        return;
                    }
                },
                Some(opened) = found.recv() => self.found(opened),
                _ = tick.tick() => self.tick().await,
                () = time::sleep_until(grace_ends.unwrap_or_else(Instant::now)), if grace_ends.is_some() => {
                    self.end_graces();
                }
            }
        }
    }

    async fn tick(&mut self) {
        let epoch = self.browser.connection().connection_epoch();
        if self.epoch != Some(epoch) && self.browser.is_connected() {
            let changed = self.epoch.is_some_and(|known| known != epoch);
            if self.discover(changed).await.is_ok() {
                self.epoch = Some(epoch);
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
                state: PageState::Reported,
            });
        }
        if changed {
            self.lifecycle = FrameLifecycleTracker::default();
            self.sessions.clear();
            self.lifecycle_sources = LifecycleSources::default();
            self.frames.clear();
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
                    self.frames.remove(&target_id);
                    let target = TargetId(target_id);
                    self.destroyed(&target);
                }
            }
            "Target.attachedToTarget" => {
                if let Some(attached) = parse(params) {
                    self.attached(session_id.as_ref(), attached).await;
                }
            }
            "Target.detachedFromTarget" => {
                if let Some(Detached { session_id }) = parse(params) {
                    self.sessions.remove(&session_id);
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
                    self.writer.hub().dialogs().closed_on_session(&session);
                }
            }
            "Runtime.consoleAPICalled" => {
                if let Some(session) = session_id
                    && let Some(target) = self.wanted(&session, PageEventKind::Console).await
                    && let Some(params) = parse::<ConsoleParams>(params)
                {
                    let message = ConsoleMessage::from_params(target, params);
                    self.writer.record(PageEvent::Console(message));
                }
            }
            "Runtime.exceptionThrown" => {
                if let Some(session) = session_id
                    && let Some(target) = self.wanted(&session, PageEventKind::PageError).await
                    && let Some(params) = parse::<ExceptionParams>(params)
                {
                    let error = PageError::from_params(target, params);
                    self.writer.record(PageEvent::PageError(error));
                }
            }
            "Browser.downloadWillBegin" => {
                if let Some(download) = parse(params) {
                    self.download(download);
                }
            }
            "Browser.downloadProgress" => self.download_progress(params),
            _ => {}
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
        let pending = self
            .pages
            .get(&target)
            .is_some_and(|page| matches!(page.state, PageState::Pending { .. }));
        if pending && !info.url.is_empty() {
            self.ready(&target, info.url, info.window_id);
        }
    }

    /// A new tab has its URL: recorded now, or once a popup's window type is known.
    fn ready(&mut self, target: &TargetId, url: String, window_id: Option<i64>) {
        let Some(page) = self.pages.get_mut(target) else {
            return;
        };
        let Some(opener) = page.opener.clone() else {
            page.state = PageState::Reported;
            self.writer.record(PageEvent::Opened(OpenedPage {
                target: target.clone(),
                opener: None,
                url,
                window: WindowKind::Tab,
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
        self.attach_log.forget(target);
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
            self.writer.hub().dialogs().closed_on_session(session);
            self.sessions.remove(session);
            self.lifecycle_sources.detach(session);
        }
        match page.state {
            PageState::Reported => {
                self.writer.record(PageEvent::Closed(target.clone()));
            }
            PageState::LookingUp => {
                self.closed_early.insert(target.clone());
            }
            // Never reported, so its closing is not news either.
            PageState::Pending { .. } => {}
        }
    }

    async fn attached(&mut self, parent: Option<&SessionId>, attached: Attached) {
        let Attached {
            session_id,
            target_info,
        } = attached;
        match target_info.kind.as_str() {
            "page" => {
                let target = TargetId(target_info.target_id);
                self.lifecycle_sources.attach(&session_id, &target);
                self.sessions.insert(session_id.clone(), target.clone());
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
                    if self.lifecycle_sources.accepts(&session_id) {
                        self.seed_lifecycle(&tab, &session_id).await;
                    }
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

    /// A context subscription also covers newly discovered pages that no SDK action has
    /// admitted yet. Core owns these sessions and their shutdown, just like action sessions.
    async fn observe_context(&self) {
        if !self.writer.hub().wants_context_lifecycle() {
            return;
        }
        if let Ok(Ok(listing)) = time::timeout(Duration::from_secs(1), self.view.list_pages()).await
        {
            for page in listing.pages {
                if !self
                    .sessions
                    .values()
                    .any(|target| *target == page.target_id)
                {
                    let _ =
                        time::timeout(Duration::from_secs(1), self.view.page_session(page.page_id))
                            .await;
                }
            }
        }
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
    async fn wanted(&mut self, session: &SessionId, event: PageEventKind) -> Option<TargetId> {
        let target = self.target_of(session).await?;
        self.writer.hub().wants(&target, event).then_some(target)
    }

    async fn dialog_opened(&mut self, session: SessionId, params: DialogParams) {
        let target = self.target_of(&session).await;
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
                HeldDialog::new(session, target.clone(), kind, listening),
            );
            self.graces.push_back(Grace {
                ends: Instant::now() + DIALOG_GRACE,
                dialog: id,
                target: target.clone(),
            });
        }
        if let Some(target) = target {
            self.writer.record(PageEvent::Dialog(DialogOpened {
                id,
                target,
                kind,
                message: params.message,
                default_value: params.default_prompt.unwrap_or_default(),
            }));
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
