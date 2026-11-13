//! The trace file: one SQLite database in WAL mode, `$BROWSERCLAW_DIR/claw-sdk/trace.db`.
//!
//! Reads run on a small pool beside the writer, which WAL allows. Every write goes through the
//! one writer task ([`spawn_writer`](crate::spawn_writer)), through storage's `TraceBatch`.
//! That keeps screenshot answers ordered with the records around them. Startup recovery and
//! [`TraceStore::salvage`] use that same write interface before the writer starts.
//!
//! Opening first runs the migrator's WAL switch, atomic baseline adoption and version fence,
//! then crash recovery. The setup connection is awaited closed on every outcome before a caller
//! can move an unusable file aside. Only after that boundary is the lazy reader/writer pool made.
//!
//! `synchronous = NORMAL`: a committed batch survives a process crash, which is the failure
//! that matters (a VM power loss loses the browser's state too). Opening the file recovers what
//! a crash left open. A run with no end becomes `interrupted` with `effects: unknown`. An open
//! write operation becomes `unknown`, and an open read becomes `not_sent`, because events
//! queued but not yet written may be gone. Every run becomes `complete`: the screenshot jobs it
//! was waiting for died with the process.
//!
//! A run's kept downloads are files beside the database too (`run_outputs`), written by
//! claw-sdk when the run ends; [`TraceStore::run`] reads their list from there.
//!
//! Screenshots are files beside the database (`screenshot_files`); an operation's `screenshot`
//! state says whether its file is there. The camera writes a file before its answer reaches the
//! writer. The retention sweep deletes a run's files before its rows, inside its transaction,
//! and keeps the rows of a run whose files it could not delete. So a row may name a file that
//! is gone (it reads as expired, and the next sweep converges), but a file never outlives its
//! run's rows.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use browseros_core::TabId;
use sea_orm::sea_query::{Expr, ExprTrait, Func, Query, SelectStatement, SimpleExpr};
use sea_orm::sqlx::ConnectOptions;
use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteSynchronous};
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, DeriveIden, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Select, SqlxSqliteConnector,
};
use serde_json::Value;

use crate::run_outputs::RunOutputs;
use crate::screenshot_files::ShotFiles;
use crate::storage::entities::{operations, run_tabs, runs};
use crate::storage::record_rows::{self, RunDetailRow, RunSummaryRow};
use crate::storage::{TraceBatch, trace_migrator};
use crate::{
    EpochMillis, OperationId, OperationRow, Page, RequestId, RunDetail, RunId, RunQuery,
    RunSummary, ShotRead, ShotState, TailQuery, TaskAct, TaskId, TraceError, TraceTail, salvage,
};

/// Operations that only subscribe to or deliver a page's events: the prelude's `page.on` and
/// `context.on` (`events.listen`), the file chooser lease that comes with a `filechooser`
/// listener, and the event pump (`events.next`), which ends on the page an event came from.
/// They name a tab without acting on it, so they never make a tab a run's or a task's: a run
/// that listens on every tab of the browser (the initial new tab page, another task's pages)
/// owns only what it opened or acted on. Their rows still record that tab.
pub(crate) const SUBSCRIPTIONS: [&str; 3] = [
    "events.listen",
    "events.interceptFileChooser",
    "events.next",
];

/// Operations that open a tab, which arrives on their end (`page_after`) rather than as their
/// target: the run opened it, so it is the run's even before the script acts on it.
pub(crate) const OPENERS: [&str; 1] = ["context.newPage"];

/// Whether the operation `name` only subscribes (see [`SUBSCRIPTIONS`]), so it makes no tab its
/// run's or its task's. The door asks too, for the task's default page it keeps in memory.
#[must_use]
pub fn is_subscription(name: &str) -> bool {
    SUBSCRIPTIONS.contains(&name)
}

