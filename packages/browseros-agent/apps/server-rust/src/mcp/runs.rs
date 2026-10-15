//! The run authority `/mcp` consults on every lease call: a lease names a conversation, and
//! only that conversation's active run may use browser or connector tools
//! (TS `guards/conversation-running.ts`). Tabs a run's tools open join its side panel.

use super::leases::LeaseScope;
use crate::chat::{ActiveRun, ConversationRuns};
use browseros_mcp::ToolResult;
use browseros_mcp_host::{AdmittedCaller, mcp::dispatch::ToolCall, seams::DispatchGuard};
use futures_util::future::BoxFuture;
use std::sync::Arc;
use uuid::Uuid;

/// The refusal a lease caller gets while its conversation has no active run.
pub const NO_ACTIVE_RUN: &str = "MCP tools require an active conversation run.";

/// What `/mcp` needs from the chat layer's run registry. `ConversationRuns` in production;
/// tests drive runs directly without the chat engines.
pub trait RunRegistry: Send + Sync {
    fn active_run(&self, conversation_id: Uuid) -> Option<ActiveRun>;

    /// Adds tabs `run`'s tools opened to its side-panel set. False once the run ended or
    /// was replaced (the registry checks run identity).
    fn associate_tabs(&self, run: &ActiveRun, tab_ids: &[u32]) -> bool;
}

impl RunRegistry for ConversationRuns {
    fn active_run(&self, conversation_id: Uuid) -> Option<ActiveRun> {
        ConversationRuns::active_run(self, conversation_id)
    }

    fn associate_tabs(&self, run: &ActiveRun, tab_ids: &[u32]) -> bool {
        ConversationRuns::associate_tabs(self, run, tab_ids)
    }
}

/// `Some(refusal)` when `caller` holds a lease whose conversation is not running. Callers
/// without a lease (external MCP clients) are never subject to the run guard.
fn run_refusal(runs: &dyn RunRegistry, caller: Option<&AdmittedCaller>) -> Option<ToolResult> {
    let lease = caller?.host_data::<LeaseScope>()?;
    runs.active_run(lease.conversation_id())
        .is_none()
        .then(|| ToolResult::error(NO_ACTIVE_RUN))
}

/// Host guard `conversation-running`. It runs after the host's built-in guards (navigate
/// scheme, browser connected) and after the host's read-only refusal, so a stopped
/// read-only conversation hears "read-only" first where TS said "no active run".
pub(super) struct ConversationRunning(pub(super) Arc<dyn RunRegistry>);

impl DispatchGuard for ConversationRunning {
    fn check<'a>(&'a self, call: &'a ToolCall) -> BoxFuture<'a, Option<ToolResult>> {
        let refusal = run_refusal(self.0.as_ref(), call.caller.as_ref());
        Box::pin(async move { refusal })
    }
}
