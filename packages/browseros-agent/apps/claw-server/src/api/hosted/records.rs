//! The hosted API's read model: what a partner backend can know about a session, assembled
//! from the stores that record it and mapped into `hosted_api` models.
//!
//! A session's record keeps filling in after its tool calls return, from three writers that
//! run on their own schedules:
//!
//! ```text
//! tool call returns ─▶ AuditWorker queue ─▶ tool_dispatches / tasks rows          (ms)
//!                           └─▶ screenshot capture ─▶ screenshots/<session>/<id>.jpg (background)
//! claw-app recorder ─▶ POST /api/v1/recordings/events ─▶ recording store          (every 2.5 s)
//! ```
//!
//! `complete` is the promise built on top of them: once a read says it, nothing more will be
//! written for the session, so a partner can archive what it has.

use crate::{
    AppState,
    clock::now_epoch_ms,
    db::{
        AuditLog,
        audit_log::{
            ListTasksQuery, SessionLifecycle, TaskDetail, TaskStatus, TaskSummary, ToolDispatchRow,
        },
    },
    error::{AppError, AppResult},
    ids::SessionId,
    services::{
        audit::AuditWorker,
        replay::{ReplayEvent, ReplayService},
        screenshots::ScreenshotService,
        sessions::Sessions,
    },
};
use axum::http::StatusCode;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

/// How long after a session ends its recording may still be arriving. claw-app's recorder
/// uploads buffered rrweb events every 2.5 s and retries a failed upload after 5 s
/// (`apps/claw-app/modules/recorder`), and the server gets no signal that the last batch is
/// in, so 10 s covers one retry. Lowering it below the recorder's cadence would let
/// `complete` report a recording that is still missing its tail. The recorder retries without
/// a limit, so a batch that keeps failing can land later still; the spec says so. Closing that
/// gap needs the recorder to report what it has delivered, not a longer wait.
const RECORDING_SETTLE_MS: i64 = 10_000;

/// How often `wait` re-reads a session that is not complete yet.
const WAIT_POLL: Duration = Duration::from_millis(500);

/// The least time a read spends waiting for an ended session's queued audit work (rows and
/// screenshot captures) before reporting it incomplete. Without a floor a request with no
/// `wait` could never see a session as complete; within one, a request with a `wait` stays
/// close to its deadline.
const FLUSH_FLOOR: Duration = Duration::from_secs(1);

/// Reads session records for `/v1`. Built per request from the shared services.
pub(super) struct SessionRecords {
    audit_log: Arc<AuditLog>,
    audit_worker: Arc<AuditWorker>,
    sessions: Arc<Sessions>,
    screenshots: Arc<ScreenshotService>,
    replay: Arc<ReplayService>,
}