/// The tab an operation `name` on `tab` touches: its target, unless it only subscribes. The one
/// rule for "the run touched this tab": the run record's tabs (`TraceBatch::link_tab`), its
/// run-end screenshots (`TouchedTabs`) and a task's follower (`TraceRecordEvent::OperationStarted`
/// `touched`) all take it, so Neo's own listeners on every known tab move none of them.
#[must_use]
pub(crate) fn touched_tab(name: &str, tab: Option<TabId>) -> Option<TabId> {
    tab.filter(|_| !is_subscription(name))
}

/// The most screenshot bytes kept on disk (D5): about 7,000 JPEGs of 150 KB.
const SCREENSHOT_CAP_BYTES: u64 = 1 << 30;

/// How long records are kept. [`Default`] is the fixed default claw-sdk runs with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Finished runs older than this are deleted with their operations, their tab links and
    /// their screenshots: 7 days by default.
    pub max_age: Duration,
    /// The most screenshot bytes kept. Past it, the oldest runs' screenshots are deleted first
    /// and read as expired: 1 GiB by default.
    pub screenshot_cap: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_age: Duration::from_secs(7 * 24 * 60 * 60),
            screenshot_cap: SCREENSHOT_CAP_BYTES,
        }
    }
}

/// What one retention sweep deleted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    /// Runs deleted.
    pub runs: u64,
    /// Operations deleted.
    pub operations: u64,
    /// Stored results for repeated request ids, dropped because their window passed.
    pub replays: u64,
    /// Screenshots deleted, with their runs or by the disk cap.
    pub screenshots: u64,
}

/// Reads the trace file and the screenshots beside it. Cheap to clone.
#[derive(Clone)]
pub struct TraceStore {
    db: DatabaseConnection,
    files: ShotFiles,
    outputs: RunOutputs,
}

