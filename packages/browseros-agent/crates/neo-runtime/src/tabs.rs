//! A run's ordered page registry and lazy CDP sessions. Handing out a page registers its
//! identity; attaching it owns a session until the run and its last writer have let go.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use browseros_cdp::runtime::RemoteObject;
use browseros_cdp::{CdpClient, CdpError};
use neo_runtime_spec::injected::InjectedPreviewNodeParams;
use neo_runtime_spec::serve::{FrameChannel, FrameInitializer, PageChannel, PageInitializer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::{OnceCell, oneshot};
use tokio::time;
use tokio_util::sync::{CancellationToken, DropGuard};
use tokio_util::task::TaskTracker;
use tracing::debug;

use crate::bridge;
use crate::click::PageError;
use crate::dialogs;
use crate::discovery::{self, TargetInfo};
use crate::downloads::{self, Downloads, Opened};
use crate::evaluate::{self, Handles};
use crate::events::{ALL_PAGES, CONTEXT_GUID, Enabling, EventHub, Needs};
use crate::frames::{FrameContext, FrameScope};
use crate::getters;
use crate::network::HopId;
use crate::page::{FrameTab, PageSession, Session, TabInputs};
use crate::page_events::{self, Kept, MAX_KEPT_ARGS, MAX_KEPT_CHOOSERS};
use crate::progress::PlaywrightError;
use crate::run::{PageId, RunPage};
use crate::screenshot::TabScreenshots;
use crate::session_state::{self, Overrides, Permissions};
use crate::switch::Switches;
use crate::workers;

/// Neo: how long `ConsoleMessage.args()` waits for its arguments' handles. A page a dialog holds
/// answers no call until the dialog is answered, which the script may be about to do.
const ARGS_WAIT: Duration = Duration::from_secs(5);

/// Cheap clones share the run's registry, context boundary, and session ownership.
#[derive(Clone)]
pub(crate) struct Tabs(Arc<Inner>);

struct Inner {
    cdp: CdpClient,
    tokio: Handle,
    inputs: TabInputs,
    /// The `Runtime`'s screenshot turns, one per tab, across runs.
    screenshots: TabScreenshots,
    tasks: TaskTracker,
    creations: TaskTracker,
    scope: Scope,
    run_page: PageId,
    pages: Mutex<Vec<Arc<PageEntry>>>,
    /// The run's event queue and subscriptions, shared with every page session it attaches.
    events: EventHub,
    /// The `Runtime`'s shared switches: target discovery, which this run holds while it listens
    /// for new pages ([`Tabs::discover`]), and download reports ([`Tabs::watch_downloads`]).
    switches: Switches,
    /// Set once this run started its discovery.
    discovering: OnceLock<()>,
    /// Holds every call of the run until discovery has its baseline ([`ALL_PAGES`]).
    discovery_enabling: Mutex<Option<Enabling>>,
    /// Set once this run started watching downloads.
    watching_downloads: OnceLock<()>,
    /// Holds every call of the run until the download reports are on ([`ALL_PAGES`]).
    downloads_enabling: Mutex<Option<Enabling>>,
    /// The downloads the run's `download` events handed out.
    downloads: Downloads,
    /// The run's session-scoped state, which each session it attaches gets (`session_state.rs`).
    overrides: Arc<Overrides>,
    /// The `Runtime`'s permission grants, and this run's id among them.
    permissions: Permissions,
    run_id: u64,
}

/// A page can be known without a session. Its closure token precedes the attachment so calls,
/// getters, and the eventual session watcher all observe the same close.
pub(crate) struct PageEntry {
    pub(crate) id: PageId,
    created: AtomicBool,
    closed_by_run: AtomicBool,
    close_requested: AtomicBool,
    pub(crate) closed: CancellationToken,
    pub(crate) frame_metadata: Mutex<getters::FrameMetadata>,
    session: OnceCell<Arc<PageSession>>,
    /// Reported by the run's discovery as a new page: its session's first lifecycle replay is
    /// the page's own new life, not history (`PageEvents`).
    pub(crate) discovered: AtomicBool,
    /// The tab that opened this one, as discovery reported it (`targetInfo.openerId`).
    opener: Mutex<Option<PageId>>,
}

struct Scope {
    context: Option<String>,
    window: Option<i64>,
}

/// BrowserOS's tab-strip record. `liveBrowserContextId` is routable; the older
/// `browserContextId` field is only a human-readable profile label.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TabInfo {
    target_id: String,
    url: String,
    window_id: Option<i64>,
    live_browser_context_id: Option<String>,
    #[serde(default)]
    pub(crate) is_active: bool,
}

#[derive(Deserialize)]
struct TabReply {
    tab: TabInfo,
}

