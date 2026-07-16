use crate::mcp::dispatch::{ToolEffect, ToolEffectContext, extract_page_id, result_page_id};
use browseros_core::PageId;
use futures_util::future::BoxFuture;

/// Updates ownership for successful tab creation and closure results.
pub fn apply(
    context: ToolEffectContext<'_>,
) -> BoxFuture<'_, anyhow::Result<Option<browseros_mcp::ToolResult>>> {
    Box::pin(async move {
        if context.result.is_error {
            return Ok(None);
        }
        let Some(identity) = &context.call.identity else {
            return Ok(None);
        };
        if context.call.flags.new_page {
            let Some(page_id) = result_page_id(context.result) else {
                return Ok(None);
            };
            if let Some(browser) = &context.call.browser_session
                && let Some(info) = browser.pages.get_info(PageId(page_id)).await
            {
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
                        tool_name: "tabs".to_string(),
                    })
                    .await;
            }
            context
                .call
                .state
                .sessions
                .ownership()
                .claim_page(identity.ownership_key.clone(), PageId(page_id))
                .await;
        } else if context.call.flags.close_page
            && let Some(page_id) = extract_page_id(context.call)
        {
            let page_id = PageId(page_id);
            context
                .call
                .state
                .sessions
                .ownership()
                .remove_page(&page_id)
                .await;
            identity.session.forget_first_capture(&page_id).await;
        }
        Ok(None)
    })
}

const _: ToolEffect = apply;

#[cfg(test)]
mod tests {
    use super::*;
    use browseros_mcp::ToolResult;
    use serde_json::json;

    #[tokio::test]
    async fn close_page_removes_owned_page_and_first_capture() -> anyhow::Result<()> {
        let call =
            crate::mcp::test_support::tool_call("tabs", json!({ "action": "close", "page": 9 }))
                .await?;
        let identity = call.identity.as_ref().unwrap_or_else(|| unreachable!());
        call.state
            .sessions
            .ownership()
            .claim_page(identity.ownership_key.clone(), PageId(9))
            .await;
        identity.session.mark_first_capture_done(PageId(9)).await;
        let result = ToolResult::text("closed page 9", Some(json!({ "page": 9 })));
        apply(ToolEffectContext {
            call: &call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await
        .unwrap_or_else(|error| panic!("effect failed: {error}"));
        assert_eq!(call.state.sessions.owner_of_page(&PageId(9)).await, None);
        assert!(!identity.session.has_first_capture(&PageId(9)).await);
        Ok(())
    }
}
