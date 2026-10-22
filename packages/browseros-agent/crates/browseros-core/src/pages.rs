use crate::{
    BrowserContextId, BrowserProfileId, CoreError, PageId, ProtocolSession, SessionId, TabId,
    TargetError, TargetId, WindowId,
    connection::{CdpConnection, EXCLUDED_URL_PREFIXES, EpochGuard},
    profiles::ProfileMeta,
    target::PageAdmission,
    timeouts,
};
use browseros_cdp::{browser, target};
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
};
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
    pub window_id: Option<WindowId>,
    pub index: Option<i64>,
    pub group_id: Option<String>,
    pub browser_profile_id: Option<BrowserProfileId>,
    pub live_browser_context_id: Option<BrowserContextId>,
}

impl PageInfo {
    /// Native target and ownership distinguish an incarnation from a reusable PageId.
    /// Readiness/navigation may update presentation fields without changing this identity.
    pub(crate) fn same_incarnation(&self, other: &Self) -> bool {
        self.page_id == other.page_id
            && self.target_id == other.target_id
            && self.tab_id == other.tab_id
            && self.window_id == other.window_id
            && self.browser_profile_id == other.browser_profile_id
            && self.live_browser_context_id == other.live_browser_context_id
    }
}

/// Identity authority taken from the native create reply, before registration/readiness can
/// await. Views retain this receipt separately from the globally reconciled page inventory.
#[derive(Clone)]
pub(crate) struct PageCreation {
    pub(crate) original: PageInfo,
    pub(crate) guard: EpochGuard,
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
    // Pages the agent opened (via new_page), as opposed to tabs it adopted from a
    // list. Only these get focus emulation on attach, so an adopted user tab keeps
    // its real focus state (#2778).
    agent_created: HashSet<PageId>,
    connection_epoch: u64,
    next_page_id: u32,
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
        self.list_with_epoch(None).await
    }

    /// View-owned reconciliation still sees the full browser, but cannot publish work to a
    /// replacement connection. Global callers retain the original unrestricted entry point.
    pub(crate) async fn list_with_epoch(
        &self,
        guard: Option<crate::connection::EpochGuard>,
    ) -> Result<Vec<PageInfo>, CoreError> {
        self.ensure_connected_with_epoch(guard.as_ref()).await?;
        let root = ProtocolSession::root(self.cdp.clone()).with_epoch_guard(guard.clone());
        let result: NativeTabs = root.send("Browser.getTabs", json!({})).await?;
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
        if let Some(guard) = &guard {
            guard.check(self.cdp.as_ref())?;
        }
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
                    let prior_window = existing.window_id;
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
                    state.agent_created.remove(&page_id);
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

    /// Snapshot only: callers must first reconcile the complete native inventory. A view must
    /// never prune global page ownership using only its projected response.
    pub async fn known_page_ids(&self) -> BTreeSet<PageId> {
        self.state.lock().await.pages.keys().cloned().collect()
    }

    /// Admission brackets attachment because PageId survives replacement of a tab's TargetId.
    /// Retry one replacement, then fail rather than attach an unadmitted incarnation.
    pub async fn get_session_admitted(
        &self,
        page_id: PageId,
        admission: &dyn PageAdmission,
    ) -> Result<PageSession, CoreError> {
        self.get_session_admitted_with_epoch(page_id, admission, None)
            .await
    }

    pub(crate) async fn get_session_admitted_with_epoch(
        &self,
        page_id: PageId,
        admission: &dyn PageAdmission,
        guard: Option<crate::connection::EpochGuard>,
    ) -> Result<PageSession, CoreError> {
        for _ in 0..2 {
            let before = self
                .refresh_with_epoch(page_id.clone(), guard.clone())
                .await?
                .ok_or_else(|| CoreError::UnknownPage(page_id.clone()))?;
            admission.admit(&before)?;
            let session_id = self
                .attach_with_epoch(before.target_id.clone(), page_id.clone(), guard.clone())
                .await?;
            let after = self
                .refresh_with_epoch(page_id.clone(), guard.clone())
                .await?
                .ok_or_else(|| CoreError::UnknownPage(page_id.clone()))?;
            if before.target_id != after.target_id {
                continue;
            }
            admission.admit(&after)?;
            return Ok(PageSession {
                target_id: after.target_id,
                session_id: session_id.clone(),
                session: ProtocolSession::for_session(self.cdp.clone(), session_id)
                    .with_epoch_guard(guard),
                url: after.url,
            });
        }
        Err(TargetError::PageTargetChanged(page_id).into())
    }

    pub(crate) async fn page_for_session(&self, session: &SessionId) -> Option<PageInfo> {
        let state = self.state.lock().await;
        let target = state
            .sessions
            .iter()
            .find_map(|(target, id)| (id == session).then_some(target))?;
        state
            .pages
            .values()
            .find(|page| &page.target_id == target)
            .cloned()
    }

    /// Synchronous signal reads use only the last reconciled inventory, never initiate CDP.
    /// A busy registry fails closed; browser operations still perform fresh async admission.
    pub(crate) fn admitted_cached(
        &self,
        page: &PageId,
        scope: &crate::CapturedScope,
        windows: &BTreeSet<WindowId>,
    ) -> bool {
        self.state
            .try_lock()
            .ok()
            .and_then(|state| state.pages.get(page).cloned())
            .is_some_and(|page| {
                page.browser_profile_id.as_ref() == Some(&scope.profile.browser_profile_id)
                    && scope.context_id.is_some()
                    && page.live_browser_context_id == scope.context_id
                    && page.window_id.is_some_and(|id| windows.contains(&id))
            })
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
        self.refresh_with_epoch(page_id, None).await
    }

    pub(crate) async fn refresh_with_epoch(
        &self,
        page_id: PageId,
        guard: Option<crate::connection::EpochGuard>,
    ) -> Result<Option<PageInfo>, CoreError> {
        self.ensure_connected_with_epoch(guard.as_ref()).await?;
        let info = {
            let state = self.state.lock().await;
            state.pages.get(&page_id).cloned()
        };
        let Some(info) = info else {
            self.list_with_epoch(guard).await?;
            return Ok(self.state.lock().await.pages.get(&page_id).cloned());
        };
        let root = ProtocolSession::root(self.cdp.clone()).with_epoch_guard(guard.clone());
        let refreshed = root
            .send::<_, NativeTabReply>("Browser.getTabInfo", json!({ "tabId": info.tab_id.0 }))
            .await;
        match refreshed {
            Ok(result) => {
                let updated = page_info_from_tab(page_id.clone(), result.tab, info.window_id);
                let mut state = self.state.lock().await;
                if let Some(guard) = &guard {
                    guard.check(self.cdp.as_ref())?;
                }
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
            Err(err @ CoreError::Target(_)) => Err(err),
            Err(_err) => {
                self.list_with_epoch(guard).await?;
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
        let (creation, _) = self.new_page_info(url, opts).await?;
        Ok(creation.original.page_id)
    }

    /// Ordinary unbound creation keeps its placement and best-effort default grouping. Once
    /// native creation returns, all follow-up work belongs to that exact target and epoch;
    /// neither readiness nor a later registry lookup may provide a new initial identity.
    pub(crate) async fn new_page_info(
        &self,
        url: &str,
        opts: NewPageOptions,
    ) -> Result<(PageCreation, PageInfo), CoreError> {
        self.ensure_connected().await?;
        let epoch = self.cdp.connection_epoch();
        let root = ProtocolSession::root(self.cdp.clone());
        let mut params = serde_json::Map::new();
        params.insert("url".to_string(), Value::String(url.to_string()));
        if let Some(context) = opts.browser_context_id {
            params.insert("browserContextId".to_string(), Value::String(context.0));
        }
        if let Some(background) = opts.background {
            params.insert("background".to_string(), Value::Bool(background));
        }
        if let Some(window_id) = opts.window_id {
            params.insert("windowId".to_string(), Value::from(window_id.0));
        }
        let created: NativeTabReply = root
            .send("Browser.createTab", Value::Object(params))
            .await?;
        let creation = self.register_created(created.tab, url, epoch, None).await?;
        let root = root.with_epoch_guard(Some(creation.guard.clone()));
        let tab_id = creation.original.tab_id.0;

        // Retry only until the new tab answers at all. Readiness used to be
        // decided here too, by polling load_progress to 1.0 for up to 3000ms,
        // which is the load event rather than the page being usable: measured
        // on this surface, three of seven page opens returned on that
        // exhaustion having learned nothing, and the work that followed each
        // took under 25ms. Stability answers it below instead.
        let mut tab = None;
        for _attempt in 0..timeouts::NEW_PAGE_ANSWERS_ATTEMPTS {
            let result = root
                .send::<_, NativeTabReply>("Browser.getTabInfo", json!({ "tabId": tab_id }))
                .await;
            match result {
                Ok(result) => {
                    tab = Some(self.created_readback(&creation, result.tab).await?);
                    break;
                }
                Err(err @ CoreError::Target(_)) => return Err(err),
                Err(_) => {}
            }
            sleep(timeouts::NEW_PAGE_READY_POLL).await;
        }
        let mut tab = tab
            .ok_or_else(|| CoreError::Message(format!("Tab {tab_id} not found after creation")))?;

        if let Some(group_id) = opts.tab_group_id {
            // The readiness loop can await registry locks and sleeps. Refresh the ORIGINAL
            // incarnation before the final native-tab mutation, never resolve its ID anew.
            let before: NativeTabReply = root
                .send("Browser.getTabInfo", json!({"tabId":tab_id}))
                .await?;
            tab = self.created_readback(&creation, before.tab).await?;
            let result: Result<browser::AddTabsToGroupResult, CoreError> = root
                .send(
                    "Browser.addTabsToGroup",
                    json!({ "groupId": group_id, "tabIds": [tab_id] }),
                )
                .await;
            match result {
                Err(err @ CoreError::Target(_)) => return Err(err),
                Err(err) => warn!("failed to add new page to default tab group: {err}"),
                Ok(_) => match root
                    .send::<_, NativeTabReply>("Browser.getTabInfo", json!({ "tabId": tab_id }))
                    .await
                {
                    Ok(result) => tab = self.created_readback(&creation, result.tab).await?,
                    Err(err @ CoreError::Target(_)) => return Err(err),
                    Err(_) => {}
                },
            }
        }
        self.publish_created_readback(&creation, &tab).await?;
        // Readiness goes last, after every call that resolves this tab's
        // identity. The wait can take a quiet interval or the whole ceiling,
        // and a tab can be reconciled onto a new target while it does, so
        // running it earlier would widen the window in which the native
        // mutations above see a replacement and fail the creation outright.
        // Its outcome is advisory: a page still moving at the ceiling is
        // handed back anyway, which is what the ceiling is for.
        let _ = crate::settle::wait_for_page_ready(
            self,
            creation.original.page_id.clone(),
            timeouts::ARRIVAL_READY_BUDGET,
        )
        .await;
        Ok((creation, tab))
    }

    /// Return the actual native create snapshot, rather than only its reusable registry ID.
    /// A bound view must seed admission from this return before its first readiness await.
    pub(crate) async fn adopt_created(
        &self,
        raw: Value,
        url: &str,
        guard: EpochGuard,
    ) -> Result<PageCreation, CoreError> {
        let tab: NativeTab =
            serde_json::from_value(raw).map_err(|err| CoreError::Message(err.to_string()))?;
        self.register_created(tab, url, guard.expected, Some(guard.invalidated))
            .await
    }

    async fn register_created(
        &self,
        mut tab: NativeTab,
        url: &str,
        epoch: u64,
        invalidated: Option<TargetError>,
    ) -> Result<PageCreation, CoreError> {
        if tab.url.is_empty() {
            tab.url = url.to_string();
        }
        let mut state = self.state.lock().await;
        let existing = find_by_target(&state.pages, &TargetId(tab.target_id.clone()))
            .or_else(|| find_by_tab(&state.pages, TabId(tab.tab_id)));
        let page_id = existing.unwrap_or_else(|| {
            let id = PageId(state.next_page_id);
            state.next_page_id += 1;
            id
        });
        let guard = EpochGuard {
            expected: epoch,
            invalidated: invalidated
                .unwrap_or_else(|| TargetError::PageTargetChanged(page_id.clone())),
        };
        guard.check(self.cdp.as_ref())?;
        let original = page_info_from_tab(page_id.clone(), tab, None);
        // A concurrent global reconciliation may have seen creation already, or may have
        // rebound its native tab. Preserve the allocated ID, but never overwrite replacement
        // identity with an older response and then treat a fresh read as original creation.
        if let Some(current) = state.pages.get(&page_id)
            && !original.same_incarnation(current)
        {
            return Err(TargetError::PageTargetChanged(page_id).into());
        }
        state.pages.insert(page_id.clone(), original.clone());
        state.agent_created.insert(page_id.clone());
        Ok(PageCreation { original, guard })
    }

    async fn created_readback(
        &self,
        creation: &PageCreation,
        tab: NativeTab,
    ) -> Result<PageInfo, CoreError> {
        let mut fresh = page_info_from_tab(
            creation.original.page_id.clone(),
            tab,
            creation.original.window_id,
        );
        if fresh.url.is_empty() {
            fresh.url.clone_from(&creation.original.url);
        }
        self.publish_created_readback(creation, &fresh).await?;
        Ok(fresh)
    }

    /// Readbacks can update loading/title/group data only while both the native response and
    /// the registry still describe creation. This check also closes the registration lock
    /// handoff: another global list cannot make a replacement authoritative for this call.
    async fn publish_created_readback(
        &self,
        creation: &PageCreation,
        fresh: &PageInfo,
    ) -> Result<(), CoreError> {
        let mut state = self.state.lock().await;
        creation.guard.check(self.cdp.as_ref())?;
        if !creation.original.same_incarnation(fresh)
            || state
                .pages
                .get(&creation.original.page_id)
                .is_none_or(|current| !creation.original.same_incarnation(current))
        {
            return Err(TargetError::PageTargetChanged(creation.original.page_id.clone()).into());
        }
        state.pages.insert(fresh.page_id.clone(), fresh.clone());
        Ok(())
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
        state.agent_created.remove(&page_id);
        drop(state);
        if let Some(callback) = &self.hooks.on_page_detached {
            callback(page_id);
        }
        Ok(())
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
        let result: NativeTabReply = root
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
        self.attach_with_epoch(target_id, page_id, None).await
    }

    async fn attach_with_epoch(
        &self,
        target_id: TargetId,
        page_id: PageId,
        guard: Option<crate::connection::EpochGuard>,
    ) -> Result<SessionId, CoreError> {
        self.ensure_connected_with_epoch(guard.as_ref()).await?;
        if let Some(cached) = self.state.lock().await.sessions.get(&target_id).cloned() {
            return Ok(cached);
        }

        let root = ProtocolSession::root(self.cdp.clone()).with_epoch_guard(guard.clone());
        let attached: target::AttachToTargetResult = root
            .send(
                "Target.attachToTarget",
                json!({ "targetId": target_id.as_str(), "flatten": true }),
            )
            .await?;
        let session_id = SessionId::from(attached.session_id);
        // Infrastructure setup and its frame-registration hook keep the epoch even though
        // they do not use the caller's page-method policy (e.g. Target.setAutoAttach).
        let session = ProtocolSession::for_session(self.cdp.clone(), session_id.clone())
            .with_epoch_guard(guard.clone());
        let _: Value = session.send("Page.enable", json!({})).await?;
        let _: Value = session.send("DOM.enable", json!({})).await?;
        let _: Value = session.send("Runtime.enable", json!({})).await?;
        let _: Value = session.send("Accessibility.enable", json!({})).await?;
        let _ = session
            .send::<_, Value>("Runtime.runIfWaitingForDebugger", json!({}))
            .await;
        // Agent tabs open in the background, where Chromium throttles layout,
        // hit-testing and the compositor, so click/fill wait seconds for a CDP
        // ack. Emulating focus keeps a tab the agent opened rendering at full
        // speed. Scoped to agent-created tabs so an adopted user tab keeps its
        // real focus state; best-effort, so it never fails attach (#2778).
        let is_agent_created = self.state.lock().await.agent_created.contains(&page_id);
        if is_agent_created
            && let Err(err) = session
                .send::<_, Value>(
                    "Emulation.setFocusEmulationEnabled",
                    json!({ "enabled": true }),
                )
                .await
        {
            warn!("failed to enable focus emulation for agent page {page_id:?}: {err}");
        }
        {
            let mut state = self.state.lock().await;
            if let Some(guard) = &guard {
                guard.check(self.cdp.as_ref())?;
            }
            state.sessions.insert(target_id, session_id.clone());
        }
        if let Some(callback) = &self.hooks.on_session_attached {
            callback(session, page_id, session_id.clone()).await?;
        }
        Ok(session_id)
    }

    async fn ensure_connected_with_epoch(
        &self,
        guard: Option<&crate::connection::EpochGuard>,
    ) -> Result<(), CoreError> {
        if let Some(guard) = guard {
            guard.check(self.cdp.as_ref())?;
            let mut state = self.state.lock().await;
            guard.check(self.cdp.as_ref())?;
            if state.connection_epoch != guard.expected {
                state.sessions.clear();
                state.connection_epoch = guard.expected;
            }
        } else {
            self.ensure_connected().await?;
        }
        Ok(())
    }

    async fn ensure_connected(&self) -> Result<bool, CoreError> {
        if !self.cdp.is_connected() {
            self.wait_for_connection().await?;
        }

        let epoch = self.cdp.connection_epoch();
        let mut state = self.state.lock().await;
        if epoch != state.connection_epoch {
            state.sessions.clear();
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

    async fn update_from_tab(
        &self,
        page_id: PageId,
        tab: NativeTab,
    ) -> Result<PageInfo, CoreError> {
        let prior_window = self
            .state
            .lock()
            .await
            .pages
            .get(&page_id)
            .and_then(|info| info.window_id);
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
    pub browser_context_id: Option<BrowserContextId>,
    pub background: Option<bool>,
    pub window_id: Option<WindowId>,
    pub tab_group_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MovePageOptions {
    pub window_id: Option<WindowId>,
    pub index: Option<i64>,
}

// Keep the full native reply: generated types on an older protocol snapshot discard the new
// identity fields. Parsing the same JSON into a sidecar works before and after protocol sync.
struct NativeTab {
    tab: browser::TabInfo,
    meta: ProfileMeta,
}
impl<'de> Deserialize<'de> for NativeTab {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(deserializer)?;
        // Deserialize twice from the same raw reply. Two serde(flatten) fields would consume
        // identity keys in the generated type after protocol sync, leaving the sidecar empty.
        Ok(Self {
            tab: serde_json::from_value(raw.clone()).map_err(serde::de::Error::custom)?,
            meta: serde_json::from_value(raw).map_err(serde::de::Error::custom)?,
        })
    }
}
impl std::ops::Deref for NativeTab {
    type Target = browser::TabInfo;
    fn deref(&self) -> &Self::Target {
        &self.tab
    }
}
impl std::ops::DerefMut for NativeTab {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.tab
    }
}
#[derive(Deserialize)]
struct NativeTabs {
    tabs: Vec<NativeTab>,
}
#[derive(Deserialize)]
struct NativeTabReply {
    tab: NativeTab,
}

fn page_info_from_tab(page_id: PageId, tab: NativeTab, prior_window: Option<WindowId>) -> PageInfo {
    let NativeTab { tab, meta } = tab;
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
        window_id: tab.window_id.map(WindowId).or_else(|| {
            // Older unbound binaries omit ownership. New metadata must never borrow stale
            // placement: a bound page needs an actual owning window from this native reply.
            if meta.browser_profile_id.is_none() && meta.live_browser_context_id.is_none() {
                prior_window
            } else {
                None
            }
        }),
        index: tab.index,
        group_id: tab.group_id,
        browser_profile_id: meta.browser_profile_id,
        live_browser_context_id: meta.live_browser_context_id,
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

#[cfg(test)]
mod tests {
    use super::{NewPageOptions, PageManager, PageManagerHooks};
    use crate::test_support::TestConnection;
    use serde_json::{Value, json};
    use std::error::Error;

    fn tab_json(tab_id: u64, target_id: &str) -> Value {
        json!({
            "tabId": tab_id,
            "targetId": target_id,
            "url": "https://example.com",
            "title": "Example",
            "isActive": true,
            "isLoading": false,
            "loadProgress": 1.0,
            "isPinned": false,
            "isHidden": false,
            "windowId": 3
        })
    }

    fn attach_responses() -> Vec<(&'static str, Value)> {
        vec![
            ("Target.attachToTarget", json!({ "sessionId": "session-7" })),
            ("Page.enable", json!({})),
            ("DOM.enable", json!({})),
            ("Runtime.enable", json!({})),
            ("Accessibility.enable", json!({})),
            ("Runtime.runIfWaitingForDebugger", json!({})),
        ]
    }

    #[tokio::test]
    async fn list_omits_hidden_tab_enumeration_option() -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([(
            "Browser.getTabs",
            json!({ "tabs": [tab_json(7, "target-7")] }),
        )]);
        let pages = PageManager::new(connection.clone(), PageManagerHooks::default())
            .list()
            .await?;

        assert_eq!(pages.len(), 1);
        assert_eq!(connection.calls()?[0].params, json!({}));
        Ok(())
    }

    #[tokio::test]
    async fn agent_created_tab_gets_focus_emulation_on_attach() -> Result<(), Box<dyn Error>> {
        let mut responses = vec![
            (
                "Browser.createTab",
                json!({ "tab": tab_json(7, "target-7") }),
            ),
            (
                "Browser.getTabInfo",
                json!({ "tab": tab_json(7, "target-7") }),
            ),
        ];
        responses.extend(attach_responses());
        responses.push(("Emulation.setFocusEmulationEnabled", json!({})));
        let connection = TestConnection::new(responses);
        let manager = PageManager::new(connection.clone(), PageManagerHooks::default());

        let page_id = manager
            .new_page("https://example.com", NewPageOptions::default())
            .await?;
        manager.get_session(page_id).await?;

        let calls = connection.calls()?;
        let focus = calls
            .iter()
            .find(|call| call.method == "Emulation.setFocusEmulationEnabled");
        assert_eq!(
            focus.map(|call| &call.params),
            Some(&json!({ "enabled": true }))
        );
        Ok(())
    }

    #[tokio::test]
    async fn adopted_tab_keeps_real_focus_state() -> Result<(), Box<dyn Error>> {
        let mut responses = vec![(
            "Browser.getTabs",
            json!({ "tabs": [tab_json(7, "target-7")] }),
        )];
        responses.extend(attach_responses());
        let connection = TestConnection::new(responses);
        let manager = PageManager::new(connection.clone(), PageManagerHooks::default());

        let pages = manager.list().await?;
        let page_id = pages.first().ok_or("adopted page missing")?.page_id.clone();
        manager.get_session(page_id).await?;

        let calls = connection.calls()?;
        assert!(
            !calls
                .iter()
                .any(|call| call.method == "Emulation.setFocusEmulationEnabled"),
            "adopted tab must not get focus emulation"
        );
        Ok(())
    }
}
