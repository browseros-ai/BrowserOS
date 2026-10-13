//! Immutable execution views over a shared browser runtime. Task hosts own logical binding;
//! views capture it once and keep management callable even when normal admission fails.
use crate::{
    BrowserContextId, BrowserProfileId, BrowserSession, CoreError, PageId, SessionId, TabId,
    TargetError, WindowId,
};
use crate::{
    input::Input,
    navigation::Navigation,
    observer::Observer,
    page_signals::PageSignals,
    pages::{MovePageOptions, NewPageOptions, PageInfo, PageSession},
    profiles::{BrowserProfileInfo, LoadProfileOutcome, ProfileSelector},
    screenshot::{ScreenshotCaptureOptions, ScreenshotCaptureResult},
    windows::WindowInfo,
};
use browseros_cdp::{CdpError, CdpEvent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum WindowSelection {
    Working {
        #[serde(rename = "windowId")]
        window_id: WindowId,
    },
    Pinned {
        #[serde(rename = "windowId")]
        window_id: WindowId,
    },
}
/// Host-owned saved-profile intent. The directory guards native handle reuse; a working window
/// chooses placement, while a pin restricts every page/window capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetIntent {
    pub browser_profile_id: BrowserProfileId,
    /// Guards against stale process-lifetime handle reuse; name is display only.
    pub directory: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowSelection>,
}
/// Revision of an intent copied at call admission; core never reads the host store mid-call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetBinding {
    #[serde(flatten)]
    pub intent: TargetIntent,
    pub revision: u64,
}
/// Live identity and placement frozen for one call. Loading or creating a window requires a
/// later capture to use its new context or placement; this scope never refreshes into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedScope {
    pub binding: TargetBinding,
    pub profile: BrowserProfileInfo,
    pub context_id: Option<BrowserContextId>,
    pub placement_window: Option<WindowId>,
    pub connection_epoch: u64,
}
pub struct PageListing {
    pub pages: Vec<PageInfo>,
    /// Complete inventory after global reconciliation, including pages hidden by this view.
    pub live_page_ids: BTreeSet<PageId>,
    pub hidden: usize,
}
/// Synchronous admission hook used before and after native attachment. Implementations must
/// validate the supplied incarnation rather than authorize only the reusable PageId.
pub trait PageAdmission: Send + Sync {
    fn admit(&self, fresh: &PageInfo) -> Result<(), CoreError>;
}
/// Task execution capability over global infrastructure. Bound views expose guarded handles
/// and projected inventories; profile management can recover an invalid captured intent.
#[derive(Clone)]
pub struct BrowserView {
    session: Arc<BrowserSession>,
    binding: Option<TargetBinding>,
    capture: Result<Option<CapturedScope>, TargetError>,
    unbound_default_window: Option<WindowId>,
    signals: Arc<PageSignals>,
    signal_windows: Arc<std::sync::RwLock<BTreeSet<WindowId>>>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetStatus {
    pub profile: Option<BrowserProfileInfo>,
    pub windows: Vec<WindowId>,
    pub placement_window_id: Option<WindowId>,
    pub error: Option<String>,
}
mod cdp_policy;
mod events;
pub(crate) mod page_access;
use page_access::PageAccess;

/// Native window plus the identity sidecar from the same reply. Keeping the raw value avoids
/// treating the legacy directory label as a context while the generated protocol evolves.
#[derive(Clone)]
struct NativeWindow {
    info: WindowInfo,
    meta: crate::ProfileMeta,
    raw: Value,
}

struct LiveScope {
    scope: CapturedScope,
    windows: Vec<NativeWindow>,
}

impl WindowSelection {
    fn id(self) -> WindowId {
        match self {
            Self::Working { window_id } | Self::Pinned { window_id } => window_id,
        }
    }
}
impl CapturedScope {
    fn pin(&self) -> Option<WindowId> {
        match self.binding.intent.window {
            Some(WindowSelection::Pinned { window_id }) => Some(window_id),
            _ => None,
        }
    }
    fn context(&self) -> Result<&BrowserContextId, TargetError> {
        self.context_id
            .as_ref()
            .ok_or_else(|| TargetError::ProfileNotLoaded(self.profile.browser_profile_id.clone()))
    }
    fn window_matches(&self, window: &NativeWindow) -> bool {
        eligible(window, &self.profile)
            && self.pin().is_none_or(|pin| pin.0 == window.info.window_id)
    }
    fn page_matches(&self, page: &PageInfo, windows: &[NativeWindow]) -> bool {
        !crate::connection::EXCLUDED_URL_PREFIXES
            .iter()
            .any(|prefix| page.url.starts_with(prefix))
            && page.browser_profile_id.as_ref() == Some(&self.profile.browser_profile_id)
            && self.context_id.is_some()
            && page.live_browser_context_id == self.context_id
            && page.window_id.is_some_and(|id| {
                windows
                    .iter()
                    .any(|window| window.info.window_id == id.0 && self.window_matches(window))
            })
    }
    fn admit_page(&self, page: &PageInfo, windows: &[NativeWindow]) -> Result<(), CoreError> {
        if self.page_matches(page, windows) {
            Ok(())
        } else {
            Err(TargetError::PageOutOfScope(page.page_id.clone()).into())
        }
    }
}

fn eligible(window: &NativeWindow, profile: &BrowserProfileInfo) -> bool {
    window.raw["windowType"] == "normal"
        && window.meta.browser_profile_id.as_ref() == Some(&profile.browser_profile_id)
        && profile.browser_context_id.is_some()
        && window.meta.live_browser_context_id == profile.browser_context_id
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, CoreError> {
    serde_json::from_value(value).map_err(|err| CoreError::Message(err.to_string()))
}
fn native_missing(err: CoreError) -> CoreError {
    match &err {
        CoreError::Cdp(CdpError::Protocol { code, message })
            if *code == -32601
                || message.to_ascii_lowercase().contains("method not found")
                || message.contains("wasn't found") =>
        {
            TargetError::UnsupportedNative.into()
        }
        _ => err,
    }
}
fn capture_error(error: CoreError, id: &BrowserProfileId) -> TargetError {
    match error {
        CoreError::Target(error) => error,
        _ => TargetError::ProfileGone(id.clone()),
    }
}

impl BrowserView {
    pub async fn capture(
        session: Arc<BrowserSession>,
        binding: Option<TargetBinding>,
        unbound_default_window: Option<WindowId>,
    ) -> Self {
        let mut view = Self::unbound(session, unbound_default_window);
        if let Some(binding) = binding {
            // No lock or host store is retained. Rebinding later cannot change this call's context
            // or chosen placement; a failed capture still carries the intent for management.
            view.capture = view
                .capture_scope(&binding)
                .await
                .map(Some)
                .map_err(|err| capture_error(err, &binding.intent.browser_profile_id));
            view.binding = Some(binding);
            let scope = view.scope().ok().flatten().cloned();
            let pages = view.session.pages.clone();
            let connection = view.session.connection();
            let windows = view.signal_windows.clone();
            view.signals = PageSignals::scoped(
                &view.session.page_signals,
                Arc::new(move |page| {
                    let Some(scope) = &scope else {
                        return false;
                    };
                    connection.is_connected()
                        && connection.connection_epoch() == scope.connection_epoch
                        && windows
                            .read()
                            .ok()
                            .is_some_and(|windows| pages.admitted_cached(page, scope, &windows))
                }),
            );
            if view.scope().ok().flatten().is_some() {
                let _ = view.validate_live().await;
            }
        }
        view
    }
    pub fn unbound(session: Arc<BrowserSession>, unbound_default_window: Option<WindowId>) -> Self {
        Self {
            signals: session.page_signals.clone(),
            signal_windows: Arc::new(std::sync::RwLock::new(BTreeSet::new())),
            session,
            binding: None,
            capture: Ok(None),
            unbound_default_window,
        }
    }
    pub fn binding(&self) -> Option<&TargetBinding> {
        self.binding.as_ref()
    }
    pub fn scope(&self) -> Result<Option<&CapturedScope>, TargetError> {
        self.capture
            .as_ref()
            .map(Option::as_ref)
            .map_err(Clone::clone)
    }
    pub fn admission_error(&self) -> Option<&TargetError> {
        self.capture.as_ref().err()
    }

    pub async fn list_profiles(&self) -> Result<Vec<BrowserProfileInfo>, CoreError> {
        #[derive(Deserialize)]
        struct Reply {
            profiles: Vec<BrowserProfileInfo>,
        }
        let value = self
            .session
            .cdp("Browser.getProfiles", json!({}), None)
            .await
            .map_err(native_missing)?;
        Ok(decode::<Reply>(value)?.profiles)
    }
    pub async fn load_profile(
        &self,
        id: &BrowserProfileId,
    ) -> Result<LoadProfileOutcome, CoreError> {
        // Management deliberately ignores the view's capture. A stale view must be recoverable,
        // but loading it does not refresh that already admitted execution into a new context.
        crate::resolve_profile(
            &self.list_profiles().await?,
            &ProfileSelector::Id(id.clone()),
        )?;
        let value = self
            .session
            .cdp(
                "Browser.loadProfile",
                json!({"browserProfileId":id.as_str()}),
                None,
            )
            .await
            .map_err(|err| TargetError::ProfileLoadFailed(err.to_string()))?;
        let outcome: LoadProfileOutcome = decode(value)?;
        if outcome.profile.browser_profile_id != *id
            || !outcome.profile.loaded
            || outcome.profile.browser_context_id.is_none()
        {
            return Err(TargetError::ProfileLoadFailed(
                "native returned an inconsistent loaded profile".into(),
            )
            .into());
        }
        Ok(outcome)
    }
    async fn read_windows(&self) -> Result<Vec<NativeWindow>, CoreError> {
        let value = self
            .session
            .cdp("Browser.getWindows", json!({}), None)
            .await?;
        value["windows"]
            .as_array()
            .ok_or_else(|| CoreError::Message("missing native windows".into()))?
            .iter()
            .map(|raw| {
                Ok(NativeWindow {
                    info: decode(raw.clone())?,
                    meta: decode(raw.clone())?,
                    raw: raw.clone(),
                })
            })
            .collect()
    }
    pub async fn resolve_intent(
        &self,
        selector: &ProfileSelector,
        window: Option<WindowSelection>,
    ) -> Result<TargetIntent, CoreError> {
        let profile = crate::resolve_profile(&self.list_profiles().await?, selector)?;
        if let Some(selection) = window {
            let windows = self.read_windows().await?;
            let native = windows
                .iter()
                .find(|window| window.info.window_id == selection.id().0)
                .ok_or(TargetError::WindowIneligible(selection.id()))?;
            if native.raw["windowType"] != "normal"
                || native.meta.browser_profile_id.is_none()
                || native.meta.live_browser_context_id.is_none()
            {
                return Err(TargetError::WindowIneligible(selection.id()).into());
            }
            if !eligible(native, &profile) {
                return Err(TargetError::WindowOutOfScope(selection.id()).into());
            }
        }
        Ok(TargetIntent {
            browser_profile_id: profile.browser_profile_id,
            directory: profile.directory,
            name: profile.name,
            window,
        })
    }
    pub async fn intent_for_window(
        &self,
        window: WindowSelection,
    ) -> Result<TargetIntent, CoreError> {
        let windows = self.read_windows().await?;
        let native = windows
            .iter()
            .find(|native| native.info.window_id == window.id().0)
            .ok_or(TargetError::WindowIneligible(window.id()))?;
        if native.raw["windowType"] != "normal" || native.meta.live_browser_context_id.is_none() {
            return Err(TargetError::WindowIneligible(window.id()).into());
        }
        let id = native
            .meta
            .browser_profile_id
            .clone()
            .ok_or(TargetError::WindowIneligible(window.id()))?;
        self.resolve_intent(&ProfileSelector::Id(id), Some(window))
            .await
    }
    async fn capture_scope(&self, binding: &TargetBinding) -> Result<CapturedScope, CoreError> {
        let epoch = self.session.connection().connection_epoch();
        let profile = self
            .list_profiles()
            .await?
            .into_iter()
            .find(|profile| {
                profile.browser_profile_id == binding.intent.browser_profile_id
                    && profile.directory == binding.intent.directory
            })
            .ok_or_else(|| TargetError::ProfileGone(binding.intent.browser_profile_id.clone()))?;
        let windows = self.read_windows().await?;
        let eligible = windows
            .iter()
            .filter(|window| eligible(window, &profile))
            .collect::<Vec<_>>();
        let selected = match binding.intent.window {
            Some(WindowSelection::Pinned { window_id }) => Some(
                eligible
                    .iter()
                    .find(|window| window.info.window_id == window_id.0)
                    .ok_or(TargetError::PinnedWindowClosed(window_id))?
                    .info
                    .window_id,
            ),
            Some(WindowSelection::Working { window_id }) => eligible
                .iter()
                .find(|window| window.info.window_id == window_id.0)
                .map(|window| window.info.window_id),
            None => None,
        };
        let placement_window = selected
            .or_else(|| {
                eligible
                    .iter()
                    .find(|window| window.info.is_active)
                    .map(|window| window.info.window_id)
            })
            .or_else(|| eligible.iter().map(|window| window.info.window_id).max())
            .map(WindowId);
        if epoch != self.session.connection().connection_epoch() || !self.session.is_connected() {
            return Err(TargetError::ProfileGone(binding.intent.browser_profile_id.clone()).into());
        }
        Ok(CapturedScope {
            binding: binding.clone(),
            context_id: profile.browser_context_id.clone(),
            profile,
            placement_window,
            connection_epoch: epoch,
        })
    }
    pub async fn status(&self, binding: &TargetBinding) -> TargetStatus {
        let profile = match self.list_profiles().await {
            Ok(profiles) => profiles.into_iter().find(|profile| {
                profile.browser_profile_id == binding.intent.browser_profile_id
                    && profile.directory == binding.intent.directory
            }),
            Err(err) => {
                return TargetStatus {
                    profile: None,
                    windows: Vec::new(),
                    placement_window_id: None,
                    error: Some(err.to_string()),
                };
            }
        };
        let windows = match (&profile, self.read_windows().await) {
            (Some(profile), Ok(windows)) => windows
                .into_iter()
                .filter(|window| eligible(window, profile))
                .map(|window| WindowId(window.info.window_id))
                .collect(),
            (_, Err(err)) => {
                return TargetStatus {
                    profile,
                    windows: Vec::new(),
                    placement_window_id: None,
                    error: Some(err.to_string()),
                };
            }
            _ => Vec::new(),
        };
        match self.capture_scope(binding).await {
            Ok(scope) => TargetStatus {
                profile,
                windows,
                placement_window_id: scope.placement_window,
                error: None,
            },
            Err(err) => TargetStatus {
                profile,
                windows,
                placement_window_id: None,
                error: Some(err.to_string()),
            },
        }
    }
    async fn validate_live(&self) -> Result<Option<LiveScope>, CoreError> {
        let Some(scope) = self.scope()? else {
            return Ok(None);
        };
        if let Ok(mut windows) = self.signal_windows.write() {
            windows.clear();
        }
        self.check_epoch(scope)?;
        let current = self
            .list_profiles()
            .await?
            .into_iter()
            .find(|profile| {
                profile.browser_profile_id == scope.profile.browser_profile_id
                    && profile.directory == scope.binding.intent.directory
            })
            .ok_or_else(|| TargetError::ProfileGone(scope.profile.browser_profile_id.clone()))?;
        // Live identity is never refreshed in place. Even a successful load in this run requires
        // a fresh call to capture the new context; old sessions cannot start using it silently.
        if current.browser_context_id != scope.context_id || current.loaded != scope.profile.loaded
        {
            return Err(TargetError::ProfileGone(scope.profile.browser_profile_id.clone()).into());
        }
        let windows = self.read_windows().await?;
        if let Some(pin) = scope.pin()
            && !windows
                .iter()
                .any(|window| window.info.window_id == pin.0 && scope.window_matches(window))
        {
            return Err(TargetError::PinnedWindowClosed(pin).into());
        }
        self.check_epoch(scope)?;
        if let Ok(mut signal_windows) = self.signal_windows.write() {
            *signal_windows = windows
                .iter()
                .filter(|window| scope.window_matches(window))
                .map(|window| WindowId(window.info.window_id))
                .collect();
        }
        Ok(Some(LiveScope {
            scope: scope.clone(),
            windows,
        }))
    }
    fn check_epoch(&self, scope: &CapturedScope) -> Result<(), CoreError> {
        if !self.session.is_connected()
            || self.session.connection().connection_epoch() != scope.connection_epoch
        {
            return Err(TargetError::ProfileGone(scope.profile.browser_profile_id.clone()).into());
        }
        Ok(())
    }
    async fn validate_window(&self, id: WindowId) -> Result<(), CoreError> {
        if let Some(live) = self.validate_live().await? {
            live.scope.context()?;
            if !live
                .windows
                .iter()
                .any(|window| window.info.window_id == id.0 && live.scope.window_matches(window))
            {
                return Err(TargetError::WindowOutOfScope(id).into());
            }
        }
        Ok(())
    }
    pub async fn list_pages(&self) -> Result<PageListing, CoreError> {
        self.scope()?;
        // Reconciliation owns detachment and tab ownership. It must see every profile before
        // projection, otherwise listing one profile would erase the others from shared state.
        let all = self.session.pages.list().await?;
        let live_page_ids = all.iter().map(|page| page.page_id.clone()).collect();
        let count = all.len();
        let pages = if let Some(live) = self.validate_live().await? {
            all.into_iter()
                .filter(|page| live.scope.page_matches(page, &live.windows))
                .collect()
        } else {
            all
        };
        Ok(PageListing {
            hidden: count - pages.len(),
            pages,
            live_page_ids,
        })
    }
    pub async fn refresh_page(&self, page: PageId) -> Result<Option<PageInfo>, CoreError> {
        self.scope()?;
        let fresh = self.session.pages.refresh(page).await?;
        if let Some(live) = self.validate_live().await? {
            live.scope.context()?;
            if let Some(page) = &fresh {
                live.scope.admit_page(page, &live.windows)?;
            }
        }
        Ok(fresh)
    }
    pub async fn page(&self, page: PageId) -> Result<PageInfo, CoreError> {
        self.refresh_page(page.clone())
            .await?
            .ok_or(CoreError::UnknownPage(page))
    }
    async fn page_access(&self, page: PageId) -> Result<Option<Arc<PageAccess>>, CoreError> {
        if self.scope()?.is_none() {
            return Ok(None);
        }
        let fresh = self.page(page).await?;
        Ok(Some(Arc::new(PageAccess::new(self.clone(), fresh))))
    }
    pub async fn page_session(&self, page: PageId) -> Result<PageSession, CoreError> {
        match self.page_access(page.clone()).await? {
            Some(access) => access.session().await,
            None => self.session.pages.get_session(page).await,
        }
    }
    pub async fn observe(&self, page: PageId) -> Result<Arc<Observer>, CoreError> {
        let access = self.page_access(page.clone()).await?;
        let observer = self.session.observe(page).await;
        Ok(match access {
            Some(access) => Arc::new(observer.with_access(access)),
            None => observer,
        })
    }
    pub async fn input(&self, page: PageId) -> Result<Input, CoreError> {
        let access = self.page_access(page.clone()).await?;
        let observer = self.session.observe(page.clone()).await;
        Ok(match access {
            Some(access) => Input::new(
                Arc::new(observer.with_access(access.clone())),
                self.session.pages.clone(),
                page,
            )
            .with_access(access),
            None => Input::new(observer, self.session.pages.clone(), page),
        })
    }
    pub async fn nav(&self, page: PageId) -> Result<Navigation, CoreError> {
        let access = self.page_access(page.clone()).await?;
        let nav = self.session.nav(page);
        Ok(match access {
            Some(access) => nav.with_access(access),
            None => nav,
        })
    }
    pub async fn screenshot(
        &self,
        page: PageId,
        o: ScreenshotCaptureOptions,
    ) -> Result<ScreenshotCaptureResult, CoreError> {
        if self.scope()?.is_none() {
            return self.session.screenshot(page, o).await;
        }
        let access = self
            .page_access(page.clone())
            .await?
            .ok_or_else(|| cdp_policy::unsupported("Page.captureScreenshot"))?;
        let session = access.session().await?;
        let observer = Arc::new(self.session.observe(page).await.with_access(access.clone()));
        let capture =
            crate::screenshot::capture_screenshot_with_annotations(session.session, observer, o)
                .await?;
        access.fresh().await?;
        Ok(capture)
    }

    pub async fn active_page(&self) -> Result<Option<PageInfo>, CoreError> {
        let Some(scope) = self.scope()? else {
            return self.session.pages.get_active().await;
        };
        scope.context()?;
        let window = scope.placement_window.ok_or_else(|| {
            TargetError::NoProfileWindow(scope.profile.browser_profile_id.clone())
        })?;
        let result = self
            .cdp("Browser.getActiveTab", json!({"windowId":window.0}), None)
            .await?;
        let Some(target) = result["tab"]["targetId"].as_str() else {
            return Ok(None);
        };
        Ok(self
            .list_pages()
            .await?
            .pages
            .into_iter()
            .find(|page| page.target_id.as_str() == target))
    }
    pub async fn new_page(
        &self,
        url: &str,
        mut opts: NewPageOptions,
    ) -> Result<PageInfo, CoreError> {
        let Some(scope) = self.scope()? else {
            opts.window_id = opts.window_id.or(self.unbound_default_window);
            let page = self.session.pages.new_page(url, opts).await?;
            return self
                .session
                .pages
                .get_info(page.clone())
                .await
                .ok_or(CoreError::UnknownPage(page));
        };
        scope.context()?;
        let window = opts.window_id.or(scope.placement_window).ok_or_else(|| {
            TargetError::NoProfileWindow(scope.profile.browser_profile_id.clone())
        })?;
        if opts
            .browser_context_id
            .as_ref()
            .is_some_and(|id| Some(id) != scope.context_id.as_ref())
        {
            return Err(TargetError::CdpUnsupported {
                method: "Browser.createTab".into(),
            }
            .into());
        }
        self.validate_window(window).await?;
        if let Some(group) = &opts.tab_group_id {
            self.validate_group(group, Some(window)).await?;
        }
        let created = self
            .cdp(
                "Browser.createTab",
                json!({"url":url,"windowId":window.0,"background":true}),
                None,
            )
            .await?;
        let page = self
            .session
            .pages
            .adopt_created(created["tab"].clone(), url)
            .await?;
        let access = self
            .page_access(page.clone())
            .await?
            .ok_or_else(|| CoreError::Message("missing bound page admission".into()))?;
        for _ in 0..crate::timeouts::NEW_PAGE_READY_ATTEMPTS {
            let fresh = access.fresh().await?;
            if !fresh.is_loading || fresh.load_progress >= 1.0 {
                break;
            }
            tokio::time::sleep(crate::timeouts::NEW_PAGE_READY_POLL).await;
        }
        if let Some(group) = opts.tab_group_id {
            let fresh = access.fresh().await?;
            self.cdp(
                "Browser.addTabsToGroup",
                json!({"groupId":group,"tabIds":[fresh.tab_id.0]}),
                None,
            )
            .await?;
        }
        access.fresh().await
    }
    pub async fn close_page(&self, page: PageId) -> Result<(), CoreError> {
        if self.scope()?.is_none() {
            return self.session.pages.close(page).await;
        }
        let fresh = self.page(page).await?;
        self.cdp(
            "Browser.closeTab",
            json!({"tabId":fresh.tab_id.0,"targetId":fresh.target_id.as_str()}),
            None,
        )
        .await?;
        self.session.pages.list().await?;
        Ok(())
    }
    pub async fn move_page(
        &self,
        page: PageId,
        opts: MovePageOptions,
    ) -> Result<PageInfo, CoreError> {
        if self.scope()?.is_none() {
            return self.session.pages.move_page(page, opts).await;
        }
        let fresh = self.page(page.clone()).await?;
        let window = opts
            .window_id
            .or(fresh.window_id)
            .ok_or(TargetError::PageOutOfScope(page.clone()))?;
        let mut params =
            json!({"tabId":fresh.tab_id.0,"targetId":fresh.target_id.as_str(),"windowId":window.0});
        if let Some(index) = opts.index {
            params["index"] = json!(index);
        }
        self.cdp("Browser.moveTab", params, None).await?;
        self.page(page).await
    }
    pub async fn resolve_tab_ids(
        &self,
        tabs: &[TabId],
    ) -> Result<HashMap<TabId, PageId>, CoreError> {
        self.scope()?;
        let all = self.session.pages.list().await?;
        let mut resolved = HashMap::new();
        for tab in tabs {
            if let Some(page) = all.iter().find(|page| &page.tab_id == tab) {
                let admitted = self.page(page.page_id.clone()).await?;
                resolved.insert(admitted.tab_id, admitted.page_id);
            } else if self.binding.is_some() {
                return Err(cdp_policy::unsupported("Browser.resolveTabIds"));
            }
        }
        Ok(resolved)
    }
    pub async fn tab_id_of(&self, page: PageId) -> Option<TabId> {
        self.session.pages.get_tab_id(page).await
    }
    pub fn signals(&self) -> &Arc<PageSignals> {
        // This synchronous API exposes a read-only cache projection. Callers must perform the
        // relevant async page admission before readback; it cannot refresh native ownership.
        &self.signals
    }
    pub fn cdp_events(&self) -> broadcast::Receiver<CdpEvent> {
        self.scoped_events()
    }
    pub async fn list_windows(&self) -> Result<Vec<WindowInfo>, CoreError> {
        match self.validate_live().await? {
            Some(live) => Ok(live
                .windows
                .into_iter()
                .filter(|window| live.scope.window_matches(window))
                .map(|window| window.info)
                .collect()),
            None => self.session.windows.list().await,
        }
    }
    pub async fn create_window(&self) -> Result<WindowInfo, CoreError> {
        if self.scope()?.is_none() {
            return self.session.windows.create().await;
        }
        let result = self.cdp("Browser.createWindow", json!({}), None).await?;
        decode(result["window"].clone())
    }
    pub async fn close_window(&self, w: WindowId) -> Result<(), CoreError> {
        if self.scope()?.is_none() {
            return self.session.windows.close(w).await;
        }
        self.cdp("Browser.closeWindow", json!({"windowId":w.0}), None)
            .await?;
        Ok(())
    }
    pub async fn cdp(
        &self,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CoreError> {
        if self.scope()?.is_none() {
            return self.session.cdp(method, params, session).await;
        }
        if let Some(session) = session {
            let page = self
                .session
                .pages
                .page_for_session(session)
                .await
                .ok_or_else(|| cdp_policy::unsupported(method))?;
            let admitted = self.page_session(page.page_id).await?;
            if &admitted.session_id != session {
                return Err(cdp_policy::unsupported(method));
            }
            return admitted.session.send_value(method, params).await;
        }
        self.scoped_root_cdp(method, params).await
    }
    pub async fn cdp_json_for_page(
        &self,
        page: PageId,
        method: &str,
        params_json: &str,
    ) -> Result<String, CoreError> {
        if self.scope()?.is_none() {
            return self
                .session
                .cdp_json_for_page(page, method, params_json)
                .await;
        }
        self.page_session(page)
            .await?
            .session
            .send_raw_json(method, params_json)
            .await
    }
    pub async fn evaluate_extension_workers(
        &self,
        expression: &str,
    ) -> Result<Vec<Value>, CoreError> {
        let cancel = tokio_util::sync::CancellationToken::new();
        let _cancel_on_drop = cancel.clone().drop_guard();
        let view = self.clone();
        let expression = expression.to_string();
        // Keep the attach request owned until its response arrives, even if its caller drops.
        // Otherwise native could create a session after cancellation and lose its ID before
        // cleanup. Cancellation stops evaluation; this owned task always completes detachment.
        tokio::spawn(async move { view.evaluate_workers_owned(&expression, &cancel).await })
            .await
            .map_err(|err| CoreError::Message(format!("extension evaluation task failed: {err}")))?
    }

    async fn evaluate_workers_owned(
        &self,
        expression: &str,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Vec<Value>, CoreError> {
        let live = self.validate_live().await?;
        let context = live.as_ref().map(|live| live.scope.context()).transpose()?;
        let targets = self
            .session
            .cdp("Target.getTargets", json!({}), None)
            .await?;
        let mut values = Vec::new();
        for target in targets["targetInfos"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|target| {
                target["type"] == "service_worker"
                    && target["url"]
                        .as_str()
                        .is_some_and(|url| url.starts_with("chrome-extension://"))
                    && context.is_none_or(|context| target["browserContextId"] == context.as_str())
            })
        {
            self.validate_live().await?;
            if cancel.is_cancelled() {
                return Err(CoreError::Message("extension evaluation cancelled".into()));
            }
            let attach_epoch = self.session.connection().connection_epoch();
            let attached = match self
                .session
                .cdp(
                    "Target.attachToTarget",
                    json!({"targetId":target["targetId"],"flatten":true}),
                    None,
                )
                .await
            {
                Ok(attached) => attached,
                Err(_) => continue,
            };
            let session = attached["sessionId"]
                .as_str()
                .map(SessionId::from)
                .ok_or_else(|| CoreError::Message("missing extension sessionId".into()))?;
            // Attachment belongs to this adapter, not the global page registry. Always detach,
            // including failed evaluation/admission; callers receive only by-value candidates.
            let evaluation = async {
                if cancel.is_cancelled() {
                    return Err(CoreError::Message("extension evaluation cancelled".into()));
                }
                self.validate_live().await?;
                let fresh_targets = self
                    .session
                    .cdp("Target.getTargets", json!({}), None)
                    .await?;
                let fresh = fresh_targets["targetInfos"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|fresh| fresh["targetId"] == target["targetId"])
                    .ok_or_else(|| cdp_policy::unsupported("Runtime.evaluate"))?;
                if fresh["type"] != "service_worker"
                    || !fresh["url"]
                        .as_str()
                        .is_some_and(|url| url.starts_with("chrome-extension://"))
                    || context.is_some_and(|context| fresh["browserContextId"] != context.as_str())
                {
                    return Err(cdp_policy::unsupported("Runtime.evaluate"));
                }
                let result = self
                    .session
                    .cdp(
                        "Runtime.evaluate",
                        json!({"expression":expression,"returnByValue":true,"awaitPromise":true}),
                        Some(&session),
                    )
                    .await?;
                self.validate_live().await?;
                Ok::<_, CoreError>(result)
            };
            let evaluation = tokio::select! {
                result = evaluation => result,
                _ = cancel.cancelled() => Err(CoreError::Message("extension evaluation cancelled".into())),
            };
            // Session IDs belong to one websocket. Disconnect already releases the attachment;
            // never detach an old ID on a new connection that might have reused its value.
            if self.session.connection().connection_epoch() != attach_epoch {
                return Err(cdp_policy::unsupported("Target.detachFromTarget"));
            }
            let detach = self
                .session
                .cdp(
                    "Target.detachFromTarget",
                    json!({"sessionId":session}),
                    None,
                )
                .await;
            detach?;
            match evaluation {
                Ok(result) if result.get("exceptionDetails").is_none() => {
                    if let Some(value) = result["result"].get("value") {
                        values.push(value.clone());
                    }
                }
                Err(CoreError::Target(err)) => return Err(err.into()),
                _ => {}
            }
        }
        if values.is_empty() {
            Err(CoreError::Message(
                "No extension worker could be evaluated".into(),
            ))
        } else {
            Ok(values)
        }
    }
}

#[cfg(test)]
mod tests;
