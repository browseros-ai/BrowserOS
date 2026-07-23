use crate::api::mcp::dispatch::{ToolEffect, ToolEffectContext, extract_page_id, result_page_id};
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
                let target_id = info.target_id.as_str().to_string();
                context
                    .call
                    .state
                    .tab_activity
                    .record_tool(crate::services::cockpit::RecordToolInput {
                        target_id: info.target_id,
                        tab_id: info.tab_id.0,
                        page_id,
                        session_id: context.call.session_id.as_str().to_string(),
                        agent_id: identity.session.convo_id().as_str().to_string(),
                        slug: identity.agent.slug().to_string(),
                        tool_name: "tabs".to_string(),
                    })
                    .await;
                let session_id = context.call.session_id.as_str().to_string();
                let agent_id = identity.session.convo_id().as_str().to_string();
                let claimed_at = context.call.started_at_ms;
                context
                    .call
                    .state
                    .session_tabs
                    .enqueue_claim_tab_for_session(
                        info.tab_id.0,
                        Some(target_id),
                        session_id,
                        agent_id,
                        claimed_at,
                    );
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
            if let Some(page) = &context.call.page_snapshot {
                let target_id = page.target_id.as_str().to_string();
                let session_id = context.call.session_id.as_str().to_string();
                context
                    .call
                    .state
                    .tab_activity
                    .remove_incarnation(page_id.0, &target_id)
                    .await;
                context
                    .call
                    .state
                    .session_tabs
                    .enqueue_release_tab_for_session(page.tab_id.0, session_id);
            }
            context
                .call
                .state
                .sessions
                .ownership()
                .remove_page(&page_id)
                .await;
        }
        Ok(None)
    })
}

const _: ToolEffect = apply;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::session_tabs::SessionTabSnapshot;
    use browseros_cdp::{CdpError, CdpEvent, SessionId};
    use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection};
    use browseros_mcp::ToolResult;
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tokio::sync::broadcast;

    struct PageListConnection {
        events: broadcast::Sender<CdpEvent>,
    }

    impl PageListConnection {
        fn new() -> Arc<Self> {
            let (events, _) = broadcast::channel(1);
            Arc::new(Self { events })
        }
    }

    impl CdpConnection for PageListConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            _params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                match method {
                    "Browser.getTabs" => Ok(json!({
                        "tabs": [{
                            "tabId": 11,
                            "targetId": "target-a",
                            "url": "https://example.com",
                            "title": "Example",
                            "isActive": true,
                            "isLoading": false,
                            "loadProgress": 1.0,
                            "isPinned": false,
                            "isHidden": false,
                            "windowId": 1,
                            "index": 0
                        }]
                    })),
                    _ => Ok(json!({})),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
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

    async fn wait_for_claim(
        call: &crate::api::mcp::dispatch::ToolCall,
        released: bool,
    ) -> anyhow::Result<SessionTabSnapshot> {
        for _ in 0..100 {
            if let Some(claim) = call.state.session_tabs.first_session_tab().await?
                && claim.released_at.is_some() == released
            {
                return Ok(claim);
            }
            tokio::task::yield_now().await;
        }
        anyhow::bail!("claim did not reach expected state")
    }

    #[tokio::test]
    async fn new_and_close_write_the_tab_claim_window() -> anyhow::Result<()> {
        let browser =
            BrowserSession::new(PageListConnection::new(), BrowserSessionHooks::default());
        assert_eq!(browser.pages.list().await?.len(), 1);
        let mut new_call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "new" })).await?;
        new_call.browser_session = Some(browser.clone());
        new_call.started_at_ms = 123;
        let result = ToolResult::text("new page", Some(json!({ "page": 1 })));

        apply(ToolEffectContext {
            call: &new_call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await?;
        let claim = wait_for_claim(&new_call, false).await?;
        assert_eq!(claim.tab_id, 11);
        assert_eq!(claim.opened_target_id.as_deref(), Some("target-a"));
        assert_eq!(claim.session_id, "s1");
        assert_eq!(claim.claimed_at, 123);

        let mut close_call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "close", "page": 1 }),
        )
        .await?;
        close_call.page_snapshot = browser.pages.get_info(PageId(1)).await;
        close_call.state.session_tabs.enqueue_claim_tab_for_session(
            11,
            Some("target-a".to_string()),
            "s1".to_string(),
            "agent".to_string(),
            100,
        );
        close_call.state.session_tabs.drain_writes().await;
        let result = ToolResult::text("closed page", None);
        apply(ToolEffectContext {
            call: &close_call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await?;
        let released = close_call
            .state
            .session_tabs
            .session_tab_claimed_at(100)
            .await?
            .unwrap_or_else(|| panic!("close claim missing"));
        for _ in 0..100 {
            let current = close_call
                .state
                .session_tabs
                .session_tab_by_id(released.id)
                .await?
                .unwrap_or_else(|| panic!("close claim missing"));
            if current.released_at.is_some() {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
        anyhow::bail!("close claim was not released")
    }

    #[tokio::test]
    async fn close_page_removes_owned_page() -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "close", "page": 9 }),
        )
        .await?;
        let identity = call.identity.as_ref().unwrap_or_else(|| unreachable!());
        call.state
            .sessions
            .ownership()
            .claim_page(identity.ownership_key.clone(), PageId(9))
            .await;
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
        Ok(())
    }

    #[tokio::test]
    async fn close_page_removes_the_exact_activity_incarnation() -> anyhow::Result<()> {
        let browser =
            BrowserSession::new(PageListConnection::new(), BrowserSessionHooks::default());
        assert_eq!(browser.pages.list().await?.len(), 1);
        let mut call = crate::api::mcp::test_support::tool_call(
            "tabs",
            json!({ "action": "close", "page": 1 }),
        )
        .await?;
        call.page_snapshot = browser.pages.get_info(PageId(1)).await;
        call.state
            .tab_activity
            .record_tool(crate::services::cockpit::RecordToolInput {
                target_id: browser
                    .pages
                    .get_info(PageId(1))
                    .await
                    .unwrap_or_else(|| unreachable!())
                    .target_id,
                tab_id: 11,
                page_id: 1,
                session_id: call.session_id.as_str().to_string(),
                agent_id: "agent".to_string(),
                slug: "codex".to_string(),
                tool_name: "snapshot".to_string(),
            })
            .await;

        apply(ToolEffectContext {
            call: &call,
            result: &ToolResult::text("closed page", None),
            cancelled: false,
            duration_ms: 1,
        })
        .await?;

        assert!(
            call.state
                .tab_activity
                .snapshot(Some(&browser))
                .await
                .is_empty()
        );
        Ok(())
    }
}