#[derive(Deserialize)]
struct TabsReply {
    tabs: Vec<TabInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetInfoReply {
    target_info: TargetInfo,
}

#[derive(Deserialize)]
struct WindowReply {
    window: CreatedWindow,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatedWindow {
    window_id: i64,
}

/// A pending newPage transaction. Dropping it wakes the creation task, which closes the
/// new tab unless the handler has committed a successful result with `keep`.
pub(crate) struct NewPage {
    pub(crate) entry: Arc<PageEntry>,
    accepted: Arc<AtomicBool>,
    _finished: DropGuard,
}

impl NewPage {
    pub(crate) fn keep(self) {
        self.accepted.store(true, Ordering::Release);
    }
}

/// The initializer records that the SDK turns into stable Page and main Frame objects.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreatedPage {
    guid: String,
    pub(crate) initializer: PageInitializer,
    main_frame: CreatedFrame,
}

#[derive(Serialize)]
struct CreatedFrame {
    guid: String,
    initializer: FrameInitializer,
}

impl Tabs {
    #[expect(
        clippy::too_many_arguments,
        reason = "each per-tab share the Runtime keeps (inputs, screenshots, switches, permissions) is one argument"
    )]
    pub(crate) async fn open(
        cdp: CdpClient,
        tokio: Handle,
        inputs: TabInputs,
        screenshots: TabScreenshots,
        switches: Switches,
        permissions: Permissions,
        tasks: TaskTracker,
        page: &PageId,
    ) -> Result<(Self, String), PlaywrightError> {
        let info = tab_info(&cdp, page)
            .await?
            .ok_or_else(PlaywrightError::target_closed)?;
        let tabs = Self(Arc::new(Inner {
            cdp,
            tokio,
            inputs,
            screenshots,
            tasks,
            creations: TaskTracker::new(),
            scope: Scope {
                context: info.live_browser_context_id,
                window: info.window_id,
            },
            run_page: page.clone(),
            pages: Mutex::new(Vec::new()),
            events: EventHub::default(),
            switches,
            discovering: OnceLock::new(),
            discovery_enabling: Mutex::new(None),
            watching_downloads: OnceLock::new(),
            downloads_enabling: Mutex::new(None),
            downloads: Downloads::default(),
            overrides: Arc::default(),
            permissions,
            run_id: session_state::next_run(),
        }));
        let entry = tabs.register(page.clone(), false)?;
        // The scope read above already checked this tab. Avoid a second startup read so
        // a getter-only run keeps its original connect/check/attach/history call path.
        entry
            .session
            .get_or_try_init(|| tabs.attach(&entry))
            .await?;
        let url = tabs
            .url(&entry)
            .await?
            .ok_or_else(PlaywrightError::target_closed)?;
        Ok((tabs, url))
    }

    pub(crate) fn cdp(&self) -> &CdpClient {
        &self.0.cdp
    }

    /// The page the run started from.
    pub(crate) fn run_page(&self) -> &PageId {
        &self.0.run_page
    }

    pub(crate) fn events(&self) -> &EventHub {
        &self.0.events
    }

    /// Runs `task` on the run's tracker, so `Dispatcher::finish` waits for it.
    pub(crate) fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        self.0.tasks.spawn_on(task, &self.0.tokio);
    }

    /// Every page this run has attached a session to, in first-seen order.
    pub(crate) fn attached(&self) -> Vec<(Arc<PageEntry>, Arc<PageSession>)> {
        self.0.pages.lock().map_or_else(
            |_| Vec::new(),
            |pages| {
                pages
                    .iter()
                    .filter_map(|entry| {
                        entry
                            .attached_session()
                            .map(|session| (Arc::clone(entry), session))
                    })
                    .collect()
            },
        )
    }

    pub(crate) fn finish_close(&self, page: Arc<PageSession>, entry: Arc<PageEntry>) {
        // Page.close can acknowledge before its beforeunload dialog arrives. Keep the
        // watcher alive after the script returns so it can answer; otherwise detaching
        // strands that dialog. Bound the cleanup if the browser never completes closure.
        self.0.tasks.spawn_on(
            async move {
                if time::timeout(Duration::from_secs(5), page.closed().cancelled())
                    .await
                    .is_ok()
                {
                    entry.closed_by_run();
                }
                drop(page);
            },
            &self.0.tokio,
        );
    }

    /// The page a guid belongs to and the frame it scopes: `page@T`, `frame@T` and a hop's
    /// `request@T:…`/`response@T:…` scope the main frame, `frame@T/F` the child frame `F`.
    pub(crate) fn resolve(&self, guid: &str) -> Option<(Arc<PageEntry>, FrameScope)> {
        let (target, scope) = if let Some(target) = guid.strip_prefix("page@") {
            if target.contains('/') {
                return None;
            }
            (target, FrameScope::Main)
        } else if let Some(frame) = guid.strip_prefix("frame@") {
            match frame.split_once('/') {
                Some((target, frame)) if !frame.is_empty() && !frame.contains('/') => {
                    (target, FrameScope::Frame(frame.to_string()))
                }
                Some(_) => return None,
                None => (frame, FrameScope::Main),
            }
        } else if let Some((target, _)) = dialogs::parse(guid) {
            (target, FrameScope::Main)
        } else if let Some(worker) = guid.strip_prefix(workers::WORKER_PREFIX) {
            // A dedicated worker runs its calls on its tab (`workers.rs`).
            (worker.split_once(':')?.0, FrameScope::Main)
        } else {
            // A hop is one of the main frame's navigation requests (network.rs).
            let (_, target, _) = HopId::parse(guid)?;
            (target, FrameScope::Main)
        };
        let page = self
            .0
            .pages
            .lock()
            .ok()?
            .iter()
            .find(|entry| entry.id.as_str() == target)
            .cloned()?;
        Some((page, scope))
    }

    /// The registered page `id`, if the run knows it.
    fn entry(&self, id: &PageId) -> Option<Arc<PageEntry>> {
        self.0
            .pages
            .lock()
            .ok()?
            .iter()
            .find(|entry| &entry.id == id)
            .cloned()
    }

    fn register(&self, id: PageId, created: bool) -> Result<Arc<PageEntry>, PlaywrightError> {
        let mut pages = self.0.pages.lock().map_err(|_| {
            PlaywrightError::error("neo-runtime: the run's pages cannot be registered")
        })?;
        if let Some(entry) = pages.iter().find(|entry| entry.id == id) {
            if created {
                entry.created.store(true, Ordering::Release);
            }
            return Ok(Arc::clone(entry));
        }
        let entry = Arc::new(PageEntry {
            id,
            created: AtomicBool::new(created),
            closed_by_run: AtomicBool::new(false),
            close_requested: AtomicBool::new(false),
            closed: CancellationToken::new(),
            frame_metadata: Mutex::new(getters::FrameMetadata::new(&self.0.cdp)),
            session: OnceCell::new(),
            discovered: AtomicBool::new(false),
            opener: Mutex::new(None),
        });
        pages.push(Arc::clone(&entry));
        Ok(entry)
    }

    pub(crate) async fn session(
        &self,
        entry: &PageEntry,
    ) -> Result<Arc<PageSession>, PlaywrightError> {
        if entry.closed.is_cancelled() {
            return Err(PlaywrightError::target_closed());
        }
        let session = entry
            .session
            .get_or_try_init(|| async {
                if self.is_closed(entry).await? {
                    return Err(PlaywrightError::target_closed());
                }
                let session = self.attach(entry).await?;
                // A context listener covers every page the run has a session on, this one
                // from now on. Registered before the call that attached it continues, so that
                // call waits for the enable (`EventHub::ready`).
                if let Some(needs) = self.0.events.context_needs()
                    && needs.of_page()
                    && let Some(entry) = self.entry(&entry.id)
                {
                    page_events::enable(self, entry, needs);
                }
                Ok(session)
            })
            .await?;
        if session.closed().is_cancelled() {
            return Err(PlaywrightError::target_closed());
        }
        Ok(Arc::clone(session))
    }

    async fn attach(&self, entry: &PageEntry) -> Result<Arc<PageSession>, PlaywrightError> {
        // An attach's answer must be read even if its waiter is dropped. The task owns
        // the returned Session; tokio drops an unclaimed output, which detaches it.
        let session = self
            .0
            .tasks
            .spawn_on(
                Session::attach(self.0.cdp.clone(), entry.id.clone(), self.0.tokio.clone()),
                &self.0.tokio,
            )
            .await
            .map_err(|error| PlaywrightError::error(format!("attaching to the tab: {error}")))?;
        let session = match session {
            Ok(session) => session,
            Err(error) => {
                if self.is_closed(entry).await? {
                    return Err(PlaywrightError::target_closed());
                }
                return Err(cdp_error(&error));
            }
        };
        if entry.closed.is_cancelled() {
            return Err(PlaywrightError::target_closed());
        }
        let input = self.0.inputs.lock_for(&entry.id).await;
        let screenshots = self.0.screenshots.turn_for(&entry.id).await;
        let page = Arc::new(PageSession::new(
            session,
            &entry.id,
            self.0.tasks.clone(),
            input,
            screenshots,
            entry.closed.clone(),
            self.0.events.clone(),
            entry.discovered.load(Ordering::Acquire),
        ));
        // The run's session state reaches this tab's sessions before the call that attached
        // each goes on: the tab's own now, each out-of-process frame's as it attaches.
        let overrides = Arc::clone(&self.0.overrides);
        let cdp = page.events.cdp().clone();
        let id = entry.id.clone();
        page.events.on_child_session(Arc::new(move |session| {
            let overrides = Arc::clone(&overrides);
            let cdp = cdp.clone();
            let id = id.clone();
            Box::pin(async move { overrides.apply_to_new(&cdp, session, &id, true).await })
        }));
        if let Some(own) = page.events.sessions().into_iter().next() {
            self.0
                .overrides
                .apply_to_new(page.events.cdp(), own, &entry.id, false)
                .await;
        }
        Ok(page)
    }

    pub(crate) async fn url(&self, entry: &PageEntry) -> Result<Option<String>, PlaywrightError> {
        let session = match self.session(entry).await {
            Ok(session) => session,
            Err(error) if error == PlaywrightError::target_closed() => return Ok(None),
            Err(error) => return Err(error),
        };
        let url = getters::url(&self.0.cdp, session.session_id(), &entry.id)
            .await
            .map_err(|error| cdp_error(&error))?;
        if url.is_none() {
            entry.closed.cancel();
        }
        Ok(url)
    }

    pub(crate) async fn is_closed(&self, entry: &PageEntry) -> Result<bool, PlaywrightError> {
        let closed = getters::is_closed(&self.0.cdp, &entry.id)
            .await
            .map_err(|error| cdp_error(&error))?;
        if closed {
            entry.closed.cancel();
            if entry.close_requested.load(Ordering::Acquire) {
                entry.closed_by_run();
            }
        }
        Ok(closed)
    }

    async fn infos(&self) -> Result<Vec<TabInfo>, PlaywrightError> {
        let reply: TabsReply = self
            .0
            .cdp
            .send_typed("Browser.getTabs", json!({}), None)
            .await
            .map_err(|error| cdp_error(&error))?;
        Ok(reply
            .tabs
            .into_iter()
            .filter(|info| self.0.scope.includes(info))
            .collect())
    }

    pub(crate) async fn pages(&self) -> Result<Vec<CreatedPage>, PlaywrightError> {
        let mut live = HashMap::new();
        for info in self.infos().await? {
            self.register(PageId::new(info.target_id.clone()), false)?;
            live.insert(info.target_id, info.url);
        }
        let pages =
            self.0.pages.lock().map_err(|_| {
                PlaywrightError::error("neo-runtime: the run's pages cannot be read")
            })?;
        Ok(pages
            .iter()
            .filter_map(|entry| {
                live.remove(entry.id.as_str())
                    .map(|url| created_page(entry, url, false))
            })
            .collect())
    }

    pub(crate) async fn initializer(
        &self,
        entry: &PageEntry,
    ) -> Result<CreatedPage, PlaywrightError> {
        let url = self.url(entry).await?;
        let closed = url.is_none();
        Ok(created_page(entry, url.unwrap_or_default(), closed))
    }

    pub(crate) async fn create(&self) -> Result<NewPage, PlaywrightError> {
        let (reply, receiver) = oneshot::channel();
        let finished = CancellationToken::new();
        let guard = finished.clone().drop_guard();
        let accepted = Arc::new(AtomicBool::new(false));
        let kept = Arc::clone(&accepted);
        let tabs = self.clone();
        // The browser owns the write once it receives createTab. This task reads and
        // registers its answer even after the waiting call is cancelled. The lease keeps
        // the task alive through readiness, and releases it to cleanup on every failure.
        let work = async move {
            match tabs.create_registered().await {
                Ok(entry) => {
                    let _sent = reply.send(Ok(Arc::clone(&entry)));
                    finished.cancelled().await;
                    if !kept.load(Ordering::Acquire) {
                        tabs.close_failed_creation(&entry).await;
                    }
                }
                Err(error) => {
                    let _sent = reply.send(Err(error));
                }
            }
        };
        self.0
            .tasks
            .spawn_on(self.0.creations.track_future(work), &self.0.tokio);
        let entry = receiver
            .await
            .map_err(|error| PlaywrightError::error(format!("creating a page: {error}")))??;
        Ok(NewPage {
            entry,
            accepted,
            _finished: guard,
        })
    }

    async fn create_registered(&self) -> Result<Arc<PageEntry>, PlaywrightError> {
        let mut params = json!({ "url": "about:blank", "background": true });
        let run_tab = tab_info(&self.0.cdp, &self.0.run_page).await?;
        let window = match run_tab {
            Some(info) => info.window_id,
            None => self.infos().await?.first().and_then(|info| info.window_id),
        };
        if let Some(window) = window {
            params["windowId"] = json!(window);
        } else if let Some(context) = &self.0.scope.context {
            // BrowserOS's context-routed windows are shown inactive. createTab alone
            // requires a surviving normal window, so recreate one after the last closes.
            let window: WindowReply = self
                .0
                .cdp
                .send_typed(
                    "Browser.createWindow",
                    json!({"browserContextId":context,"url":"about:blank"}),
                    None,
                )
                .await
                .map_err(|error| cdp_error(&error))?;
            let info = self
                .infos()
                .await?
                .into_iter()
                .find(|info| info.window_id == Some(window.window.window_id))
                .ok_or_else(|| PlaywrightError::error("the new window has no initial tab"))?;
            return self.register(PageId::new(info.target_id), true);
        } else {
            // A context with no surviving window and no routable identity cannot safely
            // create a tab: omitting both would let BrowserOS choose another profile.
            return Err(PlaywrightError::target_closed());
        }
        let reply: TabReply = self
            .0
            .cdp
            .send_typed("Browser.createTab", params, None)
            .await
            .map_err(|error| cdp_error(&error))?;
        self.register(PageId::new(reply.tab.target_id), true)
    }

    async fn close_failed_creation(&self, entry: &PageEntry) {
        entry.request_close();
        let cleanup = async {
            let _closed = self
                .0
                .cdp
                .send(
                    "Target.closeTarget",
                    json!({"targetId":entry.id.as_str()}),
                    None,
                )
                .await;
            loop {
                if self.is_closed(entry).await? {
                    return Ok::<(), PlaywrightError>(());
                }
                time::sleep(Duration::from_millis(10)).await;
            }
        };
        if !matches!(
            time::timeout(Duration::from_secs(5), cleanup).await,
            Ok(Ok(()))
        ) {
            debug!(
                target = entry.id.as_str(),
                "failed newPage cleanup did not confirm closure"
            );
        }
    }

    /// Waits for owned creation tasks within the caller's shared finish deadline. Dropping
    /// this wait does not abort a task: an unanswered creation may be absent from the
    /// outcome, but its unkept tab is closed when the reply eventually arrives.
    pub(crate) async fn finish_creations(&self) {
        self.0.creations.close();
        self.0.creations.wait().await;
    }

    /// Records the run's end on every tab it attached to (`PageSession::mark_input_ended`): no
    /// wait, no write. `Dispatcher::finish` does it before any cleanup that can wait.
    pub(crate) fn mark_input_ended(&self) {
        for session in self.attached_sessions() {
            session.mark_input_ended();
        }
    }

    /// Starts releasing, on every tab this run attached to, the keys and buttons the run left
    /// held (`PageSession::release_run_input`), once every tab has recorded the run's end.
    /// Each tab's release is its own task of the run's tracker, so a tab whose page does not
    /// answer holds back no other tab's releases, and no caller's bound cuts one off: one
    /// still writing when the run's outcome is returned keeps the tab's input turn until it is
    /// done, so the next input on the tab, whichever run sends it, lands after it, and
    /// `Runtime::drain` waits for it.
    pub(crate) fn release_input(&self) {
        let sessions = self.attached_sessions();
        for session in &sessions {
            session.mark_input_ended();
        }
        for session in sessions {
            self.spawn(async move { session.release_run_input().await });
        }
    }

    pub(crate) async fn refresh_closing(&self) {
        let pages = self
            .0
            .pages
            .lock()
            .map_or_else(|_| Vec::new(), |pages| pages.clone());
        for page in pages {
            if page.close_requested.load(Ordering::Acquire) {
                // The caller bounds the whole batch, not each page separately.
                let _checked = self.is_closed(&page).await;
            }
        }
    }

    /// Every session this run attached, for the run-end cleanup of what it left in its tabs.
    pub(crate) fn attached_sessions(&self) -> Vec<Arc<PageSession>> {
        self.0.pages.lock().map_or_else(
            |_| Vec::new(),
            |pages| {
                pages
                    .iter()
                    .filter_map(|entry| entry.attached_session())
                    .collect()
            },
        )
    }

    /// Starts the run's target discovery once (`discovery.rs`), for a `page` or `popup`
    /// listener. Every call of the run waits until it holds its baseline, so no call can open a
    /// tab the baseline would hide.
    pub(crate) fn discover(&self) {
        if self.0.discovering.set(()).is_err() {
            return;
        }
        let enabling = self.0.events.begin_enable(ALL_PAGES);
        if let Ok(mut held) = self.0.discovery_enabling.lock() {
            *held = Some(enabling);
        }
        self.spawn(discovery::run(
            self.clone(),
            self.0.switches.discovery.clone(),
        ));
    }

    /// Discovery has its baseline, or failed to start: the run's calls go on.
    pub(crate) fn discovery_ready(&self) {
        if let Ok(mut held) = self.0.discovery_enabling.lock() {
            held.take();
        }
    }

    /// Starts watching the browser's download reports once (`downloads.rs`), for a `download`
    /// listener. Every call of the run waits until the reports are on, so no call can start a
    /// download before the run hears of it.
    pub(crate) fn watch_downloads(&self) {
        if self.0.watching_downloads.set(()).is_err() {
            return;
        }
        let enabling = self.0.events.begin_enable(ALL_PAGES);
        if let Ok(mut held) = self.0.downloads_enabling.lock() {
            *held = Some(enabling);
        }
        self.spawn(downloads::run(self.clone(), self.0.switches.clone()));
    }

    /// The download reports are on, or failed to turn on: the run's calls go on.
    pub(crate) fn downloads_ready(&self) {
        if let Ok(mut held) = self.0.downloads_enabling.lock() {
            held.take();
        }
    }

    /// The downloads the run's `download` events handed out.
    pub(crate) fn downloads(&self) -> &Downloads {
        &self.0.downloads
    }

    /// The browser context of the run's tabs, when BrowserOS names it.
    pub(crate) fn context_id(&self) -> Option<&str> {
        self.0.scope.context.as_deref()
    }

    /// Playwright: `server/chromium/crBrowser.ts` `CRBrowser._onDownloadWillBegin`: `// If it's a new window download, report it on the opener page.`
    ///
    /// The tab of the run's context that a download's frame belongs to, as Playwright's
    /// `_findOwningPage` finds it: a tab by its own id (a tab's main frame has its target id,
    /// measured), by a frame of a tab the run watches, or a tab of the context the run has not
    /// met yet, which it registers. A tab whose first document never committed reports on the
    /// tab that opened it: a link with `target=_blank` to an attachment opens a tab that begins
    /// the download and closes, often before this asks of it (measured), so what discovery
    /// reported of it (`opened`) decides. A frame of a tab the run knows but does not watch is
    /// not found, nor is anything that is no tab (an extension page, a developer tools window).
    pub(crate) async fn download_page(
        &self,
        frame: &str,
        opened: Option<&Opened>,
    ) -> Option<Arc<PageEntry>> {
        let id = PageId::new(frame.to_string());
        if let Some(entry) = self.entry(&id) {
            return Some(entry);
        }
        if let Some((entry, _)) = self
            .attached()
            .into_iter()
            .find(|(_, session)| session.events.has_frame(frame))
        {
            return Some(entry);
        }
        let info = self
            .0
            .cdp
            .send_typed::<_, TargetInfoReply>(
                "Target.getTargetInfo",
                json!({ "targetId": frame }),
                None,
            )
            .await
            .ok()
            .map(|reply| reply.target_info);
        let owner = match (info, opened) {
            (Some(info), _) if info.kind != "page" => return None,
            // Discovery saw the tab open and never commit. Its target names the URL it navigates
            // to by now (measured), so only discovery's report tells.
            (info, Some(opened)) if !opened.committed => PageId::new(
                opened
                    .opener
                    .clone()
                    .or_else(|| info.and_then(|info| info.opener_id))?,
            ),
            (Some(info), None) if discovery::is_initial_empty(&info.url) => {
                PageId::new(info.opener_id?)
            }
            (Some(_), _) => id,
            (None, _) => return None,
        };
        if let Some(entry) = self.entry(&owner) {
            return Some(entry);
        }
        let tab = tab_info(&self.0.cdp, &owner).await.ok()??;
        if !self.0.scope.includes(&tab) {
            return None;
        }
        self.register(owner, false).ok()
    }

    /// The run's session-scoped state (`session_state.rs`).
    pub(crate) fn overrides(&self) -> &Overrides {
        &self.0.overrides
    }

    /// The `Runtime`'s permission grants (`session_state.rs`).
    pub(crate) fn permissions(&self) -> &Permissions {
        &self.0.permissions
    }

    /// This run's id among the `Runtime`'s runs.
    pub(crate) fn run_id(&self) -> u64 {
        self.0.run_id
    }

    /// A new page target discovery saw (`discovery.rs`). When the run listens for new pages and
    /// the target is a tab of the run's context, registers it, emits Playwright's `page` event on
    /// the context with its record, and starts its watcher, so its events from its first
    /// document on reach the run. `closed`: it closed before its first document committed,
    /// which Playwright reports too.
    pub(crate) async fn report_new(&self, info: &TargetInfo, closed: bool) {
        let hub = &self.0.events;
        // The opener is named only when the run knows it, so the SDK can emit its `popup`;
        // `page.opener()` reads the opener itself.
        let opener = info
            .opener_id
            .as_ref()
            .and_then(|opener| self.entry(&PageId::new(opener.clone())))
            .filter(|opener| !opener.closed.is_cancelled());
        let listening = hub.subscribed(CONTEXT_GUID, "page")
            || opener
                .as_ref()
                .is_some_and(|opener| hub.subscribed(&bridge::page_guid(&opener.id), "popup"));
        if !listening {
            return;
        }
        let id = PageId::new(info.target_id.clone());
        let Some(closed) = self.new_tab_state(&id, info, closed).await else {
            return;
        };
        let entry = match self.register(id, false) {
            Ok(entry) => entry,
            Err(error) => {
                debug!(?error, "a new page could not be registered");
                return;
            }
        };
        entry.discovered.store(true, Ordering::Release);
        if let Ok(mut known) = entry.opener.lock() {
            *known = info.opener_id.clone().map(PageId::new);
        }
        if closed {
            entry.closed.cancel();
        }
        let mut record = created_page(&entry, info.url.clone(), closed);
        record.initializer.opener = opener.map(|opener| PageChannel {
            guid: bridge::page_guid(&opener.id),
        });
        match serde_json::to_value(&record) {
            Ok(record) => {
                hub.emit(CONTEXT_GUID, "page", &json!({ "page": record }));
            }
            Err(error) => debug!(%error, "a new page's record could not be written"),
        }
        if !closed {
            let needs = hub.context_needs().unwrap_or(Needs::WATCHER);
            page_events::enable(
                self,
                entry,
                Needs {
                    watcher: true,
                    ..needs
                },
            );
        }
    }

    /// Whether a new target is a tab of the run's context, as `pages()` decides, and whether it
    /// is closed: `None` when it is not the run's. A live target BrowserOS knows no tab for is
    /// not a tab (an extension's action popup, a developer tools window) and is left out, as is a
    /// tab that closed between its commit and this read. A target destroyed before its first commit
    /// is the run's when it belonged to the run's browser context.
    async fn new_tab_state(&self, id: &PageId, info: &TargetInfo, closed: bool) -> Option<bool> {
        if !closed {
            return match tab_info(&self.0.cdp, id).await {
                Ok(Some(tab)) => self.0.scope.includes(&tab).then_some(false),
                Ok(None) => None,
                Err(error) => {
                    debug!(?error, "a new tab's info could not be read");
                    None
                }
            };
        }
        let context = self.0.scope.context.as_ref()?;
        (info.browser_context_id.as_ref() == Some(context)).then_some(true)
    }

    /// Playwright's `page.opener()`: the open tab of the run's context that opened `entry`, with
    /// its record, or `None`. Read live, so a closed opener is `None`, as Playwright's client
    /// returns `null` for an opener that has closed.
    pub(crate) async fn opener(
        &self,
        entry: &PageEntry,
    ) -> Result<Option<CreatedPage>, PlaywrightError> {
        let known = entry.opener.lock().ok().and_then(|opener| opener.clone());
        let opener = if let Some(opener) = known {
            opener
        } else {
            let reply = self
                .0
                .cdp
                .send_typed::<_, TargetInfoReply>(
                    "Target.getTargetInfo",
                    json!({ "targetId": entry.id.as_str() }),
                    None,
                )
                .await;
            // A closed page names no opener here; Playwright's would still name an open one,
            // but Neo never saw it.
            let Some(opener) = reply.ok().and_then(|reply| reply.target_info.opener_id) else {
                return Ok(None);
            };
            PageId::new(opener)
        };
        let Some(tab) = tab_info(&self.0.cdp, &opener).await? else {
            return Ok(None);
        };
        if !self.0.scope.includes(&tab) {
            return Ok(None);
        }
        let opener = self.register(opener, false)?;
        Ok(Some(created_page(&opener, tab.url, false)))
    }

    /// `ConsoleMessage.args()`: the run's handles of console message `id`'s arguments on page
    /// `target`, made at the first read (eval's `handle_for_remote_object`) in the frame that
    /// logged the message, and the same records after. The message's arguments are kept from when
    /// its event was produced, so a page closed by then still answers: its handles are made
    /// without the browser, and fail on use as on a closed page.
    pub(crate) async fn console_args(
        &self,
        target: &PageId,
        id: u64,
        handles: &Handles,
    ) -> Result<Value, PlaywrightError> {
        let entry = self
            .entry(target)
            .ok_or_else(|| PlaywrightError::error("neo-runtime: the run has no such page"))?;
        let session = entry
            .attached_session()
            .ok_or_else(PlaywrightError::target_closed)?;
        let kept = session.events.args.get(id).ok_or_else(|| {
            PlaywrightError::error(format!(
                "Neo keeps the arguments of a page's last {MAX_KEPT_ARGS} console messages \
                 a listener received; this message's are gone"
            ))
        })?;
        if let Some(records) = kept.handles.clone() {
            return Ok(records);
        }
        let tab = FrameTab {
            page: Arc::clone(&session),
            scope: FrameScope::Main,
        };
        let objects = kept
            .objects
            .iter()
            .map(|object| serde_json::from_value::<RemoteObject>(object.clone()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| PlaywrightError::error(error.to_string()))?;
        // A page closed before or while the script read them still has them: Playwright made its
        // handles as the message arrived. Using an object's handle then fails.
        let made = if entry.closed.is_cancelled() {
            Self::gone_args(&tab, &session, &kept, &objects, handles)?
        } else {
            match Self::live_args(&tab, &session, &kept, &objects, handles).await {
                Ok(made) => made,
                Err(_) if entry.closed.is_cancelled() => {
                    Self::gone_args(&tab, &session, &kept, &objects, handles)?
                }
                Err(error) => return Err(error),
            }
        };
        let records = Value::Array(
            made.into_iter()
                .map(serde_json::to_value)
                .collect::<Result<_, _>>()
                .map_err(|error| PlaywrightError::error(error.to_string()))?,
        );
        session.events.args.made(id, records.clone());
        Ok(records)
    }

    /// The frame a message's arguments live in: the frame that logged it, inside the
    /// out-of-process frame whose session reported it, else that frame, else the main frame.
    async fn args_frame(
        session: &Arc<PageSession>,
        kept: &Kept,
    ) -> Result<Arc<FrameContext>, PlaywrightError> {
        let main = session.frames.main();
        let parent = match &kept.host {
            Some(host) => {
                let root = session
                    .frames
                    .context(host, Some(&main.host))
                    .await
                    .map_err(|error| args_error(&error))?;
                Arc::clone(&root.host)
            }
            None => Arc::clone(&main.host),
        };
        match kept.frame.as_ref().or(kept.host.as_ref()) {
            None => Ok(main),
            Some(frame) => session
                .frames
                .context(frame, Some(&parent))
                .await
                .map_err(|error| args_error(&error)),
        }
    }

    /// The arguments' handles, each retained in the run's handle group of its frame's session.
    async fn live_args(
        tab: &FrameTab,
        session: &Arc<PageSession>,
        kept: &Kept,
        objects: &[RemoteObject],
        handles: &Handles,
    ) -> Result<Vec<evaluate::CreatedHandle>, PlaywrightError> {
        let frame = Self::args_frame(session, kept).await?;
        let mut made = Vec::with_capacity(objects.len());
        for object in objects {
            let handle = time::timeout(
                ARGS_WAIT,
                evaluate::handle_for_remote_object(tab, &frame, object.clone(), handles),
            )
            .await
            .map_err(|_| {
                PlaywrightError::error(
                    "Neo could not take the console message's arguments in time: a dialog may \
                     hold the page",
                )
            })?
            .map_err(|error| args_error(&error))?;
            made.push(handle);
        }
        Ok(made)
    }

    /// The arguments' handles after their page closed ([`evaluate::handle_for_gone_object`]),
    /// in the frame the run knew them in, else the main frame.
    fn gone_args(
        tab: &FrameTab,
        session: &Arc<PageSession>,
        kept: &Kept,
        objects: &[RemoteObject],
        handles: &Handles,
    ) -> Result<Vec<evaluate::CreatedHandle>, PlaywrightError> {
        let frame = kept
            .frame
            .as_ref()
            .or(kept.host.as_ref())
            .and_then(|frame| session.frames.cached(frame))
            .unwrap_or_else(|| session.frames.main());
        objects
            .iter()
            .map(|object| {
                evaluate::handle_for_gone_object(tab, &frame, object, handles)
                    .map_err(|error| args_error(&error))
            })
            .collect()
    }

    /// Deviation: `server/chromium/crPage.ts` `FrameSession._onFileChooserOpened`: `const utilityContext = await frame.utilityContext();`
    /// Playwright makes the input's handle in its utility world as the chooser opens; Neo makes it
    /// in the main world, where every Neo `ElementHandle` lives (`evaluate.rs`), at the first
    /// read, so `element().evaluate` sees the page's own globals.
    ///
    /// `FileChooser.element()`: the run's `ElementHandle` of the input file chooser `id` of page
    /// `target` opened for, made at the first read and the same record after.
    pub(crate) async fn file_chooser_element(
        &self,
        target: &PageId,
        id: u64,
        handles: &Handles,
    ) -> Result<Value, PlaywrightError> {
        let entry = self
            .entry(target)
            .ok_or_else(|| PlaywrightError::error("neo-runtime: the run has no such page"))?;
        let session = entry
            .attached_session()
            .ok_or_else(PlaywrightError::target_closed)?;
        let chooser = session.events.choosers.get(id).ok_or_else(|| {
            PlaywrightError::error(format!(
                "Neo keeps a page's last {MAX_KEPT_CHOOSERS} file choosers; this one is gone"
            ))
        })?;
        if let Some(record) = chooser.handle {
            return Ok(record);
        }
        let main = session.frames.main();
        // A frame inside an out-of-process frame lives on that frame's session, which the run
        // attached before it reported the chooser.
        let parent = match &chooser.host {
            Some(host) => {
                let root = session
                    .frames
                    .context(host, Some(&main.host))
                    .await
                    .map_err(|error| args_error(&error))?;
                Arc::clone(&root.host)
            }
            None => Arc::clone(&main.host),
        };
        let frame = session
            .frames
            .context(&chooser.frame, Some(&parent))
            .await
            .map_err(|error| args_error(&error))?;
        let tab = FrameTab {
            page: Arc::clone(&session),
            scope: FrameScope::Main,
        };
        let group = tab.group(&frame, "file chooser");
        let element = frame
            .element(group.group(), chooser.backend)
            .await
            .map_err(|error| args_error(&error))?;
        let preview = frame
            .call(
                Some(group.group()),
                &InjectedPreviewNodeParams {
                    node: element.clone(),
                },
            )
            .await
            .map_err(|error| args_error(&error))?
            .preview;
        let created = evaluate::element_handle_for(&tab, &frame, &element, &preview, handles)
            .await
            .map_err(|error| args_error(&error))?;
        let record = serde_json::to_value(created)
            .map_err(|error| PlaywrightError::error(error.to_string()))?;
        session.events.choosers.made(id, record.clone());
        Ok(record)
    }

    pub(crate) fn outcome(&self) -> Vec<RunPage> {
        self.0.pages.lock().map_or_else(
            |_| Vec::new(),
            |pages| {
                pages
                    .iter()
                    .map(|entry| RunPage {
                        id: entry.id.as_str().to_string(),
                        created: entry.created.load(Ordering::Acquire),
                        closed: entry.closed_by_run.load(Ordering::Acquire),
                    })
                    .collect()
            },
        )
    }
}

