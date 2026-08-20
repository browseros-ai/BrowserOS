use crate::{
    api::mcp::dispatch::{ToolCall, ToolIdentity},
    api::mcp::effects::{ownership_claims, tab_groups, tabs_list_view},
    clock::now_epoch_ms,
    db::audit_log::{RecordToolDispatchInput, bounded_args_json, result_meta},
    ids::DispatchId,
    services::helpers,
};
use browseros_core::PageId;
use browseros_mcp::{
    InnerCallHook, InnerCallRecord,
    token_estimate::{TOKEN_ESTIMATOR_VERSION, estimate_tool_input_tokens},
};
use futures_util::future::BoxFuture;
use serde_json::Value;
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
        // Strip the internal `tool:` routing prefix so the audit shows the plain
        // capability (read, wait, screenshot) rather than a routing detail.
        let tool_name = record
            .method
            .strip_prefix("tool:")
            .unwrap_or(record.method)
            .to_owned();
        let page = record.page;
        let is_error = record.is_error;
        let duration_ms = record.duration_ms;
        let from_helper = record.from_helper;
        let output_token_estimate = record.output_token_estimate;
        let raw_args = record.args.clone();
        Box::pin(async move {
            let Some(identity) = self.identity() else {
                return;
            };
            let live = match (&self.call.browser_session, page) {
                (Some(browser), Some(page)) => browser.pages.get_info(PageId(page)).await,
                _ => None,
            };
            // Liveness: keep the idle clock fresh so the cockpit shows the
            // session progressing rather than stuck; the hard session-end path
            // is unchanged.
            identity.session.touch(tokio::time::Instant::now()).await;
            // Record tab activity for the touched page so the cockpit ranks and
            // shows it, and the screenshot below selects it.
            if let (Some(info), Some(page_id)) = (&live, page) {
                self.call
                    .state
                    .tab_activity
                    .record_tool(crate::services::cockpit::RecordToolInput {
                        target_id: info.target_id.clone(),
                        tab_id: info.tab_id.0,
                        page_id,
                        session_id: self.call.session_id.as_str().to_string(),
                        agent_id: identity.session.convo_id().as_str().to_string(),
                        slug: identity.agent.slug().to_string(),
                        tool_name: tool_name.clone(),
                    })
                    .await;
            }
            let child_dispatch_id = DispatchId::new();
            // Measure the inner primitive with the same v1 estimator granular
            // tools use, so a code-mode run's session is no longer tainted by a
            // v0 child and its token savings are projected.
            let tool_input_token_estimate = estimate_tool_input_tokens(&tool_name, &raw_args);
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
                args_json: bounded_args_json(&raw_args),
                result_meta: child_result_meta(is_error, from_helper),
                duration_ms,
                // None: child rows keep their completion time so they sort after
                // the parent script dispatch, which is stamped with its start.
                created_at: None,
                dispatch_id: child_dispatch_id.clone(),
                parent_dispatch_id: Some(self.call.dispatch_id.clone()),
                tool_input_token_estimate,
                tool_output_token_estimate: output_token_estimate,
                token_estimator_version: TOKEN_ESTIMATOR_VERSION,
            };
            match self.call.state.audit_log.record_tool_dispatch(input).await {
                Ok(row_id) => {
                    // Attach a per-step screenshot, keyed to this child row, so
                    // code-mode audits show each primitive like granular tools.
                    // Only for page-targeting primitives; page-less ones (windows,
                    // tab groups) have nothing meaningful to shoot.
                    if page.is_some() {
                        crate::api::mcp::observers::audit::persist_screenshot(
                            &self.call.state,
                            self.call.session_id.as_str(),
                            &child_dispatch_id,
                            row_id,
                        )
                        .await;
                    }
                }
                Err(error) => {
                    warn!(error = %error, "script inner-call audit write failed");
                }
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

    fn annotate_pages<'a>(&'a self, pages: &'a [Value]) -> BoxFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let Some(identity) = self.identity() else {
                return pages.to_vec();
            };
            // Same tri-bucket ownership view the granular `tabs list` returns, so
            // a code-mode script can tell its own tabs from the user's and other
            // agents' tabs. Code-mode `page_json` keys the id as `pageId`.
            tabs_list_view::annotate_pages_with_ownership(
                &self.call.state,
                &identity.ownership_key,
                pages,
                "pageId",
            )
            .await
        })
    }

    fn resolve_host<'a>(&'a self, page: u32) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            let browser = self.call.browser_session.as_ref()?;
            let info = browser.pages.get_info(PageId(page)).await?;
            helpers::host_bucket(&info.url)
        })
    }

    fn save_helper<'a>(
        &'a self,
        host: &'a str,
        name: &'a str,
        source: &'a str,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(identity) = self.identity() else {
                return Err("no agent identity for this script".to_string());
            };
            let (opens_page, inputs) = helpers::analyze_source(source);
            let meta = helpers::HelperMeta {
                name: name.to_string(),
                host: host.to_string(),
                last_verified: now_epoch_ms(),
                agent: identity.agent.slug().to_string(),
                candidate: false,
                opens_page,
                inputs,
                description: format!("Saved helper for {host}"),
                session: String::new(),
            };
            helpers::save_helper(&self.call.state.config.browserclaw_dir, &meta, source)
                .map_err(|error| format!("could not save helper: {error}"))
        })
    }

    fn list_helpers<'a>(&'a self, host: &'a str) -> BoxFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let now = now_epoch_ms();
            helpers::list_helper_meta(&self.call.state.config.browserclaw_dir, host)
                .iter()
                .map(|meta| super::helper_runtime::helper_info_json(meta, now))
                .collect()
        })
    }

    fn read_helper<'a>(&'a self, host: &'a str, name: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            // The agent-facing read returns the full self-documenting doc
            // (description, call form, source), not the bare source: hot-load uses
            // the extracted source; a reader wants the context.
            helpers::read_helper(&self.call.state.config.browserclaw_dir, host, name)
        })
    }
}

