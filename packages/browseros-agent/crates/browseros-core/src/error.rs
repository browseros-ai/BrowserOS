use crate::{
    BrowserProfileId, PageId, Ref, WindowId,
    capabilities::{BrowserOsVersion, NativeFeature},
    profiles::ProfileCandidate,
};
use browseros_cdp::CdpError;
use std::fmt;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoveredElementTarget {
    pub ref_id: Option<Ref>,
    pub role: Option<String>,
    pub name: Option<String>,
    pub backend_node_id: Option<i64>,
}

impl fmt::Display for CoveredElementTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let role = self.role.as_deref().filter(|value| !value.is_empty());
        let name = self.name.as_deref().filter(|value| !value.is_empty());
        if let Some(ref_id) = &self.ref_id {
            if let (Some(role), Some(name)) = (role, name) {
                return write!(f, "{ref_id} ({role} \"{name}\")");
            }
            return write!(f, "{ref_id}");
        }
        if let (Some(role), Some(name)) = (role, name) {
            return write!(f, "{role} \"{name}\"");
        }
        if let Some(backend_node_id) = self.backend_node_id {
            return write!(f, "backend node {backend_node_id}");
        }
        write!(f, "target")
    }
}

#[derive(thiserror::Error, Debug, Clone, PartialEq)]
pub enum CoreError {
    #[error("Unknown page {0}. List pages to see what is open.")]
    UnknownPage(PageId),
    #[error("Unknown page {0}.")]
    UnknownPageShort(PageId),
    #[error("Unknown ref {0}; take a new snapshot.")]
    UnknownRef(Ref),
    #[error("Stale ref {ref_id} ({role} \"{name}\"); take a new snapshot.")]
    StaleRef {
        ref_id: Ref,
        role: String,
        name: String,
    },
    #[error("Page document changed during snapshot capture; retry.")]
    DocumentChanged,
    #[error("Drag across frame sessions is not supported.")]
    CrossFrameDrag,
    #[error("Provide either target element or both targetX and targetY.")]
    InvalidDragTarget,
    #[error(
        "Element {target} is covered by <{blocker}> at its click point; the click would hit that element instead. Dismiss or interact with the covering element first (often a dialog, banner, or sticky header)."
    )]
    ElementCovered {
        target: CoveredElementTarget,
        blocker: String,
    },
    /// A gated native operation was refused before it was sent. See `crate::capabilities`.
    #[error(
        "native_feature_unavailable: {feature} needs BrowserOS {required} or newer; {}. The operation was not sent to the browser.",
        match reported {
            Some(version) => format!("this browser reports BrowserOS {version}"),
            None => "this browser's BrowserOS version is unknown".to_string(),
        }
    )]
    NativeFeatureUnavailable {
        feature: NativeFeature,
        required: BrowserOsVersion,
        reported: Option<BrowserOsVersion>,
    },
    #[error(transparent)]
    Cdp(#[from] CdpError),
    #[error(transparent)]
    Target(#[from] TargetError),
    #[error("{0}")]
    Message(String),
}

/// Stable recovery codes shared by Rust callers and the task host. Explicit scope failures
/// reveal only the requested handle, never metadata from a different profile.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum TargetError {
    #[error("target_scope_unsupported_native: native profile targeting is unavailable")]
    UnsupportedNative,
    #[error("profile_not_found: {0}")]
    ProfileNotFound(String),
    #[error("profile_ambiguous: {name}; select a browserProfileId from {candidates:?}")]
    ProfileAmbiguous {
        name: String,
        candidates: Vec<ProfileCandidate>,
    },
    #[error("profile_gone: {0}; reselect or clear the target")]
    ProfileGone(BrowserProfileId),
    /// No explicit target and the original Default profile cannot be used for this call.
    /// Nothing falls back to another profile; selecting one explicitly recovers.
    #[error("default_profile_unavailable: {0}; select a profile with browser_target set, or retry")]
    DefaultProfileUnavailable(String),
    #[error("profile_not_loaded: {0}; use browser_profiles load")]
    ProfileNotLoaded(BrowserProfileId),
    #[error("profile_load_failed: {0}")]
    ProfileLoadFailed(String),
    #[error("no_profile_window: {0}; use windows create")]
    NoProfileWindow(BrowserProfileId),
    #[error("pinned_window_closed: {0}; clear or change the window selection")]
    PinnedWindowClosed(WindowId),
    #[error("pinned_window_restricts: clear the strict pin before creating a window")]
    PinnedRestricts,
    #[error("window_ineligible: {0}")]
    WindowIneligible(WindowId),
    #[error("window_out_of_scope: {0}")]
    WindowOutOfScope(WindowId),
    #[error("page_out_of_scope: {0}")]
    PageOutOfScope(PageId),
    #[error("group_out_of_scope: {0}")]
    GroupOutOfScope(String),
    #[error("page_target_changed: {0}; retry the call")]
    PageTargetChanged(PageId),
    #[error("target_scope_unsupported: {method}")]
    CdpUnsupported { method: String },
    #[error("not_bound: select a browser target first")]
    NotBound,
    #[error("target_change_in_run: rebind between calls")]
    ChangeInRun,
}

impl CoreError {
    #[must_use]
    pub fn is_retryable_session_loss(&self) -> bool {
        matches!(
            self,
            Self::Cdp(CdpError::SessionGone | CdpError::ConnectionLost | CdpError::NotConnected)
        )
    }
}

impl From<String> for CoreError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

impl From<&str> for CoreError {
    fn from(value: &str) -> Self {
        Self::Message(value.to_string())
    }
}