/// `GET /v1/sessions/{id}` options.
pub(super) struct GetOptions {
    /// Inline each tool call's screenshot (base64 JPEG).
    pub(super) screenshots: bool,
    /// Hold the request this long for the session to become complete.
    pub(super) wait: Duration,
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
            audit_worker: state.audit_worker.clone(),
            sessions: state.sessions.clone(),
            screenshots: state.screenshots.clone(),
            replay: state.replay.clone(),
        }
    }

    pub(super) async fn list(&self, query: ListQuery) -> AppResult<hosted_api::SessionList> {
        let page = self
            .audit_log
            .list_tasks(ListTasksQuery {
                client_name: query.client,
                since: query.since,
                until: query.until,
                search: query.search,
                cursor: query.cursor,
                limit: query.limit,
                ..ListTasksQuery::default()
            })
            .await?;
        let live = self.live_session_ids().await;
        let session_ids = page
            .tasks
            .iter()
            .map(|task| task.session_id.clone())
            .collect::<Vec<_>>();
        let clients = self.audit_log.client_names(&session_ids).await?;
        let items = page
            .tasks
            .iter()
            .map(|task| {
                summary(
                    task,
                    clients.get(&task.session_id).map(String::as_str),
                    live.contains(&task.session_id),
                )
            })
            .collect();
        let mut list = hosted_api::SessionList::new(items);
        list.next_cursor = page.next_cursor.map(|cursor| cursor.to_string());
        Ok(list)
    }

    /// One session with its tool calls, or `None` for an unknown id. With a `wait`, re-reads
    /// until the session is complete or the wait runs out, then returns whatever it has.
    pub(super) async fn get(
        &self,
        session_id: &str,
        options: GetOptions,
    ) -> AppResult<Option<hosted_api::Session>> {
        let deadline = tokio::time::Instant::now() + options.wait;
        loop {
            let Some(snapshot) = self.snapshot(session_id, deadline).await? else {
                return Ok(None);
            };
            let now = tokio::time::Instant::now();
            if snapshot.complete || now >= deadline {
                return self.session(snapshot, options.screenshots).await.map(Some);
            }
            tokio::time::sleep(WAIT_POLL.min(deadline - now)).await;
        }
    }

    /// The session's rrweb recording, one page per recorded document, or `None` for an id
    /// `get` would not find either.
    pub(super) async fn recording(
        &self,
        session_id: &str,
    ) -> AppResult<Option<hosted_api::Recording>> {
        let known = self
            .audit_log
            .get_task_summary(session_id)
            .await?
            .is_some_and(|task| task.dispatch_count > 0);
        if !known {
            return Ok(None);
        }
        let events = self.replay.read_session(session_id).await?;
        Ok(Some(hosted_api::Recording::new(pages(events))))
    }

    async fn snapshot(
        &self,
        session_id: &str,
        deadline: tokio::time::Instant,
    ) -> AppResult<Option<Snapshot>> {
        let live = self.sessions.contains(&SessionId::new(session_id)).await;
        // An ended session's last rows and screenshots are written by the audit worker after
        // its calls return, so flush before reading. A live session is read as-is, without
        // waiting on captures still in flight.
        let flushed = !live && self.flush(session_id, deadline).await;
        let Some(detail) = self.audit_log.get_task(session_id).await? else {
            return Ok(None);
        };
        let lifecycle = self.audit_log.session_lifecycle(session_id).await?;
        let complete = flushed
            && finished_at(&lifecycle, &detail.summary).is_some_and(|finished_at| {
                now_epoch_ms() >= finished_at.saturating_add(RECORDING_SETTLE_MS)
            });
        Ok(Some(Snapshot {
            detail,
            live,
            complete,
        }))
    }

    /// Waits for the session's queued audit work, up to the request's deadline or
    /// [`FLUSH_FLOOR`], whichever is later. False means work may still be pending.
    async fn flush(&self, session_id: &str, deadline: tokio::time::Instant) -> bool {
        let budget = deadline
            .saturating_duration_since(tokio::time::Instant::now())
            .max(FLUSH_FLOOR);
        match tokio::time::timeout(budget, self.audit_worker.flush_session(session_id)).await {
            Ok(Ok(())) => true,
            // The worker settled but reports captures that failed; those rows stay without
            // a screenshot, so nothing more is coming for them.
            Ok(Err(error)) => {
                tracing::warn!(session_id, %error, "audit flush reported errors");
                true
            }
            Err(_) => false,
        }
    }

    async fn session(
        &self,
        snapshot: Snapshot,
        include_screenshots: bool,
    ) -> AppResult<hosted_api::Session> {
        let Snapshot {
            detail,
            live,
            complete,
        } = snapshot;
        let client = detail
            .start_event
            .as_ref()
            .map(|start| start.client_name.as_str());
        let summary = summary(&detail.summary, client, live);
        let mut tool_calls = Vec::with_capacity(detail.dispatches.len());
        for row in detail.dispatches {
            let screenshot = if include_screenshots && row.has_screenshot {
                self.screenshot(&summary.id, row.id).await?
            } else {
                None
            };
            tool_calls.push(tool_call(row, screenshot));
        }
        // Rows come in write order, and `run` steps are written when they finish, so steps
        // that overlapped come out of order. A script's row is written after its steps, so on
        // a same-millisecond tie the script is put first; the sort is stable otherwise.
        tool_calls.sort_by_key(|call| (call.started_at, call.parent_id.is_some()));
        let mut session = hosted_api::Session::new(
            summary.id,
            summary.name,
            summary.client,
            summary.status,
            summary.started_at,
            summary.tool_call_count,
            complete,
            tool_calls,
        );
        session.summary = summary.summary;
        session.site = summary.site;
        session.ended_at = summary.ended_at;
        Ok(session)
    }

    /// A missing file reads as no screenshot: retention can delete it between the row read
    /// and this one, and that should not fail the whole session.
    async fn screenshot(&self, session_id: &str, row_id: i64) -> AppResult<Option<Vec<u8>>> {
        match self.screenshots.read(session_id, row_id).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(AppError::Http { status, .. }) if status == StatusCode::NOT_FOUND => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn live_session_ids(&self) -> HashSet<String> {
        self.sessions
            .snapshot()
            .await
            .into_iter()
            .map(|session| session.id().as_str().to_string())
            .collect()
    }
}

