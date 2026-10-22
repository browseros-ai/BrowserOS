//! The hosted API's read model: what a partner backend can know about a session, assembled
//! from the stores that record it and mapped into `hosted_api` models.

use crate::{
    AppState,
    db::{
        AuditLog,
        audit_log::{ListTasksQuery, TaskStatus, TaskSummary},
    },
    error::AppResult,
    identity::slugify_client_name,
    services::sessions::{Session as LiveSession, Sessions},
};
use std::{collections::HashMap, sync::Arc};

/// Reads session records for `/v1`. Built per request from the shared services.
pub(super) struct SessionRecords {
    audit_log: Arc<AuditLog>,
    sessions: Arc<Sessions>,
}

/// `GET /v1/sessions` filters. Every one is applied in the database query, before
/// pagination, so a page is never short.
pub(super) struct ListQuery {
    pub(super) client: Option<String>,
    pub(super) since: Option<i64>,
    pub(super) until: Option<i64>,
    pub(super) search: Option<String>,
    pub(super) cursor: Option<i64>,
    pub(super) limit: Option<i64>,
}

impl SessionRecords {
    pub(super) fn new(state: &AppState) -> Self {
        Self {
            audit_log: state.audit_log.clone(),
            sessions: state.sessions.clone(),
        }
    }

    pub(super) async fn list(&self, query: ListQuery) -> AppResult<hosted_api::SessionList> {
        let page = self
            .audit_log
            .list_tasks(ListTasksQuery {
                // Rows store the client's slug, so a filter matches however the client
                // spelled its name; an unsluggable name falls back the way identity does.
                slug: query
                    .client
                    .map(|client| slugify_client_name(&client).unwrap_or_else(|| "agent".into())),
                since: query.since,
                until: query.until,
                search: query.search,
                cursor: query.cursor,
                limit: query.limit,
                ..ListTasksQuery::default()
            })
            .await?;
        let live = self.live_sessions().await;
        let mut items = Vec::with_capacity(page.tasks.len());
        for task in &page.tasks {
            items.push(summary(task, live.get(task.session_id.as_str())).await);
        }
        let mut list = hosted_api::SessionList::new(items);
        list.next_cursor = page.next_cursor.map(|cursor| cursor.to_string());
        Ok(list)
    }

    async fn live_sessions(&self) -> HashMap<String, Arc<LiveSession>> {
        self.sessions
            .snapshot()
            .await
            .into_iter()
            .map(|session| (session.id().as_str().to_string(), session))
            .collect()
    }
}

/// A session's `/v1` summary. `live` is its in-memory MCP session, present while the
/// session is open; the stored row alone cannot tell an open session from one whose
/// process died, so liveness always comes from memory.
async fn summary(task: &TaskSummary, live: Option<&Arc<LiveSession>>) -> hosted_api::SessionSummary {
    let name = match live {
        Some(session) => session.label().await,
        None => task.title.clone(),
    };
    let mut summary = hosted_api::SessionSummary::new(
        task.session_id.clone(),
        name,
        task.agent_label.clone(),
        status(task.status, live.is_some()).to_string(),
        task.started_at,
        task.dispatch_count,
    );
    summary.summary = task.task_summary.clone();
    summary.site = task.site.clone();
    summary.ended_at = ended_at(task, live.is_some());
    summary
}

/// Matches the cockpit's reading: a row still marked live whose MCP session is gone ended
/// without an end event (the server restarted), so it reads as done.
fn status(stored: TaskStatus, live: bool) -> &'static str {
    match stored {
        TaskStatus::Live if live => "live",
        TaskStatus::Live | TaskStatus::Done => "done",
        TaskStatus::Failed => "failed",
        TaskStatus::Cancelled => "cancelled",
    }
}

/// The stored end, or for an ended session without one, its last recorded activity.
fn ended_at(task: &TaskSummary, live: bool) -> Option<i64> {
    if live {
        return None;
    }
    Some(
        task.ended_at
            .unwrap_or_else(|| task.started_at.saturating_add(task.duration_ms.max(0))),
    )
}
