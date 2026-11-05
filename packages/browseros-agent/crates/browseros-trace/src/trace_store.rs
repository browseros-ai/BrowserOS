//! The trace file: one SQLite database in WAL mode, `$BROWSERCLAW_DIR/claw-sdk/trace.db`.
//!
//! Reads run on a small pool beside the writer, which WAL allows. Every write goes through the
//! one writer task ([`spawn_writer`](crate::spawn_writer)), which calls the `pub(crate)`
//! statements here inside its batch transactions. That keeps writes in order, and keeps
//! privacy commits ordered with the records around them.
//!
//! `synchronous = NORMAL`: a committed batch survives a process crash, which is the failure
//! that matters (a VM power loss loses the browser's state too). Opening the file recovers what
//! a crash left open. A run with no end becomes `interrupted` with `effects: unknown`. An open
//! write operation becomes `unknown`, and an open read becomes `not_sent`, because events
//! queued but not yet written may be gone.

use crate::{
    Delivery, Effect, EpochMillis, ErrorRecord, IdError, OperationId, OperationKind, OperationRow,
    Outcome, Page, PageState, PrivacyRecord, PrivateReason, RequestId, RunDetail, RunEffects,
    RunId, RunQuery, RunStatus, RunSummary, TailQuery, TaskId, TraceError, TraceTail,
};
use browseros_core::TabId;
use serde_json::Value;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

