//! claw's audit observer: every finished dispatch, every primitive a script ran, and
//! every host-local tool call becomes a bounded row on the audit timeline, with a
//! screenshot of the agent's active tab attached when one is available.

use crate::{
    db::{
        AuditLog,
        audit_log::{RecordToolDispatchInput, bounded_args_json, result_meta},
    },
    ids::DispatchId,
    services::{
        audit::{AuditEvent, AuditPreview, AuditWorker},
        cockpit::SessionVisualService,
        screenshots::ScreenshotService,
        sessions::Session,
    },
};
use browseros_core::PageId;
use browseros_mcp::{
    ToolResult,
    token_estimate::{
        TOKEN_ESTIMATOR_VERSION, estimate_tool_input_tokens, estimate_tool_output_tokens,
    },
};
use browseros_mcp_host::{
    mcp::dispatch::{ToolCall, ToolObserverContext, extract_page_id, result_page_id},
    seams::{DispatchObserver, InnerCallContext, LocalToolContext},
};
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::sync::Arc;
use tracing::warn;

const AUDIT_IDENTITY_TEXT_MAX: usize = 512;
const AUDIT_TOOL_NAME_MAX: usize = 128;
const AUDIT_TARGET_ID_MAX: usize = 512;
const AUDIT_URL_MAX: usize = 4096;
const AUDIT_TITLE_MAX: usize = 1024;

pub struct AuditObserver {
    pub audit_worker: Arc<AuditWorker>,
    pub audit_log: Arc<AuditLog>,
    pub visuals: Arc<SessionVisualService>,
    pub screenshots: Arc<ScreenshotService>,
}

impl DispatchObserver for AuditObserver {
    fn name(&self) -> &'static str {
        "audit"
    }

    /// Converts the final tool outcome into a bounded event and admits it to the audit worker.
    fn observe<'a>(&'a self, context: ToolObserverContext<'a>) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let Some(event) = build_event(
                self,
                context.call,
                context.result,
                context.duration_ms,
                context.cancelled,
            )
            .await
            else {
                return Ok(());
            };
            self.audit_worker.submit(event).await?;
            Ok(())
        })
    }

    /// A script primitive becomes a child row of the script's dispatch, written directly
    /// (not through the worker) so its row id can key a per-step screenshot.
    fn observe_inner_call<'a>(&'a self, call: InnerCallContext<'a>) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let identity = call.identity;
            let child_dispatch_id = DispatchId::new();
            // Measure the inner primitive with the same v1 estimator granular
            // tools use, so a code-mode run's session is no longer tainted by a
            // v0 child and its token savings are projected.
            let tool_input_token_estimate = estimate_tool_input_tokens(call.tool_name, call.args);
            let input = RecordToolDispatchInput {
                agent_id: identity.session.convo_id().as_str().to_string(),
                slug: identity.agent.slug().to_string(),
                agent_label: identity.agent_label.clone(),
                session_id: call.call.session_id.as_str().to_string(),
                tool_name: call.tool_name.to_string(),
                page_id: call.page.map(i64::from),
                tab_id: call.live.map(|page| page.tab_id.0),
                target_id: call.live.map(|page| page.target_id.as_str().to_string()),
                url: call.live.map(|page| page.url.clone()),
                title: call.live.map(|page| page.title.clone()),
                args_json: bounded_args_json(call.args),
                result_meta: child_result_meta(call.is_error, call.from_helper),
                duration_ms: call.duration_ms,
                // None: child rows keep their completion time so they sort after
                // the parent script dispatch, which is stamped with its start.
                created_at: None,
                dispatch_id: child_dispatch_id.clone(),
                parent_dispatch_id: Some(call.call.dispatch_id.clone()),
                tool_input_token_estimate,
                tool_output_token_estimate: call.output_token_estimate,
                token_estimator_version: TOKEN_ESTIMATOR_VERSION,
            };
            match self.audit_log.record_tool_dispatch(input).await {
                Ok(row_id) => {
                    // Attach a per-step screenshot, keyed to this child row, so
                    // code-mode audits show each primitive like granular tools.
                    // Only for page-targeting primitives; page-less ones (windows,
                    // tab groups) have nothing meaningful to shoot.
                    if call.page.is_some() {
                        self.persist_screenshot(
                            call.call.session_id.as_str(),
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

    fn observe_local_tool<'a>(
        &'a self,
        dispatch: LocalToolContext<'a>,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            record_local_tool_dispatch(
                &self.audit_worker,
                LocalToolDispatch {
                    session: dispatch.session,
                    agent_label: dispatch.agent_label,
                    tool_name: dispatch.tool_name,
                    raw_args: dispatch.args,
                    result: dispatch.result,
                    duration_ms: dispatch.duration_ms,
                    dispatch_id: dispatch.dispatch_id,
                },
            )
            .await
        })
    }
}

