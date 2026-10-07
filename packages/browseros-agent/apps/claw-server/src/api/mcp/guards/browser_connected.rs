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
        let down_for = call.state.browser.link_status().await.down_for;
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

    /// Nothing has connected yet, so the browser may genuinely not be running and
    /// mentioning that is fair. The server is still retrying either way, so the
    /// advice has to lead with retrying rather than with relaunching.
    #[tokio::test]
    async fn a_link_that_never_connected_says_retry_and_may_mention_starting() -> anyhow::Result<()>
    {
        let call = crate::api::mcp::test_support::tool_call("tabs", json!({})).await?;
        let result = guard(&call)
            .await
            .unwrap_or_else(|| ToolResult::error("the guard must reject without a session"));

        let text = text_of(&result);
        assert!(text.contains("still retrying"), "{text}");
        assert!(text.contains("retry this tool"), "{text}");
        // The old wording asserted the browser was not running as a statement of
        // fact. The server cannot know that, so it must stay conditional.
        assert!(!text.contains("is not running or paired"), "{text}");
        Ok(())
    }
}
