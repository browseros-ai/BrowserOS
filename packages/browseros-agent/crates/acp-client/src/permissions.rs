//! Converts host policy decisions into option IDs offered by the agent. Policies
//! run as SDK-owned tasks rather than inside dispatch, so cancellation and later
//! messages keep flowing while a future UI approval waits for a human.

use futures_util::future::BoxFuture;

use crate::schema::{
    PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
    SelectedPermissionOutcome,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    Cancel,
}

/// Implementations must not await work that itself waits on this agent. The
/// returned future is dropped when its turn is cancelled or the connection ends.
pub trait PermissionPolicy: Send + Sync + 'static {
    fn decide(&self, request: &RequestPermissionRequest) -> BoxFuture<'static, PermissionDecision>;
}

pub struct ApproveAll;
pub struct RejectAll;

impl PermissionPolicy for ApproveAll {
    fn decide(&self, _: &RequestPermissionRequest) -> BoxFuture<'static, PermissionDecision> {
        Box::pin(async { PermissionDecision::AllowOnce })
    }
}
impl PermissionPolicy for RejectAll {
    fn decide(&self, _: &RequestPermissionRequest) -> BoxFuture<'static, PermissionDecision> {
        Box::pin(async { PermissionDecision::RejectOnce })
    }
}

pub(crate) fn outcome(
    request: &RequestPermissionRequest,
    decision: PermissionDecision,
) -> RequestPermissionOutcome {
    let kind = match decision {
        PermissionDecision::AllowOnce => PermissionOptionKind::AllowOnce,
        PermissionDecision::AllowAlways => PermissionOptionKind::AllowAlways,
        PermissionDecision::RejectOnce => PermissionOptionKind::RejectOnce,
        PermissionDecision::RejectAlways => PermissionOptionKind::RejectAlways,
        PermissionDecision::Cancel => return RequestPermissionOutcome::Cancelled,
    };
    request
        .options
        .iter()
        .find(|option| option.kind == kind)
        .map(|option| {
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                option.option_id.clone(),
            ))
        })
        .unwrap_or(RequestPermissionOutcome::Cancelled)
}
