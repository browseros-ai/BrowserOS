use crate::{
    clock::now_epoch_ms,
    db::audit::{
        AuditDb,
        entities::{
            agent_session_ends, agent_session_starts,
            prelude::{
                AgentSessionEnds, AgentSessionStarts, SessionTabs, TabClaims, Tasks, ToolDispatches,
            },
            session_tabs, tab_claims, tasks, tool_dispatches,
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
use std::path::Path;
use tokio::sync::{mpsc, oneshot};
use tracing::warn;
use url::Url;

pub use crate::db::audit::entities::tool_dispatches::Model as ToolDispatchRow;

const ARGS_JSON_MAX: usize = 4096;

#[derive(Clone)]
pub struct AuditService {
    db: AuditDb,
    claim_writes: mpsc::UnboundedSender<ClaimWrite>,
}

#[derive(Debug)]
enum ClaimWrite {
    ClaimTarget {
        target_id: String,
        session_id: String,
        agent_id: String,
        claimed_at: i64,
    },
    ReleaseTargetForSession {
        target_id: String,
        session_id: String,
    },
    ReleaseSession {
        session_id: String,
        released_at: i64,
    },
    ReleaseTarget {
        target_id: String,
    },
    ClaimTab {
        tab_id: i64,
        opened_target_id: Option<String>,
        session_id: String,
        agent_id: String,
        claimed_at: i64,
    },
    InheritTab {
        opener_tab_id: i64,
        tab_id: i64,
        opened_target_id: String,
        claimed_at: i64,
    },
    ReleaseTabForSession {
        tab_id: i64,
        session_id: String,
        released_at: i64,
    },
    Flush(oneshot::Sender<()>),
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

impl AuditService {
    /// Opens the audit store and applies its migrations.
    pub async fn open(path: impl AsRef<Path>) -> AppResult<Self> {
        let db = AuditDb::open(path).await?;
        let (claim_writes, receiver) = mpsc::unbounded_channel();
        tokio::spawn(run_claim_writes(db.clone(), receiver));
        Ok(Self { db, claim_writes })
    }

    pub(crate) fn connection(&self) -> &sea_orm::DatabaseConnection {
        self.db.connection()
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

    /// Closes every open claim when CDP reports that its target was destroyed.
    pub async fn release_claims_for_target(&self, target_id: &str) -> AppResult<u64> {
        release_claims_for_target(self.db.connection(), target_id).await
    }

    /// Opens a claim window when a session begins driving a target.
    pub async fn claim_target_for_session(
        &self,
        target_id: &str,
        session_id: &str,
        agent_id: &str,
        claimed_at: i64,
    ) -> AppResult<i64> {
        claim_target_for_session(
            self.db.connection(),
            target_id,
            session_id,
            agent_id,
            claimed_at,
        )
        .await
    }

    /// Closes this session's open claim after it closes the target.
    pub async fn release_target_for_session(
        &self,
        target_id: &str,
        session_id: &str,
    ) -> AppResult<u64> {
        release_target_for_session(self.db.connection(), target_id, session_id).await
    }

    /// Closes every open claim when an MCP session ends.
    pub async fn release_claims_for_session(&self, session_id: &str) -> AppResult<u64> {
        release_claims_for_session(self.db.connection(), session_id, now_epoch_ms()).await
    }

    pub fn enqueue_claim_target_for_session(
        &self,
        target_id: String,
        session_id: String,
        agent_id: String,
        claimed_at: i64,
    ) {
        self.enqueue_claim_write(ClaimWrite::ClaimTarget {
            target_id,
            session_id,
            agent_id,
            claimed_at,
        });
    }

    pub fn enqueue_release_target_for_session(&self, target_id: String, session_id: String) {
        self.enqueue_claim_write(ClaimWrite::ReleaseTargetForSession {
            target_id,
            session_id,
        });
    }

    pub fn enqueue_release_claims_for_session(&self, session_id: String) {
        self.enqueue_claim_write(ClaimWrite::ReleaseSession {
            session_id,
            released_at: now_epoch_ms(),
        });
    }

    pub fn enqueue_release_claims_for_target(&self, target_id: String) {
        self.enqueue_claim_write(ClaimWrite::ReleaseTarget { target_id });
    }

    pub fn enqueue_claim_tab_for_session(
        &self,
        tab_id: i64,
        opened_target_id: Option<String>,
        session_id: String,
        agent_id: String,
        claimed_at: i64,
    ) {
        self.enqueue_claim_write(ClaimWrite::ClaimTab {
            tab_id,
            opened_target_id,
            session_id,
            agent_id,
            claimed_at,
        });
    }

    pub fn enqueue_inherit_tab_ownership(
        &self,
        opener_tab_id: i64,
        tab_id: i64,
        opened_target_id: String,
        claimed_at: i64,
    ) {
        self.enqueue_claim_write(ClaimWrite::InheritTab {
            opener_tab_id,
            tab_id,
            opened_target_id,
            claimed_at,
        });
    }

    pub fn enqueue_release_tab_for_session(&self, tab_id: i64, session_id: String) {
        self.enqueue_claim_write(ClaimWrite::ReleaseTabForSession {
            tab_id,
            session_id,
            released_at: now_epoch_ms(),
        });
    }

    pub async fn drain_claim_writes(&self) {
        let (done, receiver) = oneshot::channel();
        if self.claim_writes.send(ClaimWrite::Flush(done)).is_ok() {
            let _ = receiver.await;
        }
    }

    fn enqueue_claim_write(&self, write: ClaimWrite) {
        if let Err(error) = self.claim_writes.send(write) {
            warn!(write = ?error.0, "claim write queue closed");
        }
    }

    /// Closes claims left open across an unclean server shutdown.
    pub async fn release_all_open_claims(&self) -> AppResult<u64> {
        let target_result = TabClaims::update_many()
            .col_expr(tab_claims::Column::ReleasedAt, Expr::value(now_epoch_ms()))
            .filter(tab_claims::Column::ReleasedAt.is_null())
            .exec(self.db.connection())
            .await?;
        let tab_result = SessionTabs::update_many()
            .col_expr(
                session_tabs::Column::ReleasedAt,
                Expr::value(now_epoch_ms()),
            )
            .filter(session_tabs::Column::ReleasedAt.is_null())
            .exec(self.db.connection())
            .await?;
        Ok(target_result.rows_affected + tab_result.rows_affected)
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

    /// Returns the durable browser-tab ownership windows that are still open.
    pub async fn list_open_session_tabs(
        &self,
        session_ids: &[String],
    ) -> AppResult<Vec<session_tabs::Model>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(SessionTabs::find()
            .filter(session_tabs::Column::SessionId.is_in(session_ids.iter().cloned()))
            .filter(session_tabs::Column::ReleasedAt.is_null())
            .order_by_asc(session_tabs::Column::SessionId)
            .order_by_asc(session_tabs::Column::TabId)
            .all(self.db.connection())
            .await?)
    }

    /// Returns current durable ownership for one session and Chrome tab.
    pub async fn open_session_tab(
        &self,
        session_id: &str,
        tab_id: i64,
    ) -> AppResult<Option<session_tabs::Model>> {
        Ok(SessionTabs::find()
            .filter(session_tabs::Column::SessionId.eq(session_id))
            .filter(session_tabs::Column::TabId.eq(tab_id))
            .filter(session_tabs::Column::ReleasedAt.is_null())
            .one(self.db.connection())
            .await?)
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
            .filter(|row| row.has_screenshot && !result_is_error(row.result_meta.as_deref()))
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
}

async fn run_claim_writes(db: AuditDb, mut receiver: mpsc::UnboundedReceiver<ClaimWrite>) {
    while let Some(write) = receiver.recv().await {
        let write = match write {
            ClaimWrite::Flush(done) => {
                let _ = done.send(());
                continue;
            }
            write => write,
        };
        let result = match &write {
            ClaimWrite::ClaimTarget {
                target_id,
                session_id,
                agent_id,
                claimed_at,
            } => claim_target_for_session(
                db.connection(),
                target_id,
                session_id,
                agent_id,
                *claimed_at,
            )
            .await
            .map(|_| ()),
            ClaimWrite::ReleaseTargetForSession {
                target_id,
                session_id,
            } => release_target_for_session(db.connection(), target_id, session_id)
                .await
                .map(|_| ()),
            ClaimWrite::ReleaseSession {
                session_id,
                released_at,
            } => release_claims_for_session(db.connection(), session_id, *released_at)
                .await
                .map(|_| ()),
            ClaimWrite::ReleaseTarget { target_id } => {
                release_claims_for_target(db.connection(), target_id)
                    .await
                    .map(|_| ())
            }
            ClaimWrite::ClaimTab {
                tab_id,
                opened_target_id,
                session_id,
                agent_id,
                claimed_at,
            } => claim_tab_for_session(
                db.connection(),
                *tab_id,
                opened_target_id.as_deref(),
                session_id,
                agent_id,
                *claimed_at,
            )
            .await
            .map(|_| ()),
            ClaimWrite::InheritTab {
                opener_tab_id,
                tab_id,
                opened_target_id,
                claimed_at,
            } => inherit_tab_ownership(
                db.connection(),
                *opener_tab_id,
                *tab_id,
                opened_target_id,
                *claimed_at,
            )
            .await
            .map(|_| ()),
            ClaimWrite::ReleaseTabForSession {
                tab_id,
                session_id,
                released_at,
            } => release_tab_for_session(db.connection(), *tab_id, session_id, *released_at)
                .await
                .map(|_| ()),
            ClaimWrite::Flush(_) => unreachable!(),
        };
        if let Err(error) = result {
            warn!(write = ?write, error = %error, "claim write failed");
        }
    }
}

async fn claim_target_for_session(
    db: &sea_orm::DatabaseConnection,
    target_id: &str,
    session_id: &str,
    agent_id: &str,
    claimed_at: i64,
) -> AppResult<i64> {
    let result = TabClaims::insert(tab_claims::ActiveModel {
        id: NotSet,
        target_id: Set(target_id.to_string()),
        session_id: Set(session_id.to_string()),
        agent_id: Set(agent_id.to_string()),
        claimed_at: Set(claimed_at),
        released_at: Set(None),
    })
    .exec(db)
    .await?;
    Ok(result.last_insert_id)
}

async fn release_target_for_session(
    db: &sea_orm::DatabaseConnection,
    target_id: &str,
    session_id: &str,
) -> AppResult<u64> {
    let result = TabClaims::update_many()
        .col_expr(tab_claims::Column::ReleasedAt, Expr::value(now_epoch_ms()))
        .filter(tab_claims::Column::TargetId.eq(target_id))
        .filter(tab_claims::Column::SessionId.eq(session_id))
        .filter(tab_claims::Column::ReleasedAt.is_null())
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

async fn release_claims_for_session(
    db: &sea_orm::DatabaseConnection,
    session_id: &str,
    released_at: i64,
) -> AppResult<u64> {
    let target_result = TabClaims::update_many()
        .col_expr(tab_claims::Column::ReleasedAt, Expr::value(released_at))
        .filter(tab_claims::Column::SessionId.eq(session_id))
        .filter(tab_claims::Column::ReleasedAt.is_null())
        .exec(db)
        .await?;
    let tab_result = SessionTabs::update_many()
        .col_expr(session_tabs::Column::ReleasedAt, Expr::value(released_at))
        .filter(session_tabs::Column::SessionId.eq(session_id))
        .filter(session_tabs::Column::ReleasedAt.is_null())
        .exec(db)
        .await?;
    Ok(target_result.rows_affected + tab_result.rows_affected)
}

async fn claim_tab_for_session(
    db: &sea_orm::DatabaseConnection,
    tab_id: i64,
    opened_target_id: Option<&str>,
    session_id: &str,
    agent_id: &str,
    claimed_at: i64,
) -> AppResult<i64> {
    let txn = db.begin().await?;
    let existing = SessionTabs::find()
        .filter(session_tabs::Column::TabId.eq(tab_id))
        .filter(session_tabs::Column::ReleasedAt.is_null())
        .one(&txn)
        .await?;
    if let Some(existing) = existing {
        if existing.session_id == session_id && existing.agent_id == agent_id {
            txn.commit().await?;
            return Ok(existing.id);
        }
        SessionTabs::update_many()
            .col_expr(session_tabs::Column::ReleasedAt, Expr::value(claimed_at))
            .filter(session_tabs::Column::Id.eq(existing.id))
            .exec(&txn)
            .await?;
    }
    let result = SessionTabs::insert(session_tabs::ActiveModel {
        id: NotSet,
        session_id: Set(session_id.to_string()),
        agent_id: Set(agent_id.to_string()),
        tab_id: Set(tab_id),
        opened_target_id: Set(opened_target_id.map(str::to_string)),
        claimed_at: Set(claimed_at),
        released_at: Set(None),
    })
    .exec(&txn)
    .await?;
    txn.commit().await?;
    Ok(result.last_insert_id)
}

async fn inherit_tab_ownership(
    db: &sea_orm::DatabaseConnection,
    opener_tab_id: i64,
    tab_id: i64,
    opened_target_id: &str,
    claimed_at: i64,
) -> AppResult<Option<i64>> {
    let Some(owner) = SessionTabs::find()
        .filter(session_tabs::Column::TabId.eq(opener_tab_id))
        .filter(session_tabs::Column::ReleasedAt.is_null())
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    claim_tab_for_session(
        db,
        tab_id,
        Some(opened_target_id),
        &owner.session_id,
        &owner.agent_id,
        claimed_at,
    )
    .await
    .map(Some)
}

async fn release_tab_for_session(
    db: &sea_orm::DatabaseConnection,
    tab_id: i64,
    session_id: &str,
    released_at: i64,
) -> AppResult<u64> {
    let result = SessionTabs::update_many()
        .col_expr(session_tabs::Column::ReleasedAt, Expr::value(released_at))
        .filter(session_tabs::Column::TabId.eq(tab_id))
        .filter(session_tabs::Column::SessionId.eq(session_id))
        .filter(session_tabs::Column::ReleasedAt.is_null())
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

async fn release_claims_for_target(
    db: &sea_orm::DatabaseConnection,
    target_id: &str,
) -> AppResult<u64> {
    let result = TabClaims::update_many()
        .col_expr(tab_claims::Column::ReleasedAt, Expr::value(now_epoch_ms()))
        .filter(tab_claims::Column::TargetId.eq(target_id))
        .filter(tab_claims::Column::ReleasedAt.is_null())
        .exec(db)
        .await?;
    Ok(result.rows_affected)
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
        .filter(|row| row.has_screenshot && !result_is_error(row.result_meta.as_deref()))
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
        AuditService, ClaimWrite, DispatchResultSummary, ListTasksQuery, RecordToolDispatchInput,
        TaskStatus,
    };
    use crate::db::audit::entities::prelude::{SessionTabs, TabClaims};
    use sea_orm::EntityTrait;
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
        let audit = AuditService::open(dir.path().join("audit.sqlite")).await?;
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
        let audit = AuditService::open(dir.path().join("audit.sqlite")).await?;
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

    #[tokio::test]
    async fn queued_claim_mutations_preserve_lifecycle_order() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditService::open(dir.path().join("audit.sqlite")).await?;
        audit.enqueue_claim_target_for_session(
            "target-a".to_string(),
            "session-a".to_string(),
            "agent-a".to_string(),
            100,
        );
        audit.enqueue_release_claims_for_session("session-a".to_string());
        audit.drain_claim_writes().await;

        let claim = TabClaims::find()
            .one(audit.connection())
            .await?
            .unwrap_or_else(|| panic!("queued claim missing"));
        assert!(claim.released_at.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn tab_claim_transfer_closes_the_prior_owner_at_the_boundary() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditService::open(dir.path().join("audit.sqlite")).await?;
        audit.enqueue_claim_tab_for_session(
            11,
            Some("target-a".to_string()),
            "session-a".to_string(),
            "agent-a".to_string(),
            100,
        );
        audit.enqueue_claim_tab_for_session(
            11,
            Some("target-b".to_string()),
            "session-b".to_string(),
            "agent-b".to_string(),
            200,
        );
        audit.drain_claim_writes().await;

        let claims = SessionTabs::find().all(audit.connection()).await?;
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0].session_id, "session-a");
        assert_eq!(claims[0].released_at, Some(200));
        assert_eq!(claims[1].session_id, "session-b");
        assert_eq!(claims[1].released_at, None);
        Ok(())
    }

    #[tokio::test]
    async fn queued_tab_release_preserves_the_observed_boundary() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = AuditService::open(dir.path().join("audit.sqlite")).await?;
        audit.enqueue_claim_tab_for_session(
            11,
            Some("target-a".to_string()),
            "session-a".to_string(),
            "agent-a".to_string(),
            100,
        );
        audit.enqueue_claim_write(ClaimWrite::ReleaseTabForSession {
            tab_id: 11,
            session_id: "session-a".to_string(),
            released_at: 150,
        });
        audit.drain_claim_writes().await;

        let claim = SessionTabs::find()
            .one(audit.connection())
            .await?
            .unwrap_or_else(|| panic!("queued tab claim missing"));
        assert_eq!(claim.released_at, Some(150));
        Ok(())
    }
}