pub struct LocalToolDispatch<'a> {
    pub session: &'a Arc<Session>,
    pub agent_label: &'a str,
    pub tool_name: &'a str,
    pub raw_args: &'a Value,
    pub result: &'a ToolResult,
    pub duration_ms: i64,
    pub dispatch_id: DispatchId,
}

pub async fn record_local_tool_dispatch(
    audit_worker: &AuditWorker,
    dispatch: LocalToolDispatch<'_>,
) -> anyhow::Result<()> {
    audit_worker
        .submit(AuditEvent::without_preview(RecordToolDispatchInput {
            agent_id: bounded_text(
                dispatch.session.convo_id().as_str(),
                AUDIT_IDENTITY_TEXT_MAX,
            ),
            slug: bounded_text(dispatch.session.agent().slug(), AUDIT_IDENTITY_TEXT_MAX),
            agent_label: bounded_text(dispatch.agent_label, AUDIT_IDENTITY_TEXT_MAX),
            session_id: dispatch.session.id().as_str().to_string(),
            tool_name: bounded_text(dispatch.tool_name, AUDIT_TOOL_NAME_MAX),
            page_id: None,
            tab_id: None,
            target_id: None,
            url: None,
            title: None,
            args_json: bounded_args_json(dispatch.raw_args),
            result_meta: tool_result_meta(dispatch.result, false),
            duration_ms: dispatch.duration_ms,
            created_at: None,
            dispatch_id: dispatch.dispatch_id,
            parent_dispatch_id: None,
            tool_input_token_estimate: estimate_tool_input_tokens(
                dispatch.tool_name,
                dispatch.raw_args,
            ),
            tool_output_token_estimate: estimate_tool_output_tokens(&dispatch.result.content),
            token_estimator_version: TOKEN_ESTIMATOR_VERSION,
        }))
        .await?;
    Ok(())
}

async fn build_event(
    observer: &AuditObserver,
    call: &ToolCall,
    result: &ToolResult,
    duration_ms: i64,
    cancelled: bool,
) -> Option<AuditEvent> {
    let Some(identity) = &call.identity else {
        warn!(
            tool = call.tool().name,
            session_id = %call.session_id,
            "cockpit dispatch missing identity"
        );
        return None;
    };
    let page_id = if call.flags.new_page {
        result_page_id(result)
    } else {
        extract_page_id(call)
    };
    let live = match (&call.browser_session, page_id) {
        (Some(browser), Some(page_id)) => browser.pages.get_info(PageId(page_id)).await,
        _ => None,
    }
    .or_else(|| call.page_snapshot.clone());
    Some(AuditEvent {
        input: RecordToolDispatchInput {
            agent_id: bounded_text(
                identity.session.convo_id().as_str(),
                AUDIT_IDENTITY_TEXT_MAX,
            ),
            slug: bounded_text(identity.agent.slug(), AUDIT_IDENTITY_TEXT_MAX),
            agent_label: bounded_text(&identity.agent_label, AUDIT_IDENTITY_TEXT_MAX),
            session_id: call.session_id.as_str().to_string(),
            tool_name: bounded_text(call.tool().name, AUDIT_TOOL_NAME_MAX),
            page_id: page_id.map(i64::from),
            tab_id: live.as_ref().map(|page| page.tab_id.0),
            target_id: live
                .as_ref()
                .map(|page| bounded_text(page.target_id.as_str(), AUDIT_TARGET_ID_MAX)),
            url: live
                .as_ref()
                .map(|page| bounded_text(&page.url, AUDIT_URL_MAX)),
            title: live
                .as_ref()
                .map(|page| bounded_text(&page.title, AUDIT_TITLE_MAX)),
            args_json: bounded_args_json(&call.raw_args),
            result_meta: tool_result_meta(result, cancelled),
            duration_ms,
            // Stamp the tool's start so a script dispatch sorts before the child
            // primitives it records while executing; top-level rows have no parent.
            created_at: Some(call.started_at_ms),
            dispatch_id: call.dispatch_id.clone(),
            parent_dispatch_id: None,
            tool_input_token_estimate: estimate_tool_input_tokens(call.tool().name, &call.raw_args),
            tool_output_token_estimate: estimate_tool_output_tokens(&result.content),
            token_estimator_version: TOKEN_ESTIMATOR_VERSION,
        },
        preview: Some(preview_callback(observer, &identity.session)),
    })
}

