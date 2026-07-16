use crate::mcp::dispatch::{ToolEffect, ToolEffectContext, extract_page_id};
use browseros_core::PageId;
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;

/// Records successful page-targeted dispatches in the activity registry.
pub fn apply(context: ToolEffectContext<'_>) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
    Box::pin(async move {
        if context.result.is_error {
            return Ok(None);
        }
        let (Some(identity), Some(browser), Some(page_id)) = (
            context.call.identity.as_ref(),
            context.call.browser_session.as_ref(),
            extract_page_id(context.call),
        ) else {
            return Ok(None);
        };
        let Some(info) = browser.pages.get_info(PageId(page_id)).await else {
            return Ok(None);
        };
        context
            .call
            .state
            .tab_activity
            .record_tool(crate::services::tab_activity::RecordToolInput {
                target_id: info.target_id,
                page_id,
                url: info.url,
                title: info.title,
                agent_id: identity.agent.agent_id().as_str().to_string(),
                slug: identity.agent.slug().to_string(),
                tool_name: context.call.tool().name.to_string(),
            })
            .await;
        Ok(None)
    })
}

const _: ToolEffect = apply;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn error_result_does_not_record_activity() -> anyhow::Result<()> {
        let call = crate::mcp::test_support::tool_call("navigate", json!({ "page": 1 })).await?;
        let result = ToolResult::error("failed");
        apply(ToolEffectContext {
            call: &call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await?;
        assert!(call.state.tab_activity.snapshot().await.is_empty());
        Ok(())
    }
}