impl TraceStore {
    /// Opens (creating if needed) the trace file at `path`, migrates it, and recovers what a
    /// crash left open.
    ///
    /// When it fails, every connection that read the file is closed before it returns, so the
    /// caller may move the file and its write-ahead log aside and open a fresh one at `path`.
    ///
    /// A damaged write-ahead log is refused before any connection: SQLite would read past the
    /// damage without an error, and the last connection's close would fold what it read into
    /// the file and delete the log. The commits past the damage, private flags among them, would
    /// then be lost with no trace, to this store and to [`TraceStore::salvage`] alike. A crash
    /// that cut the log's last write short is refused too: a log cannot tell it from damage.
    ///
    /// # Errors
    /// When the file cannot be opened, its log is damaged ([`TraceError::DamagedLog`]), or it
    /// holds a newer schema than this build knows.
    pub async fn open(path: &Path) -> Result<Self, TraceError> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(TraceError::Directory)?;
        }
        if salvage::read_log(path).await == salvage::Log::Damaged {
            return Err(TraceError::DamagedLog);
        }
        // Secure delete zeroes what a delete frees; with the log checkpoint after a sweep
        // (`truncate_log`), a run that retention removed (the host's `trace.retentionSeconds`)
        // leaves no readable bytes behind. Without it a deleted reply stayed findable in
        // `trace.db` until SQLite reused its page.
        let options = SqliteConnectOptions::new()
            .filename(path)
            .busy_timeout(Duration::from_secs(5))
            .pragma("secure_delete", "ON")
            .disable_statement_logging();
        // Connect lazily without a file-reading PRAGMA: a failed connect is dropped onto SQLx's
        // worker, whose last close could delete the replacement file's WAL after we return.
        // Explicitly closing this one-connection setup pool awaits every rollback and close,
        // whether migration/recovery succeeds or fails, before the caller can replace the file.
        let setup = SqlxSqliteConnector::from_sqlx_sqlite_pool(
            SqlitePoolOptions::new()
                .max_connections(1)
                .connect_lazy_with(options.clone().create_if_missing(true)),
        );
        let prepared = async {
            trace_migrator::prepare(&setup).await?;
            let batch = TraceBatch::begin(&setup).await?;
            batch.recover_crash().await?;
            batch.commit().await
        }
        .await;
        if let Err(error) = setup.close().await {
            tracing::warn!(%error, path = %path.display(), "the trace file's setup connection did not close cleanly");
        }
        prepared?;
        // The setup read the file whole. The reader/writer pool can now set synchronous on
        // first use, after the failure/replacement boundary has been passed.
        let db = SqlxSqliteConnector::from_sqlx_sqlite_pool(
            SqlitePoolOptions::new()
                .max_connections(4)
                .connect_lazy_with(options.synchronous(SqliteSynchronous::Normal)),
        );
        Ok(Self {
            db,
            files: ShotFiles::beside(path),
            outputs: RunOutputs::beside(path),
        })
    }

    pub(crate) fn db(&self) -> &DatabaseConnection {
        &self.db
    }

    /// Copies every committed page into `trace.db` and empties its write-ahead log, after a
    /// retention sweep that deleted runs (`trace_migrator::truncate_log`).
    pub(crate) async fn truncate_log(&self) {
        trace_migrator::truncate_log(&self.db).await;
    }

    pub(crate) fn files(&self) -> &ShotFiles {
        &self.files
    }

    /// The files runs keep beside the trace: claw-sdk writes a run's downloads there when it
    /// ends, and the host routes read them back.
    #[must_use]
    pub fn outputs(&self) -> &RunOutputs {
        &self.outputs
    }

    /// One run with its operations, or `None` when no run has this id.
    ///
    /// # Errors
    /// When the file cannot be read or a row is corrupt.
    pub async fn run(&self, id: RunId) -> Result<Option<RunDetail>, TraceError> {
        let Some(row) = summary_select()
            .columns([
                runs::Column::HostData,
                runs::Column::Code,
                runs::Column::CodeTruncated,
                runs::Column::CodeWithheld,
                runs::Column::Result,
                runs::Column::ResultTruncated,
            ])
            .filter(runs::Column::Id.eq(id.to_string()))
            .into_model::<RunDetailRow>()
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };
        let operations = operations::Entity::find()
            .filter(operations::Column::RunId.eq(id.to_string()))
            .order_by_asc(operations::Column::Seq)
            .all(&self.db)
            .await?
            .into_iter()
            .map(record_rows::operation_row)
            .collect::<Result<Vec<_>, _>>()?;
        // Preserve the existing two independent reads; no new read transaction is introduced.
        let mut detail = record_rows::run_detail(row, operations)?;
        detail.downloads = self.outputs.downloads(id).await?;
        Ok(Some(detail))
    }

    /// One operation, or `None`.
    ///
    /// # Errors
    /// When the file cannot be read or the row is corrupt.
    pub async fn operation(&self, id: OperationId) -> Result<Option<OperationRow>, TraceError> {
        operations::Entity::find_by_id((id.run.to_string(), i64::from(id.seq)))
            .one(&self.db)
            .await?
            .map(record_rows::operation_row)
            .transpose()
    }

    /// The screenshot taken after `op`. A stored file that is gone reads as expired: the
    /// retention sweep deletes files before its rows commit, and a crash between the two leaves
    /// rows the next sweep finishes.
    ///
    /// # Errors
    /// When the file cannot be read for another reason, or the row is corrupt.
    pub async fn screenshot(&self, op: OperationId) -> Result<ShotRead, TraceError> {
        let state = operations::Entity::find_by_id((op.run.to_string(), i64::from(op.seq)))
            .select_only()
            .column(operations::Column::Screenshot)
            .into_tuple::<Option<String>>()
            .one(&self.db)
            .await?
            .flatten();
        let Some(state) = state else {
            return Ok(ShotRead::None);
        };
        match ShotState::parse(&state).ok_or_else(|| record_rows::corrupt("screenshot", &state))? {
            ShotState::Withheld => Ok(ShotRead::Withheld),
            ShotState::Expired => Ok(ShotRead::Expired),
            ShotState::Stored => match tokio::fs::read(self.files.path(op)).await {
                Ok(jpeg) => Ok(ShotRead::Jpeg(jpeg)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ShotRead::Expired),
                Err(error) => Err(TraceError::Screenshot(error)),
            },
        }
    }

    /// One run's summary: its row and its tabs, without its operations. The service's event
    /// log reads one per run event, so it stays a single-row lookup.
    ///
    /// # Errors
    /// When the file cannot be read or the row is corrupt.
    pub async fn run_summary(&self, id: RunId) -> Result<Option<RunSummary>, TraceError> {
        summary_select()
            .filter(runs::Column::Id.eq(id.to_string()))
            .into_model::<RunSummaryRow>()
            .one(&self.db)
            .await?
            .map(record_rows::run_summary)
            .transpose()
    }

    /// The writes in flight on `tab`: operations that target it, are writes, have not ended,
    /// and belong to a running run. Taking a lock polls it until it reaches zero, so it reads
    /// only the tab's open operations (`operations_open_by_tab`) and their runs by key, never
    /// the tab's history or the run list.
    ///
    /// A run's end closes its open operations, and opening the file closes a crashed run's.
    /// The run's status still matters: an operation can start after its run ended, and nothing
    /// will ever close it.
    ///
    /// # Errors
    /// When the file cannot be read.
    pub async fn open_writes(&self, tab: TabId) -> Result<u64, TraceError> {
        let open = open_writes_select(tab)
            .into_tuple::<i64>()
            .one(&self.db)
            .await?
            .unwrap_or(0);
        Ok(u64::try_from(open).unwrap_or(0))
    }

    /// Runs, newest first.
    ///
    /// # Errors
    /// When the file cannot be read, a row is corrupt, or the cursor is not one this store gave.
    pub async fn runs(&self, query: RunQuery) -> Result<Page<RunSummary>, TraceError> {
        let limit = page_limit(query.limit);
        let cursor = query.cursor.as_deref().map(parse_cursor).transpose()?;
        let mut select = summary_select();
        if let Some(request) = query.request_id {
            select = select.filter(runs::Column::RequestId.eq(request.to_string()));
        }
        if let Some(task) = query.task_id {
            select = select.filter(runs::Column::TaskId.eq(task.to_string()));
        }
        if let Some(tab) = query.tab {
            select = select.filter(
                runs::Column::Id.in_subquery(
                    Query::select()
                        .column(run_tabs::Column::RunId)
                        .from(run_tabs::Entity)
                        .and_where(Expr::col(run_tabs::Column::TabId).eq(tab.0))
                        .to_owned(),
                ),
            );
        }
        if let Some(status) = query.status {
            select = select.filter(runs::Column::Status.eq(status.as_str()));
        }
        if let Some(since) = query.since {
            select = select.filter(runs::Column::StartedAt.gte(since.millis()));
        }
        if let Some(until) = query.until {
            select = select.filter(runs::Column::StartedAt.lte(until.millis()));
        }
        if let Some((at, id)) = cursor {
            select = select.filter(
                Condition::any().add(runs::Column::StartedAt.lt(at)).add(
                    Condition::all()
                        .add(runs::Column::StartedAt.eq(at))
                        .add(runs::Column::Id.lt(id)),
                ),
            );
        }
        let mut items = select
            .order_by_desc(runs::Column::StartedAt)
            .order_by_desc(runs::Column::Id)
            .limit(u64::from(limit) + 1)
            .into_model::<RunSummaryRow>()
            .all(&self.db)
            .await?
            .into_iter()
            .map(record_rows::run_summary)
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = next_page(&mut items, limit, |run| {
            cursor_text(run.started_at.millis(), &run.id.to_string())
        });
        Ok(Page { items, next_cursor })
    }

    /// The earliest run started with `request`, the lookup behind `requestId` replay. Dedupe is
    /// per task when the new run has one, else global:
    /// - `task = Some(t)` matches only the runs started with task `t`;
    /// - `task = None` matches every run with this request id, whatever its task.
    ///
    /// # Errors
    /// When the file cannot be read or the row is corrupt.
    pub async fn run_by_request(
        &self,
        request: &RequestId,
        task: Option<&TaskId>,
    ) -> Result<Option<RunSummary>, TraceError> {
        let mut select = summary_select().filter(runs::Column::RequestId.eq(request.as_str()));
        if let Some(task) = task {
            select = select.filter(runs::Column::TaskId.eq(task.as_str()));
        }
        select
            .order_by_asc(runs::Column::StartedAt)
            .order_by_asc(runs::Column::Id)
            .limit(1)
            .into_model::<RunSummaryRow>()
            .one(&self.db)
            .await?
            .map(record_rows::run_summary)
            .transpose()
    }

    /// The exact result a finished run handed its client, while its replay window lasts.
    ///
    /// # Errors
    /// When the file cannot be read or the stored value is corrupt.
    pub async fn replay(&self, id: RunId, now: EpochMillis) -> Result<Option<Value>, TraceError> {
        let replay = runs::Entity::find_by_id(id.to_string())
            .select_only()
            .column(runs::Column::Replay)
            .filter(runs::Column::Replay.is_not_null())
            .filter(runs::Column::ReplayUntil.gt(now.millis()))
            .into_tuple::<Option<String>>()
            .one(&self.db)
            .await?
            .flatten();
        record_rows::json_column(replay.as_deref())
    }

    /// The latest operations that match `query`, oldest first, with the summaries of their
    /// runs. zuzu's audit reads this.
    ///
    /// # Errors
    /// When the file cannot be read or a row is corrupt.
    pub async fn trace_tail(&self, query: TailQuery) -> Result<TraceTail, TraceError> {
        let tail = match query.tail {
            0 => 100,
            tail => tail.min(1000),
        };
        let mut select = operations::Entity::find();
        if let Some(task) = query.task_id {
            select = select.filter(operations::Column::RunId.in_subquery(task_runs(&task)));
        }
        if let Some(tab) = query.tab {
            select = select.filter(operations::Column::TargetTabId.eq(tab.0));
        }
        let mut operations = select
            .order_by_desc(operations::Column::StartedAt)
            .order_by_desc(operations::Column::RunId)
            .order_by_desc(operations::Column::Seq)
            .limit(u64::from(tail) + 1)
            .all(&self.db)
            .await?
            .into_iter()
            .map(record_rows::operation_row)
            .collect::<Result<Vec<_>, _>>()?;
        let tail = usize::try_from(tail).unwrap_or(usize::MAX);
        let truncated = operations.len() > tail;
        operations.truncate(tail);
        operations.reverse();
        let mut run_ids = operations.iter().map(|op| op.id.run).collect::<Vec<_>>();
        run_ids.sort_unstable();
        run_ids.dedup();
        let runs = self.run_summaries(&run_ids).await?;
        Ok(TraceTail {
            runs,
            operations,
            truncated,
        })
    }

    /// The tab the task's most recent operation acted on, and when. An operation counts with
    /// its target tab, from its start. One that had no target when it started counts with the
    /// tab of the page it ended on, from its end. That is how a `newPage` counts: its tab exists
    /// only once it ends, and it is the page the task works in next. An operation with neither
    /// is skipped. The latest by start wins.
    ///
    /// # Errors
    /// When the file cannot be read or the row is corrupt.
    pub async fn task_last_act(&self, task: &TaskId) -> Result<Option<TaskAct>, TraceError> {
        let row = operations::Entity::find()
            .select_only()
            .columns([
                operations::Column::TargetTabId,
                operations::Column::TabId,
                operations::Column::StartedAt,
                operations::Column::EndedAt,
            ])
            .filter(operations::Column::RunId.in_subquery(task_runs(task)))
            .filter(operations::Column::Name.is_not_in(SUBSCRIPTIONS))
            .filter(
                Condition::any()
                    .add(operations::Column::TargetTabId.is_not_null())
                    .add(operations::Column::TabId.is_not_null()),
            )
            .order_by_desc(operations::Column::StartedAt)
            .order_by_desc(operations::Column::RunId)
            .order_by_desc(operations::Column::Seq)
            .limit(1)
            .into_tuple::<(Option<i64>, Option<i64>, i64, Option<i64>)>()
            .one(&self.db)
            .await?;
        row.map(|(target, page_tab, started, ended)| {
            let (tab, at) = if let Some(tab) = target {
                (tab, started)
            } else {
                (
                    page_tab
                        .ok_or_else(|| TraceError::Corrupt("task operation has no tab".into()))?,
                    ended.ok_or_else(|| TraceError::Corrupt("task operation has no end".into()))?,
                )
            };
            Ok(TaskAct {
                tab: TabId(tab),
                at: EpochMillis::from_millis(at),
            })
        })
        .transpose()
    }

    /// The tab of the task's [last act](Self::task_last_act). claw-sdk's task page default reads
    /// it after a restart, when its memory of the task is gone.
    ///
    /// # Errors
    /// When the file cannot be read or the row is corrupt.
    pub async fn task_last_tab(&self, task: &TaskId) -> Result<Option<TabId>, TraceError> {
        Ok(self.task_last_act(task).await?.map(|act| act.tab))
    }

    /// The distinct tasks whose retained, committed runs touched each requested tab, sorted
    /// by task id. Untasked runs add no association; retention removes links with their runs.
    /// This is historical trace data, not an exclusive ownership claim over a live tab id.
    ///
    /// # Errors
    /// When the file cannot be read or a stored task id is corrupt.
    pub async fn tasks_for_tabs(
        &self,
        tabs: &[TabId],
    ) -> Result<HashMap<TabId, Vec<TaskId>>, TraceError> {
        if tabs.is_empty() {
            return Ok(HashMap::new());
        }
        // A creator has no target at admission: its tab arrives in page_after at the end.
        // Match task_last_act's target-first rule so newPage associations work for old traces
        // as well as new ones, without rewriting stored run/tab links.
        let tab = Expr::col((operations::Entity, operations::Column::TargetTabId))
            .if_null(Expr::col((operations::Entity, operations::Column::TabId)));
        let rows: Vec<(i64, String)> = operations::Entity::find()
            .select_only()
            .column_as(tab.clone(), "tab")
            .column(runs::Column::TaskId)
            .inner_join(runs::Entity)
            .filter(tab.is_in(tabs.iter().map(|tab| tab.0)))
            .filter(operations::Column::Name.is_not_in(SUBSCRIPTIONS))
            .filter(runs::Column::TaskId.is_not_null())
            .distinct()
            .order_by_asc(runs::Column::TaskId)
            .into_tuple::<(i64, String)>()
            .all(&self.db)
            .await?;
        let mut tasks: HashMap<TabId, Vec<TaskId>> = HashMap::new();
        for (tab, task) in rows {
            let task = TaskId::parse(&task)
                .map_err(|error| TraceError::Corrupt(format!("invalid stored task id: {error}")))?;
            tasks.entry(TabId(tab)).or_default().push(task);
        }
        Ok(tasks)
    }

    /// The summaries of `ids`, oldest first.
    async fn run_summaries(&self, ids: &[RunId]) -> Result<Vec<RunSummary>, TraceError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        summary_select()
            .filter(runs::Column::Id.is_in(ids.iter().map(ToString::to_string)))
            .order_by_asc(runs::Column::StartedAt)
            .order_by_asc(runs::Column::Id)
            .into_model::<RunSummaryRow>()
            .all(&self.db)
            .await?
            .into_iter()
            .map(record_rows::run_summary)
            .collect()
    }
}

