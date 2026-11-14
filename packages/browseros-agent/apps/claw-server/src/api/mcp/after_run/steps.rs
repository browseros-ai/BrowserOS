//! Projects SDK operations into redacted cockpit rows under the host run dispatch.
//! Only metadata enters persistence. The SDK owns execution and remains the
//! source of step timing and outcomes, including failed and cancelled operations.

use super::super::{service::StartedSession, tab_ref};
use crate::{
    AppState,
    db::audit_log::RecordToolDispatchInput,
    ids::DispatchId,
    services::{
        audit::AuditEvent,
        neo::{OperationOutcome, RunOutcome},
    },
};
use browseros_mcp::token_estimate::TOKEN_ESTIMATOR_VERSION;
use serde_json::json;

pub(in crate::api::mcp) async fn apply(
    state: &AppState,
    started: &StartedSession,
    outcome: &RunOutcome,
    parent: &DispatchId,
) {
    let began = std::time::Instant::now();
    let cancel = started.session.child_token();
    // Bound the entire projection as well as each HTTP read. A congested audit
    // queue or session shutdown must not hold a completed SDK reply indefinitely.
    let projection = async {
        for operation in state.neo.operations(&outcome.run_id).await {
            let page_id = match operation.tab_id {
                Some(tab) => tab_ref::page_for_tab(state, tab)
                    .await
                    .map(|page| i64::from(page.0)),
                None => None,
            };
            let event = AuditEvent::without_preview(RecordToolDispatchInput {
                agent_id: started.session.convo_id().as_str().to_owned(),
                slug: started.session.agent().slug().to_owned(),
                agent_label: started.agent_label.clone(),
                session_id: started.session.id().as_str().to_owned(),
                tool_name: operation.name.audit_name().to_owned(),
                page_id,
                tab_id: operation.tab_id,
                target_id: None,
                url: None,
                title: None,
                // The cockpit recognizes positional args as a script child and
                // hides its parent run. Preserve that shape with no real values.
                args_json: json!(["[redacted]"]).to_string(),
                result_meta: json!({
                    "isError": matches!(operation.outcome, Some(OperationOutcome::Error)),
                    "cancelled": matches!(operation.outcome, Some(OperationOutcome::Cancelled)),
                })
                .to_string(),
                duration_ms: operation.duration_ms.unwrap_or(0).max(0),
                created_at: Some(operation.started_at),
                dispatch_id: DispatchId::new(),
                parent_dispatch_id: Some(parent.clone()),
                // Child rows describe internal work, not extra MCP traffic.
                tool_input_token_estimate: 0,
                tool_output_token_estimate: 0,
                token_estimator_version: TOKEN_ESTIMATOR_VERSION,
            });
            if let Err(error) = state.audit_worker.submit(event).await {
                tracing::warn!(%error, "SDK child audit admission failed");
                break;
            }
        }
    };
    tokio::select! {
        biased;
        () = cancel.cancelled() => {},
        result = tokio::time::timeout(std::time::Duration::from_secs(3), projection) => {
            if result.is_err() {
                tracing::warn!("SDK child audit projection timed out");
            }
        }
    }
    tracing::debug!(
        run_id = outcome.run_id,
        elapsed_us = u64::try_from(began.elapsed().as_micros()).unwrap_or(u64::MAX),
        "SDK run steps projection completed"
    );
}
