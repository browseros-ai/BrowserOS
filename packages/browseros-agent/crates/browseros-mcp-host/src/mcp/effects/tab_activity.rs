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
            .host
            .tabs
            .record_activity(crate::seams::TabActivity {
                target_id: info.target_id.clone(),
                tab_id: info.tab_id.0,
                page_id,
                session_id: context.call.session_id.as_str().to_string(),
                convo_id: identity.session.convo_id().as_str().to_string(),
                slug: identity.agent.slug().to_string(),
                tool_name: context.call.tool().name.to_string(),
            })
            .await;
        Ok(None)
    })
}

const _: ToolEffect = apply;
