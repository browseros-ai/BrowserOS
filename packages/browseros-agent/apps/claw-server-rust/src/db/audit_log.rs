use crate::{
    clock::now_epoch_ms,
    db::{
        Database,
        entities::{
            agent_session_ends, agent_session_starts,
            prelude::{AgentSessionEnds, AgentSessionStarts, Tasks, ToolDispatches},
            tasks, tool_dispatches,
        },
    },
    error::AppResult,
    ids::DispatchId,
};
use sea_orm::{
    ActiveValue::{NotSet, Set},
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    TransactionTrait,
    sea_query::{Condition, Expr, ExprTrait, Func, OnConflict},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;

pub use crate::db::entities::tool_dispatches::Model as ToolDispatchRow;

const ARGS_JSON_MAX: usize = 4096;

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
    pub raw_args: serde_json::Value,
    pub duration_ms: i64,
    pub dispatch_id: DispatchId,
    pub result: DispatchResultSummary,
}

#[derive(Debug, Clone)]
pub struct DispatchResultSummary {
    pub is_error: bool,
    pub structured_content: serde_json::Value,
    pub content: serde_json::Value,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Live,
    Done,
    Failed,
}

impl TaskStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    fn from_db(value: String) -> Self {
        match value.as_str() {
            "done" => Self::Done,
            "failed" => Self::Failed,
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

    /// Records a tool dispatch and refreshes its task summary atomically.
    pub async fn record_tool_dispatch(&self, input: RecordToolDispatchInput) -> AppResult<i64> {
        let txn = self.db.connection().begin().await?;
        let session_id = input.session_id.clone();
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
            args_json: Set(Some(truncate(&safe_stringify(&input.raw_args)))),
            result_meta: Set(Some(summarize_result(&input.result))),
            duration_ms: Set(Some(input.duration_ms)),
            dispatch_id: Set(Some(input.dispatch_id.into_inner())),
            has_screenshot: Set(false),
        })
        .exec(&txn)
        .await?;
        recompute_task(&txn, &session_id).await?;
        txn.commit().await?;
        Ok(result.last_insert_id)
    }

    /// Marks a dispatch screenshot and refreshes its task summary when present.
    pub async fn mark_screenshot(&self, dispatch_id: i64) -> AppResult<()> {
        let txn = self.db.connection().begin().await?;
        if let Some(dispatch) = ToolDispatches::find_by_id(dispatch_id).one(&txn).await? {
            ToolDispatches::update_many()
                .col_expr(tool_dispatches::Column::HasScreenshot, Expr::value(true))
                .filter(tool_dispatches::Column::Id.eq(dispatch_id))
                .exec(&txn)
                .await?;
            recompute_task(&txn, &dispatch.session_id).await?;
        }
        txn.commit().await?;
        Ok(())
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
    let status = derive_status(error_count, end_event.as_ref());
    let tool_sequence: Vec<String> = dispatches.iter().map(|row| row.tool_name.clone()).collect();
    let screenshot_ids: Vec<i64> = dispatches
        .iter()
        .filter(|row| row.has_screenshot)
        .map(|row| row.id)
        .collect();
    let last_screenshot_dispatch_id = screenshot_ids.last().copied();
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

fn derive_status(error_count: i64, end: Option<&SessionEndEvent>) -> TaskStatus {
    if end.map(|event| event.kind.as_str()) == Some("errored") || error_count > 0 {
        TaskStatus::Failed
    } else if end.map(|event| event.kind.as_str()) == Some("closed") {
        TaskStatus::Done
    } else {
        TaskStatus::Live
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
        .and_then(|value| value.get("isError").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

fn safe_stringify(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"<unserialisable>\"".to_string())
}

fn truncate(value: &str) -> String {
    if value.len() <= ARGS_JSON_MAX {
        value.to_string()
    } else {
        format!("{}~", &value[..ARGS_JSON_MAX - 1])
    }
}

fn summarize_result(result: &DispatchResultSummary) -> String {
    let structured_keys: Vec<String> = result
        .structured_content
        .as_object()
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default();
    let content_summary = result
        .content
        .as_array()
        .map(|items| format!("{} block(s)", items.len()))
        .unwrap_or_else(|| "unknown".to_string());
    json!({
        "isError": result.is_error,
        "contentSummary": content_summary,
        "structuredKeys": structured_keys,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        AuditLog, DispatchResultSummary, ListTasksQuery, RecordToolDispatchInput, TaskStatus,
    };
    use crate::db::Database;
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
            raw_args: json!({ "url": url }),
            duration_ms: 10,
            dispatch_id: crate::ids::DispatchId::new(),
            result: DispatchResultSummary {
                is_error,
                structured_content: json!({ "page": 1 }),
                content: json!([{ "type": "text", "text": "ok" }]),
            },
        }
    }

    #[tokio::test]
    async fn migrations_and_dispatch_pagination_work() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditLog::new(Database::open(dir.path().join("audit.sqlite")).await?);
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
        let audit = AuditLog::new(Database::open(dir.path().join("audit.sqlite")).await?);
        audit
            .record_tool_dispatch(dispatch("a1", "https://alpha.example.com", false))
            .await?;
        audit.record_session_end("a1", "closed", None).await?;
        audit
            .record_tool_dispatch(dispatch("b1", "https://beta.example.com", true))
            .await?;
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
}
