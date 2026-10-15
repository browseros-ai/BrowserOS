//! Dispatch bookkeeping every host-local tool shares. Local tools bypass the catalog
//! pipeline, so they register with the session themselves: that is what lets a cockpit
//! Stop interrupt them and what turns a call Stop won into the operator-cancel result.

use crate::{
    ids::DispatchId,
    mcp::dispatch::{operator_cancellation_result, session_gone_message},
    seams::{LocalToolCall, LocalToolContext},
    sessions::Session,
};
use browseros_mcp::ToolResult;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tracing::warn;

/// Completes a local dispatch; the operator cancellation wins if Stop reached it first.
pub(crate) async fn finish_local_dispatch(
    session: &Session,
    dispatch_id: &DispatchId,
    result: ToolResult,
) -> ToolResult {
    if !session.finish_dispatch(dispatch_id).await && session.operator_stop_requested() {
        operator_cancellation_result()
    } else {
        result
    }
}

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
        finish_local_dispatch(self.session, dispatch_id, result)
            .await
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        identity::{ClientIdentity, ConversationIdentity},
        ids::SessionId,
    };
    use std::sync::Arc;

    fn usage_session() -> Arc<Session> {
        Session::new(
            SessionId::new("usage-session"),
            ClientIdentity::Ephemeral {
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            ConversationIdentity::new("codex", "usage-test".to_string()),
            "Codex".to_string(),
            tokio::time::Instant::now(),
        )
    }

    #[tokio::test]
    async fn local_tool_returns_cancellation_when_operator_stop_wins() -> anyhow::Result<()> {
        let session = usage_session();
        let dispatch_id = DispatchId::new();
        assert!(
            session
                .try_register_dispatch(dispatch_id.clone(), CancellationToken::new())
                .await
        );
        session.request_operator_stop();
        assert_eq!(session.stop_dispatches().await, 1);

        let result = finish_local_dispatch(
            session.as_ref(),
            &dispatch_id,
            ToolResult::text("renamed", None),
        )
        .await;

        assert!(result.is_error);
        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value["cancellationKind"].as_str()),
            Some("cockpit.operator-cancelled")
        );
        assert_eq!(
            session.pending_operator_cancellation_audits().await,
            [dispatch_id]
        );
        Ok(())
    }
}
