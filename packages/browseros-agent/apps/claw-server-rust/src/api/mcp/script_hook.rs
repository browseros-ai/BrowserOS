use crate::{
    AppState,
    api::mcp::dispatch::ToolIdentity,
    db::audit_log::{DispatchResultSummary, RecordToolDispatchInput},
    ids::{ConvoId, DispatchId},
};
use browseros_core::{BrowserSession, PageId};
use browseros_mcp::{InnerCallHook, InnerCallRecord};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::Arc;
use tracing::warn;

/// Host hook a `run`/`execute` script invokes around each browser primitive.
/// It enforces per-primitive ownership (the pipeline's `page_ownership` guard
/// never sees the pages a script touches) and records each primitive as a child
/// audit row linked to the script dispatch via `parent_dispatch_id`.
pub struct ScriptInnerCallHook {
    state: AppState,
    browser_session: Option<Arc<BrowserSession>>,
    ownership_key: ConvoId,
    agent_id: String,
    slug: String,
    agent_label: String,
    session_id: String,
    parent_dispatch_id: DispatchId,
}

impl ScriptInnerCallHook {
    pub fn new(
        state: AppState,
        browser_session: Option<Arc<BrowserSession>>,
        identity: &ToolIdentity,
        session_id: String,
        parent_dispatch_id: DispatchId,
    ) -> Self {
        Self {
            state,
            browser_session,
            ownership_key: identity.ownership_key.clone(),
            agent_id: identity.session.convo_id().as_str().to_string(),
            slug: identity.agent.slug().to_string(),
            agent_label: identity.agent_label.clone(),
            session_id,
            parent_dispatch_id,
        }
    }
}

impl InnerCallHook for ScriptInnerCallHook {
    fn authorize<'a>(&'a self, page: Option<u32>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(page) = page else {
                return Ok(());
            };
            // Reject pages owned by another conversation. Pages a script creates
            // itself are unclaimed and stay usable; full guard parity (rejecting
            // unclaimed pages too) requires claiming script-created pages and is
            // a follow-up.
            match self.state.sessions.owner_of_page(&PageId(page)).await {
                Some(owner) if owner != self.ownership_key => Err(format!(
                    "page {page} is not owned by this agent; call `tabs new` to open a fresh page and use the returned page id."
                )),
                _ => Ok(()),
            }
        })
    }

    fn record<'a>(&'a self, record: InnerCallRecord<'a>) -> BoxFuture<'a, ()> {
        let tool_name = record.method.to_owned();
        let page = record.page;
        let is_error = record.is_error;
        let duration_ms = record.duration_ms;
        Box::pin(async move {
            let live = match (&self.browser_session, page) {
                (Some(browser), Some(page)) => browser.pages.get_info(PageId(page)).await,
                _ => None,
            };
            let input = RecordToolDispatchInput {
                agent_id: self.agent_id.clone(),
                slug: self.slug.clone(),
                agent_label: self.agent_label.clone(),
                session_id: self.session_id.clone(),
                tool_name,
                page_id: page.map(i64::from),
                tab_id: live.as_ref().map(|page| page.tab_id.0),
                target_id: live
                    .as_ref()
                    .map(|page| page.target_id.as_str().to_string()),
                url: live.as_ref().map(|page| page.url.clone()),
                title: live.as_ref().map(|page| page.title.clone()),
                raw_args: json!({ "page": page }),
                duration_ms,
                dispatch_id: DispatchId::new(),
                parent_dispatch_id: Some(self.parent_dispatch_id.clone()),
                // Inner-primitive token traffic is not measured; version 0 is
                // the reserved unmeasured marker.
                tool_input_token_estimate: 0,
                tool_output_token_estimate: 0,
                token_estimator_version: 0,
                result: DispatchResultSummary {
                    is_error,
                    cancelled: false,
                    structured_content: Value::Null,
                    content: json!([]),
                },
            };
            if let Err(error) = self.state.audit_log.record_tool_dispatch(input).await {
                warn!(error = %error, "script inner-call audit write failed");
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::mcp::dispatch::ToolCall;
    use crate::api::mcp::test_support::tool_call;
    use crate::db::audit_log::ListDispatchesQuery;

    fn hook_for(call: &ToolCall) -> ScriptInnerCallHook {
        let identity = call.identity.as_ref().expect("identity");
        ScriptInnerCallHook::new(
            call.state.clone(),
            call.browser_session.clone(),
            identity,
            call.session_id.as_str().to_string(),
            call.dispatch_id.clone(),
        )
    }

    #[tokio::test]
    async fn authorize_rejects_foreign_owned_pages_only() -> anyhow::Result<()> {
        let call = tool_call("run", json!({ "code": "return 1" })).await?;
        let hook = hook_for(&call);

        // A page owned by another conversation is rejected.
        call.state
            .sessions
            .ownership()
            .claim_page(ConvoId::new("other"), PageId(7))
            .await;
        assert!(hook.authorize(Some(7)).await.is_err());

        // Unclaimed pages and no-page primitives are allowed so a script's own
        // freshly created tabs stay usable.
        assert!(hook.authorize(Some(9)).await.is_ok());
        assert!(hook.authorize(None).await.is_ok());

        // A page owned by the caller's own conversation is allowed.
        let mine = call.identity.as_ref().unwrap().ownership_key.clone();
        call.state
            .sessions
            .ownership()
            .claim_page(mine, PageId(3))
            .await;
        assert!(hook.authorize(Some(3)).await.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn record_writes_child_row_linked_to_parent() -> anyhow::Result<()> {
        let call = tool_call("run", json!({ "code": "return 1" })).await?;
        let hook = hook_for(&call);

        hook.record(InnerCallRecord {
            method: "input.click",
            page: Some(4),
            is_error: false,
            duration_ms: 12,
        })
        .await;

        let rows = call
            .state
            .audit_log
            .list_dispatches(ListDispatchesQuery {
                session_id: Some(call.session_id.as_str().to_string()),
                ..Default::default()
            })
            .await?
            .rows;
        let child = rows
            .iter()
            .find(|row| row.tool_name == "input.click")
            .expect("child row recorded");
        assert_eq!(
            child.parent_dispatch_id.as_deref(),
            Some(call.dispatch_id.as_str())
        );
        assert_eq!(child.page_id, Some(4));
        Ok(())
    }
}
