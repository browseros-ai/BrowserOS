use crate::mcp::dispatch::{ToolEffect, ToolEffectContext, extract_page_id};
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;

/// Records successful page-targeted dispatches in the activity registry.
pub fn apply(context: ToolEffectContext<'_>) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
    Box::pin(async move {
        if context.result.is_error || context.call.flags.close_page {
            return Ok(None);
        }
        let (Some(identity), Some(page_id)) = (
            context.call.identity.as_ref(),
            extract_page_id(context.call),
        ) else {
            return Ok(None);
        };
        let Some(info) = context
            .call
            .page_snapshot
            .as_ref()
            .filter(|info| info.page_id.0 == page_id)
        else {
            return Ok(None);
        };
        context
            .call
            .state
            .tab_activity
            .record_tool(crate::tabs::activity::RecordToolInput {
                target_id: info.target_id.clone(),
                tab_id: info.tab_id.0,
                page_id,
                session_id: context.call.session_id.as_str().to_string(),
                agent_id: identity.session.convo_id().as_str().to_string(),
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
    use browseros_core::{PageId, TabId, TargetId, pages::PageInfo};
    use serde_json::json;

    fn page_info(target_id: &str) -> PageInfo {
        PageInfo {
            page_id: PageId(1),
            target_id: TargetId::from(target_id.to_string()),
            tab_id: TabId(101),
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            is_active: true,
            is_loading: false,
            load_progress: 1.0,
            is_pinned: false,
            is_hidden: false,
            window_id: None,
            index: None,
            group_id: None,
        }
    }

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
        assert!(
            call.state
                .tab_activity
                .snapshot(call.browser_session.as_deref())
                .await
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn successful_dispatch_records_the_starting_target_incarnation() -> anyhow::Result<()> {
        let mut call =
            crate::mcp::test_support::tool_call("navigate", json!({ "page": 1 })).await?;
        call.page_snapshot = Some(page_info("target-old"));
        let result = ToolResult::text("navigated", None);

        apply(ToolEffectContext {
            call: &call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await?;

        assert!(
            call.state
                .tab_activity
                .remove_incarnation(1, "target-old")
                .await
        );
        Ok(())
    }

    #[tokio::test]
    async fn close_dispatch_does_not_restore_removed_activity() -> anyhow::Result<()> {
        let mut call =
            crate::mcp::test_support::tool_call("tabs", json!({ "action": "close", "page": 1 }))
                .await?;
        call.page_snapshot = Some(page_info("target-old"));
        let result = ToolResult::text("closed", None);

        apply(ToolEffectContext {
            call: &call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await?;

        assert!(
            !call
                .state
                .tab_activity
                .remove_incarnation(1, "target-old")
                .await
        );
        Ok(())
    }
}