/// One correlated statement supplies tabs to every summary, including detail and tail reads.
/// The outer table must remain `runs` without an alias, which the typed correlation names.
fn summary_select() -> Select<runs::Entity> {
    #[derive(DeriveIden)]
    struct GroupConcat;
    let tabs = Query::select()
        .expr(Func::cust(GroupConcat).arg(Expr::col((run_tabs::Entity, run_tabs::Column::TabId))))
        .from(run_tabs::Entity)
        .and_where(
            Expr::col((run_tabs::Entity, run_tabs::Column::RunId))
                .equals((runs::Entity, runs::Column::Id)),
        )
        .to_owned()
        .into_sub_query_statement();
    runs::Entity::find()
        .select_only()
        .columns([
            runs::Column::Id,
            runs::Column::TaskId,
            runs::Column::Status,
            runs::Column::Effects,
            runs::Column::Error,
            runs::Column::RequestId,
            runs::Column::StartedAt,
            runs::Column::EndedAt,
            runs::Column::OperationCount,
            runs::Column::TraceGaps,
            runs::Column::Complete,
        ])
        .column_as(SimpleExpr::SubQuery(None, Box::new(tabs)), "tabs")
}

fn open_writes_select(tab: TabId) -> Select<operations::Entity> {
    operations::Entity::find()
        .select_only()
        .column_as(operations::Column::Seq.count(), "n")
        .inner_join(runs::Entity)
        .filter(operations::Column::TargetTabId.eq(tab.0))
        .filter(operations::Column::Outcome.is_null())
        .filter(operations::Column::Effect.eq("write"))
        .filter(runs::Column::Status.eq("running"))
}

