//! Dispatch bookkeeping every host-local tool shares. Local tools bypass the catalog
//! pipeline, so they register with the session themselves: that is what lets a cockpit
//! Stop interrupt them and what turns a call Stop won into the operator-cancel result.

use crate::{
    ids::DispatchId,
    mcp::dispatch::{operator_cancellation_result, session_gone_message},
    seams::{LocalToolCall, LocalToolContext},
};
use browseros_mcp::ToolResult;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tracing::warn;

impl LocalToolCall<'_> {
    /// Registers one dispatch with the session. `None` means teardown already stopped
    /// accepting work; answer with [`Self::session_gone`]. `cancel` fires on Stop.
    pub async fn begin(&self, cancel: CancellationToken) -> Option<DispatchId> {
        let dispatch_id = DispatchId::new();
        self.session
            .try_register_dispatch(dispatch_id.clone(), cancel)
            .await
            .then_some(dispatch_id)
    }

    /// Completes a dispatch from [`Self::begin`]. If Stop won the race, the operator
    /// cancellation replaces the tool's own result, exactly as for catalog tools.
    pub async fn finish(&self, dispatch_id: &DispatchId, result: ToolResult) -> CallToolResult {
        if !self.session.finish_dispatch(dispatch_id).await
            && self.session.operator_stop_requested()
        {
            operator_cancellation_result()
        } else {
            result
        }
        .into_call_tool_result()
    }

    /// The refusal for a call whose session stopped accepting dispatches.
    #[must_use]
    pub fn session_gone(&self) -> CallToolResult {
        CallToolResult::error(vec![ContentBlock::text(session_gone_message(self.host))])
    }

    /// Hands a finished local dispatch to every observer (claw writes its audit row).
    /// Failures are logged, never surfaced: the tool already did its work.
    pub async fn record(
        &self,
        tool_name: &str,
        args: &Value,
        result: &ToolResult,
        duration_ms: i64,
        dispatch_id: &DispatchId,
    ) {
        for observer in &self.host.observers {
            if let Err(error) = observer
                .observe_local_tool(LocalToolContext {
                    session: self.session,
                    agent_label: self.agent_label,
                    tool_name,
                    args,
                    result,
                    duration_ms,
                    dispatch_id: dispatch_id.clone(),
                })
                .await
            {
                warn!(error = %error, "local tool audit submission failed");
            }
        }
    }
}
