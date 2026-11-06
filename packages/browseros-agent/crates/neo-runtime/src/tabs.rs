//! A run's ordered page registry and lazy CDP sessions. Handing out a page registers its
//! identity; attaching it owns a session until the run and its last writer have let go.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use browseros_cdp::{CdpClient, CdpError};
use neo_runtime_spec::serve::{FrameChannel, FrameInitializer, PageInitializer};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::runtime::Handle;
use tokio::sync::OnceCell;
use tokio::time;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::bridge;
use crate::getters;
use crate::page::{PageSession, Session, TabInputs};
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
    scope: Scope,
    run_page: PageId,
    pages: Mutex<Vec<Arc<PageEntry>>>,
}

/// A page can be known without a session. Its closure token precedes the attachment so calls,
/// getters, and the eventual session watcher all observe the same close.
pub(crate) struct PageEntry {
    pub(crate) id: PageId,
    created: bool,
    closed_by_run: AtomicBool,
    pub(crate) closed: CancellationToken,
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
            scope: Scope {
                context: info.live_browser_context_id,
                window: info.window_id,
            },
            run_page: page.clone(),
            pages: Mutex::new(Vec::new()),
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

    pub(crate) fn finish_close(&self, page: Arc<PageSession>) {
        // Page.close can acknowledge before its beforeunload dialog arrives. Keep the
        // watcher alive after the script returns so it can answer; otherwise detaching
        // strands that dialog. Bound the cleanup if the browser never completes closure.
        self.0.tasks.spawn_on(
            async move {
                let _settled =
                    time::timeout(Duration::from_secs(5), page.closed().cancelled()).await;
                drop(page);
            },
            &self.0.tokio,
        );
    }

    pub(crate) fn resolve(&self, guid: &str) -> Option<Arc<PageEntry>> {
        let target = guid
            .strip_prefix("page@")
            .or_else(|| guid.strip_prefix("frame@"))?;
        self.0
            .pages
            .lock()
            .ok()?
            .iter()
            .find(|entry| entry.id.as_str() == target)
            .cloned()
    }

    fn register(&self, id: PageId, created: bool) -> Result<Arc<PageEntry>, PlaywrightError> {
        let mut pages = self.0.pages.lock().map_err(|_| {
            PlaywrightError::error("neo-runtime: the run's pages cannot be registered")
        })?;
        if let Some(entry) = pages.iter().find(|entry| entry.id == id) {
            return Ok(Arc::clone(entry));
        }
        let entry = Arc::new(PageEntry {
            id,
            created,
            closed_by_run: AtomicBool::new(false),
            closed: CancellationToken::new(),
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
                self.attach(entry).await
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
        let mut pages = Vec::new();
        for info in self.infos().await? {
            let entry = self.register(PageId::new(info.target_id), false)?;
            pages.push(created_page(&entry, info.url, false));
        }
        Ok(pages)
    }

    pub(crate) async fn initializer(
        &self,
        entry: &PageEntry,
    ) -> Result<CreatedPage, PlaywrightError> {
        let url = self.url(entry).await?;
        let closed = url.is_none();
        Ok(created_page(entry, url.unwrap_or_default(), closed))
    }

    pub(crate) async fn create(&self) -> Result<Arc<PageEntry>, PlaywrightError> {
        let mut params = json!({ "url": "about:blank", "background": true });
        let run_tab = tab_info(&self.0.cdp, &self.0.run_page).await?;
        let window = match run_tab {
            Some(info) => info.window_id,
            None => self.infos().await?.first().and_then(|info| info.window_id),
        };
        if let Some(window) = window {
            params["windowId"] = json!(window);
        } else if let Some(context) = &self.0.scope.context {
            params["browserContextId"] = json!(context);
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

    pub(crate) fn outcome(&self) -> Vec<RunPage> {
        self.0.pages.lock().map_or_else(
            |_| Vec::new(),
            |pages| {
                pages
                    .iter()
                    .map(|entry| RunPage {
                        id: entry.id.as_str().to_string(),
                        created: entry.created,
                        closed: entry.closed_by_run.load(Ordering::Acquire),
                    })
                    .collect()
            },
        )
    }
}

impl PageEntry {
    /// An attached frame can enter its handler without a cancellation point. The handler
    /// records its start before checking progress, even if the run already ended.
    pub(crate) fn attached_session(&self) -> Option<Arc<PageSession>> {
        self.session.get().map(Arc::clone)
    }

    pub(crate) fn closed_by_run(&self) {
        self.closed_by_run.store(true, Ordering::Release);
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
