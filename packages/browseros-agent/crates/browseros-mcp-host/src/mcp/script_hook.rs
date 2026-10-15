use crate::{
    clock::now_epoch_ms,
    helpers,
    mcp::{
        dispatch::{ToolCall, ToolIdentity},
        effects::{ownership_claims, tab_groups, tabs_list_view},
    },
    seams::{InnerCallContext, TabActivity},
};
use browseros_core::PageId;
use browseros_mcp::{InnerCallHook, InnerCallRecord};
use futures_util::future::BoxFuture;
use serde_json::Value;

/// Host hook a `run`/`execute` script invokes around each browser primitive.
/// A script uses the captured BrowserView directly, bypassing the outer pipeline
/// effects. Core enforces scope; this hook reproduces what the host effects would have
/// done: it notes whose tab each primitive touched, hands each primitive to the host's
/// observers as an inner call linked to the script dispatch (claw writes a child audit
/// row), and (on page creation) claims and groups the page exactly as a `tabs new` would. Explicit group tools share the
/// detached grouper's mutex for that inner operation, without locking a whole
/// script. It holds the script's own `ToolCall` to retain the same captured view.
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
    fn acquire_tool_guard<'a>(
        &'a self,
        tool_name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Option<Box<dyn Send>>, String>> {
        Box::pin(async move {
            // MCP retains this opaque RAII guard through handler and readback,
            // then drops it before child auditing. The mutex supplies ordering;
            // it never replaces the immutable BrowserView's deep scope guards.
            self.call
                .acquire_group_operation_guard(tool_name, args)
                .await
                .map(|guard| guard.map(|guard| Box::new(guard) as Box<dyn Send>))
                .map_err(|()| "cancelled".to_string())
        })
    }

    fn authorize<'a>(&'a self, page: Option<u32>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(page) = page else {
                return Ok(());
            };
            let Some(identity) = self.identity() else {
                return Ok(());
            };
            // Ownership is a label, not a permission, so nothing is refused here.
            //
            // This hook used to reject any page owned by another conversation, which
            // made the user's own tabs unusable from a script and, because ownership
            // keys on a per-session id, orphaned the script's own pages whenever a
            // session handle went missing. Do not reintroduce a refusal here.
            //
            // It still has to *notice*. `run` carries no top-level `page` argument, so
            // the outer effect cannot see which pages a script touched; without this,
            // a script could read, navigate, fill or close someone else's tab and be
            // told nothing, which is the promise the refusal used to keep badly.
            let owner = self.call.host.sessions.owner_of_page(&PageId(page)).await;
            let describe = match owner {
                Some(owner) if owner == identity.ownership_key => None,
                Some(owner) => Some(
                    self.call
                        .host
                        .sessions
                        .snapshot()
                        .await
                        .into_iter()
                        .find(|session| session.convo_id() == &owner)
                        .map_or_else(
                            || format!("another agent ({owner})"),
                            |session| format!("another agent ({})", session.agent().label()),
                        ),
                ),
                None => Some("the user".to_string()),
            };
            if let Some(description) = describe
                && let Ok(mut pages) = self.call.foreign_pages.lock()
            {
                pages.insert(page, description);
            }
            Ok(())
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
            // shows it, and an observer's screenshot below selects it.
            if let (Some(info), Some(page_id)) = (&live, page) {
                self.call
                    .host
                    .tabs
                    .record_activity(TabActivity {
                        target_id: info.target_id.clone(),
                        tab_id: info.tab_id.0,
                        page_id,
                        session_id: self.call.session_id.as_str().to_string(),
                        convo_id: identity.session.convo_id().as_str().to_string(),
                        slug: identity.agent.slug().to_string(),
                        tool_name: tool_name.clone(),
                    })
                    .await;
            }
            for observer in &self.call.host.observers {
                observer
                    .observe_inner_call(InnerCallContext {
                        call: &self.call,
                        identity,
                        tool_name: &tool_name,
                        page,
                        live: live.as_ref(),
                        args: &raw_args,
                        is_error,
                        from_helper,
                        duration_ms,
                        output_token_estimate,
                    })
                    .await;
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
                &self.call.host,
                identity,
                self.call.browser_session.as_ref(),
                self.call.session_id.as_str(),
                page_id,
                self.call.started_at_ms,
            )
            .await;
            // Ensure the agent's tab group and place the page in it. Detached so
            // browser grouping does not block the script, matching the effect.
            // Retain the created incarnation and its guarded view before spawning,
            // including the original epoch for an unbound run. Missing metadata
            // skips this cosmetic effect rather than dispatching without a guard.
            if let Some(created) = tab_groups::created_page(&self.call, page_id).await {
                tokio::spawn(tab_groups::run_tab_group_work(
                    self.call.clone(),
                    Some(created),
                ));
            }
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
                &self.call.host,
                &identity.ownership_key,
                pages,
                "pageId",
            )
            .await
        })
    }

    fn resolve_host<'a>(&'a self, page: u32) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            // Helper selection is visible to the script, so it admits the requested
            // page through the same capture as all browser primitives.
            let browser = self.call.view.as_ref()?;
            let info = browser.page(PageId(page)).await.ok()?;
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
            helpers::save_helper(&self.call.host.helpers_root, &meta, source)
                .map_err(|error| format!("could not save helper: {error}"))
        })
    }

    fn list_helpers<'a>(&'a self, host: &'a str) -> BoxFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let now = now_epoch_ms();
            helpers::list_helper_meta(&self.call.host.helpers_root, host)
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
            helpers::read_helper(&self.call.host.helpers_root, host, name)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ids::ConvoId;

    use crate::test_support::tool_call;

    use serde_json::json;

    fn hook_for(call: &crate::mcp::dispatch::ToolCall) -> ScriptInnerCallHook {
        ScriptInnerCallHook::new(call.clone())
    }

    #[tokio::test]
    /// Ownership never refuses. A script may act on the user's tabs and on other
    /// agents' tabs; it is told whose they are, it is not stopped.
    ///
    /// This test previously asserted the opposite. It was changed deliberately: the
    /// refusal made the user's own tabs unusable from a script, and orphaned a
    /// script's own pages whenever a session handle went missing.
    async fn authorize_never_refuses_on_ownership() -> anyhow::Result<()> {
        let call = tool_call("run", json!({ "code": "return 1" })).await?;
        let hook = hook_for(&call);

        // Another conversation's page: allowed.
        call.host
            .sessions
            .ownership()
            .claim_page(ConvoId::new("other"), PageId(7))
            .await;
        assert!(hook.authorize(Some(7)).await.is_ok());

        // An unclaimed page is one of the user's own: allowed.
        assert!(hook.authorize(Some(9)).await.is_ok());
        assert!(hook.authorize(None).await.is_ok());

        // And the caller's own page, as before.
        let mine = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .ownership_key
            .clone();
        call.host
            .sessions
            .ownership()
            .claim_page(mine, PageId(3))
            .await;
        assert!(hook.authorize(Some(3)).await.is_ok());
        Ok(())
    }

    /// Allowing is not the same as staying silent. `run` has no top-level `page`
    /// argument, so unless the hook records what the script touched the agent gets
    /// no signal at all that it worked in somebody else's tab.
    #[tokio::test]
    async fn authorize_records_pages_that_belong_to_someone_else() -> anyhow::Result<()> {
        let call = tool_call("run", json!({ "code": "return 1" })).await?;
        let mine = call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .ownership_key
            .clone();
        call.host
            .sessions
            .ownership()
            .claim_page(mine, PageId(3))
            .await;
        call.host
            .sessions
            .ownership()
            .claim_page(ConvoId::new("other"), PageId(7))
            .await;
        let hook = hook_for(&call);

        hook.authorize(Some(3)).await.ok();
        hook.authorize(Some(7)).await.ok();
        hook.authorize(Some(9)).await.ok();
        hook.authorize(None).await.ok();

        let recorded = call
            .foreign_pages
            .lock()
            .map_err(|_| anyhow::anyhow!("foreign_pages poisoned"))?
            .clone();
        assert!(!recorded.contains_key(&3), "own page needs no notice");
        assert_eq!(
            recorded.get(&7).map(String::as_str),
            Some("another agent (other)")
        );
        assert_eq!(recorded.get(&9).map(String::as_str), Some("the user"));
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
        call.host
            .sessions
            .ownership()
            .claim_page(mine, PageId(3))
            .await;
        call.host
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
            call.host.sessions.owner_of_page(&PageId(5)).await,
            Some(mine)
        );
        assert!(hook.authorize(Some(5)).await.is_ok());
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
