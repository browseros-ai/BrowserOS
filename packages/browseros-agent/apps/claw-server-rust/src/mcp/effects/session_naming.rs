use crate::mcp::{
    dispatch::{ToolEffect, ToolEffectContext},
    effects::tab_groups::retitle_agent_tab_group,
    naming::{
        build_session_group_title, client_prefix_from_slug, elicit_session_name,
        peer_elicit_session_name,
    },
};
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;
use rmcp::service::ElicitationMode;
use std::sync::atomic::Ordering;
use tracing::{debug, warn};

/// Starts naming on the first successful `tabs new` dispatch for a session.
pub fn apply(context: ToolEffectContext<'_>) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
    Box::pin(async move {
        if context.result.is_error || !context.call.flags.new_page {
            return Ok(None);
        }
        if context.call.naming_started.swap(true, Ordering::SeqCst) {
            return Ok(None);
        }
        let Some(identity) = &context.call.identity else {
            return Ok(None);
        };
        let Some(peer) = context.call.peer.clone() else {
            warn!(
                session_id = %context.call.session_id,
                request_id = ?context.call.request_id,
                "mcp session naming peer unavailable"
            );
            return Ok(None);
        };
        if !peer
            .supported_elicitation_modes()
            .contains(&ElicitationMode::Form)
        {
            tracing::info!(
                session_id = %context.call.session_id,
                "mcp client lacks elicitation capability"
            );
            return Ok(None);
        }
        let Some(tab_groups) = context.call.tool_named("tab_groups").cloned() else {
            warn!("tab_groups tool unavailable for session naming");
            return Ok(None);
        };
        let state = context.call.state.clone();
        let session = identity.session.clone();
        let ownership_key = identity.ownership_key.clone();
        // rmcp 2.1 routes server-initiated requests only through its common SSE channel;
        // it has no related-request API that can target the still-open tool POST stream.
        tokio::spawn(async move {
            let prefix = client_prefix_from_slug(session.agent().slug()).to_string();
            let name = tokio::select! {
                name = elicit_session_name(|| peer_elicit_session_name(&peer, &prefix)) => name,
                () = session.child_token().cancelled_owned() => {
                    debug!(session_id = %session.id(), "session closed during naming elicitation");
                    return;
                }
            };
            let Some(name) = name else {
                return;
            };
            if state.sessions.lookup(session.id()).await.is_none() {
                debug!(session_id = %session.id(), "session closed before naming applied");
                return;
            }
            session.set_session_label(name.clone()).await;
            let title = build_session_group_title(&prefix, &name);
            tracing::info!(session_id = %session.id(), title = %title, "mcp session named");
            let Some(browser) = state.browser.session().await else {
                return;
            };
            retitle_agent_tab_group(
                &tab_groups,
                &browser,
                &state.sessions.ownership(),
                &ownership_key,
                &title,
                session.child_token(),
            )
            .await;
        });
        Ok(None)
    })
}

const _: ToolEffect = apply;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn failed_tabs_new_does_not_consume_naming_attempt() -> anyhow::Result<()> {
        let call = crate::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        let result = ToolResult::error("failed");
        apply(ToolEffectContext {
            call: &call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await
        .unwrap_or_else(|error| panic!("effect failed: {error}"));
        assert!(!call.naming_started.load(Ordering::SeqCst));
        Ok(())
    }

    #[tokio::test]
    async fn first_successful_tabs_new_consumes_naming_attempt() -> anyhow::Result<()> {
        let call = crate::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        let result = ToolResult::text("opened", Some(json!({ "page": 1 })));
        apply(ToolEffectContext {
            call: &call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await?;
        assert!(call.naming_started.load(Ordering::SeqCst));
        Ok(())
    }
}
