//! claw's cockpit side of the shared effects and script hook: the activity view and the
//! durable tab-claim ledger fed through `ClawTabJournal`, and the child audit rows a
//! script's primitives produce. The ownership logic itself is tested in the crate.

mod ownership_claims {
    use crate::api::mcp::test_support::claw_tool_call;

    use crate::db::session_tabs::SessionTabSnapshot;
    use crate::{
        AppState,
        api::mcp::{dispatch::ToolEffectContext, effects::ownership_claims::apply},
    };
    use browseros_cdp::{CdpError, CdpEvent, SessionId};
    use browseros_core::PageId;
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
            params: Value,
            session: Option<&'a browseros_core::SessionId>,
        ) -> futures_util::future::BoxFuture<'a, Result<Value, browseros_cdp::CdpError>> {
            self.send_at_epoch(1, method, params, session)
        }

        fn send_at_epoch<'a>(
            &'a self,
            expected_epoch: u64,
            method: &'a str,
            _params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                // This fixture's epoch is immutable; validation and any synchronous
                // recorded publication cannot be separated by a reconnect.
                if expected_epoch != 1 {
                    return Err(browseros_cdp::CdpError::ConnectionEpochChanged {
                        expected: expected_epoch,
                        actual: 1,
                    });
                }
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

        fn send_raw_json_at_epoch<'a>(
            &'a self,
            expected_epoch: u64,
            method: &'a str,
            params_json: &'a str,
            session: Option<&'a browseros_core::SessionId>,
        ) -> futures_util::future::BoxFuture<'a, Result<String, browseros_cdp::CdpError>> {
            Box::pin(async move {
                let params = serde_json::from_str(params_json).map_err(|error| {
                    browseros_cdp::CdpError::Protocol {
                        code: -1,
                        message: error.to_string(),
                    }
                })?;
                Ok(self
                    .send_at_epoch(expected_epoch, method, params, session)
                    .await?
                    .to_string())
            })
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
        state: &AppState,
        released: bool,
    ) -> anyhow::Result<SessionTabSnapshot> {
        // Polls for up to 30 s: a background write can lag far behind under load.
        for _ in 0..3_000 {
            if let Some(claim) = state.session_tabs.first_session_tab().await?
                && claim.released_at.is_some() == released
            {
                return Ok(claim);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        anyhow::bail!("claim did not reach expected state")
    }

    #[tokio::test]
    async fn new_and_close_write_the_tab_claim_window() -> anyhow::Result<()> {
        let browser =
            BrowserSession::new(PageListConnection::new(), BrowserSessionHooks::default());
        assert_eq!(browser.pages.list().await?.len(), 1);
        let (mut new_call, new_call_state) =
            claw_tool_call("tabs", json!({ "action": "new" })).await?;
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
        let claim = wait_for_claim(&new_call_state, false).await?;
        assert_eq!(claim.tab_id, 11);
        assert_eq!(claim.opened_target_id.as_deref(), Some("target-a"));
        assert_eq!(claim.session_id, "s1");
        assert_eq!(claim.claimed_at, 123);

        let (mut close_call, close_call_state) =
            claw_tool_call("tabs", json!({ "action": "close", "page": 1 })).await?;
        close_call.page_snapshot = browser.pages.get_info(PageId(1)).await;
        close_call_state.session_tabs.enqueue_claim_tab_for_session(
            11,
            Some("target-a".to_string()),
            "s1".to_string(),
            "agent".to_string(),
            100,
        );
        close_call_state.session_tabs.drain_writes().await;
        let result = ToolResult::text("closed page", None);
        apply(ToolEffectContext {
            call: &close_call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await?;
        let released = close_call_state
            .session_tabs
            .session_tab_claimed_at(100)
            .await?
            .unwrap_or_else(|| panic!("close claim missing"));
        // Polls for up to 30 s: a background write can lag far behind under load.
        for _ in 0..3_000 {
            let current = close_call_state
                .session_tabs
                .session_tab_by_id(released.id)
                .await?
                .unwrap_or_else(|| panic!("close claim missing"));
            if current.released_at.is_some() {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        anyhow::bail!("close claim was not released")
    }

    #[tokio::test]
    async fn close_page_removes_owned_page() -> anyhow::Result<()> {
        let (call, _state) =
            claw_tool_call("tabs", json!({ "action": "close", "page": 9 })).await?;
        let identity = call.identity.as_ref().unwrap_or_else(|| unreachable!());
        call.host
            .sessions()
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
        assert_eq!(call.host.sessions().owner_of_page(&PageId(9)).await, None);
        Ok(())
    }

    #[tokio::test]
    async fn close_page_removes_the_exact_activity_incarnation() -> anyhow::Result<()> {
        let browser =
            BrowserSession::new(PageListConnection::new(), BrowserSessionHooks::default());
        assert_eq!(browser.pages.list().await?.len(), 1);
        let (mut call, call_state) =
            claw_tool_call("tabs", json!({ "action": "close", "page": 1 })).await?;
        call.page_snapshot = browser.pages.get_info(PageId(1)).await;
        call_state
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
            call_state
                .tab_activity
                .snapshot(Some(&browser))
                .await
                .is_empty()
        );
        Ok(())
    }
}

mod tab_activity {
    use crate::api::mcp::test_support::claw_tool_call;

    use crate::api::mcp::{dispatch::ToolEffectContext, effects::tab_activity::apply};
    use browseros_core::{PageId, TabId, TargetId, pages::PageInfo};
    use browseros_mcp::ToolResult;
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
            window_id: None,
            index: None,
            group_id: None,
            browser_profile_id: None,
            live_browser_context_id: None,
        }
    }

    #[tokio::test]
    async fn error_result_does_not_record_activity() -> anyhow::Result<()> {
        let (call, call_state) = claw_tool_call("navigate", json!({ "page": 1 })).await?;
        let result = ToolResult::error("failed");
        apply(ToolEffectContext {
            call: &call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await?;
        assert!(
            call_state
                .tab_activity
                .snapshot(call.browser_session.as_deref())
                .await
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn successful_dispatch_records_the_starting_target_incarnation() -> anyhow::Result<()> {
        let (mut call, call_state) = claw_tool_call("navigate", json!({ "page": 1 })).await?;
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
            call_state
                .tab_activity
                .remove_incarnation(1, "target-old")
                .await
        );
        Ok(())
    }

    #[tokio::test]
    async fn close_dispatch_does_not_restore_removed_activity() -> anyhow::Result<()> {
        let (mut call, call_state) =
            claw_tool_call("tabs", json!({ "action": "close", "page": 1 })).await?;
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
            !call_state
                .tab_activity
                .remove_incarnation(1, "target-old")
                .await
        );
        Ok(())
    }
}

mod script_hook {
    use crate::api::mcp::script_hook::ScriptInnerCallHook;
    use crate::api::mcp::test_support::claw_tool_call;
    use browseros_mcp::{InnerCallHook, InnerCallRecord};

    use crate::db::audit_log::ListDispatchesQuery;

    use browseros_cdp::{CdpError, CdpEvent, SessionId as CdpSessionId};

    use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection};

    use futures_util::future::BoxFuture;

    use browseros_mcp::token_estimate::TOKEN_ESTIMATOR_VERSION;
    use serde_json::{Value, json};
    use std::sync::Arc;

    use tokio::sync::broadcast;

    fn hook_for(call: &crate::api::mcp::dispatch::ToolCall) -> ScriptInnerCallHook {
        ScriptInnerCallHook::new(call.clone())
    }

    #[tokio::test]
    async fn bound_script_creation_receipt_survives_ownership_handoff() -> anyhow::Result<()> {
        assert_creation_survives_ownership_handoff(true).await
    }

    #[tokio::test]
    async fn unbound_script_creation_receipt_survives_ownership_handoff() -> anyhow::Result<()> {
        assert_creation_survives_ownership_handoff(false).await
    }

    async fn assert_creation_survives_ownership_handoff(bound: bool) -> anyhow::Result<()> {
        use crate::api::mcp::test_support::ProfileConnection;
        use browseros_core::{PageId, WindowId, pages::NewPageOptions, target::BrowserView};
        let native = ProfileConnection::new();
        let browser = BrowserSession::new(native.clone(), BrowserSessionHooks::default());
        browser.pages.list().await?;
        let (mut call, call_state) = claw_tool_call("run", json!({"code":"return 1"})).await?;
        call.browser_session = Some(browser.clone());
        call.view = Some(if bound {
            BrowserView::capture(
                browser.clone(),
                Some(ProfileConnection::binding("eng", 1, 1)),
                None,
            )
            .await
        } else {
            BrowserView::unbound(browser.clone(), Some(WindowId(1)))
        });
        let original = call
            .view
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("view missing"))?
            .new_page(
                "https://synthetic.test/ownership-handoff",
                NewPageOptions::default(),
            )
            .await?;
        let identity = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?;
        let ownership = call.host.sessions().ownership();
        let operation = ownership
            .group_operation_lock(&identity.ownership_key)
            .await;
        let queued = operation.lock().await;
        let paused = call_state.tab_activity.pause_recording_for_testing().await;
        let hook = hook_for(&call);
        let mut work = hook.on_page_created(original.page_id.0);
        // Poll the actual hook until record_new_page waits on its recording lock.
        // No grouping capture has run; the native creation has already returned.
        assert!(futures_util::poll!(work.as_mut()).is_pending());
        assert!(call.created_page_snapshots.lock().await.is_empty());
        {
            let mut tabs = native.tabs.lock().unwrap_or_else(|p| p.into_inner());
            let tab = tabs
                .iter_mut()
                .find(|tab| tab["tabId"] == original.tab_id.0)
                .ok_or_else(|| anyhow::anyhow!("created tab missing"))?;
            tab["targetId"] = json!("replacement-before-script-capture");
        }
        browser.pages.list().await?;
        let replaced = browser
            .pages
            .get_info(PageId(original.page_id.0))
            .await
            .ok_or_else(|| anyhow::anyhow!("reconciled page missing"))?;
        assert_eq!(replaced.tab_id, original.tab_id);
        assert_eq!(replaced.window_id, original.window_id);
        assert_ne!(replaced.target_id, original.target_id);
        drop(paused);
        tokio::time::timeout(std::time::Duration::from_secs(30), work).await?;
        // Let any incorrectly spawned callback queue behind the real operation
        // lock, then wait for it to finish before inspecting native mutations.
        tokio::task::yield_now().await;
        drop(queued);
        let _finished = operation.lock().await;
        {
            let calls = native.calls.lock().unwrap_or_else(|p| p.into_inner());
            let mutations = calls
                .iter()
                .filter(|(method, _)| {
                    method.contains("TabGroup") && method != "Browser.getTabGroups"
                })
                .collect::<Vec<_>>();
            assert!(
                mutations.is_empty(),
                "replacement received script group writes: {mutations:?}"
            );
        }
        assert!(
            call.created_page_snapshots.lock().await.is_empty(),
            "replacement was adopted as the creation snapshot"
        );
        call_state.session_tabs.drain_writes().await;
        let recorded = call_state
            .session_tabs
            .first_session_tab()
            .await?
            .ok_or_else(|| anyhow::anyhow!("creation ownership missing"))?;
        assert_eq!(
            recorded.opened_target_id.as_deref(),
            Some(original.target_id.as_str())
        );
        Ok(())
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
            params: Value,
            session: Option<&'a browseros_core::SessionId>,
        ) -> futures_util::future::BoxFuture<'a, Result<Value, browseros_cdp::CdpError>> {
            self.send_at_epoch(1, method, params, session)
        }

        fn send_at_epoch<'a>(
            &'a self,
            expected_epoch: u64,
            method: &'a str,
            _params: Value,
            _session: Option<&'a CdpSessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                // This fixture's epoch is immutable; validation and any synchronous
                // recorded publication cannot be separated by a reconnect.
                if expected_epoch != 1 {
                    return Err(browseros_cdp::CdpError::ConnectionEpochChanged {
                        expected: expected_epoch,
                        actual: 1,
                    });
                }
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

        fn send_raw_json_at_epoch<'a>(
            &'a self,
            expected_epoch: u64,
            method: &'a str,
            params_json: &'a str,
            session: Option<&'a browseros_core::SessionId>,
        ) -> futures_util::future::BoxFuture<'a, Result<String, browseros_cdp::CdpError>> {
            Box::pin(async move {
                let params = serde_json::from_str(params_json).map_err(|error| {
                    browseros_cdp::CdpError::Protocol {
                        code: -1,
                        message: error.to_string(),
                    }
                })?;
                Ok(self
                    .send_at_epoch(expected_epoch, method, params, session)
                    .await?
                    .to_string())
            })
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
        let (mut call, call_state) = claw_tool_call("run", json!({ "code": "return 1" })).await?;
        call.browser_session = Some(browser);
        call.started_at_ms = 123;
        let hook = ScriptInnerCallHook::new(call.clone());

        // The session-tab ownership window is what replay attribution and
        // per-tab screenshot selection join on; a code-mode tab must open it.
        hook.on_page_created(1).await;
        call_state.session_tabs.drain_writes().await;
        let claim = call_state
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
    async fn record_reports_tab_activity_so_the_session_shows_progress() -> anyhow::Result<()> {
        let browser = BrowserSession::new(OnePageConnection::new(), BrowserSessionHooks::default());
        assert_eq!(browser.pages.list().await?.len(), 1);
        let (mut call, call_state) = claw_tool_call("run", json!({ "code": "return 1" })).await?;
        call.browser_session = Some(browser);
        let hook = ScriptInnerCallHook::new(call.clone());

        // A code-mode session showed Idle with zero tabs because tab activity is
        // populated only by the bypassed effects; the hook now records it, so
        // the cockpit sees the tab and the session as progressing.
        assert!(
            call_state
                .tab_activity
                .snapshot(call.browser_session.as_deref())
                .await
                .is_empty()
        );
        hook.record(InnerCallRecord {
            method: "input.click",
            page: Some(1),
            args: &json!([1, "e5"]),
            from_helper: false,
            is_error: false,
            duration_ms: 3,
            output_token_estimate: 0,
        })
        .await;
        assert!(
            !call_state
                .tab_activity
                .snapshot(call.browser_session.as_deref())
                .await
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn record_writes_child_row_linked_to_parent() -> anyhow::Result<()> {
        let (call, call_state) = claw_tool_call("run", json!({ "code": "return 1" })).await?;
        let hook = hook_for(&call);

        hook.record(InnerCallRecord {
            method: "input.click",
            page: Some(4),
            args: &json!([4, "e5"]),
            from_helper: false,
            is_error: false,
            duration_ms: 12,
            output_token_estimate: 42,
        })
        .await;

        let rows = call_state
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
        // The inner primitive is measured with the v1 estimator so it no longer
        // taints the session: a non-zero input estimate from the method + args,
        // the passed-through output estimate, and the eligible version.
        assert_eq!(child.token_estimator_version, TOKEN_ESTIMATOR_VERSION);
        assert!(child.tool_input_token_estimate > 0);
        assert_eq!(child.tool_output_token_estimate, 42);
        Ok(())
    }
}