impl PageEntry {
    pub(crate) fn request_close(&self) {
        if let Ok(mut metadata) = self.frame_metadata.lock() {
            metadata.closing();
        }
        self.close_requested.store(true, Ordering::Release);
    }
    /// An attached frame can enter its handler without a cancellation point. The handler
    /// records its start before checking progress, even if the run already ended.
    pub(crate) fn attached_session(&self) -> Option<Arc<PageSession>> {
        self.session.get().map(Arc::clone)
    }

    pub(crate) fn closed_by_run(&self) {
        self.closed_by_run.store(true, Ordering::Release);
        self.close_requested.store(false, Ordering::Release);
    }
}

impl Scope {
    fn includes(&self, info: &TabInfo) -> bool {
        self.context.as_ref().map_or_else(
            || self.window.is_some() && self.window == info.window_id,
            |context| info.live_browser_context_id.as_ref() == Some(context),
        )
    }
}

pub(crate) async fn tab_info(
    cdp: &CdpClient,
    page: &PageId,
) -> Result<Option<TabInfo>, PlaywrightError> {
    let reply = cdp
        .send_typed::<_, TabReply>(
            "Browser.getTabInfo",
            json!({ "targetId": page.as_str() }),
            None,
        )
        .await;
    match reply {
        Ok(reply) => Ok(Some(reply.tab)),
        Err(CdpError::Protocol { message, .. })
            if message.contains(getters::CLOSED_TAB_MESSAGE) =>
        {
            Ok(None)
        }
        Err(error) => Err(cdp_error(&error)),
    }
}

