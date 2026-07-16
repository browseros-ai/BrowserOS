use crate::mcp::dispatch::{ToolCall, ToolGuard, extract_page_id};
use browseros_core::PageId;
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;
use tracing::warn;

/// Prevents an identified agent from dispatching against another agent's page.
pub fn guard(call: &ToolCall) -> BoxFuture<'_, Option<ToolResult>> {
    Box::pin(async move {
        let page_id = PageId(extract_page_id(call)?);
        let identity = call.identity.as_ref()?;
        let ownership = call.state.sessions.ownership();
        if let Some(owner) = call.state.sessions.owner_of_page(&page_id).await {
            if page_missing_after_refresh(call, &page_id).await {
                ownership.remove_page(&page_id).await;
                identity.session.forget_first_capture(&page_id).await;
            } else if owner == identity.ownership_key {
                return None;
            }
        }
        warn!(
            tool = call.tool().name,
            session_id = %call.session_id,
            key = %identity.ownership_key,
            agent_id = %identity.agent.agent_id(),
            page = page_id.0,
            "cockpit rejected foreign-page dispatch"
        );
        Some(ToolResult::error(format!(
            "page {} is not owned by this agent; call `tabs new` to open a fresh page and use the returned page id.",
            page_id.0
        )))
    })
}

async fn page_missing_after_refresh(call: &ToolCall, page_id: &PageId) -> bool {
    let Some(browser) = &call.browser_session else {
        return false;
    };
    if browser.pages.get_info(page_id.clone()).await.is_some() {
        return false;
    }
    match browser.pages.list().await {
        Ok(pages) => !pages.iter().any(|page| page.page_id == *page_id),
        Err(error) => {
            warn!(
                error = %error,
                page_id = page_id.0,
                "page ownership stale-prune refresh failed"
            );
            false
        }
    }
}

const _: ToolGuard = guard;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AgentKey;
    use rmcp::model::ContentBlock;
    use serde_json::json;

    #[tokio::test]
    async fn denies_different_agent_key_with_ts_message() -> anyhow::Result<()> {
        let call = crate::mcp::test_support::tool_call("navigate", json!({ "page": 7 })).await?;
        call.state
            .sessions
            .ownership()
            .claim_page(AgentKey::new("other"), PageId(7))
            .await;
        let result = guard(&call)
            .await
            .unwrap_or_else(|| ToolResult::error("missing"));
        let text = result.content.iter().find_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        });
        assert_eq!(
            text,
            Some(
                "page 7 is not owned by this agent; call `tabs new` to open a fresh page and use the returned page id."
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn denies_user_tab_without_claiming_it() -> anyhow::Result<()> {
        let call = crate::mcp::test_support::tool_call("navigate", json!({ "page": 7 })).await?;
        let result = guard(&call)
            .await
            .unwrap_or_else(|| ToolResult::error("missing"));
        let text = result.content.iter().find_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        });
        assert_eq!(
            text,
            Some(
                "page 7 is not owned by this agent; call `tabs new` to open a fresh page and use the returned page id."
            )
        );
        assert!(
            call.state
                .sessions
                .owner_of_page(&PageId(7))
                .await
                .is_none()
        );
        Ok(())
    }
}
