use crate::{
    CoreError, PageId, ProtocolSession, SessionId, TabId, TargetId, WindowId,
    connection::{CdpConnection, EXCLUDED_URL_PREFIXES},
    timeouts,
};
use browseros_cdp::{browser, target};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
use tokio::{sync::Mutex, time::sleep};
use tracing::warn;

pub type OnSessionAttached = Arc<
    dyn Fn(ProtocolSession, PageId, SessionId) -> BoxFuture<'static, Result<(), CoreError>>
        + Send
        + Sync,
>;
pub type OnPageDetached = Arc<dyn Fn(PageId) + Send + Sync>;

#[derive(Clone, Default)]
pub struct PageManagerHooks {
    pub on_session_attached: Option<OnSessionAttached>,
    pub on_page_detached: Option<OnPageDetached>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageInfo {
    pub page_id: PageId,
    pub target_id: TargetId,
    pub tab_id: TabId,
    pub url: String,
    pub title: String,
    pub is_active: bool,
    pub is_loading: bool,
    pub load_progress: f64,
    pub is_pinned: bool,
    pub is_hidden: bool,
    pub window_id: Option<WindowId>,
    pub index: Option<i64>,
    pub group_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PageSession {
    pub target_id: TargetId,
    pub session_id: SessionId,
    pub session: ProtocolSession,
    pub url: String,
}

#[derive(Debug, Default)]
struct PageState {
    pages: HashMap<PageId, PageInfo>,
    sessions: HashMap<TargetId, SessionId>,
    connection_epoch: u64,
    next_page_id: u32,
    hidden_window_id: Option<WindowId>,
}

pub struct PageManager {
    cdp: Arc<dyn CdpConnection>,
    hooks: PageManagerHooks,
    state: Mutex<PageState>,
}

impl PageManager {
    #[must_use]
    pub fn new(cdp: Arc<dyn CdpConnection>, hooks: PageManagerHooks) -> Self {
        Self {
            state: Mutex::new(PageState {
                connection_epoch: cdp.connection_epoch(),
                next_page_id: 1,
                ..PageState::default()
            }),
            cdp,
            hooks,
        }
    }

    pub async fn list(&self) -> Result<Vec<PageInfo>, CoreError> {
        self.ensure_connected().await?;
        let root = ProtocolSession::root(self.cdp.clone());
        let result: browser::GetTabsResult = root
            .send("Browser.getTabs", json!({ "includeHidden": true }))
            .await?;
        let tabs = result
            .tabs
            .into_iter()
            .filter(|tab| {
                !EXCLUDED_URL_PREFIXES
                    .iter()
                    .any(|prefix| tab.url.starts_with(prefix))
            })
            .collect::<Vec<_>>();

        let mut detached = Vec::new();
        let mut state = self.state.lock().await;
        let mut seen = Vec::<TargetId>::new();
        for tab in tabs {
            let target_id = TargetId(tab.target_id.clone());
            seen.push(target_id.clone());
            let existing_page_id = find_by_target(&state.pages, &target_id)
                .or_else(|| find_by_tab(&state.pages, TabId(tab.tab_id)));
            if let Some(page_id) = existing_page_id {
                let mut stale_target = None;
                if let Some(existing) = state.pages.get_mut(&page_id) {
                    if existing.target_id != target_id {
                        stale_target = Some(existing.target_id.clone());
                    }
                    let prior_window = existing.window_id.clone();
                    *existing = page_info_from_tab(page_id.clone(), tab, prior_window);
                }
                if let Some(stale_target) = stale_target {
                    state.sessions.remove(&stale_target);
                }
            } else {
                let page_id = PageId(state.next_page_id);
                state.next_page_id += 1;
                state
                    .pages
                    .insert(page_id.clone(), page_info_from_tab(page_id, tab, None));
            }
        }

        let existing = state.pages.keys().cloned().collect::<Vec<_>>();
        for page_id in existing {
            let Some(info) = state.pages.get(&page_id) else {
                continue;
            };
            if !seen.contains(&info.target_id) {
                let info = state.pages.remove(&page_id);
                if let Some(info) = info {
                    state.sessions.remove(&info.target_id);
                    detached.push(page_id);
                }
            }
        }

        let mut pages = state.pages.values().cloned().collect::<Vec<_>>();
        pages.sort_by_key(|page| page.page_id.0);
        drop(state);
        for page_id in detached {
            if let Some(callback) = &self.hooks.on_page_detached {
                callback(page_id);
            }
        }
        Ok(pages)
    }

    pub async fn get_info(&self, page_id: PageId) -> Option<PageInfo> {
        self.state.lock().await.pages.get(&page_id).cloned()
    }

    pub async fn get_tab_id(&self, page_id: PageId) -> Option<TabId> {
        self.state
            .lock()
            .await
            .pages
            .get(&page_id)
            .map(|info| info.tab_id.clone())
    }

    pub async fn get_session(&self, page_id: PageId) -> Result<PageSession, CoreError> {
        let reconnected = self.ensure_connected().await?;
        let mut info = self.state.lock().await.pages.get(&page_id).cloned();
        if info.is_none() || reconnected {
            self.list().await?;
            info = self.state.lock().await.pages.get(&page_id).cloned();
        }
        let info = info.ok_or_else(|| CoreError::UnknownPage(page_id.clone()))?;
        let session_id = self.attach(info.target_id.clone(), page_id).await?;
        Ok(PageSession {
            target_id: info.target_id,
            session_id: session_id.clone(),
            session: ProtocolSession::for_session(self.cdp.clone(), session_id),
            url: info.url,
        })
    }

    pub async fn get_attached_session(&self, page_id: PageId) -> Option<ProtocolSession> {
        let state = self.state.lock().await;
        let info = state.pages.get(&page_id)?;
        let session_id = state.sessions.get(&info.target_id)?;
        Some(ProtocolSession::for_session(
            self.cdp.clone(),
            session_id.clone(),
        ))
    }

    pub async fn get_active(&self) -> Result<Option<PageInfo>, CoreError> {
        self.ensure_connected().await?;
        let root = ProtocolSession::root(self.cdp.clone());
        let result: browser::GetActiveTabResult =
            root.send("Browser.getActiveTab", json!({})).await?;
        let Some(tab) = result.tab else {
            return Ok(None);
        };
        self.list().await?;
        let state = self.state.lock().await;
        Ok(find_by_target(&state.pages, &TargetId(tab.target_id))
            .and_then(|page_id| state.pages.get(&page_id).cloned()))
    }

    pub async fn get_active_session_for_window(
        &self,
        window_id: WindowId,
    ) -> Result<PageSession, CoreError> {
        self.ensure_connected().await?;
        let root = ProtocolSession::root(self.cdp.clone());
        let result: browser::GetActiveTabResult = root
            .send("Browser.getActiveTab", json!({ "windowId": window_id.0 }))
            .await?;
        let Some(tab) = result.tab else {
            return Err(CoreError::Message(format!(
                "No active tab in window {window_id}"
            )));
        };
        let page_id = self
            .ensure_page_id_for_target(TargetId(tab.target_id.clone()))
            .await?;
        let session_id = self
            .attach(TargetId(tab.target_id.clone()), page_id)
            .await?;
        Ok(PageSession {
            target_id: TargetId(tab.target_id),
            session_id: session_id.clone(),
            session: ProtocolSession::for_session(self.cdp.clone(), session_id),
            url: tab.url,
        })
    }

    pub async fn refresh(&self, page_id: PageId) -> Result<Option<PageInfo>, CoreError> {
        self.ensure_connected().await?;
        let info = {
            let state = self.state.lock().await;
            state.pages.get(&page_id).cloned()
        };
        let Some(info) = info else {
            self.list().await?;
            return Ok(self.state.lock().await.pages.get(&page_id).cloned());
        };
        let root = ProtocolSession::root(self.cdp.clone());
        let refreshed = root
            .send::<_, browser::GetTabInfoResult>(
                "Browser.getTabInfo",
                json!({ "tabId": info.tab_id.0 }),
            )
            .await;
        match refreshed {
            Ok(result) => {
                let updated =
                    page_info_from_tab(page_id.clone(), result.tab, info.window_id.clone());
                let mut state = self.state.lock().await;
                let Some(current) = state.pages.get(&page_id) else {
                    return Ok(None);
                };
                // A newer list or refresh may have rebound this reusable page id during CDP work.
                if current != &info {
                    return Ok(Some(current.clone()));
                }
                if updated.target_id != info.target_id {
                    state.sessions.remove(&info.target_id);
                }
                state.pages.insert(page_id, updated.clone());
                Ok(Some(updated))
            }
            Err(_err) => {
                self.list().await?;
                Ok(self.state.lock().await.pages.get(&page_id).cloned())
            }
        }
    }

    pub async fn resolve_tab_ids(
        &self,
        tab_ids: &[TabId],
    ) -> Result<HashMap<TabId, PageId>, CoreError> {
        self.list().await?;
        let state = self.state.lock().await;
        let mut out = HashMap::new();
        for info in state.pages.values() {
            if tab_ids.contains(&info.tab_id) {
                out.insert(info.tab_id.clone(), info.page_id.clone());
            }
        }
        Ok(out)
    }

    pub async fn new_page(&self, url: &str, opts: NewPageOptions) -> Result<PageId, CoreError> {
        self.ensure_connected().await?;
        let window_id = self.resolve_window_id_for_new_page(&opts).await?;
        let root = ProtocolSession::root(self.cdp.clone());
        let mut params = serde_json::Map::new();
        params.insert("url".to_string(), Value::String(url.to_string()));
        if let Some(background) = opts.background {
            params.insert("background".to_string(), Value::Bool(background));
        }
        if let Some(window_id) = window_id {
            params.insert("windowId".to_string(), Value::from(window_id.0));
        }
        let created: browser::CreateTabResult = root
            .send("Browser.createTab", Value::Object(params))
            .await?;
        let tab_id = created.tab.tab_id;

        let mut tab = None;
        for _attempt in 0..timeouts::NEW_PAGE_READY_ATTEMPTS {
            let result = root
                .send::<_, browser::GetTabInfoResult>(
                    "Browser.getTabInfo",
                    json!({ "tabId": tab_id }),
                )
                .await;
            if let Ok(result) = result {
                let ready = !result.tab.is_loading || result.tab.load_progress >= 1.0;
                tab = Some(result.tab);
                if ready {
                    break;
                }
            }
            sleep(timeouts::NEW_PAGE_READY_POLL).await;
        }
        let mut tab = tab
            .ok_or_else(|| CoreError::Message(format!("Tab {tab_id} not found after creation")))?;

        if let Some(group_id) = opts.tab_group_id {
            let result: Result<browser::AddTabsToGroupResult, CoreError> = root
                .send(
                    "Browser.addTabsToGroup",
                    json!({ "groupId": group_id, "tabIds": [tab_id] }),
                )
                .await;
            if let Err(err) = result {
                warn!("failed to add new page to default tab group: {err}");
            } else if let Ok(result) = root
                .send::<_, browser::GetTabInfoResult>(
                    "Browser.getTabInfo",
                    json!({ "tabId": tab_id }),
                )
                .await
            {
                tab = result.tab;
            }
        }

        let mut state = self.state.lock().await;
        let page_id = PageId(state.next_page_id);
        state.next_page_id += 1;
        if tab.url.is_empty() {
            tab.url = url.to_string();
        }
        state.pages.insert(
            page_id.clone(),
            page_info_from_tab(page_id.clone(), tab, None),
        );
        Ok(page_id)
    }

    pub async fn close(&self, page_id: PageId) -> Result<(), CoreError> {
        let info = self
            .state
            .lock()
            .await
            .pages
            .get(&page_id)
            .cloned()
            .ok_or_else(|| CoreError::UnknownPageShort(page_id.clone()))?;
        let root = ProtocolSession::root(self.cdp.clone());
        let _: Value = root
            .send("Browser.closeTab", json!({ "tabId": info.tab_id.0 }))
            .await?;
        let mut state = self.state.lock().await;
        state.pages.remove(&page_id);
        state.sessions.remove(&info.target_id);
        drop(state);
        if let Some(callback) = &self.hooks.on_page_detached {
            callback(page_id);
        }
        Ok(())
    }

    pub async fn show(
        &self,
        page_id: PageId,
        opts: ShowPageOptions,
    ) -> Result<PageInfo, CoreError> {
        self.ensure_connected().await?;
        let info = self
            .refresh(page_id.clone())
            .await?
            .ok_or_else(|| CoreError::UnknownPage(page_id.clone()))?;
        if !info.is_hidden {
            return Err(CoreError::Message(format!(
                "Page {page_id} is already visible."
            )));
        }
        let root = ProtocolSession::root(self.cdp.clone());
        let result: browser::ShowTabResult = root
            .send(
                "Browser.showTab",
                json!({
                    "tabId": info.tab_id.0,
                    "windowId": opts.window_id.map(|id| id.0),
                    "index": opts.index,
                    "activate": opts.activate
                }),
            )
            .await?;
        self.update_from_tab(page_id, result.tab).await
    }

    pub async fn move_page(
        &self,
        page_id: PageId,
        opts: MovePageOptions,
    ) -> Result<PageInfo, CoreError> {
        self.ensure_connected().await?;
        let info = self
            .refresh(page_id.clone())
            .await?
            .ok_or_else(|| CoreError::UnknownPage(page_id.clone()))?;
        let root = ProtocolSession::root(self.cdp.clone());
        let result: browser::MoveTabResult = root
            .send(
                "Browser.moveTab",
                json!({
                    "tabId": info.tab_id.0,
                    "windowId": opts.window_id.map(|id| id.0),
                    "index": opts.index
                }),
            )
            .await?;
        self.update_from_tab(page_id, result.tab).await
    }

    pub async fn detach_session(&self, session_id: &SessionId) {
        let mut state = self.state.lock().await;
        let target = state
            .sessions
            .iter()
            .find_map(|(target_id, existing)| (existing == session_id).then(|| target_id.clone()));
        if let Some(target) = target {
            state.sessions.remove(&target);
        }
    }

    async fn attach(&self, target_id: TargetId, page_id: PageId) -> Result<SessionId, CoreError> {
        self.ensure_connected().await?;
        if let Some(cached) = self.state.lock().await.sessions.get(&target_id).cloned() {
            return Ok(cached);
        }

        let root = ProtocolSession::root(self.cdp.clone());
        let attached: target::AttachToTargetResult = root
            .send(
                "Target.attachToTarget",
                json!({ "targetId": target_id.as_str(), "flatten": true }),
            )
            .await?;
        let session_id = SessionId::from(attached.session_id);
        let session = ProtocolSession::for_session(self.cdp.clone(), session_id.clone());
        let _: Value = session.send("Page.enable", json!({})).await?;
        let _: Value = session.send("DOM.enable", json!({})).await?;
        let _: Value = session.send("Runtime.enable", json!({})).await?;
        let _: Value = session.send("Accessibility.enable", json!({})).await?;
        let _ = session
            .send::<_, Value>("Runtime.runIfWaitingForDebugger", json!({}))
            .await;
        self.state
            .lock()
            .await
            .sessions
            .insert(target_id, session_id.clone());
        if let Some(callback) = &self.hooks.on_session_attached {
            callback(session, page_id, session_id.clone()).await?;
        }
        Ok(session_id)
    }

    async fn ensure_connected(&self) -> Result<bool, CoreError> {
        if !self.cdp.is_connected() {
            self.wait_for_connection().await?;
        }

        let epoch = self.cdp.connection_epoch();
        let mut state = self.state.lock().await;
        if epoch != state.connection_epoch {
            state.sessions.clear();
            state.hidden_window_id = None;
            state.connection_epoch = epoch;
            return Ok(true);
        }
        Ok(false)
    }

    async fn wait_for_connection(&self) -> Result<(), CoreError> {
        let deadline = tokio::time::Instant::now() + timeouts::WAIT_FOR_CONNECTION_TIMEOUT;
        while !self.cdp.is_connected() && tokio::time::Instant::now() < deadline {
            sleep(timeouts::WAIT_FOR_CONNECTION_POLL).await;
        }
        if self.cdp.is_connected() {
            Ok(())
        } else {
            Err(CoreError::Cdp(browseros_cdp::CdpError::NotConnected))
        }
    }

    async fn ensure_page_id_for_target(&self, target_id: TargetId) -> Result<PageId, CoreError> {
        if let Some(page_id) = find_by_target(&self.state.lock().await.pages, &target_id) {
            return Ok(page_id);
        }
        self.list().await?;
        find_by_target(&self.state.lock().await.pages, &target_id).ok_or_else(|| {
            CoreError::Message(format!("Could not resolve pageId for target {target_id}"))
        })
    }

    async fn resolve_window_id_for_new_page(
        &self,
        opts: &NewPageOptions,
    ) -> Result<Option<WindowId>, CoreError> {
        if !opts.hidden.unwrap_or(false) {
            return Ok(opts.window_id.clone());
        }

        let root = ProtocolSession::root(self.cdp.clone());
        let windows: browser::GetWindowsResult = root.send("Browser.getWindows", json!({})).await?;
        if let Some(requested) = &opts.window_id {
            if let Some(window) = windows
                .windows
                .iter()
                .find(|window| window.window_id == requested.0)
            {
                if !window.is_visible {
                    self.state.lock().await.hidden_window_id = Some(requested.clone());
                    return Ok(Some(requested.clone()));
                }
                warn!(
                    "requested hidden page target window is visible; creating hidden window instead"
                );
            }
            let hidden: browser::CreateWindowResult = root
                .send("Browser.createWindow", json!({ "hidden": true }))
                .await?;
            let window_id = WindowId(hidden.window.window_id);
            self.state.lock().await.hidden_window_id = Some(window_id.clone());
            return Ok(Some(window_id));
        }

        let cached = self.state.lock().await.hidden_window_id.clone();
        if let Some(cached) = cached
            && windows
                .windows
                .iter()
                .any(|window| window.window_id == cached.0 && !window.is_visible)
        {
            return Ok(Some(cached));
        }

        let hidden: browser::CreateWindowResult = root
            .send("Browser.createWindow", json!({ "hidden": true }))
            .await?;
        let window_id = WindowId(hidden.window.window_id);
        self.state.lock().await.hidden_window_id = Some(window_id.clone());
        Ok(Some(window_id))
    }

    async fn update_from_tab(
        &self,
        page_id: PageId,
        tab: browser::TabInfo,
    ) -> Result<PageInfo, CoreError> {
        let prior_window = self
            .state
            .lock()
            .await
            .pages
            .get(&page_id)
            .and_then(|info| info.window_id.clone());
        let updated = page_info_from_tab(page_id.clone(), tab, prior_window);
        self.state
            .lock()
            .await
            .pages
            .insert(page_id, updated.clone());
        Ok(updated)
    }
}

#[derive(Debug, Clone, Default)]
pub struct NewPageOptions {
    pub background: Option<bool>,
    pub hidden: Option<bool>,
    pub window_id: Option<WindowId>,
    pub tab_group_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ShowPageOptions {
    pub window_id: Option<WindowId>,
    pub index: Option<i64>,
    pub activate: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct MovePageOptions {
    pub window_id: Option<WindowId>,
    pub index: Option<i64>,
}

fn page_info_from_tab(
    page_id: PageId,
    tab: browser::TabInfo,
    prior_window: Option<WindowId>,
) -> PageInfo {
    PageInfo {
        page_id,
        target_id: TargetId(tab.target_id),
        tab_id: TabId(tab.tab_id),
        url: tab.url,
        title: tab.title,
        is_active: tab.is_active,
        is_loading: tab.is_loading,
        load_progress: tab.load_progress,
        is_pinned: tab.is_pinned,
        is_hidden: tab.is_hidden,
        window_id: tab.window_id.map(WindowId).or(prior_window),
        index: tab.index,
        group_id: tab.group_id,
    }
}

fn find_by_target(pages: &HashMap<PageId, PageInfo>, target_id: &TargetId) -> Option<PageId> {
    pages
        .values()
        .find(|info| &info.target_id == target_id)
        .map(|info| info.page_id.clone())
}

fn find_by_tab(pages: &HashMap<PageId, PageInfo>, tab_id: TabId) -> Option<PageId> {
    pages
        .values()
        .find(|info| info.tab_id == tab_id)
        .map(|info| info.page_id.clone())
}
