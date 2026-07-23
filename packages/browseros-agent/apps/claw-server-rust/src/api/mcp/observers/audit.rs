use crate::{
    AppState,
    api::mcp::dispatch::{
        ToolCall, ToolObserver, ToolObserverContext, extract_page_id, result_page_id,
    },
    db::audit_log::{RecordToolDispatchInput, bounded_args_json, result_meta},
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
            args_json: bounded_args_json(dispatch.raw_args),
            result_meta: tool_result_meta(dispatch.result, false),
            duration_ms: dispatch.duration_ms,
            dispatch_id: dispatch.dispatch_id,
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
            dispatch_id: call.dispatch_id.clone(),
            tool_input_token_estimate: estimate_tool_input_tokens(call.tool().name, &call.raw_args),
            tool_output_token_estimate: estimate_tool_output_tokens(&result.content),
            token_estimator_version: TOKEN_ESTIMATOR_VERSION,
        },
        preview: Some(preview_callback(call, &identity.session)),
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
            is_hidden: false,
            window_id: None,
            index: None,
            group_id: None,
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