/// One read of a session, and whether anything can still be added to it.
struct Snapshot {
    detail: TaskDetail,
    live: bool,
    complete: bool,
}

/// A session's `/v1` summary. Liveness comes from the in-memory MCP session map, because the
/// stored row alone cannot tell an open session from one whose process died. `name` is the
/// stored title and `client` the recorded `clientInfo.name` (falling back to the rows' label
/// for sessions recorded before start events), so neither changes when the session ends.
fn summary(task: &TaskSummary, client: Option<&str>, live: bool) -> hosted_api::SessionSummary {
    let mut summary = hosted_api::SessionSummary::new(
        task.session_id.clone(),
        task.title.clone(),
        client.map_or_else(|| task.agent_label.clone(), str::to_string),
        status(task.status, live).to_string(),
        task.started_at,
        task.dispatch_count,
    );
    summary.summary = task.task_summary.clone();
    summary.site = task.site.clone();
    summary.ended_at = ended_at(task, live);
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

/// When the session finished for good, or `None` while it can still gain rows. A session is
/// finished once every start has its end (see [`SessionLifecycle`]); rows recorded before start
/// events existed have no lifecycle and finished at their last activity.
fn finished_at(lifecycle: &SessionLifecycle, task: &TaskSummary) -> Option<i64> {
    if lifecycle.starts == 0 {
        return ended_at(task, false);
    }
    if lifecycle.ends >= lifecycle.starts {
        lifecycle.last_end_at
    } else {
        None
    }
}

fn tool_call(row: ToolDispatchRow, screenshot: Option<Vec<u8>>) -> hosted_api::ToolCall {
    // A step inside a `run` script is recorded after it finishes, so its row carries its end;
    // every other call row is stamped with its start.
    let started_at = match (&row.parent_dispatch_id, row.duration_ms) {
        (Some(_), Some(duration_ms)) => row.created_at.saturating_sub(duration_ms.max(0)),
        _ => row.created_at,
    };
    let mut call = hosted_api::ToolCall::new(
        // Rows written before dispatch ids existed fall back to their row id.
        row.dispatch_id.unwrap_or_else(|| row.id.to_string()),
        row.tool_name,
        outcome(row.result_meta.as_deref()).to_string(),
        started_at,
        row.has_screenshot,
    );
    // The audit keeps at most 4 KiB of arguments and cuts longer ones mid-JSON, so a stored
    // copy that no longer parses was truncated.
    match row.args_json.as_deref().map(serde_json::from_str::<Value>) {
        Some(Ok(args)) => call.args = Some(Some(args)),
        Some(Err(_)) => call.args_truncated = Some(true),
        None => {}
    }
    call.parent_id = row.parent_dispatch_id;
    call.duration_ms = row.duration_ms;
    if row.tab_id.is_some() || row.url.is_some() || row.title.is_some() {
        let mut page = hosted_api::ToolCallPage::new();
        page.tab_id = row.tab_id;
        page.url = row.url;
        page.title = row.title;
        call.page = Some(Box::new(page));
    }
    call.screenshot = screenshot.map(|bytes| {
        Box::new(hosted_api::Screenshot::new(
            "image/jpeg".to_string(),
            BASE64.encode(bytes),
        ))
    });
    call
}

/// Reads the outcome from the stored result metadata. Cancellation wins over the error flag,
/// as it does for session status, since a cancelled call can carry `isError` too.
fn outcome(result_meta: Option<&str>) -> &'static str {
    let meta = result_meta.and_then(|raw| serde_json::from_str::<Value>(raw).ok());
    let flag = |key: &str| {
        meta.as_ref()
            .and_then(|meta| meta.get(key))
            .and_then(Value::as_bool)
            == Some(true)
    };
    if flag("cancelled") {
        "cancelled"
    } else if flag("isError") {
        "error"
    } else {
        "ok"
    }
}