fn preview_callback(observer: &AuditObserver, session: &Arc<Session>) -> AuditPreview {
    let session = Arc::downgrade(session);
    let visuals = Arc::downgrade(&observer.visuals);
    let screenshots = Arc::downgrade(&observer.screenshots);
    Arc::new(move |session_id, row_id| {
        let session = session.clone();
        let visuals = visuals.clone();
        let screenshots = screenshots.clone();
        Box::pin(async move {
            let (Some(session), Some(visuals), Some(screenshots)) =
                (session.upgrade(), visuals.upgrade(), screenshots.upgrade())
            else {
                return Ok(false);
            };
            let Some(bytes) = visuals.capture_for_session(&session).await? else {
                return Ok(false);
            };
            screenshots.write(&session_id, row_id, &bytes).await?;
            Ok(true)
        })
    })
}

impl AuditObserver {
    /// Captures a screenshot of the session's active owned tab and attaches it to
    /// the given audit row. Called synchronously for each script primitive so a
    /// script's primitives get per-step screenshots like granular tools; failures
    /// are non-fatal.
    async fn persist_screenshot(&self, session_id: &str, dispatch_id: &DispatchId, row_id: i64) {
        let bytes = match self.visuals.capture(session_id, None).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return,
        Err(error) => {
            warn!(error = %error, dispatch_id = %dispatch_id, "audit screenshot capture failed");
            return;
        }
    };
    if let Err(error) = self.screenshots.write(session_id, row_id, &bytes).await {
        warn!(error = %error, dispatch_id = %dispatch_id, "session screenshot write failed");
        return;
    }
    if let Err(error) = self.audit_log.mark_screenshot(row_id).await {
        warn!(error = %error, dispatch_id = %dispatch_id, "audit screenshot marker failed");
    }
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

fn tool_result_meta(result: &ToolResult, cancelled: bool) -> String {
    match result.structured_content.as_ref() {
        Some(structured_content) => result_meta(
            cancelled || result.is_error,
            cancelled,
            structured_content,
            result.content.len(),
        ),
        None => result_meta(
            cancelled || result.is_error,
            cancelled,
            &Value::Null,
            result.content.len(),
        ),
    }
}

fn bounded_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes.saturating_sub(1);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}~", &value[..end])
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::audit_log::ListDispatchesQuery;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use browseros_mcp::token_estimate::{
        TOKEN_ESTIMATOR_VERSION, estimate_tool_input_tokens, estimate_tool_output_tokens,
    };
    use rmcp::model::ContentBlock;
    use serde_json::json;

    fn png_header(width: u32, height: u32) -> String {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        STANDARD.encode(bytes)
    }

    #[tokio::test]
    async fn records_ordinary_errors_and_cancellations() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let failed = ToolResult::error("failed");
        apply(ToolObserverContext {
            call: &call,
            result: &failed,
            cancelled: false,
            duration_ms: 4,
        })
        .await?;

        let cancelled = ToolResult {
            content: vec![ContentBlock::text("Operation cancelled by the User")],
            is_error: true,
            structured_content: Some(json!({
                "cancellationReason": "Operation cancelled by the User",
                "cancellationKind": "cockpit.operator-cancelled"
            })),
        };
        apply(ToolObserverContext {
            call: &call,
            result: &cancelled,
            cancelled: true,
            duration_ms: 5,
        })
        .await?;
        call.state
            .audit_worker
            .flush_session(call.session_id.as_str())
            .await?;

        let rows = call
            .state
            .audit_log
            .list_dispatches(ListDispatchesQuery::default())
            .await?
            .rows;
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|row| row.token_estimator_version == TOKEN_ESTIMATOR_VERSION)
        );
        assert!(rows.iter().all(|row| {
            row.tool_input_token_estimate
                == estimate_tool_input_tokens(call.tool().name, &call.raw_args)
        }));
        assert_eq!(
            rows[0].tool_output_token_estimate,
            estimate_tool_output_tokens(&cancelled.content)
        );
        assert_eq!(
            rows[1].tool_output_token_estimate,
            estimate_tool_output_tokens(&failed.content)
        );
        assert!(rows.iter().all(|row| {
            row.result_meta
                .as_deref()
                .is_some_and(|meta| meta.contains("\"isError\":true"))
        }));
        Ok(())
    }

    #[tokio::test]
    async fn local_tool_uses_the_shared_mixed_content_estimator() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let session = &call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .session;
        let raw_args = json!({ "name": "focus" });
        let result = ToolResult {
            content: vec![
                ContentBlock::text("abc"),
                ContentBlock::image(png_header(33, 65), "image/png"),
            ],
            is_error: false,
            structured_content: Some(json!({ "ignored": true })),
        };

        record_local_tool_dispatch(
            &call.state,
            LocalToolDispatch {
                session,
                agent_label: "Codex",
                tool_name: "name_session",
                raw_args: &raw_args,
                result: &result,
                duration_ms: 1,
                dispatch_id: DispatchId::new(),
            },
        )
        .await?;
        call.state
            .audit_worker
            .flush_session(call.session_id.as_str())
            .await?;

        let row = call
            .state
            .audit_log
            .list_dispatches(ListDispatchesQuery::default())
            .await?
            .rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("dispatch missing"))?;
        assert_eq!(row.token_estimator_version, TOKEN_ESTIMATOR_VERSION);
        assert_eq!(
            row.tool_input_token_estimate,
            estimate_tool_input_tokens("name_session", &raw_args)
        );
        assert_eq!(row.tool_output_token_estimate, 7);
        Ok(())
    }

    #[tokio::test]
    async fn event_bounds_variable_size_fields_before_submission() -> anyhow::Result<()> {
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "value": "x".repeat(9000) }))
                .await?;
        call.page_snapshot = Some(browseros_core::pages::PageInfo {
            page_id: PageId(7),
            tab_id: browseros_core::TabId(8),
            target_id: browseros_core::TargetId::from("t".repeat(9000)),
            url: "u".repeat(9000),
            title: "i".repeat(9000),
            is_active: true,
            is_loading: false,
            load_progress: 1.0,
            is_pinned: false,
            window_id: None,
            index: None,
            group_id: None,
            browser_profile_id: None,
            live_browser_context_id: None,
        });
        let result = ToolResult::text(
            "x".repeat(9000),
            Some(json!({
                "x".repeat(9000): "y"
            })),
        );
        let event = build_event(&call, &result, 1, false)
            .await
            .ok_or_else(|| anyhow::anyhow!("event missing"))?;

        assert!(event.input.args_json.len() <= 4096);
        assert!(event.input.result_meta.len() <= 4096);
        assert!(
            event
                .input
                .target_id
                .as_deref()
                .is_some_and(|value| value.len() <= 512)
        );
        assert!(
            event
                .input
                .url
                .as_deref()
                .is_some_and(|value| value.len() <= 4096)
        );
        assert!(
            event
                .input
                .title
                .as_deref()
                .is_some_and(|value| value.len() <= 1024)
        );
        Ok(())
    }
}