/// The schema this build writes. A file with a newer one is refused, never rewritten.
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS runs (
  id TEXT PRIMARY KEY,
  task_id TEXT,
  started_at INTEGER NOT NULL,
  ended_at INTEGER,
  status TEXT NOT NULL,
  effects TEXT,
  error TEXT,
  request_id TEXT,
  host_data TEXT,
  code TEXT,
  code_truncated INTEGER NOT NULL DEFAULT 0,
  code_withheld TEXT,
  result TEXT,
  result_truncated INTEGER NOT NULL DEFAULT 0,
  replay TEXT,
  replay_until INTEGER,
  operation_count INTEGER NOT NULL DEFAULT 0,
  trace_gaps INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS runs_by_start ON runs(started_at, id);
-- Not unique on purpose: admission keeps a request id from running twice (per task when the run
-- has one, else across Neo), and a unique index here would fail a whole writer batch (and lose
-- its records) if two runs ever raced past it.
CREATE INDEX IF NOT EXISTS runs_by_request ON runs(request_id, task_id)
  WHERE request_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS runs_by_task ON runs(task_id, started_at)
  WHERE task_id IS NOT NULL;
CREATE TABLE IF NOT EXISTS operations (
  run_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  parent_id TEXT,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  attempt INTEGER,
  effect TEXT NOT NULL,
  -- The tab the operation targeted. page_id, tab_id, url and title are the page after it
  -- (`page_after`), as the browser reported it.
  target_tab_id INTEGER,
  page_id INTEGER,
  tab_id INTEGER,
  url TEXT,
  title TEXT,
  started_at INTEGER NOT NULL,
  sent_at INTEGER,
  ended_at INTEGER,
  outcome TEXT,
  delivery TEXT,
  args TEXT,
  args_truncated INTEGER NOT NULL DEFAULT 0,
  result TEXT,
  result_truncated INTEGER NOT NULL DEFAULT 0,
  error TEXT,
  PRIMARY KEY (run_id, seq)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS operations_by_start ON operations(started_at, run_id, seq);
CREATE INDEX IF NOT EXISTS operations_by_tab ON operations(target_tab_id, started_at, run_id, seq)
  WHERE target_tab_id IS NOT NULL;
-- Which runs acted on which tab: a row per run and target tab, written with the operation. The
-- primary key answers runs on this tab; the second index reads a run's tabs and sweeps them.
CREATE TABLE IF NOT EXISTS run_tabs (
  tab_id INTEGER NOT NULL,
  run_id TEXT NOT NULL,
  PRIMARY KEY (tab_id, run_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS run_tabs_by_run ON run_tabs(run_id);
-- Sticky pixel privacy, one row per private tab. Privacy deletes a row when its tab closes;
-- the retention sweep never does, because a private tab can outlive any run on it.
CREATE TABLE IF NOT EXISTS private_tabs (
  tab_id INTEGER PRIMARY KEY,
  since INTEGER NOT NULL,
  reason TEXT NOT NULL
);
";

/// A run summary's columns. Every query that selects them reads `FROM runs` without an alias,
/// because the `tabs` subquery names the table.
const RUN_SUMMARY_COLUMNS: &str = "id, task_id, status, effects, error, request_id, \
    started_at, ended_at, operation_count, trace_gaps, \
    (SELECT group_concat(tab_id) FROM run_tabs WHERE run_tabs.run_id = runs.id) AS tabs";

const OPERATION_COLUMNS: &str = "run_id, seq, parent_id, name, kind, attempt, effect, \
    target_tab_id, page_id, tab_id, url, title, started_at, sent_at, ended_at, outcome, \
    delivery, args, args_truncated, result, result_truncated, error";

/// How long records are kept. [`Default`] is the fixed default claw-sdk runs with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Finished runs older than this are deleted with their operations and their tab links:
    /// 7 days by default. Private tab flags are never swept.
    pub max_age: Duration,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_age: Duration::from_secs(7 * 24 * 60 * 60),
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
}

/// Reads the trace file. Cheap to clone.
#[derive(Clone)]
pub struct TraceStore {
    pool: SqlitePool,
}

impl TraceStore {
    /// Opens (creating if needed) the trace file at `path`, migrates it, and recovers what a
    /// crash left open.
    ///
    /// # Errors
    /// When the file cannot be opened, or holds a newer schema than this build knows.
    pub async fn open(path: &Path) -> Result<Self, TraceError> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(TraceError::Directory)?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        store.recover().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), TraceError> {
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&self.pool)
            .await?;
        if version > SCHEMA_VERSION {
            return Err(TraceError::Corrupt(format!(
                "trace schema {version} is newer than this build's {SCHEMA_VERSION}"
            )));
        }
        let mut tx = self.pool.begin().await?;
        sqlx::raw_sql(SCHEMA).execute(&mut *tx).await?;
        sqlx::raw_sql(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn recover(&self) -> Result<(), TraceError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE operations SET outcome = 'cancelled', \
             delivery = CASE WHEN effect = 'write' THEN 'unknown' ELSE 'not_sent' END \
             WHERE outcome IS NULL",
        )
        .execute(&mut *tx)
        .await?;
        // Code reaches disk only at a run's end, so an interrupted run has none to show.
        sqlx::query(
            "UPDATE runs SET status = 'interrupted', effects = 'unknown', \
             code_withheld = COALESCE(code_withheld, 'unavailable') WHERE ended_at IS NULL",
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Every private tab, for [`Privacy::new`](crate::Privacy::new) at startup.
    ///
    /// # Errors
    /// When the file cannot be read.
    pub async fn private_tabs(&self) -> Result<HashMap<TabId, PrivacyRecord>, TraceError> {
        let rows = sqlx::query("SELECT tab_id, since, reason FROM private_tabs")
            .fetch_all(&self.pool)
            .await?;
        let mut out = HashMap::with_capacity(rows.len());
        for row in rows {
            let reason: String = row.try_get("reason")?;
            out.insert(
                TabId(row.try_get("tab_id")?),
                PrivacyRecord {
                    since: EpochMillis::from_millis(row.try_get("since")?),
                    // An unknown stored reason still keeps the tab private.
                    reason: PrivateReason::parse(&reason).unwrap_or(PrivateReason::Host),
                },
            );
        }
        Ok(out)
    }

    /// One run with its operations, or `None` when no run has this id.
    ///
    /// # Errors
    /// When the file cannot be read or a row is corrupt.
    pub async fn run(&self, id: RunId) -> Result<Option<RunDetail>, TraceError> {
        let Some(row) = sqlx::query(&format!(
            "SELECT {RUN_SUMMARY_COLUMNS}, host_data, code, code_truncated, code_withheld, result, \
             result_truncated FROM runs WHERE id = ?"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let operations = sqlx::query(&format!(
            "SELECT {OPERATION_COLUMNS} FROM operations WHERE run_id = ? ORDER BY seq"
        ))
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(operation_row)
        .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(RunDetail {
            summary: run_summary(&row)?,
            host_data: json_column(&row, "host_data")?,
            code: row.try_get("code")?,
            code_truncated: row.try_get::<i64, _>("code_truncated")? != 0,
            code_withheld: row.try_get("code_withheld")?,
            result: json_column(&row, "result")?,
            result_truncated: row.try_get::<i64, _>("result_truncated")? != 0,
            operations,
        }))
    }

    /// One operation, or `None`.
    ///
    /// # Errors
    /// When the file cannot be read or the row is corrupt.
    pub async fn operation(&self, id: OperationId) -> Result<Option<OperationRow>, TraceError> {
        sqlx::query(&format!(
            "SELECT {OPERATION_COLUMNS} FROM operations WHERE run_id = ? AND seq = ?"
        ))
        .bind(id.run.to_string())
        .bind(i64::from(id.seq))
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(operation_row)
        .transpose()
    }

    /// Runs, newest first.
    ///
    /// # Errors
    /// When the file cannot be read, a row is corrupt, or the cursor is not one this store gave.
    pub async fn runs(&self, query: RunQuery) -> Result<Page<RunSummary>, TraceError> {
        let limit = page_limit(query.limit);
        let cursor = query.cursor.as_deref().map(parse_cursor).transpose()?;
        let mut sql = format!("SELECT {RUN_SUMMARY_COLUMNS} FROM runs WHERE 1 = 1");
        let mut binds: Vec<Bind> = Vec::new();
        if let Some(request_id) = &query.request_id {
            sql.push_str(" AND request_id = ?");
            binds.push(Bind::Text(request_id.to_string()));
        }
        if let Some(task) = &query.task_id {
            sql.push_str(" AND task_id = ?");
            binds.push(Bind::Text(task.to_string()));
        }
        if let Some(tab) = query.tab {
            sql.push_str(" AND id IN (SELECT run_id FROM run_tabs WHERE tab_id = ?)");
            binds.push(Bind::Int(tab.0));
        }
        if let Some(status) = query.status {
            sql.push_str(" AND status = ?");
            binds.push(Bind::Text(status.as_str().to_string()));
        }
        if let Some(since) = query.since {
            sql.push_str(" AND started_at >= ?");
            binds.push(Bind::Int(since.millis()));
        }
        if let Some(until) = query.until {
            sql.push_str(" AND started_at <= ?");
            binds.push(Bind::Int(until.millis()));
        }
        if let Some((at, id)) = cursor {
            sql.push_str(" AND (started_at < ? OR (started_at = ? AND id < ?))");
            binds.extend([Bind::Int(at), Bind::Int(at), Bind::Text(id)]);
        }
        sql.push_str(" ORDER BY started_at DESC, id DESC LIMIT ?");
        binds.push(Bind::Int(i64::from(limit) + 1));
        let rows = bind_all(sqlx::query(&sql), binds)
            .fetch_all(&self.pool)
            .await?;
        let mut items = rows
            .iter()
            .map(run_summary)
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
        let mut sql = format!("SELECT {RUN_SUMMARY_COLUMNS} FROM runs WHERE request_id = ?");
        let mut binds = vec![Bind::Text(request.to_string())];
        if let Some(task) = task {
            sql.push_str(" AND task_id = ?");
            binds.push(Bind::Text(task.to_string()));
        }
        sql.push_str(" ORDER BY started_at, id LIMIT 1");
        bind_all(sqlx::query(&sql), binds)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(run_summary)
            .transpose()
    }

    /// The exact result a finished run handed its client, while its replay window lasts.
    ///
    /// # Errors
    /// When the file cannot be read or the stored value is corrupt.
    pub async fn replay(&self, id: RunId, now: EpochMillis) -> Result<Option<Value>, TraceError> {
        let row = sqlx::query(
            "SELECT replay FROM runs WHERE id = ? AND replay IS NOT NULL AND replay_until > ?",
        )
        .bind(id.to_string())
        .bind(now.millis())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref()
            .map(|row| json_column(row, "replay"))
            .transpose()
            .map(Option::flatten)
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
        let mut sql = format!("SELECT {OPERATION_COLUMNS} FROM operations WHERE 1 = 1");
        let mut binds = Vec::new();
        if let Some(task) = &query.task_id {
            sql.push_str(" AND run_id IN (SELECT id FROM runs WHERE task_id = ?)");
            binds.push(Bind::Text(task.to_string()));
        }
        if let Some(tab) = query.tab {
            sql.push_str(" AND target_tab_id = ?");
            binds.push(Bind::Int(tab.0));
        }
        // One row past the tail says whether older matches were cut, without a count.
        sql.push_str(" ORDER BY started_at DESC, run_id DESC, seq DESC LIMIT ?");
        binds.push(Bind::Int(i64::from(tail) + 1));
        let mut operations = bind_all(sqlx::query(&sql), binds)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(operation_row)
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

    /// The tab the task's most recent operation with a target tab acted on. claw-sdk's task page
    /// default reads it after a restart, when its memory of the task is gone.
    ///
    /// # Errors
    /// When the file cannot be read.
    pub async fn task_last_tab(&self, task: &TaskId) -> Result<Option<TabId>, TraceError> {
        let tab: Option<i64> = sqlx::query_scalar(
            "SELECT target_tab_id FROM operations \
             WHERE run_id IN (SELECT id FROM runs WHERE task_id = ?) \
               AND target_tab_id IS NOT NULL \
             ORDER BY started_at DESC, run_id DESC, seq DESC LIMIT 1",
        )
        .bind(task.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(tab.map(TabId))
    }

    /// The summaries of `ids`, oldest first.
    async fn run_summaries(&self, ids: &[RunId]) -> Result<Vec<RunSummary>, TraceError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; ids.len()].join(", ");
        let sql = format!(
            "SELECT {RUN_SUMMARY_COLUMNS} FROM runs WHERE id IN ({placeholders}) \
             ORDER BY started_at, id"
        );
        let binds = ids.iter().map(|id| Bind::Text(id.to_string())).collect();
        bind_all(sqlx::query(&sql), binds)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(run_summary)
            .collect()
    }
}

// ---- Writes. Only the writer task calls these, inside its batch transaction. ----

pub(crate) async fn persist_private(
    tx: &mut Transaction<'_, Sqlite>,
    tab: TabId,
    record: PrivacyRecord,
) -> Result<(), TraceError> {
    // A tab that is already private keeps its first record.
    sqlx::query(
        "INSERT INTO private_tabs (tab_id, since, reason) VALUES (?, ?, ?) \
         ON CONFLICT(tab_id) DO NOTHING",
    )
    .bind(tab.0)
    .bind(record.since.millis())
    .bind(record.reason.as_str())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(crate) async fn forget_private(
    tx: &mut Transaction<'_, Sqlite>,
    tab: TabId,
) -> Result<(), TraceError> {
    sqlx::query("DELETE FROM private_tabs WHERE tab_id = ?")
        .bind(tab.0)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// A run's start row, as the writer stores it.
pub(crate) struct RunStartRow<'a> {
    pub(crate) id: RunId,
    pub(crate) task: Option<&'a TaskId>,
    pub(crate) at: EpochMillis,
    pub(crate) request_id: Option<&'a RequestId>,
    pub(crate) host_data: Option<&'a Value>,
}

pub(crate) async fn insert_run(
    tx: &mut Transaction<'_, Sqlite>,
    run: RunStartRow<'_>,
) -> Result<(), TraceError> {
    sqlx::query(
        "INSERT INTO runs (id, task_id, started_at, status, request_id, host_data) \
         VALUES (?, ?, ?, 'running', ?, ?) ON CONFLICT(id) DO NOTHING",
    )
    .bind(run.id.to_string())
    .bind(run.task.map(TaskId::as_str))
    .bind(run.at.millis())
    .bind(run.request_id.map(RequestId::as_str))
    .bind(run.host_data.map(Value::to_string))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// What a run's end stores of its code.
pub(crate) enum RunCode<'a> {
    /// The masked, bounded code.
    Stored { text: &'a str, truncated: bool },
    /// No code, and why: `sensitive_input`, or `unavailable` when the writer never saw it.
    Withheld(&'static str),
}

/// A run's end row, as the writer stores it.
pub(crate) struct RunEndRow<'a> {
    pub(crate) id: RunId,
    pub(crate) at: EpochMillis,
    pub(crate) status: RunStatus,
    pub(crate) error: Option<&'a ErrorRecord>,
    pub(crate) result: Option<&'a Value>,
    pub(crate) result_truncated: bool,
    pub(crate) code: RunCode<'a>,
    pub(crate) replay: Option<&'a Value>,
    pub(crate) replay_until: Option<EpochMillis>,
    pub(crate) gaps: u64,
}

/// Closes a run: open operations get their verdict, then the run its effects.
pub(crate) async fn end_run(
    tx: &mut Transaction<'_, Sqlite>,
    run: RunEndRow<'_>,
) -> Result<RunEffects, TraceError> {
    let id = run.id.to_string();
    sqlx::query(
        "UPDATE operations SET outcome = 'cancelled', ended_at = ?, \
         delivery = CASE WHEN effect = 'write' THEN 'unknown' ELSE 'not_sent' END \
         WHERE run_id = ? AND outcome IS NULL",
    )
    .bind(run.at.millis())
    .bind(&id)
    .execute(&mut **tx)
    .await?;
    let counts = sqlx::query(
        "SELECT \
           COALESCE(SUM(delivery = 'unknown'), 0) AS unknown, \
           COALESCE(SUM(delivery IN ('done', 'unknown')), 0) AS attempted \
         FROM operations WHERE run_id = ? AND effect = 'write'",
    )
    .bind(&id)
    .fetch_one(&mut **tx)
    .await?;
    let unknown: i64 = counts.try_get("unknown")?;
    let attempted: i64 = counts.try_get("attempted")?;
    let gaps: i64 = sqlx::query_scalar("SELECT trace_gaps FROM runs WHERE id = ?")
        .bind(&id)
        .fetch_optional(&mut **tx)
        .await?
        .unwrap_or(0);
    let gaps = gaps.saturating_add(i64::try_from(run.gaps).unwrap_or(i64::MAX));
    // A dropped event may have been a write: gaps make the run's effects unknown.
    let effects = if unknown > 0 || gaps > 0 {
        RunEffects::Unknown
    } else if attempted > 0 {
        RunEffects::Done
    } else {
        RunEffects::None
    };
    let (code, code_truncated, code_withheld) = match run.code {
        RunCode::Stored { text, truncated } => (Some(text), truncated, None),
        RunCode::Withheld(reason) => (None, false, Some(reason)),
    };
    let error = run
        .error
        .map(serde_json::to_string)
        .transpose()
        .map_err(json_error)?;
    sqlx::query(
        "UPDATE runs SET ended_at = ?, status = ?, effects = ?, error = ?, result = ?, \
         result_truncated = ?, trace_gaps = ?, replay = ?, replay_until = ?, \
         code = ?, code_truncated = ?, code_withheld = ? \
         WHERE id = ?",
    )
    .bind(run.at.millis())
    .bind(run.status.as_str())
    .bind(effects.as_str())
    .bind(error)
    .bind(run.result.map(Value::to_string))
    .bind(i64::from(run.result_truncated))
    .bind(gaps)
    .bind(run.replay.map(Value::to_string))
    .bind(run.replay_until.map(EpochMillis::millis))
    .bind(code)
    .bind(i64::from(code_truncated))
    .bind(code_withheld)
    .bind(&id)
    .execute(&mut **tx)
    .await?;
    Ok(effects)
}

/// An operation's start row, as the writer stores it (args already conservatively redacted).
/// An operation with a target tab also lists its run under that tab.
pub(crate) struct OperationStartRow<'a> {
    pub(crate) id: OperationId,
    pub(crate) parent: Option<OperationId>,
    pub(crate) name: &'a str,
    pub(crate) kind: OperationKind,
    pub(crate) effect: Effect,
    pub(crate) tab: Option<TabId>,
    pub(crate) at: EpochMillis,
    pub(crate) args: &'a Value,
    pub(crate) args_truncated: bool,
}

pub(crate) async fn insert_operation(
    tx: &mut Transaction<'_, Sqlite>,
    op: OperationStartRow<'_>,
) -> Result<(), TraceError> {
    let inserted = sqlx::query(
        "INSERT INTO operations (run_id, seq, parent_id, name, kind, attempt, effect, \
         target_tab_id, started_at, args, args_truncated) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(run_id, seq) DO NOTHING",
    )
    .bind(op.id.run.to_string())
    .bind(i64::from(op.id.seq))
    .bind(op.parent.map(|parent| parent.to_string()))
    .bind(op.name)
    .bind(op.kind.as_str())
    .bind(op.kind.attempt().map(i64::from))
    .bind(op.effect.as_str())
    .bind(op.tab.map(|tab| tab.0))
    .bind(op.at.millis())
    .bind(op.args.to_string())
    .bind(i64::from(op.args_truncated))
    .execute(&mut **tx)
    .await?;
    if inserted.rows_affected() > 0 {
        sqlx::query("UPDATE runs SET operation_count = operation_count + 1 WHERE id = ?")
            .bind(op.id.run.to_string())
            .execute(&mut **tx)
            .await?;
    }
    if let Some(tab) = op.tab {
        sqlx::query("INSERT OR IGNORE INTO run_tabs (tab_id, run_id) VALUES (?, ?)")
            .bind(tab.0)
            .bind(op.id.run.to_string())
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// An operation's end row, as the writer stores it (everything already masked and bounded).
pub(crate) struct OperationEndRow<'a> {
    pub(crate) id: OperationId,
    pub(crate) at: EpochMillis,
    pub(crate) outcome: Outcome,
    pub(crate) delivery: Delivery,
    pub(crate) sent_at: Option<EpochMillis>,
    pub(crate) args: Option<(&'a Value, bool)>,
    pub(crate) result: Option<(&'a Value, bool)>,
    pub(crate) error: Option<&'a ErrorRecord>,
    pub(crate) page: Option<&'a PageState>,
}

/// Ends an operation. Returns false when no open row matched (its start event was dropped).
pub(crate) async fn end_operation(
    tx: &mut Transaction<'_, Sqlite>,
    op: OperationEndRow<'_>,
) -> Result<bool, TraceError> {
    let error = op
        .error
        .map(serde_json::to_string)
        .transpose()
        .map_err(json_error)?;
    let page = op.page.cloned().unwrap_or_default();
    let updated = sqlx::query(
        "UPDATE operations SET ended_at = ?, outcome = ?, delivery = ?, sent_at = ?, \
         args = COALESCE(?, args), args_truncated = COALESCE(?, args_truncated), \
         result = ?, result_truncated = ?, error = ?, \
         page_id = ?, tab_id = ?, url = ?, title = ? \
         WHERE run_id = ? AND seq = ? AND outcome IS NULL",
    )
    .bind(op.at.millis())
    .bind(op.outcome.as_str())
    .bind(op.delivery.as_str())
    .bind(op.sent_at.map(EpochMillis::millis))
    .bind(op.args.map(|(args, _)| args.to_string()))
    .bind(op.args.map(|(_, cut)| i64::from(cut)))
    .bind(op.result.map(|(result, _)| result.to_string()))
    .bind(i64::from(op.result.is_some_and(|(_, cut)| cut)))
    .bind(error)
    .bind(page.page_id.map(i64::from))
    .bind(page.tab_id.map(|tab| tab.0))
    .bind(page.url)
    .bind(page.title)
    .bind(op.id.run.to_string())
    .bind(i64::from(op.id.seq))
    .execute(&mut **tx)
    .await?;
    Ok(updated.rows_affected() > 0)
}

pub(crate) async fn sweep(
    tx: &mut Transaction<'_, Sqlite>,
    policy: RetentionPolicy,
    now: EpochMillis,
) -> Result<Swept, TraceError> {
    let max_age = i64::try_from(policy.max_age.as_millis()).unwrap_or(i64::MAX);
    let cutoff = now.plus_millis(-max_age).millis();
    sqlx::query(
        "DELETE FROM run_tabs WHERE run_id IN \
         (SELECT id FROM runs WHERE started_at < ? AND status != 'running')",
    )
    .bind(cutoff)
    .execute(&mut **tx)
    .await?;
    let operations = sqlx::query(
        "DELETE FROM operations WHERE run_id IN \
         (SELECT id FROM runs WHERE started_at < ? AND status != 'running')",
    )
    .bind(cutoff)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    // Interrupted runs have no end time but are finished: only a running run is kept.
    let runs = sqlx::query("DELETE FROM runs WHERE started_at < ? AND status != 'running'")
        .bind(cutoff)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    let replays = sqlx::query(
        "UPDATE runs SET replay = NULL, replay_until = NULL \
         WHERE replay_until IS NOT NULL AND replay_until <= ?",
    )
    .bind(now.millis())
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(Swept {
        runs,
        operations,
        replays,
    })
}

// ---- Row mapping. ----

enum Bind {
    Int(i64),
    Text(String),
}

fn bind_all<'q>(
    mut query: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    binds: Vec<Bind>,
) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    for bind in binds {
        query = match bind {
            Bind::Int(value) => query.bind(value),
            Bind::Text(value) => query.bind(value),
        };
    }
    query
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

fn parse_id<T: FromStr>(text: &str) -> Result<T, TraceError> {
    text.parse()
        .map_err(|_| TraceError::Corrupt(format!("invalid id {text:?}")))
}

/// A stored host id, read back through the same check it passed on the way in.
fn host_id_column<T>(
    row: &SqliteRow,
    column: &str,
    parse: fn(&str) -> Result<T, IdError>,
) -> Result<Option<T>, TraceError> {
    row.try_get::<Option<String>, _>(column)?
        .map(|text| parse(&text).map_err(|_| corrupt(column, &text)))
        .transpose()
}

/// A run's tabs, from the `group_concat` in [`RUN_SUMMARY_COLUMNS`], whose order SQLite leaves
/// open.
fn tabs_column(row: &SqliteRow) -> Result<Vec<TabId>, TraceError> {
    let Some(text) = row.try_get::<Option<String>, _>("tabs")? else {
        return Ok(Vec::new());
    };
    let mut tabs = text
        .split(',')
        .map(|tab| tab.parse().map(TabId).map_err(|_| corrupt("tabs", &text)))
        .collect::<Result<Vec<_>, _>>()?;
    tabs.sort_unstable();
    Ok(tabs)
}

fn json_error(error: serde_json::Error) -> TraceError {
    TraceError::Corrupt(error.to_string())
}

fn json_column(row: &SqliteRow, column: &str) -> Result<Option<Value>, TraceError> {
    row.try_get::<Option<String>, _>(column)?
        .map(|text| serde_json::from_str(&text).map_err(json_error))
        .transpose()
}

fn error_column(row: &SqliteRow, column: &str) -> Result<Option<ErrorRecord>, TraceError> {
    row.try_get::<Option<String>, _>(column)?
        .map(|text| serde_json::from_str(&text).map_err(json_error))
        .transpose()
}

fn time_column(row: &SqliteRow, column: &str) -> Result<Option<EpochMillis>, TraceError> {
    Ok(row
        .try_get::<Option<i64>, _>(column)?
        .map(EpochMillis::from_millis))
}

fn count_column(row: &SqliteRow, column: &str) -> Result<u64, TraceError> {
    Ok(u64::try_from(row.try_get::<i64, _>(column)?).unwrap_or(0))
}

fn corrupt(column: &str, value: &str) -> TraceError {
    TraceError::Corrupt(format!("{column} {value:?}"))
}

fn run_summary(row: &SqliteRow) -> Result<RunSummary, TraceError> {
    let status: String = row.try_get("status")?;
    let effects: Option<String> = row.try_get("effects")?;
    Ok(RunSummary {
        id: parse_id(&row.try_get::<String, _>("id")?)?,
        task_id: host_id_column(row, "task_id", TaskId::parse)?,
        tabs: tabs_column(row)?,
        status: run_status(&status).ok_or_else(|| corrupt("status", &status))?,
        effects: effects
            .as_deref()
            .map(|text| run_effects(text).ok_or_else(|| corrupt("effects", text)))
            .transpose()?,
        error: error_column(row, "error")?,
        request_id: host_id_column(row, "request_id", RequestId::parse)?,
        started_at: EpochMillis::from_millis(row.try_get("started_at")?),
        ended_at: time_column(row, "ended_at")?,
        operation_count: count_column(row, "operation_count")?,
        trace_gaps: count_column(row, "trace_gaps")?,
    })
}

fn operation_row(row: &SqliteRow) -> Result<OperationRow, TraceError> {
    let run: RunId = parse_id(&row.try_get::<String, _>("run_id")?)?;
    let seq = u32::try_from(row.try_get::<i64, _>("seq")?)
        .map_err(|_| TraceError::Corrupt("operation seq".into()))?;
    let kind: String = row.try_get("kind")?;
    let attempt = row
        .try_get::<Option<i64>, _>("attempt")?
        .and_then(|attempt| u32::try_from(attempt).ok());
    let effect: String = row.try_get("effect")?;
    let outcome: Option<String> = row.try_get("outcome")?;
    let delivery: Option<String> = row.try_get("delivery")?;
    Ok(OperationRow {
        id: OperationId::new(run, seq),
        parent: row
            .try_get::<Option<String>, _>("parent_id")?
            .as_deref()
            .map(parse_id)
            .transpose()?,
        tab: row.try_get::<Option<i64>, _>("target_tab_id")?.map(TabId),
        name: row.try_get("name")?,
        kind: operation_kind(&kind, attempt).ok_or_else(|| corrupt("kind", &kind))?,
        effect: match effect.as_str() {
            "read" => Effect::Read,
            "write" => Effect::Write,
            other => return Err(corrupt("effect", other)),
        },
        page: PageState {
            page_id: row
                .try_get::<Option<i64>, _>("page_id")?
                .and_then(|page| u32::try_from(page).ok()),
            tab_id: row.try_get::<Option<i64>, _>("tab_id")?.map(TabId),
            url: row.try_get("url")?,
            title: row.try_get("title")?,
        },
        args: json_column(row, "args")?,
        args_truncated: row.try_get::<i64, _>("args_truncated")? != 0,
        result: json_column(row, "result")?,
        result_truncated: row.try_get::<i64, _>("result_truncated")? != 0,
        outcome: outcome
            .as_deref()
            .map(|text| outcome_named(text).ok_or_else(|| corrupt("outcome", text)))
            .transpose()?,
        delivery: delivery
            .as_deref()
            .map(|text| delivery_named(text).ok_or_else(|| corrupt("delivery", text)))
            .transpose()?,
        error: error_column(row, "error")?,
        started_at: EpochMillis::from_millis(row.try_get("started_at")?),
        sent_at: time_column(row, "sent_at")?,
        ended_at: time_column(row, "ended_at")?,
    })
}

fn run_status(text: &str) -> Option<RunStatus> {
    [
        RunStatus::Running,
        RunStatus::Ok,
        RunStatus::Error,
        RunStatus::Cancelled,
        RunStatus::Timeout,
        RunStatus::Interrupted,
    ]
    .into_iter()
    .find(|status| status.as_str() == text)
}

fn run_effects(text: &str) -> Option<RunEffects> {
    [RunEffects::None, RunEffects::Done, RunEffects::Unknown]
        .into_iter()
        .find(|effects| effects.as_str() == text)
}

fn outcome_named(text: &str) -> Option<Outcome> {
    [Outcome::Ok, Outcome::Error, Outcome::Cancelled]
        .into_iter()
        .find(|outcome| outcome.as_str() == text)
}

fn delivery_named(text: &str) -> Option<Delivery> {
    [Delivery::NotSent, Delivery::Done, Delivery::Unknown]
        .into_iter()
        .find(|delivery| delivery.as_str() == text)
}

fn operation_kind(text: &str, attempt: Option<u32>) -> Option<OperationKind> {
    match text {
        "builtin" => Some(OperationKind::Builtin),
        "defined" => Some(OperationKind::Defined),
        "next" => Some(OperationKind::Next {
            attempt: attempt.unwrap_or(1),
        }),
        "host_call" => Some(OperationKind::HostCall),
        "protected" => Some(OperationKind::Protected),
        _ => None,
    }
}
