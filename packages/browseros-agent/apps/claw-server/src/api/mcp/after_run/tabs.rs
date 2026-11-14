//! Reconciles SDK task tabs with the host's claims, groups and activity projection.
//! This projection hook currently performs no reconciliation.

use super::super::service::StartedSession;
use crate::{AppState, services::neo::RunOutcome};

pub(in crate::api::mcp) async fn apply(
    _state: &AppState,
    _started: &StartedSession,
    _outcome: &RunOutcome,
) -> Vec<String> {
    Vec::new()
}