fn task_runs(task: &TaskId) -> SelectStatement {
    Query::select()
        .column(runs::Column::Id)
        .from(runs::Entity)
        .and_where(Expr::col(runs::Column::TaskId).eq(task.as_str()))
        .to_owned()
}
fn page_limit(limit: u32) -> u32 {
    match limit {
        0 => 25,
        limit => limit.min(100),
    }
}

/// Keeps `limit` items and returns the cursor past the last one when more remain.
fn next_page<T>(items: &mut Vec<T>, limit: u32, cursor: impl Fn(&T) -> String) -> Option<String> {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    if items.len() <= limit {
        return None;
    }
    items.truncate(limit);
    items.last().map(cursor)
}

fn cursor_text(at: i64, id: &str) -> String {
    format!("{at}:{id}")
}

fn parse_cursor(cursor: &str) -> Result<(i64, String), TraceError> {
    let invalid = || TraceError::Corrupt(format!("invalid cursor {cursor:?}"));
    let (at, id) = cursor.split_once(':').ok_or_else(invalid)?;
    Ok((at.parse().map_err(|_| invalid())?, id.to_string()))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_core::TabId;
    use sea_orm::{DbBackend, QueryTrait};

    use super::{TraceStore, open_writes_select, summary_select};
    use crate::storage::test_faults;

    #[test]
    fn a_summary_keeps_its_tabs_in_one_statement() {
        let sql = summary_select().build(DbBackend::Sqlite).to_string();
        assert!(
            sql.contains("(SELECT group_concat("),
            "tabs stay a correlated aggregate: {sql}"
        );
        assert_eq!(
            sql.matches("FROM \"runs\"").count(),
            1,
            "one outer run read: {sql}"
        );
        assert!(
            sql.contains("\"run_tabs\".\"run_id\" = \"runs\".\"id\""),
            "tabs belong to the selected run: {sql}"
        );
        assert!(
            !sql.contains("\"code\"") && !sql.contains("\"replay\""),
            "list reads exclude payloads: {sql}"
        );
    }

    #[tokio::test]
    async fn open_writes_uses_operations_open_by_tab() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let store = TraceStore::open(&dir.path().join("trace.db")).await?;
        let plan = test_faults::query_plan(
            store.db(),
            open_writes_select(TabId(3)).build(DbBackend::Sqlite),
        )
        .await?;
        assert!(
            plan.contains("operations_open_by_tab"),
            "lock settling must not scan tab history: {plan}"
        );
        Ok(())
    }
}
