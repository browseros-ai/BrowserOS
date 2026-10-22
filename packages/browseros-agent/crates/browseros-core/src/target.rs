//! Immutable execution views over a shared browser runtime. Task hosts own logical binding;
//! views capture it once and keep management callable even when normal admission fails.
use crate::{
    BrowserContextId, BrowserProfileId, BrowserSession, CoreError, NativeCapabilities,
    NativeFeature, PageId, SessionId, TabId, TargetError, WindowId,
};
use crate::{
    input::Input,
    navigation::Navigation,
    observer::Observer,
    page_signals::PageSignals,
    pages::{MovePageOptions, NewPageOptions, PageCreation, PageInfo, PageSession},
    profiles::{BrowserProfileInfo, LoadProfileOutcome, ProfileSelector, default_profile},
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
    pub source: TargetSource,
}
/// Where a capture's target came from. An explicit binding is the task host's saved override;
/// the default (the original Default profile) is resolved again from the live catalog on every
/// capture, so its process-lifetime handles are always fresh and it is never persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetSource {
    Explicit,
    Default,
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
    /// A saved binding this view did not apply because the native browser lacks profile
    /// targeting. Routing is unbound; hosts only disclose it, and it stays persisted so a
    /// capable browser retries it later.
    unapplied: Option<TargetBinding>,
    /// Which target this capture used; None for unbound views (host infrastructure, and every
    /// call on a native without profile targeting).
    source: Option<TargetSource>,
    capture: Result<Option<CapturedScope>, TargetError>,
    unbound_default_window: Option<WindowId>,
    signals: Arc<PageSignals>,
    signal_windows: Arc<std::sync::RwLock<BTreeSet<WindowId>>>,
    // Ordinary unbound calls follow the live connection. Supplied snapshots without a
    // creation receipt retain call-entry epoch; native creations retain their own later epoch.
    initial_epoch: u64,
    expected_created_page: Option<PageInfo>,
    expected_creation_epoch: Option<u64>,
    // Caller hooks and response snapshots can await before deriving a creation capability.
    // View clones share immutable native creation identities/epochs; global reconciliation
    // can replace a PageId's registry entry without rewriting this call's creation authority.
    creation_receipts: Arc<std::sync::RwLock<HashMap<PageId, PageCreation>>>,
    // The window this admitted view opened for a windowless Default profile. Placement is
    // captured once and never refreshes, so without this every page opened through the same
    // call (Promise.all in a run) would open its own window. Shared by clones; the async lock
    // makes creation single-flight.
    default_window: Arc<tokio::sync::Mutex<Option<WindowId>>>,
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

/// Both event attribution and OOPIF admission inspect the same native frame-tree shape.
fn contains_frame(tree: &Value, id: &str) -> bool {
    tree["frame"]["id"] == id
        || tree["childFrames"]
            .as_array()
            .is_some_and(|children| children.iter().any(|child| contains_frame(child, id)))
}

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
    /// The failure once this capture's live identity can no longer be trusted (connection epoch
    /// changed, handle or context gone). An explicit binding needs explicit reselection; the
    /// default has no saved handle to repair, so the next call simply resolves it afresh.
    fn gone(&self) -> TargetError {
        match self.source {
            TargetSource::Explicit => {
                TargetError::ProfileGone(self.profile.browser_profile_id.clone())
            }
            TargetSource::Default => TargetError::DefaultProfileUnavailable(
                "the Default profile or browser connection changed during this call".into(),
            ),
        }
    }
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
/// Every failure to resolve the default is the same recoverable state for the agent: select
/// a profile explicitly, or retry once the browser settles. The reason is kept for diagnosis.
fn default_capture_error(error: CoreError) -> TargetError {
    match error {
        CoreError::Target(TargetError::DefaultProfileUnavailable(reason)) => {
            TargetError::DefaultProfileUnavailable(reason)
        }
        error => TargetError::DefaultProfileUnavailable(error.to_string()),
    }
}

