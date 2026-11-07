//! A run's ordered page registry and lazy CDP sessions. Handing out a page registers its
//! identity; attaching it owns a session until the run and its last writer have let go.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use browseros_cdp::{CdpClient, CdpError};
use neo_runtime_spec::serve::{FrameChannel, FrameInitializer, PageInitializer};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::runtime::Handle;
use tokio::sync::{OnceCell, oneshot};
use tokio::time;
use tokio_util::sync::{CancellationToken, DropGuard};
use tokio_util::task::TaskTracker;
use tracing::debug;

use crate::bridge;
use crate::dialogs;
use crate::events::EventHub;
use crate::frames::FrameScope;
use crate::getters;
use crate::network::HopId;
use crate::page::{PageSession, Session, TabInputs};
use crate::page_events;
use crate::progress::PlaywrightError;
use crate::run::{PageId, RunPage};

/// Cheap clones share the run's registry, context boundary, and session ownership.
#[derive(Clone)]
pub(crate) struct Tabs(Arc<Inner>);

struct Inner {
    cdp: CdpClient,
    tokio: Handle,
    inputs: TabInputs,
    tasks: TaskTracker,
    creations: TaskTracker,
    scope: Scope,
    run_page: PageId,
    pages: Mutex<Vec<Arc<PageEntry>>>,
    /// The run's event queue and subscriptions, shared with every page session it attaches.
    events: EventHub,
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
    initializer: PageInitializer,
    main_frame: CreatedFrame,
}

#[derive(Serialize)]
struct CreatedFrame {
    guid: String,
    initializer: FrameInitializer,
}

impl Tabs {
    pub(crate) async fn open(
        cdp: CdpClient,
        tokio: Handle,
        inputs: TabInputs,
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
            tasks,
            creations: TaskTracker::new(),
            scope: Scope {
                context: info.live_browser_context_id,
                window: info.window_id,
            },
            run_page: page.clone(),
            pages: Mutex::new(Vec::new()),
            events: EventHub::default(),
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
        Ok(Arc::new(PageSession::new(
            session,
            &entry.id,
            self.0.tasks.clone(),
            input,
            entry.closed.clone(),
            self.0.events.clone(),
        )))
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
