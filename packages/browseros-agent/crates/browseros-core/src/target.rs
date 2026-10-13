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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetBinding {
    #[serde(flatten)]
    pub intent: TargetIntent,
    pub revision: u64,
}
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
pub trait PageAdmission: Send + Sync {
    fn admit(&self, fresh: &PageInfo) -> Result<(), CoreError>;
}
#[derive(Clone)]
pub struct BrowserView {
    session: Arc<BrowserSession>,
    binding: Option<TargetBinding>,
    capture: Result<Option<CapturedScope>, TargetError>,
    unbound_default_window: Option<WindowId>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetStatus {
    pub profile: Option<BrowserProfileInfo>,
    pub windows: Vec<WindowId>,
    pub placement_window_id: Option<WindowId>,
    pub error: Option<String>,
}
impl BrowserView {
    pub async fn capture(
        session: Arc<BrowserSession>,
        binding: Option<TargetBinding>,
        unbound_default_window: Option<WindowId>,
    ) -> Self {
        let capture = if binding.is_some() {
            Err(TargetError::UnsupportedNative)
        } else {
            Ok(None)
        };
        Self {
            session,
            binding,
            capture,
            unbound_default_window,
        }
    }
    pub fn unbound(session: Arc<BrowserSession>, unbound_default_window: Option<WindowId>) -> Self {
        Self {
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
            .map_err(|err| match err {
                CoreError::Cdp(CdpError::Protocol { code: -32601, .. }) => {
                    CoreError::Target(TargetError::UnsupportedNative)
                }
                err => err,
            })?;
        let reply: Reply =
            serde_json::from_value(value).map_err(|err| CoreError::Message(err.to_string()))?;
        Ok(reply.profiles)
    }
    pub async fn load_profile(
        &self,
        id: &BrowserProfileId,
    ) -> Result<LoadProfileOutcome, CoreError> {
        let value = self
            .session
            .cdp(
                "Browser.loadProfile",
                json!({"browserProfileId":id.as_str()}),
                None,
            )
            .await
            .map_err(|err| TargetError::ProfileLoadFailed(err.to_string()))?;
        serde_json::from_value(value).map_err(|err| CoreError::Message(err.to_string()))
    }
    pub async fn resolve_intent(
        &self,
        selector: &ProfileSelector,
        window: Option<WindowSelection>,
    ) -> Result<TargetIntent, CoreError> {
        let profile = crate::resolve_profile(&self.list_profiles().await?, selector)?;
        if window.is_some() {
            return Err(TargetError::UnsupportedNative.into());
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
        _window: WindowSelection,
    ) -> Result<TargetIntent, CoreError> {
        Err(TargetError::UnsupportedNative.into())
    }
    pub async fn status(&self, _binding: &TargetBinding) -> TargetStatus {
        TargetStatus {
            profile: None,
            windows: Vec::new(),
            placement_window_id: None,
            error: Some(TargetError::UnsupportedNative.to_string()),
        }
    }
    pub async fn list_pages(&self) -> Result<PageListing, CoreError> {
        self.scope()?;
        let pages = self.session.pages.list().await?;
        let live_page_ids = pages.iter().map(|page| page.page_id.clone()).collect();
        Ok(PageListing {
            pages,
            live_page_ids,
            hidden: 0,
        })
    }
    pub async fn refresh_page(&self, page: PageId) -> Result<Option<PageInfo>, CoreError> {
        self.scope()?;
        self.session.pages.refresh(page).await
    }
    pub async fn page(&self, page: PageId) -> Result<PageInfo, CoreError> {
        self.refresh_page(page.clone())
            .await?
            .ok_or(CoreError::UnknownPage(page))
    }
    pub async fn page_session(&self, page: PageId) -> Result<PageSession, CoreError> {
        self.scope()?;
        self.session.pages.get_session(page).await
    }
    pub async fn observe(&self, page: PageId) -> Result<Arc<Observer>, CoreError> {
        self.scope()?;
        Ok(self.session.observe(page).await)
    }
    pub async fn input(&self, page: PageId) -> Result<Input, CoreError> {
        self.scope()?;
        Ok(self.session.input(page).await)
    }
    pub async fn nav(&self, page: PageId) -> Result<Navigation, CoreError> {
        self.scope()?;
        Ok(self.session.nav(page))
    }
    pub async fn screenshot(
        &self,
        page: PageId,
        o: ScreenshotCaptureOptions,
    ) -> Result<ScreenshotCaptureResult, CoreError> {
        self.scope()?;
        self.session.screenshot(page, o).await
    }
    pub async fn active_page(&self) -> Result<Option<PageInfo>, CoreError> {
        self.scope()?;
        self.session.pages.get_active().await
    }
    pub async fn new_page(
        &self,
        url: &str,
        mut opts: NewPageOptions,
    ) -> Result<PageInfo, CoreError> {
        self.scope()?;
        opts.window_id = opts
            .window_id
            .or_else(|| self.unbound_default_window.clone());
        let page = self.session.pages.new_page(url, opts).await?;
        self.session
            .pages
            .get_info(page.clone())
            .await
            .ok_or(CoreError::UnknownPage(page))
    }
    pub async fn close_page(&self, page: PageId) -> Result<(), CoreError> {
        self.scope()?;
        self.session.pages.close(page).await
    }
    pub async fn move_page(
        &self,
        page: PageId,
        opts: MovePageOptions,
    ) -> Result<PageInfo, CoreError> {
        self.scope()?;
        self.session.pages.move_page(page, opts).await
    }
    pub async fn resolve_tab_ids(
        &self,
        tabs: &[TabId],
    ) -> Result<HashMap<TabId, PageId>, CoreError> {
        self.scope()?;
        self.session.pages.resolve_tab_ids(tabs).await
    }
    pub async fn tab_id_of(&self, page: PageId) -> Option<TabId> {
        self.session.pages.get_tab_id(page).await
    }
    pub fn signals(&self) -> &Arc<PageSignals> {
        &self.session.page_signals
    }
    pub fn cdp_events(&self) -> broadcast::Receiver<CdpEvent> {
        self.session.cdp_events()
    }
    pub async fn list_windows(&self) -> Result<Vec<WindowInfo>, CoreError> {
        self.scope()?;
        self.session.windows.list().await
    }
    pub async fn create_window(&self) -> Result<WindowInfo, CoreError> {
        self.scope()?;
        self.session.windows.create().await
    }
    pub async fn close_window(&self, w: WindowId) -> Result<(), CoreError> {
        self.scope()?;
        self.session.windows.close(w).await
    }
    pub async fn cdp(
        &self,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CoreError> {
        self.scope()?;
        self.session.cdp(method, params, session).await
    }
    pub async fn cdp_json_for_page(
        &self,
        page: PageId,
        method: &str,
        params_json: &str,
    ) -> Result<String, CoreError> {
        self.scope()?;
        self.session
            .cdp_json_for_page(page, method, params_json)
            .await
    }
    pub async fn evaluate_extension_workers(
        &self,
        expression: &str,
    ) -> Result<Vec<Value>, CoreError> {
        self.scope()?;
        let targets = self
            .session
            .cdp("Target.getTargets", json!({}), None)
            .await?;
        let mut values = Vec::new();
        for target in targets["targetInfos"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| {
                t["type"] == "service_worker"
                    && t["url"]
                        .as_str()
                        .is_some_and(|url| url.starts_with("chrome-extension://"))
            })
        {
            let attached = self
                .session
                .cdp(
                    "Target.attachToTarget",
                    json!({"targetId":target["targetId"],"flatten":true}),
                    None,
                )
                .await?;
            let session = attached["sessionId"]
                .as_str()
                .map(SessionId::from)
                .ok_or_else(|| CoreError::Message("missing extension sessionId".into()))?;
            let result = self
                .session
                .cdp(
                    "Runtime.evaluate",
                    json!({"expression":expression,"returnByValue":true,"awaitPromise":true}),
                    Some(&session),
                )
                .await;
            let _ = self
                .session
                .cdp(
                    "Target.detachFromTarget",
                    json!({"sessionId":session}),
                    None,
                )
                .await;
            if let Ok(result) = result {
                if result.get("exceptionDetails").is_none() {
                    if let Some(value) = result["result"].get("value") {
                        values.push(value.clone());
                    }
                }
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
