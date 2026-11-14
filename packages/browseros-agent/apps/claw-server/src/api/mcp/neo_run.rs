//! Desktop Playwright runs bypass the old script dispatcher. The SDK owns execution;
//! this boundary owns session correlation, cancellation and a redacted host audit row.

use super::{
    after_run, before_run, dispatch::linked_cancel_token, observers::audit::preview_callback,
    service::StartedSession,
};
use crate::{
    AppState,
    db::audit_log::RecordToolDispatchInput,
    ids::DispatchId,
    services::{
        audit::{AuditEvent, AuditPreview},
        neo::{RunCall, RunRecordRead, RunStatus},
        sessions::Session,
    },
};
use browseros_mcp::token_estimate::{
    TOKEN_ESTIMATOR_VERSION, estimate_tool_input_tokens, estimate_tool_output_tokens,
};
use rmcp::model::{CallToolResult, ContentBlock, JsonObject};
use serde_json::{Value, json};
use std::{sync::Arc, time::Instant};
use tokio_util::sync::CancellationToken;

/// Prepare the window, forward, reconcile tabs/naming and prepare reply notes.
/// Persist the parent before publishing children so every visible child has its run.
/// Dropping the engine future closes its HTTP connection so the SDK cancels the run.
pub(super) async fn call(
    state: &AppState,
    started: &StartedSession,
    args: JsonObject,
    ct: CancellationToken,
) -> CallToolResult {
    let dispatch_id = DispatchId::new();
    let dispatch_cancel = CancellationToken::new();
    let cancel = linked_cancel_token(started.session.child_token(), ct, dispatch_cancel.clone());
    let _cancel_on_drop = cancel.clone().drop_guard();
    if !started
        .session
        .try_register_dispatch(dispatch_id.clone(), dispatch_cancel)
        .await
    {
        return CallToolResult::error(vec![ContentBlock::text(
            "BrowserOS neo session is no longer live",
        )]);
    }
    let began = Instant::now();
    let created_at = crate::clock::now_epoch_ms();
    let input_tokens = estimate_tool_input_tokens("run", &Value::Object(args.clone()));
    let call = RunCall {
        args,
        task: Some(state.neo.current_task(started.session.convo_id())),
        host_data: json!({
            "session": started.session.id().as_str(),
            "client": started.session.client_name(),
        }),
    };
    let mut window_note = None;
    let outcome = tokio::select! {
        biased;
        () = cancel.cancelled() => None,
        outcome = async {
            window_note = before_run::ensure_window(state).await;
            state.neo.run(call).await
        } => Some(outcome),
    };
    let cancelled = match &outcome {
        None => true,
        Some(Ok(outcome)) => outcome.status == RunStatus::Cancelled,
        Some(Err(_)) => false,
    };
    let mut pending_steps = None;
    let mut sdk_run_id = None;
    let mut record = RunRecordRead::unread();
    let mut result = match outcome {
        Some(Ok(outcome)) => {
            state
                .neo
                .observe_tasks(started.session.convo_id(), &outcome.tasks);
            // Ownership and audit projection share one bounded SDK snapshot.
            // Resolve it before claim admission so a created tab's checkpoint
            // remains inside its ownership window without reading page contents.
            record = tokio::select! {
                biased;
                () = cancel.cancelled() => RunRecordRead::unread(),
                result = tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    state.neo.operations(&outcome.run_id),
                ) => result.unwrap_or_else(|_| {
                    tracing::warn!("SDK run metadata read timed out");
                    RunRecordRead::unread()
                }),
            };
            let operations = &record.operations;
            let mut notes = after_run::tabs::apply(state, started, operations, created_at).await;
            notes.extend(after_run::naming::apply(state, started, &outcome).await);
            let mut steps =
                after_run::steps::prepare(state, started, &outcome, operations, &dispatch_id).await;
            notes.append(&mut steps.notes);
            pending_steps = Some(steps);
            sdk_run_id = Some(outcome.run_id.clone());
            let mut reply = outcome.reply;
            reply
                .content
                .extend(notes.into_iter().map(ContentBlock::text));
            reply
        }
        Some(Err(error)) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
        None => CallToolResult::error(vec![ContentBlock::text("run was cancelled")]),
    };
    result.content.extend(window_note.map(ContentBlock::text));
    let duration_ms = i64::try_from(began.elapsed().as_millis()).unwrap_or(i64::MAX);
    // A script and its return value can both contain secrets. Keep the audit row's
    // input limited to outcome flags, opaque run correlation and token counts.
    let event = AuditEvent {
        input: RecordToolDispatchInput {
            agent_id: started.session.convo_id().as_str().to_string(),
            slug: started.session.agent().slug().to_string(),
            agent_label: started.agent_label.clone(),
            session_id: started.session.id().as_str().to_string(),
            tool_name: "run".to_string(),
            page_id: None,
            tab_id: None,
            target_id: None,
            url: None,
            title: None,
            args_json: json!({"code": "[redacted]"}).to_string(),
            result_meta: json!({
                "isError": result.is_error.unwrap_or(false),
                "cancelled": cancelled,
                "sdkRunId": sdk_run_id,
                "contentSummary": format!("{} block(s)", result.content.len()),
            })
            .to_string(),
            duration_ms,
            created_at: Some(created_at),
            dispatch_id: dispatch_id.clone(),
            parent_dispatch_id: None,
            tool_input_token_estimate: input_tokens,
            tool_output_token_estimate: estimate_tool_output_tokens(&result.content),
            token_estimator_version: TOKEN_ESTIMATOR_VERSION,
        },
        preview: run_preview(state, &started.session, cancelled, &record),
    };
    match state.audit_worker.submit(event).await {
        Ok(()) => {
            if let Some(steps) = pending_steps {
                steps.record(state, started.session.id().as_str()).await;
            }
        }
        Err(error) => tracing::warn!(%error, "Playwright run audit admission failed"),
    }
    if !started.session.finish_dispatch(&dispatch_id).await
        && started.session.operator_stop_requested()
    {
        result = CallToolResult::error(vec![ContentBlock::text("Operation cancelled by the User")]);
    }
    result
}

