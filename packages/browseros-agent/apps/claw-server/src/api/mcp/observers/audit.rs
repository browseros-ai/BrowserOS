use crate::{
    AppState,
    api::mcp::dispatch::{
        ToolCall, ToolObserver, ToolObserverContext, extract_page_id, result_page_id,
    },
    db::audit_log::{RecordToolDispatchInput, redacted_args_json, result_meta},
    ids::DispatchId,
    services::{
        audit::{AuditEvent, AuditPreview},
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
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::sync::Arc;
use tracing::warn;

const AUDIT_IDENTITY_TEXT_MAX: usize = 512;
const AUDIT_TOOL_NAME_MAX: usize = 128;
const AUDIT_TARGET_ID_MAX: usize = 512;
const AUDIT_URL_MAX: usize = 4096;
const AUDIT_TITLE_MAX: usize = 1024;

/** Converts the final tool outcome into a bounded event and admits it to the audit worker. */
pub fn apply(context: ToolObserverContext<'_>) -> BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let Some(event) = build_event(
            context.call,
            context.result,
            context.duration_ms,
            context.cancelled,
        )
        .await
        else {
            return Ok(());
        };
        context.call.state.audit_worker.submit(event).await?;
        Ok(())
    })
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
    state: &AppState,
    dispatch: LocalToolDispatch<'_>,
) -> anyhow::Result<()> {
    state
        .audit_worker
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
            args_json: redacted_args_json(dispatch.raw_args, dispatch.result.audit_hints.as_ref()),
            result_meta: tool_result_meta(dispatch.result, false),
            duration_ms: dispatch.duration_ms,
            // Recorded right after the tool returns, so back-date by its duration: every
            // top-level row is stamped with its start, which readers order calls by.
            created_at: Some(crate::clock::now_epoch_ms().saturating_sub(dispatch.duration_ms)),
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
    // A sensitive dispatch skips its screenshot: a password or payment form would
    // otherwise sit on disk as an image beside the redacted args.
    let sensitive_script = call
        .sensitive_credentials
        .load(std::sync::atomic::Ordering::Relaxed);
    let script_hints = browseros_mcp::AuditHints {
        redact_all_values: true,
        skip_screenshot: true,
        ..Default::default()
    };
    // Source can embed credentials even when admission, syntax checking or
    // cancellation prevents the bridge from discovering that call. Keep child
    // step metadata, but never persist raw run source in the audit ledger.
    let audit_hints = if sensitive_script || call.tool().name == "run" {
        Some(&script_hints)
    } else {
        result.audit_hints.as_ref()
    };
    let skip_screenshot = sensitive_script
        || call.tool().name == "credentials"
        || result
            .audit_hints
            .as_ref()
            .is_some_and(|hints| hints.skip_screenshot);
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
            args_json: redacted_args_json(&call.raw_args, audit_hints),
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
        preview: if skip_screenshot {
            None
        } else {
            Some(preview_callback(call, &identity.session))
        },
    })
}

