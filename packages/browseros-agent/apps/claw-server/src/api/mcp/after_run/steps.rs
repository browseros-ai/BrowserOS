//! Projects SDK operations into redacted cockpit rows under the host run dispatch.
//! Only metadata enters persistence. The SDK owns execution and remains the
//! source of step timing and outcomes, including failed and cancelled operations.
//! The same bounded read supplies advisory labels for sent writes to foreign tabs.

use super::super::{service::StartedSession, tab_ref};
use crate::{
    AppState,
    db::audit_log::RecordToolDispatchInput,
    ids::DispatchId,
    services::{
        audit::AuditEvent,
        neo::{OperationOutcome, RunOperation, RunOutcome},
    },
};
use browseros_mcp::token_estimate::TOKEN_ESTIMATOR_VERSION;
use serde_json::json;
use std::collections::BTreeMap;

/// A run's reply notes and deferred child rows share one deadline. The caller
/// records the parent first, then publishes these children through the audit actor.
pub(in crate::api::mcp) struct PendingSteps {
    pub notes: Vec<String>,
    events: Vec<AuditEvent>,
    began: std::time::Instant,
    deadline: tokio::time::Instant,
}

impl PendingSteps {
    /// The flush acknowledges the parent's database write, including failures.
    /// FIFO admission alone would allow orphan children after a failed parent insert.
    pub(in crate::api::mcp) async fn record(self, state: &AppState, session_id: &str) {
        let publish = async {
            state.audit_worker.flush_session(session_id).await?;
            for event in self.events {
                state.audit_worker.submit(event).await?;
            }
            Ok::<_, crate::error::AppError>(())
        };
        match tokio::time::timeout_at(self.deadline, publish).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%error, "SDK child audit publication skipped"),
            Err(_) => tracing::warn!("SDK child audit publication timed out"),
        }
        tracing::debug!(
            elapsed_us = u64::try_from(self.began.elapsed().as_micros()).unwrap_or(u64::MAX),
            "SDK run steps publication completed"
        );
    }
}

pub(in crate::api::mcp) async fn prepare(
    state: &AppState,
    started: &StartedSession,
    outcome: &RunOutcome,
    operations: &[RunOperation],
    parent: &DispatchId,
) -> PendingSteps {
    let began = std::time::Instant::now();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    let cancel = started.session.child_token();
    let mut notes = Vec::new();
    // Bound projection and publication together. A congested audit queue or
    // session shutdown must not hold a completed SDK reply indefinitely.
    let projection = async {
        let mut events = Vec::new();
        // Tab claims have already reconciled the SDK's holders. Only execution
        // metadata selects affected tabs; the ownership lookup never gates access.
        notes = foreign_write_notice(state, started, operations)
            .await
            .into_iter()
            .collect();
        for operation in operations {
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
            events.push(event);
        }
        events
    };
    let events = tokio::select! {
        biased;
        () = cancel.cancelled() => Vec::new(),
        result = tokio::time::timeout_at(deadline, projection) => {
            result.unwrap_or_else(|_| {
                tracing::warn!("SDK child audit projection timed out");
                Vec::new()
            })
        }
    };
    tracing::debug!(
        run_id = outcome.run_id,
        elapsed_us = u64::try_from(began.elapsed().as_micros()).unwrap_or(u64::MAX),
        "SDK run steps projection completed"
    );
    PendingSteps {
        notes,
        events,
        began,
        deadline,
    }
}

/// Reuse the legacy script notice's wording, with native tab ids that Playwright
/// callers can resolve. Deduplication keeps repeated writes to a tab to one label.
async fn foreign_write_notice(
    state: &AppState,
    started: &StartedSession,
    operations: &[RunOperation],
) -> Option<String> {
    let mut foreign = BTreeMap::new();
    let mut refreshed = false;
    for operation in operations
        .iter()
        .filter(|operation| operation.is_sent_write())
    {
        let Some(tab) = operation.tab_id else {
            continue;
        };
        if foreign.contains_key(&tab) {
            continue;
        }
        let mut page = tab_ref::page_for_tab(state, tab).await;
        // SDK and host receive native events independently. Reconcile once for
        // a missing foreign tab too, rather than relying on own-tab claim refresh.
        if page.is_none() && !refreshed {
            refreshed = true;
            if let Some(browser) = state.browser.session().await {
                let _ = browser.pages.list().await;
                page = tab_ref::page_for_tab(state, tab).await;
            }
        }
        // This is a post-run projection. A tab already closed has no live host
        // PageId, so its ownership cannot be inferred from a recycled native id.
        let Some(page) = page else {
            continue;
        };
        let owner = state.sessions.owner_of_page(&page).await;
        if owner.as_ref() != Some(started.session.convo_id()) {
            foreign.insert(tab, owner);
        }
    }
    if foreign.is_empty() {
        return None;
    }
    let sessions = state.sessions.snapshot().await;
    let listed = foreign
        .into_iter()
        .map(|(tab, owner)| {
            let owner = owner.map_or_else(
                || "the user".to_owned(),
                |owner| {
                    let session = sessions.iter().find(|session| session.convo_id() == &owner);
                    let label = session.map_or_else(
                        || owner.as_str().to_owned(),
                        |session| session.agent().label().to_owned(),
                    );
                    // A reconnect changes the conversation id. Preserve the old
                    // notice's attribution to the caller's own earlier work.
                    let same_client = session.map_or_else(
                        || {
                            crate::identity::convo_id_belongs_to_slug(
                                &owner,
                                started.session.agent().slug(),
                            )
                        },
                        |session| session.agent().slug() == started.session.agent().slug(),
                    );
                    if same_client {
                        format!("your earlier session ({label})")
                    } else {
                        format!("another agent ({label})")
                    }
                },
            );
            format!("tab {tab} belongs to {owner}")
        })
        .collect::<Vec<_>>()
        .join("; ");
    Some(format!(
        "Note: this script used tabs that are not yours. {listed}. You are allowed to use them; leave them as you found them unless the user asked you to change them."
    ))
}
