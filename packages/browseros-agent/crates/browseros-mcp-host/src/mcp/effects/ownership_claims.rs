use crate::mcp::dispatch::{
    ToolEffect, ToolEffectContext, ToolIdentity, extract_page_id, result_page_id,
};
use crate::{
    ctx::HostCtx,
    seams::{TabActivity, TabClaim},
};
use browseros_core::{BrowserSession, PageId};
use futures_util::future::BoxFuture;
use std::sync::Arc;

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
            record_new_page(
                &context.call.host,
                identity,
                context.call.browser_session.as_ref(),
                context.call.session_id.as_str(),
                page_id,
                context.call.started_at_ms,
            )
            .await;
        } else if context.call.flags.close_page
            && let Some(page_id) = extract_page_id(context.call)
        {
            let page_id = PageId(page_id);
            if let Some(page) = &context.call.page_snapshot {
                let target_id = page.target_id.as_str().to_string();
                let session_id = context.call.session_id.as_str().to_string();
                context
                    .call
                    .host
                    .tabs
                    .forget_incarnation(page_id.0, &target_id)
                    .await;
                context
                    .call
                    .host
                    .tabs
                    .release_tab(page.tab_id.0, session_id);
            }
            context
                .call
                .host
                .sessions
                .ownership()
                .remove_page(&page_id)
                .await;
        }
        Ok(None)
    })
}

/// Records a newly created page as owned by the agent: tab activity, the
/// session-tab ownership window, and the page-ownership claim. Shared by the
/// tabs-new effect and the code-mode script hook so a page a script opens is
/// claimed the same way a `tabs new` page is.
pub(crate) async fn record_new_page(
    host: &HostCtx,
    identity: &ToolIdentity,
    browser: Option<&Arc<BrowserSession>>,
    session_id: &str,
    page_id: u32,
    claimed_at: i64,
) {
    if let Some(browser) = browser
        && let Some(info) = browser.pages.get_info(PageId(page_id)).await
    {
        let target_id = info.target_id.as_str().to_string();
        host.tabs
            .record_activity(TabActivity {
                target_id: info.target_id,
                tab_id: info.tab_id.0,
                page_id,
                session_id: session_id.to_string(),
                convo_id: identity.session.convo_id().as_str().to_string(),
                slug: identity.agent.slug().to_string(),
                tool_name: "tabs".to_string(),
            })
            .await;
        host.tabs.claim_tab(TabClaim {
            tab_id: info.tab_id.0,
            target_id: Some(target_id),
            session_id: session_id.to_string(),
            convo_id: identity.session.convo_id().as_str().to_string(),
            claimed_at_ms: claimed_at,
        });
    }
    host.sessions
        .ownership()
        .claim_page(identity.ownership_key.clone(), PageId(page_id))
        .await;
}

const _: ToolEffect = apply;