/// Why a console message's argument, or a file chooser's input, could not become a handle.
fn args_error(error: &PageError) -> PlaywrightError {
    match error {
        PageError::Closed => PlaywrightError::target_closed(),
        PageError::NotConnected => {
            PlaywrightError::error("the value is gone with the document that held it")
        }
        PageError::Threw(message) | PageError::Contract(message) => {
            PlaywrightError::error(message.clone())
        }
    }
}

pub(crate) fn cdp_error(error: &CdpError) -> PlaywrightError {
    PlaywrightError::error(error.to_string())
}

fn created_page(entry: &PageEntry, url: String, closed: bool) -> CreatedPage {
    let frame = bridge::frame_guid(&entry.id);
    CreatedPage {
        guid: bridge::page_guid(&entry.id),
        initializer: PageInitializer {
            main_frame: FrameChannel {
                guid: frame.clone(),
            },
            viewport_size: None,
            is_closed: closed,
            opener: None,
            video: None,
        },
        main_frame: CreatedFrame {
            guid: frame,
            initializer: FrameInitializer {
                url,
                name: String::new(),
                parent_frame: None,
                load_states: Vec::new(),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_browser::{FakeBrowser, PAGE, Replies};

    #[tokio::test]
    async fn guids_resolve_the_registered_tab_and_starting_frame_scope() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let (tabs, _) = Tabs::open(
            browser.cdp().await,
            Handle::current(),
            TabInputs::default(),
            TabScreenshots::default(),
            Switches::default(),
            Permissions::default(),
            TaskTracker::new(),
            &PageId::new(PAGE.to_string()),
        )
        .await
        .unwrap();
        for (guid, scope) in [
            (format!("page@{PAGE}"), FrameScope::Main),
            (format!("frame@{PAGE}"), FrameScope::Main),
            (
                format!("frame@{PAGE}/child"),
                FrameScope::Frame("child".to_string()),
            ),
            // goto's Request and Response belong to the main frame's navigation.
            (format!("request@{PAGE}:loader:0"), FrameScope::Main),
            (format!("response@{PAGE}:loader:1"), FrameScope::Main),
        ] {
            let (page, actual) = tabs.resolve(&guid).unwrap();
            assert_eq!(page.id.as_str(), PAGE);
            assert_eq!(actual, scope);
        }
        for guid in [
            format!("page@{PAGE}/child"),
            format!("frame@{PAGE}/"),
            format!("frame@{PAGE}/one/two"),
            "frame@unknown/child".to_string(),
            "response@unknown:loader:0".to_string(),
            "element@fake-page".to_string(),
        ] {
            assert!(tabs.resolve(&guid).is_none(), "{guid}");
        }
    }
}