fn preview_callback(call: &ToolCall, session: &Arc<Session>) -> AuditPreview {
    let session = Arc::downgrade(session);
    let visuals = Arc::downgrade(&call.state.visuals);
    let screenshots = Arc::downgrade(&call.state.screenshots);
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

/// Captures a screenshot of the session's active owned tab and attaches it to
/// the given audit row. Called synchronously by the code-mode script hook so a
/// script's primitives get per-step screenshots like granular tools; failures
/// are non-fatal.
pub(crate) async fn persist_screenshot(
    state: &AppState,
    session_id: &str,
    dispatch_id: &DispatchId,
    row_id: i64,
) {
    let bytes = match state.visuals.capture(session_id, None).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return,
        Err(error) => {
            warn!(error = %error, dispatch_id = %dispatch_id, "audit screenshot capture failed");
            return;
        }
    };
    if let Err(error) = state.screenshots.write(session_id, row_id, &bytes).await {
        warn!(error = %error, dispatch_id = %dispatch_id, "session screenshot write failed");
        return;
    }
    if let Err(error) = state.audit_log.mark_screenshot(row_id).await {
        warn!(error = %error, dispatch_id = %dispatch_id, "audit screenshot marker failed");
    }
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

const _: ToolObserver = apply;

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
            audit_hints: None,
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
            audit_hints: None,
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
    async fn local_tool_rows_are_stamped_with_their_start() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let session = &call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .session;
        let result = ToolResult {
            content: vec![ContentBlock::text("done")],
            is_error: false,
            structured_content: None,
            audit_hints: None,
        };
        let finished_at = crate::clock::now_epoch_ms();

        record_local_tool_dispatch(
            &call.state,
            LocalToolDispatch {
                session,
                agent_label: "Codex",
                tool_name: "pursue",
                raw_args: &json!({}),
                result: &result,
                duration_ms: 5_000,
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
        // Recorded right after it finished, so its start is about five seconds back.
        assert!(row.created_at <= finished_at - 4_000, "{}", row.created_at);
        assert!(row.created_at >= finished_at - 6_000, "{}", row.created_at);
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

    #[tokio::test]
    async fn sensitive_dispatch_redacts_args_and_skips_screenshot() -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call(
            "act",
            json!({ "page": 1, "kind": "fill", "ref": "e1", "value": "hunter2" }),
        )
        .await?;
        let result = ToolResult {
            content: vec![ContentBlock::text("ok (fill)")],
            is_error: false,
            structured_content: Some(json!({ "kind": "fill" })),
            audit_hints: Some(browseros_mcp::AuditHints {
                sensitive_refs: vec!["e1".to_string()],
                redact_all_values: false,
                skip_screenshot: true,
            }),
        };

        let event = build_event(&call, &result, 1, false)
            .await
            .ok_or_else(|| anyhow::anyhow!("event missing"))?;

        assert!(event.input.args_json.contains("[redacted]"));
        assert!(!event.input.args_json.contains("hunter2"));
        // A sensitive dispatch is never screenshotted.
        assert!(event.preview.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn credential_early_failure_redacts_password_without_runtime_hints() -> anyhow::Result<()>
    {
        let call = crate::api::mcp::test_support::tool_call(
            "credentials",
            json!({
                "action":"fill", "page":1, "username":"synthetic-user",
                "password":{"unexpected":"synthetic-credential-secret"}
            }),
        )
        .await?;
        let result = ToolResult::error("unsupported native browser");
        let event = build_event(&call, &result, 1, true)
            .await
            .ok_or_else(|| anyhow::anyhow!("event missing"))?;
        assert!(event.input.args_json.contains("synthetic-user"));
        assert!(event.input.args_json.contains("[redacted]"));
        assert!(
            !event
                .input
                .args_json
                .contains("synthetic-credential-secret")
        );
        assert!(event.preview.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_credential_script_redacts_parent_source_and_suppresses_preview()
    -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call(
            "run",
            json!({
                "code":"await browser.credentials({action:'add',password:'synthetic-run-secret'});"
            }),
        )
        .await?;
        call.sensitive_credentials
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let result = ToolResult::error("cancelled");
        let event = build_event(&call, &result, 1, true)
            .await
            .ok_or_else(|| anyhow::anyhow!("missing event"))?;
        assert!(event.input.args_json.contains("[redacted]"));
        assert!(!event.input.args_json.contains("synthetic-run-secret"));
        assert!(event.preview.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn credential_script_is_sensitive_even_before_the_bridge_runs() -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call("run", json!({
            "code":"await sleep(99999); await browser.credentials({action:'add',password:'synthetic-unreached-secret'});"
        })).await?;
        let result = ToolResult::error("cancelled before execution");
        let event = build_event(&call, &result, 1, true)
            .await
            .ok_or_else(|| anyhow::anyhow!("missing event"))?;
        assert!(!event.input.args_json.contains("synthetic-unreached-secret"));
        assert!(event.preview.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn ordinary_dispatch_keeps_its_value_and_screenshot() -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call(
            "act",
            json!({ "page": 1, "kind": "fill", "ref": "e1", "value": "london" }),
        )
        .await?;
        let result = ToolResult {
            content: vec![ContentBlock::text("ok (fill)")],
            is_error: false,
            structured_content: Some(json!({ "kind": "fill" })),
            audit_hints: None,
        };

        let event = build_event(&call, &result, 1, false)
            .await
            .ok_or_else(|| anyhow::anyhow!("event missing"))?;

        // No hints: the ledger is unchanged and the screenshot is still attached.
        assert!(event.input.args_json.contains("london"));
        assert!(event.preview.is_some());
        Ok(())
    }
}