/// Result metadata for a child primitive: the standard summary, plus a
/// `fromHelper` marker when the primitive ran inside a hot-loaded helper, so the
/// distiller can skip a successful reuse's replayed actions.
fn child_result_meta(is_error: bool, from_helper: bool) -> String {
    let base = result_meta(is_error, false, &Value::Null, 0);
    if !from_helper {
        return base;
    }
    let mut value: Value =
        serde_json::from_str(&base).unwrap_or_else(|_| Value::Object(serde_json::Map::new()));
    if let Some(object) = value.as_object_mut() {
        object.insert("fromHelper".to_string(), Value::Bool(true));
    }
    value.to_string()
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
    use serde_json::json;
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
    async fn annotate_pages_tags_pageid_keyed_pages_into_ownership_buckets() -> anyhow::Result<()> {
        let call = tool_call("run", json!({ "code": "return 1" })).await?;
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
        call.state
            .sessions
            .ownership()
            .claim_page(ConvoId::new("other"), PageId(7))
            .await;
        let hook = hook_for(&call);

        // Code-mode pages key the id as `pageId` (not `page`); the hook must tag
        // each with its ownership bucket so the script can tell tabs apart.
        let annotated = hook
            .annotate_pages(&[
                json!({ "pageId": 1, "url": "https://user.test" }),
                json!({ "pageId": 3, "url": "https://mine.test" }),
                json!({ "pageId": 7, "url": "https://other.test" }),
            ])
            .await;
        let bucket = |page: &Value| {
            page.get("ownership")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        assert_eq!(bucket(&annotated[0]), "user");
        assert_eq!(bucket(&annotated[1]), "mine");
        assert_eq!(bucket(&annotated[2]), "other-agent");
        assert_eq!(
            annotated[2].get("ownerAgentId").and_then(Value::as_str),
            Some("other")
        );
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
    async fn record_reports_tab_activity_so_the_session_shows_progress() -> anyhow::Result<()> {
        let browser = BrowserSession::new(OnePageConnection::new(), BrowserSessionHooks::default());
        assert_eq!(browser.pages.list().await?.len(), 1);
        let mut call = tool_call("run", json!({ "code": "return 1" })).await?;
        call.browser_session = Some(browser);
        let hook = ScriptInnerCallHook::new(call.clone());

        // A code-mode session showed Idle with zero tabs because tab activity is
        // populated only by the bypassed effects; the hook now records it, so
        // the cockpit sees the tab and the session as progressing.
        assert!(
            call.state
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
            !call
                .state
                .tab_activity
                .snapshot(call.browser_session.as_deref())
                .await
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn record_writes_child_row_linked_to_parent() -> anyhow::Result<()> {
        let call = tool_call("run", json!({ "code": "return 1" })).await?;
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
        // The inner primitive is measured with the v1 estimator so it no longer
        // taints the session: a non-zero input estimate from the method + args,
        // the passed-through output estimate, and the eligible version.
        assert_eq!(child.token_estimator_version, TOKEN_ESTIMATOR_VERSION);
        assert!(child.tool_input_token_estimate > 0);
        assert_eq!(child.tool_output_token_estimate, 42);
        Ok(())
    }

    #[tokio::test]
    async fn save_helper_writes_provenance_and_lists_and_reads_back() -> anyhow::Result<()> {
        let call = tool_call("run", json!({ "code": "return 1" })).await?;
        let hook = hook_for(&call);
        hook.save_helper("linkedin.com", "greet", "async (browser, page) => 1")
            .await
            .map_err(|error| anyhow::anyhow!(error))?;

        // Lists with provenance: a fresh save reads back candidate=false, ageDays 0.
        let listed = hook.list_helpers("linkedin.com").await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["name"], json!("greet"));
        assert_eq!(listed[0]["candidate"], json!(false));
        assert_eq!(listed[0]["ageDays"], json!(0));

        // Reads back the full self-documenting doc, which carries the source.
        let doc = hook
            .read_helper("linkedin.com", "greet")
            .await
            .unwrap_or_default();
        assert!(
            doc.contains("async (browser, page) => 1"),
            "doc missing source: {doc}"
        );
        // A distinct host does not collide.
        assert!(hook.list_helpers("docs.google.com").await.is_empty());
        Ok(())
    }
}