impl BrowserView {
    /// Admits one call. Precedence: an explicit saved binding, then (with no binding) the
    /// browser's original Default profile; both run through the same scoped execution. Natives
    /// without profile targeting keep the unbound routing they always had. The result is
    /// immutable: rebinding, loading or reconnecting later never refreshes this view.
    pub async fn capture(
        session: Arc<BrowserSession>,
        binding: Option<TargetBinding>,
        unbound_default_window: Option<WindowId>,
    ) -> Self {
        Self::capture_with(session, binding, unbound_default_window, true).await
    }
    /// Admission for profile management (discovery and selection). Same precedence as
    /// [`Self::capture`], but an unloaded Default is captured as it is instead of being loaded:
    /// choosing or listing profiles must never load an unrelated profile as a side effect.
    pub async fn capture_for_management(
        session: Arc<BrowserSession>,
        binding: Option<TargetBinding>,
        unbound_default_window: Option<WindowId>,
    ) -> Self {
        Self::capture_with(session, binding, unbound_default_window, false).await
    }
    async fn capture_with(
        session: Arc<BrowserSession>,
        binding: Option<TargetBinding>,
        unbound_default_window: Option<WindowId>,
        load_default: bool,
    ) -> Self {
        let mut view = Self::unbound(session, unbound_default_window);
        // Older natives cannot honor regular-profile routing (they ignore the context params),
        // so they keep the unbound path, send no getProfiles, and only disclose an explicit
        // binding saved on a newer browser. A capable browser retries it on a later capture.
        if !view.supports(NativeFeature::ProfileTargeting) {
            view.unapplied = binding;
            return view;
        }
        // No lock or host store is retained. Rebinding later cannot change this call's context
        // or chosen placement; a failed capture still carries an explicit intent for management.
        let (binding, capture) = match binding {
            Some(binding) => {
                let capture = view
                    .capture_scope(&binding, TargetSource::Explicit)
                    .await
                    .map(Some)
                    .map_err(|err| capture_error(err, &binding.intent.browser_profile_id));
                view.source = Some(TargetSource::Explicit);
                (Some(binding), capture)
            }
            None => {
                view.source = Some(TargetSource::Default);
                match view.capture_default(load_default).await {
                    Ok(scope) => (Some(scope.binding.clone()), Ok(Some(scope))),
                    // No silent fallback to the focused window or another profile: ordinary
                    // calls fail admission, while profile management stays usable.
                    Err(err) => (None, Err(default_capture_error(err))),
                }
            }
        };
        view.capture = capture;
        view.binding = binding;
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
        view
    }
    pub fn unbound(session: Arc<BrowserSession>, unbound_default_window: Option<WindowId>) -> Self {
        Self {
            initial_epoch: session.connection().connection_epoch(),
            expected_created_page: None,
            expected_creation_epoch: None,
            creation_receipts: Arc::new(std::sync::RwLock::new(HashMap::new())),
            default_window: Arc::new(tokio::sync::Mutex::new(None)),
            signals: session.page_signals.clone(),
            signal_windows: Arc::new(std::sync::RwLock::new(BTreeSet::new())),
            session,
            binding: None,
            unapplied: None,
            source: None,
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
    /// The saved binding capture left unapplied on a native without profile targeting.
    pub fn unapplied_binding(&self) -> Option<&TargetBinding> {
        self.unapplied.as_ref()
    }
    /// Explicit binding or the implicit default; None for unbound views. With the default,
    /// `binding()` is the effective (unsaved) binding, so every scoped reader treats it alike.
    #[must_use]
    pub fn target_source(&self) -> Option<TargetSource> {
        self.source
    }
    #[must_use]
    pub fn native(&self) -> NativeCapabilities {
        self.session.native()
    }
    #[must_use]
    pub fn supports(&self, feature: NativeFeature) -> bool {
        self.session.native().supports(feature)
    }
    pub fn require(&self, feature: NativeFeature) -> Result<(), CoreError> {
        self.session.native().require(feature)
    }

    /// Derive before spawning creation follow-up work, then carry this same view through every
    /// awaited helper and MCP dispatch. PageId/native tab/window IDs can survive target reuse;
    /// a creation receipt validates even a snapshot read after ownership/response awaits.
    /// Preserving that original incarnation guards title/expansion writes as well as final add.
    /// For ordinary unbound calls, a native creation receipt also retains its creation epoch.
    pub fn with_expected_created_page(&self, created: &PageInfo) -> Result<Self, CoreError> {
        if created.window_id.is_none() {
            return Err(TargetError::PageOutOfScope(created.page_id.clone()).into());
        }
        if let Some(scope) = self.scope()? {
            scope.context()?;
            self.check_epoch(scope)?;
            if created.browser_profile_id.as_ref() != Some(&scope.profile.browser_profile_id)
                || created.live_browser_context_id != scope.context_id
                || scope
                    .pin()
                    .is_some_and(|pin| created.window_id != Some(pin))
            {
                return Err(TargetError::PageOutOfScope(created.page_id.clone()).into());
            }
        }
        let receipt = self
            .creation_receipts
            .read()
            .map_err(|_| CoreError::Message("creation receipt lock poisoned".into()))?
            .get(&created.page_id)
            .cloned();
        if let Some(original) = &receipt
            && !original.original.same_incarnation(created)
        {
            return Err(TargetError::PageTargetChanged(original.original.page_id.clone()).into());
        }
        if let Some(expected) = &self.expected_created_page
            && !expected.same_incarnation(created)
        {
            return Err(TargetError::PageTargetChanged(expected.page_id.clone()).into());
        }
        let mut derived = self.clone();
        if let Some(receipt) = receipt {
            derived.expected_created_page = Some(receipt.original);
            derived.expected_creation_epoch = Some(receipt.guard.expected);
        } else {
            derived.expected_created_page = Some(created.clone());
        }
        if let Some(guard) = derived.epoch_guard()? {
            guard.check(derived.session.connection().as_ref())?;
        }
        Ok(derived)
    }

    /// Publish native creation authority before returning to the host. Bound creation records
    /// it before readiness; unbound readiness retains the same private authority in PageManager.
    /// Later reads may refresh presentation fields, but cannot replace this target or epoch.
    fn retain_created_page(&self, creation: PageCreation) -> Result<(), CoreError> {
        let mut receipts = self
            .creation_receipts
            .write()
            .map_err(|_| CoreError::Message("creation receipt lock poisoned".into()))?;
        creation.guard.check(self.session.connection().as_ref())?;
        if let Some(original) = receipts.get(&creation.original.page_id) {
            if !original.original.same_incarnation(&creation.original)
                || original.guard.expected != creation.guard.expected
            {
                return Err(
                    TargetError::PageTargetChanged(original.original.page_id.clone()).into(),
                );
            }
        } else {
            receipts.insert(creation.original.page_id.clone(), creation);
        }
        Ok(())
    }

    pub(super) fn epoch_guard(&self) -> Result<Option<crate::connection::EpochGuard>, CoreError> {
        if let Some(scope) = self.scope()? {
            return Ok(Some(crate::connection::EpochGuard {
                expected: scope.connection_epoch,
                invalidated: scope.gone(),
            }));
        }
        Ok(self
            .expected_created_page
            .as_ref()
            .map(|page| crate::connection::EpochGuard {
                expected: self.expected_creation_epoch.unwrap_or(self.initial_epoch),
                invalidated: TargetError::PageTargetChanged(page.page_id.clone()),
            }))
    }

    /// Policy-free native discovery/cleanup still uses the capture's atomic transport guard.
    /// Callers own routing admission; using this helper does not authorize a new root method.
    async fn send_at_capture(
        &self,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CoreError> {
        let endpoint = match session {
            Some(session) => {
                crate::ProtocolSession::for_session(self.session.connection(), session.clone())
            }
            None => crate::ProtocolSession::root(self.session.connection()),
        };
        endpoint
            .with_epoch_guard(self.epoch_guard()?)
            .send_value(method, params)
            .await
    }

    async fn validate_expected_created_page(&self) -> Result<(), CoreError> {
        let Some(expected) = &self.expected_created_page else {
            return Ok(());
        };
        let fresh = self
            .session
            .pages
            .refresh_with_epoch(expected.page_id.clone(), self.epoch_guard()?)
            .await?
            .ok_or_else(|| TargetError::PageTargetChanged(expected.page_id.clone()))?;
        if !expected.same_incarnation(&fresh) {
            return Err(TargetError::PageTargetChanged(expected.page_id.clone()).into());
        }
        Ok(())
    }

    /// Every profile management path (load, resolve, status) reads the catalog here first,
    /// so this is where an unsupported native is refused before `Browser.getProfiles`.
    pub async fn list_profiles(&self) -> Result<Vec<BrowserProfileInfo>, CoreError> {
        self.require(NativeFeature::ProfileTargeting)?;
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
        self.read_windows_with_epoch(None).await
    }

    async fn read_windows_with_epoch(
        &self,
        guard: Option<crate::connection::EpochGuard>,
    ) -> Result<Vec<NativeWindow>, CoreError> {
        let value = crate::ProtocolSession::root(self.session.connection())
            .with_epoch_guard(guard)
            .send_value("Browser.getWindows", json!({}))
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
        // Older natives' windows lack the identity sidecar; refuse as unsupported rather
        // than reporting every window as ineligible.
        self.require(NativeFeature::ProfileTargeting)?;
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
    async fn read_catalog(
        &self,
        guard: &crate::connection::EpochGuard,
    ) -> Result<Vec<BrowserProfileInfo>, CoreError> {
        let catalog = crate::ProtocolSession::root(self.session.connection())
            .with_epoch_guard(Some(guard.clone()))
            .send_value("Browser.getProfiles", json!({}))
            .await
            .map_err(native_missing)?;
        decode(catalog["profiles"].clone())
    }
    async fn capture_scope(
        &self,
        binding: &TargetBinding,
        source: TargetSource,
    ) -> Result<CapturedScope, CoreError> {
        let epoch = self.session.connection().connection_epoch();
        let guard = crate::connection::EpochGuard {
            expected: epoch,
            invalidated: TargetError::ProfileGone(binding.intent.browser_profile_id.clone()),
        };
        let catalog = self.read_catalog(&guard).await?;
        self.scope_in_catalog(catalog, binding, source, guard).await
    }
    /// Resolves the original Default profile afresh for this call. Its handles are process
    /// lifetime, so they are never saved: each capture reads the live catalog, loads Default if
    /// needed and allowed (an existing profile only; loading opens no window), and then
    /// captures it like an explicit binding, including placement and the epoch guard.
    async fn capture_default(&self, load: bool) -> Result<CapturedScope, CoreError> {
        let epoch = self.session.connection().connection_epoch();
        let guard = crate::connection::EpochGuard {
            expected: epoch,
            invalidated: TargetError::DefaultProfileUnavailable(
                "the browser connection changed while resolving the Default profile".into(),
            ),
        };
        let mut catalog = self.read_catalog(&guard).await?;
        let profile = default_profile(&catalog).cloned().ok_or_else(|| {
            TargetError::DefaultProfileUnavailable(format!(
                "no profile with directory \"{}\" exists",
                crate::DEFAULT_PROFILE_DIRECTORY
            ))
        })?;
        if load && (!profile.loaded || profile.browser_context_id.is_none()) {
            // A concurrent capture may load it too; the native joins that load. Cancellation
            // can leave the load running, which only leaves Default loaded for the next call.
            crate::ProtocolSession::root(self.session.connection())
                .with_epoch_guard(Some(guard.clone()))
                .send_value(
                    "Browser.loadProfile",
                    json!({"browserProfileId": profile.browser_profile_id.as_str()}),
                )
                .await
                .map_err(|err| {
                    TargetError::DefaultProfileUnavailable(format!(
                        "loading the Default profile failed: {err}"
                    ))
                })?;
            catalog = self.read_catalog(&guard).await?;
        }
        // The effective binding uses this capture's live handle. Revision 0 never collides with
        // a saved override, whose revisions start at 1; nothing writes it to the host store.
        let binding = TargetBinding {
            intent: TargetIntent {
                browser_profile_id: profile.browser_profile_id,
                directory: profile.directory,
                name: profile.name,
                window: None,
            },
            revision: 0,
        };
        self.scope_in_catalog(catalog, &binding, TargetSource::Default, guard)
            .await
    }
    async fn scope_in_catalog(
        &self,
        catalog: Vec<BrowserProfileInfo>,
        binding: &TargetBinding,
        source: TargetSource,
        guard: crate::connection::EpochGuard,
    ) -> Result<CapturedScope, CoreError> {
        let epoch = guard.expected;
        let profile = catalog
            .into_iter()
            .find(|profile| {
                profile.browser_profile_id == binding.intent.browser_profile_id
                    && profile.directory == binding.intent.directory
            })
            .ok_or_else(|| TargetError::ProfileGone(binding.intent.browser_profile_id.clone()))?;
        let windows = self.read_windows_with_epoch(Some(guard)).await?;
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
            source,
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
        match self.capture_scope(binding, TargetSource::Explicit).await {
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
            self.validate_expected_created_page().await?;
            return Ok(None);
        };
        if let Ok(mut windows) = self.signal_windows.write() {
            windows.clear();
        }
        self.check_epoch(scope)?;
        let catalog = self
            .send_at_capture("Browser.getProfiles", json!({}), None)
            .await
            .map_err(native_missing)?;
        let current = decode::<Vec<BrowserProfileInfo>>(catalog["profiles"].clone())?
            .into_iter()
            .find(|profile| {
                profile.browser_profile_id == scope.profile.browser_profile_id
                    && profile.directory == scope.binding.intent.directory
            })
            .ok_or_else(|| scope.gone())?;
        // Catalog discovery is an async handoff. A restart invalidates the saved identity
        // even when its returned handle/directory happen to match an unloaded capture.
        self.check_epoch(scope)?;
        // Live identity is never refreshed in place. Even a successful load in this run requires
        // a fresh call to capture the new context; old sessions cannot start using it silently.
        if current.browser_context_id != scope.context_id || current.loaded != scope.profile.loaded
        {
            // Loading does not remove the saved profile: this call still lacks a captured
            // live context. Preserve that recovery error instead of misclassifying it as gone.
            let error = if !scope.profile.loaded && scope.context_id.is_none() {
                TargetError::ProfileNotLoaded(scope.profile.browser_profile_id.clone())
            } else {
                scope.gone()
            };
            return Err(error.into());
        }
        let windows = self.read_windows_with_epoch(self.epoch_guard()?).await?;
        if let Some(pin) = scope.pin()
            && !windows
                .iter()
                .any(|window| window.info.window_id == pin.0 && scope.window_matches(window))
        {
            return Err(TargetError::PinnedWindowClosed(pin).into());
        }
        self.check_epoch(scope)?;
        self.validate_expected_created_page().await?;
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
            return Err(scope.gone().into());
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
        let all = self
            .session
            .pages
            .list_with_epoch(self.epoch_guard()?)
            .await?;
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
        let fresh = self
            .session
            .pages
            .refresh_with_epoch(page, self.epoch_guard()?)
            .await?;
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
        if self.epoch_guard()?.is_none() {
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
        if self.epoch_guard()?.is_none() {
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
            if self.expected_created_page.is_some() {
                let result = self.cdp("Browser.getActiveTab", json!({}), None).await?;
                let target = result["tab"]["targetId"].as_str();
                return Ok(self
                    .list_pages()
                    .await?
                    .pages
                    .into_iter()
                    .find(|page| Some(page.target_id.as_str()) == target));
            }
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
        // An explicit context asks createTab for profile routing, the same gated behavior the
        // raw guard refuses; older natives would ignore it and open in the last-active window.
        if opts.browser_context_id.is_some() {
            self.require(NativeFeature::ProfileTargeting)?;
        }
        let Some(scope) = self.scope()? else {
            if self.expected_created_page.is_some() {
                return Err(cdp_policy::unsupported("Browser.createTab"));
            }
            opts.window_id = opts.window_id.or(self.unbound_default_window);
            let (creation, fresh) = self.session.pages.new_page_info(url, opts).await?;
            self.retain_created_page(creation)?;
            return Ok(fresh);
        };
        scope.context()?;
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
        let window = match opts.window_id.or(scope.placement_window) {
            Some(window) => window,
            None if scope.source == TargetSource::Default => self.create_default_window().await?,
            None => {
                return Err(
                    TargetError::NoProfileWindow(scope.profile.browser_profile_id.clone()).into(),
                );
            }
        };
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
        let guard = self
            .epoch_guard()?
            .ok_or_else(|| CoreError::Message("missing bound page epoch".into()))?;
        let creation = self
            .session
            .pages
            .adopt_created(created["tab"].clone(), url, guard)
            .await?;
        self.retain_created_page(creation.clone())?;
        // Seed the capability from the native create reply BEFORE its first refresh. Asking
        // page_access(PageId) here would adopt a replacement reconciled during that await.
        let creating_view = self.with_expected_created_page(&creation.original)?;
        let access = PageAccess::new(creating_view.clone(), creation.original);
        if let Some(group) = opts.tab_group_id {
            let fresh = access.fresh().await?;
            creating_view
                .cdp(
                    "Browser.addTabsToGroup",
                    json!({"groupId":group,"tabIds":[fresh.tab_id.0]}),
                    None,
                )
                .await?;
        }
        // The same question the unbound creation path asks, answered the same
        // way: the page's content going quiet rather than load_progress
        // reaching 1.0, and asked last for the same reason. The wait can hold
        // for a whole ceiling, and this tab can be reconciled onto a new
        // target while it does, so asking any earlier widens the window in
        // which the grouping above sees a replacement and fails the creation.
        let settled = access.fresh().await?;
        let _ = crate::settle::wait_for_page_ready(
            self.session.pages.as_ref(),
            settled.page_id.clone(),
            crate::timeouts::ARRIVAL_READY_BUDGET,
        )
        .await;
        Ok(settled)
    }
    /// The default profile has no normal window yet: open one (normal and inactive, in the
    /// captured Default context) instead of failing or using another profile's window. It goes
    /// through the scoped root policy, which validates the reply against this capture, and is
    /// reused by every later creation through the same admitted view. The caller then creates
    /// its tab in it with createTab, so each page is identified by its own creation reply (the
    /// window keeps the browser's initial tab). An explicit target without a window keeps
    /// `no_profile_window` instead.
    pub(super) async fn create_default_window(&self) -> Result<WindowId, CoreError> {
        let mut opened = self.default_window.lock().await;
        if let Some(window) = *opened
            && self.validate_window(window).await.is_ok()
        {
            return Ok(window);
        }
        let created = Box::pin(self.scoped_root_cdp("Browser.createWindow", json!({}))).await?;
        let window = created["window"]["windowId"]
            .as_i64()
            .map(WindowId)
            .ok_or_else(|| cdp_policy::unsupported("Browser.createWindow"))?;
        *opened = Some(window);
        Ok(window)
    }
    pub async fn close_page(&self, page: PageId) -> Result<(), CoreError> {
        if self.epoch_guard()?.is_none() {
            return self.session.pages.close(page).await;
        }
        let fresh = self.page(page).await?;
        self.cdp(
            "Browser.closeTab",
            json!({"targetId":fresh.target_id.as_str()}),
            None,
        )
        .await?;
        self.session
            .pages
            .list_with_epoch(self.epoch_guard()?)
            .await?;
        Ok(())
    }
    pub async fn move_page(
        &self,
        page: PageId,
        opts: MovePageOptions,
    ) -> Result<PageInfo, CoreError> {
        if self.epoch_guard()?.is_none() {
            return self.session.pages.move_page(page, opts).await;
        }
        let fresh = self.page(page.clone()).await?;
        let window = opts
            .window_id
            .or(fresh.window_id)
            .ok_or(TargetError::PageOutOfScope(page.clone()))?;
        let mut params = json!({"targetId":fresh.target_id.as_str(),"windowId":window.0});
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
        let all = self
            .session
            .pages
            .list_with_epoch(self.epoch_guard()?)
            .await?;
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
            None if self.expected_created_page.is_some() => Ok(self
                .read_windows_with_epoch(self.epoch_guard()?)
                .await?
                .into_iter()
                .map(|window| window.info)
                .collect()),
            None => self.session.windows.list().await,
        }
    }
    pub async fn create_window(&self) -> Result<WindowInfo, CoreError> {
        if self.epoch_guard()?.is_none() {
            return self.session.windows.create().await;
        }
        let result = self.cdp("Browser.createWindow", json!({}), None).await?;
        decode(result["window"].clone())
    }
    pub async fn close_window(&self, w: WindowId) -> Result<(), CoreError> {
        if self.epoch_guard()?.is_none() {
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
        // Raw CDP passes the native gate before any routing. Only methods and (method,
        // parameter) pairs a FEATURES row lists can be refused; internal calls never send them.
        self.native().check_raw(method, &params)?;
        if self.scope()?.is_none() {
            if self.expected_created_page.is_some() {
                // An unbound creation callback remains global in routing, but it cannot
                // replace its original page or connection after an awaited lookup.
                self.validate_expected_created_page().await?;
                let result = self.send_at_capture(method, params, session).await?;
                self.validate_expected_created_page().await?;
                return Ok(result);
            }
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
        self.native().check_raw_json(method, params_json)?;
        if self.epoch_guard()?.is_none() {
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
        // This adapter owns a short-lived root/session pair. Even an unbound evaluation
        // cannot carry its attached session ID onto a replacement socket during cleanup.
        let worker_guard = self
            .epoch_guard()?
            .unwrap_or(crate::connection::EpochGuard {
                expected: self.session.connection().connection_epoch(),
                invalidated: TargetError::CdpUnsupported {
                    method: "evaluate_extension_workers".into(),
                },
            });
        let root = crate::ProtocolSession::root(self.session.connection())
            .with_epoch_guard(Some(worker_guard.clone()));
        let targets = root.send_value("Target.getTargets", json!({})).await?;
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
            let attached = match root
                .send_value(
                    "Target.attachToTarget",
                    json!({"targetId":target["targetId"],"flatten":true}),
                )
                .await
            {
                Ok(attached) => attached,
                Err(err @ CoreError::Target(_)) => return Err(err),
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
                let fresh_targets = root.send_value("Target.getTargets", json!({})).await?;
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
                let result = crate::ProtocolSession::for_session(
                    self.session.connection(),
                    session.clone(),
                )
                .with_epoch_guard(Some(worker_guard.clone()))
                .send_value(
                    "Runtime.evaluate",
                    json!({"expression":expression,"returnByValue":true,"awaitPromise":true}),
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
            let detach = root
                .send_value("Target.detachFromTarget", json!({"sessionId":session}))
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

#[path = "credential_routing.rs"]
mod credentials;
