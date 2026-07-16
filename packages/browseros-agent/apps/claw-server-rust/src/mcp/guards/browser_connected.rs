use crate::mcp::dispatch::{ToolCall, ToolGuard};
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;
use tracing::warn;

const NOT_CONNECTED: &str = "browser session not connected; the agent browser is not running or paired. Tell the user to start BrowserClaw and check the cockpit connection status; do not fall back to another browser tool.";

/// Rejects calls until the server is attached to a live browser session.
pub fn guard(call: &ToolCall) -> BoxFuture<'_, Option<ToolResult>> {
    Box::pin(async move {
        if call.browser_session.is_some() {
            return None;
        }
        warn!(
            tool = call.tool().name,
            session_id = %call.session_id,
            reason = "browser session not connected",
            "cockpit tool dispatch rejected"
        );
        Some(ToolResult::error(NOT_CONNECTED))
    })
}

const _: ToolGuard = guard;

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::ContentBlock;
    use serde_json::json;

    #[tokio::test]
    async fn rejection_matches_ts_prompt() -> anyhow::Result<()> {
        let call = crate::mcp::test_support::tool_call("tabs", json!({})).await?;
        let result = guard(&call)
            .await
            .unwrap_or_else(|| ToolResult::error("missing"));
        let text = result.content.iter().find_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        });
        assert_eq!(text, Some(NOT_CONNECTED));
        Ok(())
    }
}
