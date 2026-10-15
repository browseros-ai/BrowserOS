//! Managed connector tools (Klavis `connector_mcp_servers` + Strata proxies) on `/mcp`.
//! Klavis's provider enforces the call policy itself; this is the lease-side half it asks
//! for on every call: the conversation's selected connectors and, as TS `authorizeCall`
//! did (`browser-mcp-module.ts:167-186`), a refusal while the lease's conversation is not
//! running. Klavis refuses read-only callers after that.

use super::{leases::LeaseScope, runs::NO_ACTIVE_RUN, runs::RunRegistry};
use crate::klavis::{ConnectorScope, ScopeResolver};
use std::sync::Arc;

/// Resolved per call, never cached from `tools/list`: a lease outlives its turns, so the
/// run must be the live one. External clients (no lease) select nothing and need no run.
pub fn connector_scope(runs: Arc<dyn RunRegistry>) -> ScopeResolver {
    Arc::new(move |caller| {
        let Some(lease) = caller.and_then(|caller| caller.host_data::<LeaseScope>()) else {
            return ConnectorScope::default();
        };
        ConnectorScope {
            selected_server_names: lease.enabled_mcp_servers().to_vec(),
            rejection: runs
                .active_run(lease.conversation_id())
                .is_none()
                .then(|| NO_ACTIVE_RUN.to_owned()),
        }
    })
}
