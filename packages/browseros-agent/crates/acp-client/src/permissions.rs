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
    use PermissionOptionKind::{AllowAlways, AllowOnce, RejectAlways, RejectOnce};
    let preferred = match decision {
        PermissionDecision::AllowOnce => [AllowOnce, AllowAlways],
        PermissionDecision::AllowAlways => [AllowAlways, AllowOnce],
        PermissionDecision::RejectOnce => [RejectOnce, RejectAlways],
        PermissionDecision::RejectAlways => [RejectAlways, RejectOnce],
        PermissionDecision::Cancel => return RequestPermissionOutcome::Cancelled,
    };
    let option = preferred
        .iter()
        .find_map(|kind| request.options.iter().find(|option| option.kind == *kind));
    // acpx approve-all ultimately accepts the first offered option, even when
    // the agent offers only rejection. Reject policies never fall back to allow.
    let option = option.or_else(|| match decision {
        PermissionDecision::AllowOnce | PermissionDecision::AllowAlways => request.options.first(),
        _ => None,
    });
    option
        .map(|option| {
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                option.option_id.clone(),
            ))
        })
        .unwrap_or(RequestPermissionOutcome::Cancelled)
}
