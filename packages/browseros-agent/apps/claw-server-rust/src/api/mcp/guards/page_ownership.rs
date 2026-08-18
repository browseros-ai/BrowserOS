use crate::api::mcp::dispatch::{ToolCall, ToolGuard, extract_page_id};
use browseros_core::PageId;
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;
use std::time::Duration;
use tracing::warn;

/// A page mid-navigation is momentarily unlistable. Only prune its ownership
/// claim if it stays absent across this settle window, so a navigating or
/// reloading page the agent owns keeps its claim.
const PRUNE_ATTEMPTS: usize = 3;
const PRUNE_SETTLE: Duration = Duration::from_millis(150);

/// Prevents an identified agent from dispatching against another agent's page.
pub fn guard(call: &ToolCall) -> BoxFuture<'_, Option<ToolResult>> {
    Box::pin(async move {
        let page_id = PageId(extract_page_id(call)?);
        let identity = call.identity.as_ref()?;
        let ownership = call.state.sessions.ownership();
        // Dispatch requires a pre-existing claim for this conversation; unclaimed user
        // pages are rejected here and never auto-claimed.
        if let Some(owner) = call.state.sessions.owner_of_page(&page_id).await {
            if page_missing_after_refresh(call, &page_id).await {
                ownership.remove_page(&page_id).await;
            } else if owner == identity.ownership_key {
                return None;
            }
        }
        warn!(
            tool = call.tool().name,
            session_id = %call.session_id,
            key = %identity.ownership_key,
            agent_id = %identity.session.convo_id(),
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
    // Poll across a short settle window; the page reappearing on any attempt (a
    // navigation finishing) means it is not gone and keeps its claim. Only a page
    // absent the whole window is treated as closed.
    for attempt in 0..PRUNE_ATTEMPTS {
        if browser.pages.get_info(page_id.clone()).await.is_some() {
            return false;
        }
        match browser.pages.list().await {
            Ok(pages) if pages.iter().any(|page| page.page_id == *page_id) => return false,
            Ok(_) => {}
            Err(error) => {
                warn!(
                    error = %error,
                    page_id = page_id.0,
                    "page ownership stale-prune refresh failed"
                );
                // A refresh error is inconclusive; do not prune on a flake.
                return false;
            }
        }
        if attempt + 1 < PRUNE_ATTEMPTS {
            tokio::time::sleep(PRUNE_SETTLE).await;
        }
    }
    true
}

const _: ToolGuard = guard;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ConvoId;
    use rmcp::model::ContentBlock;
    use serde_json::json;

    #[tokio::test]
    async fn denies_different_agent_key_with_ts_message() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("navigate", json!({ "page": 7 })).await?;
        call.state
            .sessions
            .ownership()
            .claim_page(ConvoId::new("other"), PageId(7))
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
        let call =
            crate::api::mcp::test_support::tool_call("navigate", json!({ "page": 7 })).await?;
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
