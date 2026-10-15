use crate::mcp::dispatch::{ToolCall, ToolGuard};
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;
use serde_json::Value;
use tracing::warn;

const BLOCKED_SCHEMES: &[&str] = &["javascript:", "file:", "data:"];

/// Rejects navigate schemes that must never reach the CDP layer.
pub fn guard(call: &ToolCall) -> BoxFuture<'_, Option<ToolResult>> {
    Box::pin(async move {
        if call.tool().name != "navigate" {
            return None;
        }
        let url = call.raw_args.get("url").and_then(Value::as_str)?;
        let trimmed = url.trim();
        let scheme_end = trimmed.find(':')?;
        let scheme = trimmed[..=scheme_end].to_ascii_lowercase();
        if !BLOCKED_SCHEMES.contains(&scheme.as_str()) {
            return None;
        }
        warn!(
            tool = call.tool().name,
            session_id = %call.session_id,
            reason = "blocked navigate scheme",
            "cockpit tool dispatch rejected"
        );
        Some(ToolResult::error(format!(
            "navigate refuses {scheme} URLs; only http(s) is allowed"
        )))
    })
}

const _: ToolGuard = guard;

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::ContentBlock;
    use serde_json::json;

    #[tokio::test]
    async fn rejects_trimmed_javascript_scheme_with_ts_text() -> anyhow::Result<()> {
        let call = crate::mcp::test_support::tool_call(
            "navigate",
            json!({ "url": "  JaVaScRiPt:alert(1)" }),
        )
        .await?;
        let result = guard(&call)
            .await
            .unwrap_or_else(|| ToolResult::error("missing"));
        let text = result.content.iter().find_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        });
        assert_eq!(
            text,
            Some("navigate refuses javascript: URLs; only http(s) is allowed")
        );
        Ok(())
    }
}
