use crate::api::mcp::dispatch::{ToolEffect, ToolEffectContext, extract_page_id};
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;

pub fn apply(context: ToolEffectContext<'_>) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
    Box::pin(async move {
        if context.call.tool().name != "snapshot" {
            return Ok(None);
        }
        let Some(captcha) = &context.call.state.captcha else {
            return Ok(None);
        };
        let Some(page) = extract_page_id(context.call) else {
            return Ok(None);
        };
        let Some(status) = captcha.reservations().status_line(page) else {
            return Ok(None);
        };
        let mut annotated = context.result.clone();
        annotated
            .content
            .push(rmcp::model::ContentBlock::text(status));
        Ok(Some(annotated))
    })
}

const _: ToolEffect = apply;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::mcp::{guards::captcha_busy::tests::enable_captcha, test_support::tool_call};
    use serde_json::json;

    async fn apply_to(
        call: &crate::api::mcp::dispatch::ToolCall,
        result: &ToolResult,
    ) -> anyhow::Result<Option<ToolResult>> {
        apply(ToolEffectContext {
            call,
            result,
            cancelled: false,
            duration_ms: 1,
        })
        .await
    }

    #[tokio::test]
    async fn tests_that_only_reserved_snapshots_get_an_appended_status() -> anyhow::Result<()> {
        for tool in ["snapshot", "read", "screenshot", "evaluate"] {
            let mut call = tool_call(tool, json!({"page":42})).await?;
            enable_captcha(&mut call)?;
            let service = call
                .state
                .captcha
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing service"))?;
            let lease = service
                .reservations()
                .try_reserve(42, "cap_test")
                .map_err(|_| anyhow::anyhow!("busy"))?;
            let result = ToolResult::text("original", Some(json!({"page":42})));
            let annotated = apply_to(&call, &result).await?;
            if tool == "snapshot" {
                let annotated = annotated.ok_or_else(|| anyhow::anyhow!("status missing"))?;
                assert_eq!(annotated.content.len(), result.content.len() + 1);
                assert_eq!(
                    serde_json::to_value(&annotated.content[0])?,
                    serde_json::to_value(&result.content[0])?
                );
                assert_eq!(annotated.structured_content, result.structured_content);
                assert_eq!(annotated.is_error, result.is_error);
                assert_eq!(
                    serde_json::to_value(&annotated.content[1])?,
                    json!({"type":"text", "text":service.reservations().status_line(42)})
                );
            } else {
                assert!(annotated.is_none(), "{tool}");
            }
            call.raw_args["page"] = json!(7);
            assert!(apply_to(&call, &result).await?.is_none());
            call.raw_args["page"] = json!(42);
            drop(lease);
            assert!(apply_to(&call, &result).await?.is_none());
            call.state.captcha = None;
            assert!(apply_to(&call, &result).await?.is_none());
        }
        Ok(())
    }
}
