use crate::{
    clock::now_epoch_ms,
    db::{
        Database,
        entities::{
            agent_session_ends, agent_session_starts,
            prelude::{AgentSessionEnds, AgentSessionStarts, Tasks, ToolDispatches},
            tasks, tool_dispatches,
        },
        session_efficiency_stats::ELIGIBLE_TOKEN_ESTIMATOR_VERSION,
    },
    error::AppResult,
    ids::DispatchId,
};
use browseros_mcp::token_estimate::estimate_tool_output_tokens;
use rmcp::model::ContentBlock;
use sea_orm::{
    ActiveValue::{NotSet, Set},
    ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Statement, TransactionTrait,
    sea_query::{Condition, Expr, ExprTrait, Func, OnConflict},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;

pub use crate::db::entities::tool_dispatches::Model as ToolDispatchRow;

const ARGS_JSON_MAX: usize = 4096;
const RESULT_META_MAX: usize = 4096;
const RESULT_STRUCTURED_KEY_MAX: usize = 128;
const RESULT_STRUCTURED_KEYS_MAX: usize = 16;
const RESULT_JSON_STRING_MAX: usize = 192;
const SUSTAINED_ERROR_TAIL: usize = 3;
const OPERATOR_CANCELLATION_REASON: &str = "Operation cancelled by the User";

#[derive(Clone)]
pub struct AuditLog {
    db: Database,
}

#[derive(Debug, Clone)]
pub struct RecordToolDispatchInput {
    pub agent_id: String,
    pub slug: String,
    pub agent_label: String,
    pub session_id: String,
    pub tool_name: String,
    pub page_id: Option<i64>,
    pub tab_id: Option<i64>,
    pub target_id: Option<String>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub args_json: String,
    pub result_meta: String,
    pub duration_ms: i64,
    pub dispatch_id: DispatchId,
    /// Approximate semantic traffic into BrowserClaw: tool name plus compact arguments.
    pub tool_input_token_estimate: i64,
    /// Approximate semantic content returned by BrowserClaw after result effects.
    pub tool_output_token_estimate: i64,
    /// Formula identity; version 0 is reserved for legacy or otherwise unmeasured rows.
    pub token_estimator_version: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDispatchesResult {
    pub rows: Vec<ToolDispatchRow>,
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct ListDispatchesQuery {
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub cursor: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionScreenshotRow {
    pub screenshot_id: i64,
    pub captured_at: i64,
    pub tool_name: String,
}

/// Status derived from persisted dispatches and the session end event. Active and cancelled
/// sessions keep lifecycle precedence; completed browser work fails only after three consecutive
/// final tool errors. Sessions without browser work retain their explicit end status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Live,
    Done,
    Failed,
    Cancelled,
}

impl TaskStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn from_db(value: String) -> Self {
        match value.as_str() {
            "done" => Self::Done,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Live,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub session_id: String,
    pub agent_id: String,
    pub slug: String,
    pub agent_label: String,
    pub title: String,
    pub site: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_ms: i64,
    pub dispatch_count: i64,
    pub tool_sequence: Vec<String>,
    pub status: TaskStatus,
    pub error_count: i64,
    pub last_screenshot_dispatch_id: Option<i64>,
    pub cursor_id: i64,
    pub has_screenshots: bool,
    /// Session totals of the per-dispatch semantic token estimates, trustworthy only when
    /// `tokens_measured`. `tokens_measured` is true iff the session has dispatches and every one
    /// carries token-estimator v1; legacy/unmeasured sessions leave it false and the sums at 0.
    pub tool_input_token_estimate: i64,
    pub tool_output_token_estimate: i64,
    pub tokens_measured: bool,
}

impl From<tasks::Model> for TaskSummary {
    fn from(model: tasks::Model) -> Self {
        let tool_sequence =
            serde_json::from_str::<Vec<String>>(&model.tool_sequence_json).unwrap_or_default();
        Self {
            session_id: model.session_id,
            agent_id: model.agent_id,
            slug: model.slug,
            agent_label: model.agent_label,
            title: model.title,
            site: model.site,
            started_at: model.started_at,
            ended_at: model.ended_at,
            duration_ms: model.duration_ms,
            dispatch_count: model.dispatch_count,
            tool_sequence,
            status: TaskStatus::from_db(model.status),
            error_count: model.error_count,
            last_screenshot_dispatch_id: model.last_screenshot_dispatch_id,
            cursor_id: model.cursor_id,
            has_screenshots: model.has_screenshots,
            tool_input_token_estimate: model.tool_input_token_estimate,
            tool_output_token_estimate: model.tool_output_token_estimate,
            tokens_measured: model.tokens_measured,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDetail {
    #[serde(flatten)]
    pub summary: TaskSummary,
    pub dispatches: Vec<ToolDispatchRow>,
    pub screenshot_dispatch_ids: Vec<i64>,
    pub start_event: Option<SessionStartEvent>,
    pub end_event: Option<SessionEndEvent>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStartEvent {
    pub created_at: i64,
    pub client_name: String,
    pub client_version: String,
}

impl From<agent_session_starts::Model> for SessionStartEvent {
    fn from(model: agent_session_starts::Model) -> Self {
        Self {
            created_at: model.created_at,
            client_name: model.client_name,
            client_version: model.client_version,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEndEvent {
    pub created_at: i64,
    pub kind: String,
    pub reason: Option<String>,
}

impl From<agent_session_ends::Model> for SessionEndEvent {
    fn from(model: agent_session_ends::Model) -> Self {
        Self {
            created_at: model.created_at,
            kind: model.kind,
            reason: model.reason,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksResult {
    pub tasks: Vec<TaskSummary>,
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct ListTasksQuery {
    pub agent_id: Option<String>,
    pub slug: Option<String>,
    pub status: Option<TaskStatus>,
    pub site: Option<String>,
    pub search: Option<String>,
    pub since: Option<i64>,
    pub cursor: Option<i64>,
    pub limit: Option<i64>,
}

impl AuditLog {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Convenience path for callers that need an immediately refreshed task projection.
    pub async fn record_tool_dispatch(&self, input: RecordToolDispatchInput) -> AppResult<i64> {
        let session_id = input.session_id.clone();
        let row_id = self.append_tool_dispatch(input).await?;
        self.refresh_task(&session_id).await?;
        Ok(row_id)
    }

    /// Appends one persistence-ready dispatch without recomputing its session projection.
    pub async fn append_tool_dispatch(&self, input: RecordToolDispatchInput) -> AppResult<i64> {
        let result = ToolDispatches::insert(tool_dispatches::ActiveModel {
            id: NotSet,
            created_at: Set(now_epoch_ms()),
            agent_id: Set(input.agent_id),
            slug: Set(input.slug),
            agent_label: Set(input.agent_label),
            session_id: Set(input.session_id),
            tool_name: Set(input.tool_name),
            page_id: Set(input.page_id),
            tab_id: Set(input.tab_id),
            target_id: Set(input.target_id),
            url: Set(input.url),
            title: Set(input.title),
            args_json: Set(Some(input.args_json)),
            result_meta: Set(Some(input.result_meta)),
            duration_ms: Set(Some(input.duration_ms)),
            tool_input_token_estimate: Set(input.tool_input_token_estimate.max(0)),
            tool_output_token_estimate: Set(input.tool_output_token_estimate.max(0)),
            token_estimator_version: Set(input.token_estimator_version.max(0)),
            dispatch_id: Set(Some(input.dispatch_id.into_inner())),
            has_screenshot: Set(false),
        })
        .exec(self.db.connection())
        .await?;
        Ok(result.last_insert_id)
    }

    pub async fn refresh_task(&self, session_id: &str) -> AppResult<()> {
        let txn = self.db.connection().begin().await?;
        recompute_task(&txn, session_id).await?;
        txn.commit().await?;
        Ok(())
    }

    /// Reconciles the recorded outcome when Stop wins after the audit effect ran.
    pub async fn mark_dispatch_operator_cancelled(
        &self,
        dispatch_id: &DispatchId,
    ) -> AppResult<bool> {
        let txn = self.db.connection().begin().await?;
        let Some(dispatch) = ToolDispatches::find()
            .filter(tool_dispatches::Column::DispatchId.eq(dispatch_id.as_str()))
            .one(&txn)
            .await?
        else {
            txn.commit().await?;
            return Ok(false);
        };
        let structured_content = json!({
            "cancellationReason": OPERATOR_CANCELLATION_REASON,
            "cancellationKind": "cockpit.operator-cancelled",
        });
        let output_token_estimate =
            estimate_tool_output_tokens(&[ContentBlock::text(OPERATOR_CANCELLATION_REASON)]);
        ToolDispatches::update_many()
            .col_expr(
                tool_dispatches::Column::ResultMeta,
                Expr::value(Some(result_meta(true, true, &structured_content, 1))),
            )
            .col_expr(
                tool_dispatches::Column::ToolOutputTokenEstimate,
                Expr::value(output_token_estimate),
            )
            .filter(tool_dispatches::Column::Id.eq(dispatch.id))
            .exec(&txn)
            .await?;
        recompute_task(&txn, &dispatch.session_id).await?;
        txn.commit().await?;
        Ok(true)
    }

    /// Marks a dispatch screenshot and refreshes its task summary when present.
    pub async fn mark_screenshot(&self, dispatch_id: i64) -> AppResult<()> {
        let txn = self.db.connection().begin().await?;
        if let Some(dispatch) = ToolDispatches::find_by_id(dispatch_id).one(&txn).await? {
            mark_screenshot(&txn, dispatch_id).await?;
            recompute_task(&txn, &dispatch.session_id).await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn mark_screenshot_without_projection(&self, dispatch_id: i64) -> AppResult<()> {
        mark_screenshot(self.db.connection(), dispatch_id).await
    }

    /// Records a session start and refreshes its task summary atomically.
    pub async fn record_session_start(
        &self,
        session_id: &str,
        agent_id: &str,
        slug: &str,
        agent_label: &str,
        client_name: &str,
        client_version: &str,
    ) -> AppResult<()> {
        let txn = self.db.connection().begin().await?;
        AgentSessionStarts::insert(agent_session_starts::ActiveModel {
            id: NotSet,
            created_at: Set(now_epoch_ms()),
            session_id: Set(session_id.to_owned()),
            agent_id: Set(agent_id.to_owned()),
            slug: Set(slug.to_owned()),
            agent_label: Set(agent_label.to_owned()),
            client_name: Set(client_name.to_owned()),
            client_version: Set(client_version.to_owned()),
        })
        .exec(&txn)
        .await?;
        recompute_task(&txn, session_id).await?;
        txn.commit().await?;
        Ok(())
    }

    /// Records a session end and refreshes its task summary atomically.
    pub async fn record_session_end(
        &self,
        session_id: &str,
        kind: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        let txn = self.db.connection().begin().await?;
        AgentSessionEnds::insert(agent_session_ends::ActiveModel {
            id: NotSet,
            created_at: Set(now_epoch_ms()),
            session_id: Set(session_id.to_owned()),
            kind: Set(kind.to_owned()),
            reason: Set(reason.map(str::to_owned)),
        })
        .exec(&txn)
        .await?;
        recompute_task(&txn, session_id).await?;
        txn.commit().await?;
        Ok(())
    }

    /// Lists dispatches using stable descending-id cursor pagination.
    pub async fn list_dispatches(
        &self,
        query: ListDispatchesQuery,
    ) -> AppResult<ListDispatchesResult> {
        let limit = query.limit.unwrap_or(100).clamp(1, 500);
        let page_size = usize::try_from(limit).unwrap_or(500);
        let condition = Condition::all()
            .add_option(
                query
                    .agent_id
                    .map(|value| tool_dispatches::Column::AgentId.eq(value)),
            )
            .add_option(
                query
                    .session_id
                    .map(|value| tool_dispatches::Column::SessionId.eq(value)),
            )
            .add_option(
                query
                    .cursor
                    .map(|value| tool_dispatches::Column::Id.lt(value)),
            );
        let mut rows = ToolDispatches::find()
            .filter(condition)
            .order_by_desc(tool_dispatches::Column::Id)
            .limit(u64::try_from(limit + 1).unwrap_or(501))
            .all(self.db.connection())
            .await?;
        let next_cursor = if rows.len() > page_size {
            rows.truncate(page_size);
            rows.last().map(|row| row.id)
        } else {
            None
        };
        Ok(ListDispatchesResult { rows, next_cursor })
    }

    /// Lists task summaries using composable filters and cursor pagination.
    pub async fn list_tasks(&self, query: ListTasksQuery) -> AppResult<ListTasksResult> {
        let limit = query.limit.unwrap_or(25).clamp(1, 100);
        let page_size = usize::try_from(limit).unwrap_or(100);
        let search_condition = query.search.map(|search| {
            let pattern = format!("%{}%", search.to_ascii_lowercase());
            Condition::any()
                .add(Func::lower(Expr::col(tasks::Column::Title)).like(pattern.clone()))
                .add(Func::lower(Expr::col(tasks::Column::AgentLabel)).like(pattern.clone()))
                .add(
                    Func::lower(Func::coalesce([
                        Expr::col(tasks::Column::Site).into(),
                        Expr::value(""),
                    ]))
                    .like(pattern),
                )
        });
        let condition = Condition::all()
            .add(tasks::Column::DispatchCount.gt(0))
            .add_option(query.agent_id.map(|value| tasks::Column::AgentId.eq(value)))
            .add_option(query.slug.map(|value| tasks::Column::Slug.eq(value)))
            .add_option(
                query
                    .status
                    .map(|value| tasks::Column::Status.eq(value.as_str())),
            )
            .add_option(query.site.map(|value| tasks::Column::Site.eq(value)))
            .add_option(query.since.map(|value| tasks::Column::StartedAt.gte(value)))
            .add_option(search_condition)
            .add_option(query.cursor.map(|value| tasks::Column::CursorId.lt(value)));
        let mut tasks = Tasks::find()
            .filter(condition)
            .order_by_desc(tasks::Column::CursorId)
            .order_by_desc(tasks::Column::StartedAt)
            .limit(u64::try_from(limit + 1).unwrap_or(101))
            .all(self.db.connection())
            .await?
            .into_iter()
            .map(TaskSummary::from)
            .collect::<Vec<_>>();
        let next_cursor = if tasks.len() > page_size {
            tasks.truncate(page_size);
            tasks.last().map(|task| task.cursor_id)
        } else {
            None
        };
        Ok(ListTasksResult { tasks, next_cursor })
    }

    /// Returns the audit summary for one session without loading its dispatch history.
    pub async fn get_task_summary(&self, session_id: &str) -> AppResult<Option<TaskSummary>> {
        Ok(Tasks::find_by_id(session_id.to_owned())
            .one(self.db.connection())
            .await?
            .map(TaskSummary::from))
    }

    /// Returns a task summary with its ordered events and dispatches.
    pub async fn get_task(&self, session_id: &str) -> AppResult<Option<TaskDetail>> {
        let Some(summary) = Tasks::find_by_id(session_id.to_owned())
            .filter(tasks::Column::DispatchCount.gt(0))
            .one(self.db.connection())
            .await?
            .map(TaskSummary::from)
        else {
            return Ok(None);
        };
        let dispatches = query_dispatches_for_session(self.db.connection(), session_id).await?;
        let screenshot_dispatch_ids = dispatches
            .iter()
            .filter(|row| row.has_screenshot)
            .map(|row| row.id)
            .collect();
        let start_event = query_start(self.db.connection(), session_id)
            .await?
            .map(SessionStartEvent::from);
        let end_event = query_end(self.db.connection(), session_id)
            .await?
            .map(SessionEndEvent::from);
        Ok(Some(TaskDetail {
            summary,
            dispatches,
            screenshot_dispatch_ids,
            start_event,
            end_event,
        }))
    }

    pub async fn list_session_screenshots(
        &self,
        session_id: &str,
    ) -> AppResult<Option<Vec<SessionScreenshotRow>>> {
        if Tasks::find_by_id(session_id.to_owned())
            .one(self.db.connection())
            .await?
            .is_none()
        {
            return Ok(None);
        }
        let rows = ToolDispatches::find()
            .filter(tool_dispatches::Column::SessionId.eq(session_id))
            .filter(tool_dispatches::Column::HasScreenshot.eq(true))
            .order_by_asc(tool_dispatches::Column::CreatedAt)
            .order_by_asc(tool_dispatches::Column::Id)
            .all(self.db.connection())
            .await?
            .into_iter()
            .map(|row| SessionScreenshotRow {
                screenshot_id: row.id,
                captured_at: row.created_at,
                tool_name: row.tool_name,
            })
            .collect();
        Ok(Some(rows))
    }

    pub async fn session_owns_screenshot(
        &self,
        session_id: &str,
        screenshot_id: i64,
    ) -> AppResult<bool> {
        Ok(ToolDispatches::find_by_id(screenshot_id)
            .filter(tool_dispatches::Column::SessionId.eq(session_id))
            .filter(tool_dispatches::Column::HasScreenshot.eq(true))
            .one(self.db.connection())
            .await?
            .is_some())
    }
}

async fn mark_screenshot<C: ConnectionTrait>(conn: &C, dispatch_id: i64) -> AppResult<()> {
    ToolDispatches::update_many()
        .col_expr(tool_dispatches::Column::HasScreenshot, Expr::value(true))
        .filter(tool_dispatches::Column::Id.eq(dispatch_id))
        .exec(conn)
        .await?;
    Ok(())
}

async fn recompute_task<C: ConnectionTrait>(conn: &C, session_id: &str) -> AppResult<()> {
    let dispatches = query_dispatches_for_session(conn, session_id).await?;
    let start = query_start(conn, session_id).await?;
    let end = query_end(conn, session_id).await?;
    if dispatches.is_empty() && start.is_none() {
        return Ok(());
    }
    let first_dispatch = dispatches.first();
    let last_dispatch = dispatches.last();
    let started_at = start
        .as_ref()
        .map(|event| event.created_at)
        .or_else(|| first_dispatch.map(|row| row.created_at))
        .unwrap_or_else(now_epoch_ms);
    let ended_at = end.as_ref().map(|event| event.created_at);
    let agent_id = first_dispatch
        .map(|row| row.agent_id.clone())
        .or_else(|| start.as_ref().map(|event| event.agent_id.clone()))
        .unwrap_or_default();
    let slug = first_dispatch
        .map(|row| row.slug.clone())
        .or_else(|| start.as_ref().map(|event| event.slug.clone()))
        .unwrap_or_default();
    let agent_label = first_dispatch
        .map(|row| row.agent_label.clone())
        .or_else(|| start.as_ref().map(|event| event.agent_label.clone()))
        .unwrap_or_else(|| "agent".to_string());
    let site = first_site_of(&dispatches);
    let title = site
        .as_ref()
        .map(|site| format!("Browsed {site}"))
        .unwrap_or_else(|| format!("Session on {agent_label}"));
    let cursor_id = last_dispatch.map(|row| row.id).unwrap_or(0);
    let last_at = last_dispatch
        .map(|row| row.created_at)
        .unwrap_or(started_at);
    let duration_ms = ended_at.unwrap_or(last_at).saturating_sub(started_at);
    let error_count = dispatches
        .iter()
        .filter(|row| result_is_error(row.result_meta.as_deref()))
        .count() as i64;
    let end_event = end.clone().map(SessionEndEvent::from);
    let status = derive_status(&dispatches, end_event.as_ref());
    let tool_sequence: Vec<String> = dispatches.iter().map(|row| row.tool_name.clone()).collect();
    let screenshot_ids: Vec<i64> = dispatches
        .iter()
        .filter(|row| row.has_screenshot)
        .map(|row| row.id)
        .collect();
    let last_screenshot_dispatch_id = screenshot_ids.last().copied();
    // Materialize the live-updating token totals so the audit surfaces read one field instead of
    // re-summing dispatches. `tokens_measured` matches the efficiency projection's eligibility, so a
    // legacy/unmeasured session reports absent rather than a misleading zero downstream.
    let tokens_measured = !dispatches.is_empty()
        && dispatches
            .iter()
            .all(|row| row.token_estimator_version == ELIGIBLE_TOKEN_ESTIMATOR_VERSION);
    let tool_input_token_estimate = dispatches.iter().fold(0i64, |total, row| {
        total.saturating_add(row.tool_input_token_estimate.max(0))
    });
    let tool_output_token_estimate = dispatches.iter().fold(0i64, |total, row| {
        total.saturating_add(row.tool_output_token_estimate.max(0))
    });
    Tasks::insert(tasks::ActiveModel {
        session_id: Set(session_id.to_owned()),
        agent_id: Set(agent_id),
        slug: Set(slug),
        agent_label: Set(agent_label),
        title: Set(title),
        site: Set(site),
        started_at: Set(started_at),
        ended_at: Set(ended_at),
        duration_ms: Set(duration_ms),
        dispatch_count: Set(i64::try_from(dispatches.len()).unwrap_or(i64::MAX)),
        tool_sequence_json: Set(serde_json::to_string(&tool_sequence)?),
        status: Set(status.as_str().to_owned()),
        error_count: Set(error_count),
        last_screenshot_dispatch_id: Set(last_screenshot_dispatch_id),
        cursor_id: Set(cursor_id),
        has_screenshots: Set(!screenshot_ids.is_empty()),
        tool_input_token_estimate: Set(tool_input_token_estimate),
        tool_output_token_estimate: Set(tool_output_token_estimate),
        tokens_measured: Set(tokens_measured),
        updated_at: Set(now_epoch_ms()),
    })
    .on_conflict(
        OnConflict::column(tasks::Column::SessionId)
            .update_columns([
                tasks::Column::AgentId,
                tasks::Column::Slug,
                tasks::Column::AgentLabel,
                tasks::Column::Title,
                tasks::Column::Site,
                tasks::Column::StartedAt,
                tasks::Column::EndedAt,
                tasks::Column::DurationMs,
                tasks::Column::DispatchCount,
                tasks::Column::ToolSequenceJson,
                tasks::Column::Status,
                tasks::Column::ErrorCount,
                tasks::Column::LastScreenshotDispatchId,
                tasks::Column::CursorId,
                tasks::Column::HasScreenshots,
                tasks::Column::ToolInputTokenEstimate,
                tasks::Column::ToolOutputTokenEstimate,
                tasks::Column::TokensMeasured,
                tasks::Column::UpdatedAt,
            ])
            .to_owned(),
    )
    .exec_without_returning(conn)
    .await?;
    Ok(())
}

async fn query_dispatches_for_session<C: ConnectionTrait>(
    conn: &C,
    session_id: &str,
) -> AppResult<Vec<ToolDispatchRow>> {
    Ok(ToolDispatches::find()
        .filter(tool_dispatches::Column::SessionId.eq(session_id))
        .order_by_asc(tool_dispatches::Column::Id)
        .all(conn)
        .await?)
}

async fn query_start<C: ConnectionTrait>(
    conn: &C,
    session_id: &str,
) -> AppResult<Option<agent_session_starts::Model>> {
    Ok(AgentSessionStarts::find()
        .filter(agent_session_starts::Column::SessionId.eq(session_id))
        .order_by_asc(agent_session_starts::Column::Id)
        .one(conn)
        .await?)
}

async fn query_end<C: ConnectionTrait>(
    conn: &C,
    session_id: &str,
) -> AppResult<Option<agent_session_ends::Model>> {
    Ok(AgentSessionEnds::find()
        .filter(agent_session_ends::Column::SessionId.eq(session_id))
        .order_by_asc(agent_session_ends::Column::Id)
        .one(conn)
        .await?)
}

fn derive_status(dispatches: &[ToolDispatchRow], end: Option<&SessionEndEvent>) -> TaskStatus {
    let Some(end) = end else {
        return TaskStatus::Live;
    };
    match end.kind.as_str() {
        "cancelled" => TaskStatus::Cancelled,
        "errored" if dispatches.is_empty() => TaskStatus::Failed,
        "closed" if dispatches.is_empty() => TaskStatus::Done,
        "closed" | "errored"
            if dispatches.len() >= SUSTAINED_ERROR_TAIL
                && dispatches[dispatches.len() - SUSTAINED_ERROR_TAIL..]
                    .iter()
                    .all(|row| result_is_error(row.result_meta.as_deref())) =>
        {
            TaskStatus::Failed
        }
        "closed" | "errored" => TaskStatus::Done,
        _ => TaskStatus::Live,
    }
}

fn first_site_of(dispatches: &[ToolDispatchRow]) -> Option<String> {
    for row in dispatches {
        if let Some(url) = row.url.as_deref().and_then(hostname_of) {
            return Some(url);
        }
    }
    for row in dispatches {
        if let Some(url) = row
            .args_json
            .as_deref()
            .and_then(url_from_args)
            .and_then(|url| hostname_of(&url))
        {
            return Some(url);
        }
    }
    None
}

fn hostname_of(raw: &str) -> Option<String> {
    Url::parse(raw)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
}

fn url_from_args(raw: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| {
            value
                .get("url")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
}

fn result_is_error(result_meta: Option<&str>) -> bool {
    result_meta
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .is_some_and(|value| {
            value.get("isError").and_then(serde_json::Value::as_bool) == Some(true)
                && value.get("cancelled").and_then(serde_json::Value::as_bool) != Some(true)
        })
}

fn safe_stringify(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"<unserialisable>\"".to_string())
}

pub fn bounded_args_json(value: &serde_json::Value) -> String {
    truncate_utf8(&safe_stringify(value), ARGS_JSON_MAX)
}

pub fn result_meta(
    is_error: bool,
    cancelled: bool,
    structured_content: &serde_json::Value,
    content_block_count: usize,
) -> String {
    let structured_keys: Vec<String> = structured_content
        .as_object()
        .map(|obj| {
            obj.keys()
                .take(RESULT_STRUCTURED_KEYS_MAX)
                .map(|key| bounded_json_string(key))
                .collect()
        })
        .unwrap_or_default();
    let mut summary = json!({
        "isError": is_error,
        "cancelled": cancelled,
        "contentSummary": format!("{content_block_count} block(s)"),
        "structuredKeys": structured_keys,
    });
    if let Some(cancellation_kind) = structured_content
        .get("cancellationKind")
        .and_then(serde_json::Value::as_str)
    {
        summary["cancellationKind"] = json!(bounded_json_string(cancellation_kind));
    }
    let serialized = summary.to_string();
    debug_assert!(serialized.len() <= RESULT_META_MAX);
    serialized
}

fn bounded_json_string(value: &str) -> String {
    let value = truncate_utf8(value, RESULT_STRUCTURED_KEY_MAX);
    if serde_json::to_string(&value).map_or(usize::MAX, |encoded| encoded.len())
        <= RESULT_JSON_STRING_MAX
    {
        return value;
    }
    let mut end = value.len().saturating_sub(1);
    loop {
        while !value.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        let candidate = format!("{}~", &value[..end]);
        if serde_json::to_string(&candidate).map_or(usize::MAX, |encoded| encoded.len())
            <= RESULT_JSON_STRING_MAX
        {
            return candidate;
        }
        end = end.saturating_sub(1);
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
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
    use super::{
        AuditLog, ListTasksQuery, RecordToolDispatchInput, TaskStatus, bounded_args_json,
        result_meta,
    };
    use crate::db::{DATABASE_FILENAME, Database};
    use browseros_mcp::token_estimate::estimate_tool_output_tokens;
    use rmcp::model::ContentBlock;
    use serde_json::json;
    use tempfile::tempdir;

    fn dispatch(session_id: &str, url: &str, is_error: bool) -> RecordToolDispatchInput {
        RecordToolDispatchInput {
            agent_id: if session_id.starts_with("a") {
                "agent-a"
            } else {
                "agent-b"
            }
            .to_string(),
            slug: "agent".to_string(),
            agent_label: "Agent".to_string(),
            session_id: session_id.to_string(),
            tool_name: "navigate".to_string(),
            page_id: Some(1),
            tab_id: Some(11),
            target_id: Some("target".to_string()),
            url: Some(url.to_string()),
            title: None,
            args_json: bounded_args_json(&json!({ "url": url })),
            result_meta: result_meta(is_error, false, &json!({ "page": 1 }), 1),
            duration_ms: 10,
            dispatch_id: crate::ids::DispatchId::new(),
            tool_input_token_estimate: 11,
            tool_output_token_estimate: 22,
            token_estimator_version: 1,
        }
    }

    #[test]
    fn result_metadata_bounds_json_escaped_strings() -> anyhow::Result<()> {
        let mut structured = serde_json::Map::new();
        for index in 0..16 {
            structured.insert(
                format!("{}-{index:02}", "\0".repeat(124)),
                serde_json::Value::Null,
            );
        }
        structured.insert(
            "cancellationKind".to_string(),
            serde_json::Value::String("\0".repeat(1024)),
        );

        let meta = result_meta(false, false, &serde_json::Value::Object(structured), 1);

        assert!(meta.len() <= 4096);
        serde_json::from_str::<serde_json::Value>(&meta)?;
        Ok(())
    }

    #[tokio::test]
    async fn dispatch_token_estimates_persist_and_sum_by_session() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        let mut first = dispatch("a1", "https://one.example.com", false);
        first.tool_input_token_estimate = 12;
        first.tool_output_token_estimate = 34;
        audit.record_tool_dispatch(first).await?;
        let mut second = dispatch("a1", "https://two.example.com", false);
        second.tool_input_token_estimate = 56;
        second.tool_output_token_estimate = 78;
        audit.record_tool_dispatch(second).await?;

        let rows = audit
            .list_dispatches(super::ListDispatchesQuery {
                session_id: Some("a1".to_string()),
                ..Default::default()
            })
            .await?
            .rows;
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.token_estimator_version == 1));
        assert_eq!(
            rows.iter()
                .map(|row| row.tool_input_token_estimate)
                .sum::<i64>(),
            68
        );
        assert_eq!(
            rows.iter()
                .map(|row| row.tool_output_token_estimate)
                .sum::<i64>(),
            112
        );

        let task = audit
            .get_task_summary("a1")
            .await?
            .ok_or_else(|| anyhow::anyhow!("task missing"))?;
        assert_eq!(task.dispatch_count, 2);
        assert!(task.tokens_measured);
        assert_eq!(task.tool_input_token_estimate, 68);
        assert_eq!(task.tool_output_token_estimate, 112);
        Ok(())
    }

    #[tokio::test]
    async fn task_summary_reports_unmeasured_for_any_legacy_dispatch() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        // One v1 dispatch alone measures cleanly.
        audit
            .record_tool_dispatch(dispatch("solo-v1", "https://one.example.com", false))
            .await?;
        let measured = audit
            .get_task_summary("solo-v1")
            .await?
            .ok_or_else(|| anyhow::anyhow!("task missing"))?;
        assert!(measured.tokens_measured);
        assert_eq!(measured.tool_input_token_estimate, 11);
        assert_eq!(measured.tool_output_token_estimate, 22);

        // A single unmeasured (v0) dispatch taints the whole session, and a negative per-dispatch
        // estimate never drags the materialized total below zero.
        audit
            .record_tool_dispatch(dispatch("mixed", "https://one.example.com", false))
            .await?;
        let mut legacy = dispatch("mixed", "https://two.example.com", false);
        legacy.token_estimator_version = 0;
        legacy.tool_input_token_estimate = -5;
        legacy.tool_output_token_estimate = 7;
        audit.record_tool_dispatch(legacy).await?;
        let mixed = audit
            .get_task_summary("mixed")
            .await?
            .ok_or_else(|| anyhow::anyhow!("mixed task missing"))?;
        assert!(!mixed.tokens_measured);
        assert_eq!(mixed.tool_input_token_estimate, 11);
        assert_eq!(mixed.tool_output_token_estimate, 29);
        Ok(())
    }

    #[tokio::test]
    async fn dispatch_token_estimates_are_clamped_non_negative() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        let mut input = dispatch("a1", "https://example.com", false);
        input.tool_input_token_estimate = -1;
        input.tool_output_token_estimate = -2;
        input.token_estimator_version = -3;
        audit.record_tool_dispatch(input).await?;

        let row = audit
            .list_dispatches(Default::default())
            .await?
            .rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("dispatch missing"))?;
        assert_eq!(row.tool_input_token_estimate, 0);
        assert_eq!(row.tool_output_token_estimate, 0);
        assert_eq!(row.token_estimator_version, 0);
        Ok(())
    }

    #[tokio::test]
    async fn operator_cancellation_replaces_only_the_output_estimate() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        let mut input = dispatch("a1", "https://example.com", false);
        input.tool_input_token_estimate = 123;
        input.tool_output_token_estimate = 456;
        let dispatch_id = input.dispatch_id.clone();
        audit.record_tool_dispatch(input).await?;

        assert!(audit.mark_dispatch_operator_cancelled(&dispatch_id).await?);

        let row = audit
            .list_dispatches(Default::default())
            .await?
            .rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("dispatch missing"))?;
        assert_eq!(row.tool_input_token_estimate, 123);
        assert_eq!(row.token_estimator_version, 1);
        assert_eq!(
            row.tool_output_token_estimate,
            estimate_tool_output_tokens(&[ContentBlock::text("Operation cancelled by the User")])
        );
        Ok(())
    }

    #[tokio::test]
    async fn migrations_and_dispatch_pagination_work() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        assert!(
            audit
                .list_dispatches(Default::default())
                .await?
                .rows
                .is_empty()
        );
        for idx in 0..5 {
            let url = format!("https://example{idx}.com");
            audit
                .record_tool_dispatch(dispatch("a1", &url, false))
                .await?;
        }
        let first = audit
            .list_dispatches(super::ListDispatchesQuery {
                limit: Some(2),
                ..Default::default()
            })
            .await?;
        assert_eq!(first.rows.len(), 2);
        assert!(first.next_cursor.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn task_filters_compose_before_pagination() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        audit
            .record_tool_dispatch(dispatch("a1", "https://alpha.example.com", false))
            .await?;
        audit.record_session_end("a1", "closed", None).await?;
        for _ in 0..3 {
            audit
                .record_tool_dispatch(dispatch("b1", "https://beta.example.com", true))
                .await?;
        }
        audit.record_session_end("b1", "closed", None).await?;
        let done = audit
            .list_tasks(ListTasksQuery {
                status: Some(TaskStatus::Done),
                search: Some("alpha".to_string()),
                limit: Some(1),
                ..Default::default()
            })
            .await?;
        assert_eq!(done.tasks.len(), 1);
        assert_eq!(done.tasks[0].session_id, "a1");
        assert_eq!(done.next_cursor, None);
        let failed = audit
            .list_tasks(ListTasksQuery {
                status: Some(TaskStatus::Failed),
                site: Some("beta.example.com".to_string()),
                ..Default::default()
            })
            .await?;
        assert_eq!(failed.tasks.len(), 1);
        assert_eq!(failed.tasks[0].session_id, "b1");
        Ok(())
    }

    #[tokio::test]
    async fn completed_session_recovers_after_tool_error() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        audit
            .record_tool_dispatch(dispatch("recovered", "https://example.com", true))
            .await?;
        audit
            .record_tool_dispatch(dispatch("recovered", "https://example.com", false))
            .await?;
        audit
            .record_session_end("recovered", "errored", None)
            .await?;

        let summary = audit
            .get_task_summary("recovered")
            .await?
            .ok_or_else(|| anyhow::anyhow!("recovered task missing"))?;
        assert_eq!(summary.status, TaskStatus::Done);
        assert_eq!(summary.error_count, 1);
        Ok(())
    }

    #[tokio::test]
    async fn completed_session_failure_requires_three_error_tail() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        for (session_id, error_tail) in [("two-errors", 2), ("three-errors", 3)] {
            audit
                .record_tool_dispatch(dispatch(session_id, "https://example.com", false))
                .await?;
            for _ in 0..error_tail {
                audit
                    .record_tool_dispatch(dispatch(session_id, "https://example.com", true))
                    .await?;
            }
            audit.record_session_end(session_id, "closed", None).await?;
        }

        let two_errors = audit
            .get_task_summary("two-errors")
            .await?
            .ok_or_else(|| anyhow::anyhow!("two-error task missing"))?;
        assert_eq!(two_errors.status, TaskStatus::Done);
        assert_eq!(two_errors.error_count, 2);

        let three_errors = audit
            .get_task_summary("three-errors")
            .await?
            .ok_or_else(|| anyhow::anyhow!("three-error task missing"))?;
        assert_eq!(three_errors.status, TaskStatus::Failed);
        assert_eq!(three_errors.error_count, 3);
        Ok(())
    }

    #[tokio::test]
    async fn active_session_stays_live_after_three_error_tail() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        for _ in 0..3 {
            audit
                .record_tool_dispatch(dispatch("active", "https://example.com", true))
                .await?;
        }

        let summary = audit
            .get_task_summary("active")
            .await?
            .ok_or_else(|| anyhow::anyhow!("active task missing"))?;
        assert_eq!(summary.status, TaskStatus::Live);
        assert_eq!(summary.error_count, 3);
        Ok(())
    }

    #[tokio::test]
    async fn zero_dispatch_tasks_keep_end_event_status() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        for (session_id, kind) in [("empty-closed", "closed"), ("empty-errored", "errored")] {
            audit
                .record_session_start(session_id, "agent", "agent", "Agent", "client", "1")
                .await?;
            audit.record_session_end(session_id, kind, None).await?;
        }

        let closed = audit
            .get_task_summary("empty-closed")
            .await?
            .ok_or_else(|| anyhow::anyhow!("empty closed task missing"))?;
        assert_eq!(closed.status, TaskStatus::Done);

        let errored = audit
            .get_task_summary("empty-errored")
            .await?
            .ok_or_else(|| anyhow::anyhow!("empty errored task missing"))?;
        assert_eq!(errored.status, TaskStatus::Failed);
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_end_is_terminal_even_when_dispatches_include_errors() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join("audit.sqlite")).await?);
        audit
            .record_session_start("cancelled-1", "agent", "codex", "Codex", "Codex", "1")
            .await?;
        audit
            .record_tool_dispatch(dispatch("cancelled-1", "https://example.com", true))
            .await?;
        audit
            .record_session_end("cancelled-1", "cancelled", Some("operator requested stop"))
            .await?;

        let summary = audit
            .get_task_summary("cancelled-1")
            .await?
            .ok_or_else(|| anyhow::anyhow!("cancelled task missing"))?;
        assert_eq!(summary.status, TaskStatus::Cancelled);
        assert_eq!(summary.error_count, 1);
        Ok(())
    }

    #[tokio::test]
    async fn zero_dispatch_tasks_are_excluded_before_pagination() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        for session_id in ["handshake-1", "handshake-2"] {
            audit
                .record_session_start(session_id, "agent", "agent", "Agent", "client", "1")
                .await?;
        }
        audit
            .record_tool_dispatch(dispatch("a1", "https://alpha.example.com", false))
            .await?;
        audit
            .record_tool_dispatch(dispatch("b1", "https://beta.example.com", false))
            .await?;

        let page = audit
            .list_tasks(ListTasksQuery {
                limit: Some(2),
                ..Default::default()
            })
            .await?;

        assert_eq!(
            page.tasks
                .iter()
                .map(|task| task.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b1", "a1"]
        );
        assert_eq!(page.next_cursor, None);
        assert!(
            audit
                .get_task_summary("handshake-1")
                .await?
                .is_some_and(|task| task.dispatch_count == 0)
        );
        assert!(audit.get_task("handshake-1").await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn retention_queries_select_and_delete_sessions() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        let first = audit
            .record_tool_dispatch(dispatch("a1", "https://one.example.com", false))
            .await?;
        audit
            .record_tool_dispatch(dispatch("a1", "https://two.example.com", false))
            .await?;
        audit
            .record_tool_dispatch(dispatch("b2", "https://three.example.com", false))
            .await?;
        audit.mark_screenshot(first).await?;

        // A cutoff in the future makes every session "older than" it.
        let future = crate::clock::now_epoch_ms() + 1_000_000;
        let mut older = audit.sessions_older_than(future).await?;
        older.sort();
        assert_eq!(older, vec!["a1".to_string(), "b2".to_string()]);
        assert!(audit.sessions_older_than(0).await?.is_empty());

        let shots = audit
            .screenshot_dispatches_for_sessions(&["a1".to_string()])
            .await?;
        assert_eq!(shots, vec![("a1".to_string(), first)]);
        assert!(
            audit
                .screenshot_dispatches_for_sessions(&[])
                .await?
                .is_empty()
        );

        let counts = audit.delete_sessions(&["a1".to_string()]).await?;
        assert_eq!(counts.dispatches, 2);
        assert_eq!(counts.tasks, 1);
        assert!(audit.get_task_summary("a1").await?.is_none());
        assert_eq!(
            audit.sessions_older_than(future).await?,
            vec!["b2".to_string()]
        );

        audit.reclaim_disk().await?;
        Ok(())
    }

    #[tokio::test]
    async fn retention_includes_sessions_without_dispatches() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        audit
            .record_session_start("c3", "agent-c", "agent", "Agent", "cli", "1.0")
            .await?;

        let future = crate::clock::now_epoch_ms() + 1_000_000;
        assert_eq!(
            audit.sessions_older_than(future).await?,
            vec!["c3".to_string()]
        );

        let counts = audit.delete_sessions(&["c3".to_string()]).await?;
        assert_eq!(counts.session_starts, 1);
        assert!(audit.get_task_summary("c3").await?.is_none());
        assert!(audit.sessions_older_than(future).await?.is_empty());
        Ok(())
    }
}

/// Row counts removed by an audit-retention delete.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AuditDeleteCounts {
    pub dispatches: u64,
    pub session_starts: u64,
    pub session_ends: u64,
    pub tasks: u64,
}

#[derive(FromQueryResult)]
struct SessionIdRow {
    session_id: String,
}

impl AuditLog {
    /// Session ids whose most recent activity is older than `cutoff` (unix
    /// millis). "Activity" spans dispatches AND lifecycle events, so a session
    /// that only started/ended without any tool dispatch is still swept once it
    /// ages out. The UNION-then-GROUP BY has no query-builder form, so it uses a
    /// bound raw statement (as the recording-stream join does).
    pub async fn sessions_older_than(&self, cutoff: i64) -> AppResult<Vec<String>> {
        let statement = Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "SELECT session_id FROM (\
               SELECT session_id, created_at FROM tool_dispatches \
               UNION ALL SELECT session_id, created_at FROM agent_session_starts \
               UNION ALL SELECT session_id, created_at FROM agent_session_ends\
             ) GROUP BY session_id HAVING max(created_at) < ?",
            [cutoff.into()],
        );
        Ok(SessionIdRow::find_by_statement(statement)
            .all(self.db.connection())
            .await?
            .into_iter()
            .map(|row| row.session_id)
            .collect())
    }

    /// `(session_id, dispatch_id)` for every dispatch in `session_ids` carrying
    /// a screenshot. The dispatch id is the screenshot id on disk.
    pub async fn screenshot_dispatches_for_sessions(
        &self,
        session_ids: &[String],
    ) -> AppResult<Vec<(String, i64)>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(ToolDispatches::find()
            .filter(tool_dispatches::Column::SessionId.is_in(session_ids.iter().cloned()))
            .filter(tool_dispatches::Column::HasScreenshot.eq(true))
            .all(self.db.connection())
            .await?
            .into_iter()
            .map(|row| (row.session_id, row.id))
            .collect())
    }

    /// Deletes every audit row for `session_ids` across the four audit tables in
    /// one transaction. `tasks` is included so the derived summary never shows a
    /// phantom session.
    pub async fn delete_sessions(&self, session_ids: &[String]) -> AppResult<AuditDeleteCounts> {
        if session_ids.is_empty() {
            return Ok(AuditDeleteCounts::default());
        }
        let txn = self.db.connection().begin().await?;
        let dispatches = ToolDispatches::delete_many()
            .filter(tool_dispatches::Column::SessionId.is_in(session_ids.iter().cloned()))
            .exec(&txn)
            .await?
            .rows_affected;
        let session_starts = AgentSessionStarts::delete_many()
            .filter(agent_session_starts::Column::SessionId.is_in(session_ids.iter().cloned()))
            .exec(&txn)
            .await?
            .rows_affected;
        let session_ends = AgentSessionEnds::delete_many()
            .filter(agent_session_ends::Column::SessionId.is_in(session_ids.iter().cloned()))
            .exec(&txn)
            .await?
            .rows_affected;
        let tasks = Tasks::delete_many()
            .filter(tasks::Column::SessionId.is_in(session_ids.iter().cloned()))
            .exec(&txn)
            .await?
            .rows_affected;
        txn.commit().await?;
        Ok(AuditDeleteCounts {
            dispatches,
            session_starts,
            session_ends,
            tasks,
        })
    }

    /// Every dispatch id that currently owns a screenshot on disk. The orphan
    /// sweep treats any screenshot file whose id is absent here as removable.
    pub async fn screenshot_dispatch_ids(&self) -> AppResult<Vec<i64>> {
        Ok(ToolDispatches::find()
            .select_only()
            .column(tool_dispatches::Column::Id)
            .filter(tool_dispatches::Column::HasScreenshot.eq(true))
            .into_tuple::<i64>()
            .all(self.db.connection())
            .await?)
    }

    /// Returns freed database pages to the OS. New databases open in incremental
    /// auto-vacuum mode, where this is a cheap `incremental_vacuum`. Legacy
    /// databases are converted once (set incremental + full `VACUUM`) so later
    /// reclaims are cheap.
    pub async fn reclaim_disk(&self) -> AppResult<()> {
        let conn = self.db.connection();
        let mode = conn
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA auto_vacuum",
            ))
            .await?
            .map(|row| row.try_get::<i64>("", "auto_vacuum"))
            .transpose()?
            .unwrap_or(0);
        if mode == 2 {
            conn.execute(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA incremental_vacuum",
            ))
            .await?;
        } else {
            conn.execute(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA auto_vacuum = INCREMENTAL",
            ))
            .await?;
            conn.execute(Statement::from_string(DbBackend::Sqlite, "VACUUM"))
                .await?;
        }
        Ok(())
    }
}
