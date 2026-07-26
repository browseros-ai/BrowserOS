use crate::{PageId, Ref};
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
    #[error(transparent)]
    Cdp(#[from] CdpError),
    #[error("{0}")]
    Message(String),
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
