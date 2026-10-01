use crate::api::mcp::dispatch::{ToolCall, ToolGuard, browser_unavailable_message};
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;
use tracing::warn;

/// Rejects calls until the server is attached to a live browser session.
///
/// This guard, not the dispatch fallback, is what most tools hit, so the two have
/// to give the same answer. Both defer to one helper for that reason.
pub fn guard(call: &ToolCall) -> BoxFuture<'_, Option<ToolResult>> {
    Box::pin(async move {
        if call.browser_session.is_some() {
            return None;
        }
        let down_for = call.state.browser.link_down_for().await;
        warn!(
            tool = call.tool().name,
            session_id = %call.session_id,
            reason = "browser session not connected",
            down_ms = down_for.map(|down| down.as_millis()),
            "cockpit tool dispatch rejected"
        );
        Some(ToolResult::error(
            browser_unavailable_message(&call.state).await,
        ))
    })
}

const _: ToolGuard = guard;

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::ContentBlock;
    use serde_json::json;

    fn text_of(result: &ToolResult) -> String {
        result
            .content
            .iter()
            .find_map(|block| match block {
                ContentBlock::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// With no link ever established there is nothing to reconnect, so the advice
    /// to start the browser is correct. The reconnecting wording is what must not
    /// appear here.
    #[tokio::test]
    async fn a_browser_that_was_never_there_is_told_to_start() -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call("tabs", json!({})).await?;
        let result = guard(&call)
            .await
            .unwrap_or_else(|| ToolResult::error("the guard must reject without a session"));

        let text = text_of(&result);
        assert!(text.contains("not running or paired"), "{text}");
        assert!(!text.contains("reconnecting"), "{text}");
        Ok(())
    }
}
