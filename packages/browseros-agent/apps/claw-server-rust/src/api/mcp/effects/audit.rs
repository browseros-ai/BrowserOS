use crate::{
    AppState,
    api::mcp::dispatch::{
        ToolCall, ToolEffect, ToolEffectContext, extract_page_id, result_page_id,
    },
    db::audit_log::{DispatchResultSummary, RecordToolDispatchInput},
    ids::{DispatchId, SessionId},
    services::sessions::Session,
};
use browseros_core::PageId;
use browseros_mcp::{
    ToolResult,
    token_estimate::{
        TOKEN_ESTIMATOR_VERSION, estimate_tool_input_tokens, estimate_tool_output_tokens,
    },
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tracing::warn;

#[derive(Debug, Clone, Copy)]
struct AuditRecord {
    row_id: i64,
}

/** Persists every executed dispatch, then best-effort captures its session without changing the tool result. */
pub fn apply(context: ToolEffectContext<'_>) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
    Box::pin(async move {
        let Some(identity) = &context.call.identity else {
            warn!(
                tool = context.call.tool().name,
                session_id = %context.call.session_id,
                "cockpit dispatch missing identity"
            );
            return Ok(None);
        };
        let Some(record) = record_dispatch(
            context.call,
            context.result,
            context.duration_ms,
            context.cancelled,
            identity,
        )
        .await
        else {
            return Ok(None);
        };
        persist_screenshot(
            &context.call.state,
            &context.call.session_id,
            &context.call.dispatch_id,
            record,
        )
        .await;
        Ok(None)
    })
}

/// Records a Claw-local tool without capturing an unrelated browser page.
pub async fn record_local_tool_dispatch(
    state: &AppState,
    session: &Session,
    agent_label: &str,
    tool_name: &str,
    raw_args: &Value,
    result: &ToolResult,
    duration_ms: i64,
) {
    let dispatch_id = DispatchId::new();
    let input = RecordToolDispatchInput {
        agent_id: session.convo_id().as_str().to_string(),
        slug: session.agent().slug().to_string(),
        agent_label: agent_label.to_string(),
        session_id: session.id().as_str().to_string(),
        tool_name: tool_name.to_string(),
        page_id: None,
        tab_id: None,
        target_id: None,
        url: None,
        title: None,
        raw_args: raw_args.clone(),
        duration_ms,
        dispatch_id: dispatch_id.clone(),
        tool_input_token_estimate: estimate_tool_input_tokens(tool_name, raw_args),
        tool_output_token_estimate: estimate_tool_output_tokens(&result.content),
        token_estimator_version: TOKEN_ESTIMATOR_VERSION,
        result: result_summary(result, false),
    };
    let _ = write_dispatch(state, input, &dispatch_id).await;
}

async fn record_dispatch(
    call: &ToolCall,
    result: &ToolResult,
    duration_ms: i64,
    cancelled: bool,
    identity: &crate::api::mcp::dispatch::ToolIdentity,
) -> Option<AuditRecord> {
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
    write_dispatch(
        &call.state,
        RecordToolDispatchInput {
            agent_id: identity.session.convo_id().as_str().to_string(),
            slug: identity.agent.slug().to_string(),
            agent_label: identity.agent_label.clone(),
            session_id: call.session_id.as_str().to_string(),
            tool_name: call.tool().name.to_string(),
            page_id: page_id.map(i64::from),
            tab_id: live.as_ref().map(|page| page.tab_id.0),
            target_id: live
                .as_ref()
                .map(|page| page.target_id.as_str().to_string()),
            url: live.as_ref().map(|page| page.url.clone()),
            title: live.as_ref().map(|page| page.title.clone()),
            raw_args: call.raw_args.clone(),
            duration_ms,
            dispatch_id: call.dispatch_id.clone(),
            tool_input_token_estimate: estimate_tool_input_tokens(call.tool().name, &call.raw_args),
            tool_output_token_estimate: estimate_tool_output_tokens(&result.content),
            token_estimator_version: TOKEN_ESTIMATOR_VERSION,
            result: result_summary(result, cancelled),
        },
        &call.dispatch_id,
    )
    .await
}

async fn write_dispatch(
    state: &AppState,
    input: RecordToolDispatchInput,
    dispatch_id: &DispatchId,
) -> Option<AuditRecord> {
    match state.audit_log.record_tool_dispatch(input).await {
        Ok(row_id) => Some(AuditRecord { row_id }),
        Err(error) => {
            warn!(
                error = %error,
                dispatch_id = %dispatch_id,
                "audit writer failed"
            );
            None
        }
    }
}

fn result_summary(result: &ToolResult, cancelled: bool) -> DispatchResultSummary {
    let content = serde_json::to_value(&result.content).unwrap_or_else(|error| {
        warn!(error = %error, "tool content serialization failed");
        json!([])
    });
    DispatchResultSummary {
        is_error: cancelled || result.is_error,
        cancelled,
        structured_content: result.structured_content.clone().unwrap_or(Value::Null),
        content,
    }
}

async fn persist_screenshot(
    state: &AppState,
    session_id: &SessionId,
    dispatch_id: &DispatchId,
    record: AuditRecord,
) {
    let bytes = match state.visuals.capture(session_id.as_str()).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return,
        Err(error) => {
            warn!(error = %error, dispatch_id = %dispatch_id, "audit screenshot capture failed");
            return;
        }
    };
    if let Err(error) = state
        .screenshots
        .write(session_id.as_str(), record.row_id, &bytes)
        .await
    {
        warn!(error = %error, dispatch_id = %dispatch_id, "session screenshot write failed");
        return;
    }
    if let Err(error) = state.audit_log.mark_screenshot(record.row_id).await {
        warn!(error = %error, dispatch_id = %dispatch_id, "audit screenshot marker failed");
    }
}

const _: ToolEffect = apply;

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
        apply(ToolEffectContext {
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
        apply(ToolEffectContext {
            call: &call,
            result: &cancelled,
            cancelled: true,
            duration_ms: 5,
        })
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
    async fn capture_absence_is_non_fatal_and_keeps_the_dispatch() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let result = ToolResult::text("ok", Some(json!({ "pages": [] })));
        assert!(
            apply(ToolEffectContext {
                call: &call,
                result: &result,
                cancelled: false,
                duration_ms: 1,
            })
            .await?
            .is_none()
        );
        let rows = call
            .state
            .audit_log
            .list_dispatches(ListDispatchesQuery::default())
            .await?
            .rows;
        assert_eq!(rows.len(), 1);
        let row = rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("dispatch missing"))?;
        assert_eq!(row.token_estimator_version, TOKEN_ESTIMATOR_VERSION);
        assert_eq!(row.tool_output_token_estimate, 1);
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
            session,
            "Codex",
            "name_session",
            &raw_args,
            &result,
            1,
        )
        .await;

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
    async fn dispatch_without_identity_writes_no_row() -> anyhow::Result<()> {
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        call.identity = None;
        let result = ToolResult::text("ok", Some(json!({ "pages": [] })));
        apply(ToolEffectContext {
            call: &call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await?;
        assert!(
            call.state
                .audit_log
                .list_dispatches(ListDispatchesQuery::default())
                .await?
                .rows
                .is_empty()
        );
        Ok(())
    }
}
