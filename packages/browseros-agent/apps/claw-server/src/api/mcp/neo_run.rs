//! Desktop Playwright runs bypass the old script dispatcher. The SDK owns execution;
//! this boundary owns session correlation, cancellation and a redacted host audit row.

use super::{after_run, before_run, dispatch::linked_cancel_token, service::StartedSession};
use crate::{
    AppState,
    db::audit_log::RecordToolDispatchInput,
    ids::DispatchId,
    services::{
        audit::AuditEvent,
        neo::{RunCall, RunStatus},
    },
};
use browseros_mcp::token_estimate::{
    TOKEN_ESTIMATOR_VERSION, estimate_tool_input_tokens, estimate_tool_output_tokens,
};
use rmcp::model::{CallToolResult, ContentBlock, JsonObject};
use serde_json::{Value, json};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

/// Prepare the window, forward, then reconcile tabs, naming and audit, in that order.
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
    let mut result = match outcome {
        Some(Ok(outcome)) => {
            state
                .neo
                .observe_tasks(started.session.convo_id(), &outcome.tasks);
            let mut notes = after_run::tabs::apply(state, started, &outcome).await;
            notes.extend(after_run::naming::apply(state, started, &outcome).await);
            let mut steps = after_run::steps::prepare(state, started, &outcome, &dispatch_id).await;
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
    // A script and its return value can both contain secrets. Persist only fixed
    // outcome flags and token counts, even for invalid arguments and engine errors.
    let event = AuditEvent::without_preview(RecordToolDispatchInput {
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
    });
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
