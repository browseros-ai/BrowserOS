//! Projects named SDK tasks onto the host session label after tab reconciliation.
//! This projection hook currently performs no renaming.

use super::super::service::StartedSession;
use crate::{AppState, services::neo::RunOutcome};

pub(in crate::api::mcp) async fn apply(
    _state: &AppState,
    _started: &StartedSession,
    _outcome: &RunOutcome,
) -> Vec<String> {
    Vec::new()
}
