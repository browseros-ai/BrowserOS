use crate::api::mcp::{
    dispatch::{ToolEffect, ToolEffectContext},
    naming::{build_session_group_title, client_prefix_from_slug},
};
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;
use rmcp::model::ContentBlock;

/// Appends a bounded rename reminder while the generated session label remains active.
pub fn apply(context: ToolEffectContext<'_>) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
    Box::pin(async move {
        if context.result.is_error {
            return Ok(None);
        }
        let Some(identity) = &context.call.identity else {
            return Ok(None);
        };
        let Some(label) = identity.session.take_rename_nudge().await else {
            return Ok(None);
        };
        let title = build_session_group_title(
            client_prefix_from_slug(identity.session.agent().slug()),
            &label,
        );
        let mut result = context.result.clone();
        result.content.push(ContentBlock::text(format!(
            "Tip: this session is \"{title}\" — rename it with name_session name=\"<2-3 word task label>\""
        )));
        Ok(Some(result))
    })
}

const _: ToolEffect = apply;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn apply_result(
        call: &crate::api::mcp::dispatch::ToolCall,
        result: &ToolResult,
    ) -> anyhow::Result<ToolResult> {
        Ok(apply(ToolEffectContext {
            call,
            result,
            cancelled: false,
            duration_ms: 1,
        })
        .await?
        .unwrap_or_else(|| result.clone()))
    }

    fn assert_result_eq(actual: &ToolResult, expected: &ToolResult) {
        assert_eq!(actual.content, expected.content);
        assert_eq!(actual.is_error, expected.is_error);
        assert_eq!(actual.structured_content, expected.structured_content);
    }

    #[tokio::test]
    async fn appends_exactly_five_trailing_tips_and_preserves_image_content() -> anyhow::Result<()>
    {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let image = ContentBlock::image("aW1hZ2U=".to_string(), "image/png".to_string());
        let original = ToolResult {
            content: vec![ContentBlock::text("ok"), image.clone()],
            is_error: false,
            structured_content: Some(json!({ "pages": [] })),
        };

        for _ in 0..5 {
            let result = apply_result(&call, &original).await?;
            assert_eq!(result.content[..2], original.content);
            assert_eq!(
                result
                    .content
                    .last()
                    .and_then(ContentBlock::as_text)
                    .map(|text| text.text.as_str()),
                Some(
                    "Tip: this session is \"codex/agile-alpaca\" — rename it with name_session name=\"<2-3 word task label>\""
                )
            );
            assert_eq!(result.structured_content, original.structured_content);
        }
        assert_result_eq(&apply_result(&call, &original).await?, &original);
        Ok(())
    }

    #[tokio::test]
    async fn errors_do_not_consume_a_nudge_and_rename_stops_tips() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        let error = ToolResult::error("failed");
        assert_result_eq(&apply_result(&call, &error).await?, &error);

        let success = ToolResult::text("opened", Some(json!({ "page": 1 })));
        assert_eq!(apply_result(&call, &success).await?.content.len(), 2);
        let session = &call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .session;
        session.rename("invoice-processing".to_string()).await;
        assert_result_eq(&apply_result(&call, &success).await?, &success);
        Ok(())
    }

    #[tokio::test]
    async fn separate_sessions_have_independent_nudge_budgets() -> anyhow::Result<()> {
        let first =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let second =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let success = ToolResult::text("ok", None);

        for _ in 0..5 {
            assert_eq!(apply_result(&first, &success).await?.content.len(), 2);
        }
        assert_result_eq(&apply_result(&first, &success).await?, &success);
        assert_eq!(apply_result(&second, &success).await?.content.len(), 2);
        Ok(())
    }
}