/// Keep image capture deferred until the parent row is durable. Unknown or
/// incomplete metadata cannot prove the absence of raw CDP required for a preview.
fn run_preview(
    state: &AppState,
    session: &Arc<Session>,
    cancelled: bool,
    record: &RunRecordRead,
) -> Option<AuditPreview> {
    (!cancelled && record.proves_no_raw_cdp()).then(|| preview_callback(state, session))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::mcp::test_support::tool_call;

    #[tokio::test]
    async fn parent_run_preview_requires_uncancelled_complete_cdp_free_metadata()
    -> anyhow::Result<()> {
        let call = tool_call("run", json!({ "code": "return 1" })).await?;
        let session = &call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("test session missing"))?
            .session;
        let operation = |name: &str| {
            json!({
                "name": name,
                "startedAt": 123,
                "effect": "write",
                "delivery": "done",
                "outcome": "ok"
            })
        };
        let plain = operation("locator.fill");
        let cases = [
            (
                "settled plain run",
                false,
                json!({"complete": true, "operations": [plain.clone()]}),
                true,
            ),
            (
                "cancelled run",
                true,
                json!({"complete": true, "operations": [plain.clone()]}),
                false,
            ),
            (
                "partial record",
                false,
                json!({"complete": false, "operations": [plain.clone()]}),
                false,
            ),
            (
                "missing completeness",
                false,
                json!({"operations": [plain.clone()]}),
                false,
            ),
            (
                "trace gap",
                false,
                json!({"complete": true, "traceGaps": 1, "operations": [plain.clone()]}),
                false,
            ),
            (
                "zero operations",
                false,
                json!({"complete": true, "operations": []}),
                false,
            ),
            (
                "raw CDP",
                false,
                json!({"complete": true, "operations": [plain, operation("neo.cdp")]}),
                false,
            ),
        ];
        for (label, cancelled, value, expected) in cases {
            let record = serde_json::from_value(value)?;
            assert_eq!(
                run_preview(&call.state, session, cancelled, &record).is_some(),
                expected,
                "{label}"
            );
        }
        assert!(
            run_preview(&call.state, session, false, &RunRecordRead::unread()).is_none(),
            "unread metadata cannot permit a parent preview"
        );
        Ok(())
    }
}
