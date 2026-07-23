use crate::{
    api::mcp::dispatch::{ToolCall, ToolIdentity},
    api::mcp::effects::{ownership_claims, tab_groups},
    db::audit_log::{DispatchResultSummary, RecordToolDispatchInput},
    ids::DispatchId,
};
use browseros_core::PageId;
use browseros_mcp::{InnerCallHook, InnerCallRecord};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tracing::warn;

/// Host hook a `run`/`execute` script invokes around each browser primitive.
/// A script drives the shared browser session directly, bypassing the pipeline
/// guards and effects, so this hook reproduces what those effects would have
/// done: it enforces per-primitive ownership, records each primitive as a child
/// audit row linked to the script dispatch, and (on page creation) claims and
/// groups the page exactly as a `tabs new` would. It holds the script's own
/// `ToolCall` so it can reuse the effect helpers with the same inputs.
pub struct ScriptInnerCallHook {
    call: ToolCall,
}

impl ScriptInnerCallHook {
    #[must_use]
    pub fn new(call: ToolCall) -> Self {
        Self { call }
    }

    fn identity(&self) -> Option<&ToolIdentity> {
        self.call.identity.as_ref()
    }
}

impl InnerCallHook for ScriptInnerCallHook {
    fn authorize<'a>(&'a self, page: Option<u32>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(page) = page else {
                return Ok(());
            };
            let Some(identity) = self.identity() else {
                return Ok(());
            };
            // Reject pages owned by another conversation. Pages a script creates
            // itself are claimed via on_page_created, so they pass; a page the
            // agent never owned is rejected.
            match self.call.state.sessions.owner_of_page(&PageId(page)).await {
                Some(owner) if owner != identity.ownership_key => Err(format!(
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
            let Some(identity) = self.identity() else {
                return;
            };
            let live = match (&self.call.browser_session, page) {
                (Some(browser), Some(page)) => browser.pages.get_info(PageId(page)).await,
                _ => None,
            };
            let input = RecordToolDispatchInput {
                agent_id: identity.session.convo_id().as_str().to_string(),
                slug: identity.agent.slug().to_string(),
                agent_label: identity.agent_label.clone(),
                session_id: self.call.session_id.as_str().to_string(),
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
                parent_dispatch_id: Some(self.call.dispatch_id.clone()),
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
            if let Err(error) = self.call.state.audit_log.record_tool_dispatch(input).await {
                warn!(error = %error, "script inner-call audit write failed");
            }
        })
    }

    fn on_page_created<'a>(&'a self, page_id: u32) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(identity) = self.identity() else {
                return;
            };
            // Claim the page, record tab activity, and open the session-tab
            // window, exactly as the tabs-new effect does. Awaited so the claim
            // lands before the script's next primitive on this page.
            ownership_claims::record_new_page(
                &self.call.state,
                identity,
                self.call.browser_session.as_ref(),
                self.call.session_id.as_str(),
                page_id,
                self.call.started_at_ms,
            )
            .await;
            // Ensure the agent's tab group and place the page in it. Detached so
            // browser grouping does not block the script, matching the effect.
            tokio::spawn(tab_groups::run_tab_group_work(
                self.call.clone(),
                Some(page_id),
            ));
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::mcp::test_support::tool_call;
    use crate::db::audit_log::ListDispatchesQuery;
    use crate::ids::ConvoId;
    use browseros_cdp::{CdpError, CdpEvent, SessionId as CdpSessionId};
    use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection};
    use futures_util::future::BoxFuture;
    use std::sync::Arc;
    use tokio::sync::broadcast;

    fn hook_for(call: &crate::api::mcp::dispatch::ToolCall) -> ScriptInnerCallHook {
        ScriptInnerCallHook::new(call.clone())
    }

    /// Minimal browser connection exposing one page (tab 11), enough for
    /// `pages.get_info` so the hook can open a session-tab window.
    struct OnePageConnection {
        events: broadcast::Sender<CdpEvent>,
    }

    impl OnePageConnection {
        fn new() -> Arc<Self> {
            let (events, _) = broadcast::channel(1);
            Arc::new(Self { events })
        }
    }

    impl CdpConnection for OnePageConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            _params: Value,
            _session: Option<&'a CdpSessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                match method {
                    "Browser.getTabs" => Ok(json!({ "tabs": [{
                        "tabId": 11, "targetId": "target-a", "url": "https://example.com",
                        "title": "Example", "isActive": true, "isLoading": false,
                        "loadProgress": 1.0, "isPinned": false, "isHidden": false,
                        "windowId": 1, "index": 0
                    }] })),
                    _ => Ok(json!({})),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a CdpSessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_string()) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            self.events.subscribe()
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    #[tokio::test]
    async fn on_page_created_opens_the_session_tab_window_for_replay() -> anyhow::Result<()> {
        let browser = BrowserSession::new(OnePageConnection::new(), BrowserSessionHooks::default());
        assert_eq!(browser.pages.list().await?.len(), 1);
        let mut call = tool_call("run", json!({ "code": "return 1" })).await?;
        call.browser_session = Some(browser);
        call.started_at_ms = 123;
        let hook = ScriptInnerCallHook::new(call.clone());

        // The session-tab ownership window is what replay attribution and
        // per-tab screenshot selection join on; a code-mode tab must open it.
        hook.on_page_created(1).await;
        call.state.session_tabs.drain_writes().await;
        let claim = call
            .state
            .session_tabs
            .first_session_tab()
            .await?
            .ok_or_else(|| anyhow::anyhow!("session-tab window not opened"))?;
        assert_eq!(claim.tab_id, 11);
        assert_eq!(claim.opened_target_id.as_deref(), Some("target-a"));
        assert_eq!(claim.claimed_at, 123);
        assert!(claim.released_at.is_none());
        Ok(())
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
        let mine = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .ownership_key
            .clone();
        call.state
            .sessions
            .ownership()
            .claim_page(mine, PageId(3))
            .await;
        assert!(hook.authorize(Some(3)).await.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn on_page_created_claims_the_page_for_the_agent() -> anyhow::Result<()> {
        let call = tool_call("run", json!({ "code": "return 1" })).await?;
        let mine = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .ownership_key
            .clone();
        let hook = hook_for(&call);

        // A page the script opens is claimed for the agent, so a subsequent
        // primitive on it authorizes and the cockpit can attribute it.
        hook.on_page_created(5).await;
        assert_eq!(
            call.state.sessions.owner_of_page(&PageId(5)).await,
            Some(mine)
        );
        assert!(hook.authorize(Some(5)).await.is_ok());
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
            .ok_or_else(|| anyhow::anyhow!("child row recorded"))?;
        assert_eq!(
            child.parent_dispatch_id.as_deref(),
            Some(call.dispatch_id.as_str())
        );
        assert_eq!(child.page_id, Some(4));
        Ok(())
    }
}