/// Groups a session's events, already in time order, into one page per document. rrweb
/// replays a single document's event stream, so a partner plays each page in its own
/// `Replayer`; pages come out in the order their first event was recorded.
fn pages(events: Vec<ReplayEvent>) -> Vec<hosted_api::RecordingPage> {
    let mut pages: Vec<hosted_api::RecordingPage> = Vec::new();
    let mut index_by_document = HashMap::<String, usize>::new();
    for event in events {
        // rrweb cannot replay an event without its numeric `EventType`.
        let Some(event_type) = event
            .event
            .event_type
            .as_ref()
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
        else {
            continue;
        };
        let mut recorded = hosted_api::RecordingEvent::new(event_type, event.event.ts);
        recorded.data = event.event.data;
        let index = *index_by_document
            .entry(event.document_id.clone())
            .or_insert_with(|| {
                pages.push(hosted_api::RecordingPage::new(
                    event.tab_id,
                    event.document_id,
                    Vec::new(),
                ));
                pages.len() - 1
            });
        pages[index].events.push(recorded);
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::{SessionLifecycle, TaskStatus, TaskSummary, finished_at};

    fn task(started_at: i64, duration_ms: i64) -> TaskSummary {
        TaskSummary {
            session_id: "s".to_string(),
            agent_id: "agent-task".to_string(),
            slug: "agent".to_string(),
            agent_label: "Agent".to_string(),
            title: "Browsed example.com".to_string(),
            site: None,
            started_at,
            ended_at: None,
            duration_ms,
            dispatch_count: 1,
            tool_sequence: Vec::new(),
            status: TaskStatus::Live,
            error_count: 0,
            last_screenshot_dispatch_id: None,
            cursor_id: 1,
            has_screenshots: false,
            tool_input_token_estimate: 0,
            tool_output_token_estimate: 0,
            tokens_measured: false,
            task_summary: None,
        }
    }

    fn lifecycle(starts: u64, ends: u64, last_end_at: Option<i64>) -> SessionLifecycle {
        SessionLifecycle {
            starts,
            ends,
            last_end_at,
        }
    }

    #[test]
    fn a_session_finishes_only_when_every_start_has_its_end() {
        let task = task(1_000, 500);
        // Out of the live map, end not written yet: teardown is still draining calls.
        assert_eq!(finished_at(&lifecycle(1, 0, None), &task), None);
        // A reused id's second life: the first life's end does not finish it.
        assert_eq!(finished_at(&lifecycle(2, 1, Some(1_200)), &task), None);
        assert_eq!(
            finished_at(&lifecycle(1, 1, Some(1_700)), &task),
            Some(1_700)
        );
        assert_eq!(
            finished_at(&lifecycle(2, 2, Some(9_000)), &task),
            Some(9_000)
        );
    }

    #[test]
    fn rows_without_lifecycle_events_finished_at_their_last_activity() {
        assert_eq!(
            finished_at(&lifecycle(0, 0, None), &task(1_000, 500)),
            Some(1_500)
        );
    }
}
