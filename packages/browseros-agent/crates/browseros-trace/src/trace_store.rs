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
    Delivery, Effect, EpochMillis, ErrorRecord, OperationId, OperationKind, OperationRow, Outcome,
    Page, PageState, PrivacyRecord, PrivateReason, RunDetail, RunEffects, RunId, RunQuery,
    RunStatus, RunSummary, SessionDetail, SessionId, SessionQuery, SessionState, SessionSummary,
    SessionTrace, TraceError,
};
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
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  client TEXT,
  started_at INTEGER NOT NULL,
  ended_at INTEGER,
  last_activity_at INTEGER NOT NULL,
  private_since INTEGER,
  private_reason TEXT
);
CREATE INDEX IF NOT EXISTS sessions_by_activity ON sessions(last_activity_at, id);
CREATE TABLE IF NOT EXISTS runs (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  started_at INTEGER NOT NULL,
  ended_at INTEGER,
  status TEXT NOT NULL,
  effects TEXT,
  error TEXT,
  host_ref TEXT,
  idempotency_key TEXT,
  host TEXT,
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
CREATE INDEX IF NOT EXISTS runs_by_session ON runs(session_id, started_at);
CREATE INDEX IF NOT EXISTS runs_by_start ON runs(started_at, id);
CREATE INDEX IF NOT EXISTS runs_by_host_ref ON runs(host_ref) WHERE host_ref IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS runs_by_key ON runs(session_id, idempotency_key)
  WHERE idempotency_key IS NOT NULL;
CREATE TABLE IF NOT EXISTS operations (
  run_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  parent_id TEXT,
  session_id TEXT NOT NULL,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  attempt INTEGER,
  effect TEXT NOT NULL,
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
CREATE INDEX IF NOT EXISTS operations_by_session ON operations(session_id, started_at, run_id, seq);
";

const RUN_SUMMARY_COLUMNS: &str = "id, session_id, status, effects, error, host_ref, \
    idempotency_key, started_at, ended_at, operation_count, trace_gaps";

const OPERATION_COLUMNS: &str = "run_id, seq, parent_id, session_id, name, kind, attempt, \
    effect, page_id, tab_id, url, title, started_at, sent_at, ended_at, outcome, delivery, \
    args, args_truncated, result, result_truncated, error";

/// How long records are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Runs older than this are deleted with their operations; then sessions with no runs left
    /// and no activity in this long. `[trace] retention_days`, 7 by default.
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
    /// Sessions deleted.
    pub sessions: u64,
    /// Idempotent-replay results dropped because their window passed.
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
        store.recover(EpochMillis::now()).await?;
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

    async fn recover(&self, now: EpochMillis) -> Result<(), TraceError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE operations SET outcome = 'cancelled', \
             delivery = CASE WHEN effect = 'write' THEN 'unknown' ELSE 'not_sent' END \
             WHERE outcome IS NULL",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE runs SET status = 'interrupted', effects = 'unknown' WHERE ended_at IS NULL",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE sessions SET ended_at = ? WHERE ended_at IS NULL")
            .bind(now.millis())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Every private session, for [`Privacy::new`](crate::Privacy::new) at startup.
    ///
    /// # Errors
    /// When the file cannot be read.
    pub async fn private_sessions(&self) -> Result<HashMap<SessionId, PrivacyRecord>, TraceError> {
        let rows = sqlx::query(
            "SELECT id, private_since, private_reason FROM sessions WHERE private_since IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = HashMap::with_capacity(rows.len());
        for row in rows {
            let id = parse_id::<SessionId>(&row.try_get::<String, _>("id")?)?;
            let reason = row.try_get::<Option<String>, _>("private_reason")?;
            out.insert(
                id,
                PrivacyRecord {
                    since: EpochMillis::from_millis(row.try_get("private_since")?),
                    // An unknown stored reason still keeps the session private.
                    reason: reason
                        .as_deref()
                        .and_then(PrivateReason::parse)
                        .unwrap_or(PrivateReason::Host),
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
            "SELECT {RUN_SUMMARY_COLUMNS}, host, code, code_truncated, code_withheld, result, \
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
            host: json_column(&row, "host")?,
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
        if let Some(session) = query.session {
            sql.push_str(" AND session_id = ?");
            binds.push(Bind::Text(session.to_string()));
        }
        if let Some(host_ref) = query.host_ref {
            sql.push_str(" AND host_ref = ?");
            binds.push(Bind::Text(host_ref));
        }
        if let Some(key) = query.idempotency_key {
            sql.push_str(" AND idempotency_key = ?");
            binds.push(Bind::Text(key));
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

    /// The run with this idempotency key in `session`, if any.
    ///
    /// # Errors
    /// When the file cannot be read or the row is corrupt.
    pub async fn run_by_key(
        &self,
        session: SessionId,
        key: &str,
    ) -> Result<Option<RunSummary>, TraceError> {
        sqlx::query(&format!(
            "SELECT {RUN_SUMMARY_COLUMNS} FROM runs WHERE session_id = ? AND idempotency_key = ?"
        ))
        .bind(session.to_string())
        .bind(key)
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

    /// One session with its runs, or `None`.
    ///
    /// # Errors
    /// When the file cannot be read or a row is corrupt.
    pub async fn session(&self, id: SessionId) -> Result<Option<SessionDetail>, TraceError> {
        let Some(summary) = self.session_summary(id).await? else {
            return Ok(None);
        };
        let runs = sqlx::query(&format!(
            "SELECT {RUN_SUMMARY_COLUMNS} FROM runs WHERE session_id = ? \
             ORDER BY started_at DESC, id DESC"
        ))
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(run_summary)
        .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(SessionDetail { summary, runs }))
    }

    /// Sessions, most recently active first.
    ///
    /// # Errors
    /// When the file cannot be read, a row is corrupt, or the cursor is not one this store gave.
    pub async fn sessions(&self, query: SessionQuery) -> Result<Page<SessionSummary>, TraceError> {
        let limit = page_limit(query.limit);
        let cursor = query.cursor.as_deref().map(parse_cursor).transpose()?;
        let mut sql = String::from(
            "SELECT s.id, s.client, s.started_at, s.ended_at, s.last_activity_at, \
             s.private_since, s.private_reason, \
             (SELECT COUNT(*) FROM runs r WHERE r.session_id = s.id) AS run_count \
             FROM sessions s WHERE 1 = 1",
        );
        let mut binds = Vec::new();
        match query.state {
            Some(SessionState::Open) => sql.push_str(" AND s.ended_at IS NULL"),
            Some(SessionState::Ended) => sql.push_str(" AND s.ended_at IS NOT NULL"),
            None => {}
        }
        if let Some(since) = query.since {
            sql.push_str(" AND s.started_at >= ?");
            binds.push(Bind::Int(since.millis()));
        }
        if let Some(until) = query.until {
            sql.push_str(" AND s.started_at <= ?");
            binds.push(Bind::Int(until.millis()));
        }
        if let Some((at, id)) = cursor {
            sql.push_str(" AND (s.last_activity_at < ? OR (s.last_activity_at = ? AND s.id < ?))");
            binds.extend([Bind::Int(at), Bind::Int(at), Bind::Text(id)]);
        }
        sql.push_str(" ORDER BY s.last_activity_at DESC, s.id DESC LIMIT ?");
        binds.push(Bind::Int(i64::from(limit) + 1));
        let rows = bind_all(sqlx::query(&sql), binds)
            .fetch_all(&self.pool)
            .await?;
        let mut items = rows
            .iter()
            .map(session_summary)
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = next_page(&mut items, limit, |session| {
            cursor_text(session.last_activity_at.millis(), &session.id.to_string())
        });
        Ok(Page { items, next_cursor })
    }

    /// The session's runs and its operations, cut to the last `tail` operations when given.
    ///
    /// # Errors
    /// When the file cannot be read or a row is corrupt.
    pub async fn session_trace(
        &self,
        id: SessionId,
        tail: Option<u32>,
    ) -> Result<Option<SessionTrace>, TraceError> {
        let Some(summary) = self.session_summary(id).await? else {
            return Ok(None);
        };
        let runs = sqlx::query(&format!(
            "SELECT {RUN_SUMMARY_COLUMNS} FROM runs WHERE session_id = ? ORDER BY started_at, id"
        ))
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(run_summary)
        .collect::<Result<Vec<_>, _>>()?;
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM operations WHERE session_id = ?")
            .bind(id.to_string())
            .fetch_one(&self.pool)
            .await?;
        let take = tail.map_or(total, i64::from);
        let mut operations = sqlx::query(&format!(
            "SELECT {OPERATION_COLUMNS} FROM operations WHERE session_id = ? \
             ORDER BY started_at DESC, run_id DESC, seq DESC LIMIT ?"
        ))
        .bind(id.to_string())
        .bind(take)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(operation_row)
        .collect::<Result<Vec<_>, _>>()?;
        operations.reverse();
        Ok(Some(SessionTrace {
            summary,
            runs,
            truncated: total > take,
            operations,
        }))
    }

    async fn session_summary(&self, id: SessionId) -> Result<Option<SessionSummary>, TraceError> {
        sqlx::query(
            "SELECT s.id, s.client, s.started_at, s.ended_at, s.last_activity_at, \
             s.private_since, s.private_reason, \
             (SELECT COUNT(*) FROM runs r WHERE r.session_id = s.id) AS run_count \
             FROM sessions s WHERE s.id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(session_summary)
        .transpose()
    }
}

// ---- Writes. Only the writer task calls these, inside its batch transaction. ----

pub(crate) async fn touch_session(
    tx: &mut Transaction<'_, Sqlite>,
    session: SessionId,
    client: Option<&str>,
    at: EpochMillis,
) -> Result<(), TraceError> {
    sqlx::query(
        "INSERT INTO sessions (id, client, started_at, last_activity_at) VALUES (?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
           client = COALESCE(sessions.client, excluded.client), \
           last_activity_at = MAX(sessions.last_activity_at, excluded.last_activity_at)",
    )
    .bind(session.to_string())
    .bind(client)
    .bind(at.millis())
    .bind(at.millis())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(crate) async fn end_session(
    tx: &mut Transaction<'_, Sqlite>,
    session: SessionId,
    at: EpochMillis,
) -> Result<(), TraceError> {
    sqlx::query("UPDATE sessions SET ended_at = ? WHERE id = ?")
        .bind(at.millis())
        .bind(session.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(crate) async fn persist_private(
    tx: &mut Transaction<'_, Sqlite>,
    session: SessionId,
    record: PrivacyRecord,
) -> Result<(), TraceError> {
    sqlx::query(
        "INSERT INTO sessions (id, started_at, last_activity_at, private_since, private_reason) \
         VALUES (?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET \
           private_since = COALESCE(sessions.private_since, excluded.private_since), \
           private_reason = COALESCE(sessions.private_reason, excluded.private_reason)",
    )
    .bind(session.to_string())
    .bind(record.since.millis())
    .bind(record.since.millis())
    .bind(record.since.millis())
    .bind(record.reason.as_str())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// A run's start row, as the writer stores it.
pub(crate) struct RunStartRow<'a> {
    pub(crate) id: RunId,
    pub(crate) session: SessionId,
    pub(crate) at: EpochMillis,
    pub(crate) code: Option<&'a str>,
    pub(crate) code_truncated: bool,
    pub(crate) host_ref: Option<&'a str>,
    pub(crate) idempotency_key: Option<&'a str>,
    pub(crate) host: Option<&'a Value>,
}

pub(crate) async fn insert_run(
    tx: &mut Transaction<'_, Sqlite>,
    run: RunStartRow<'_>,
) -> Result<(), TraceError> {
    sqlx::query(
        "INSERT INTO runs (id, session_id, started_at, status, code, code_truncated, host_ref, \
         idempotency_key, host) VALUES (?, ?, ?, 'running', ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO NOTHING",
    )
    .bind(run.id.to_string())
    .bind(run.session.to_string())
    .bind(run.at.millis())
    .bind(run.code)
    .bind(i64::from(run.code_truncated))
    .bind(run.host_ref)
    .bind(run.idempotency_key)
    .bind(run.host.map(Value::to_string))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// A run's end row, as the writer stores it.
pub(crate) struct RunEndRow<'a> {
    pub(crate) id: RunId,
    pub(crate) at: EpochMillis,
    pub(crate) status: RunStatus,
    pub(crate) error: Option<&'a ErrorRecord>,
    pub(crate) result: Option<&'a Value>,
    pub(crate) result_truncated: bool,
    pub(crate) withhold_code: Option<&'a str>,
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
    let error = run
        .error
        .map(serde_json::to_string)
        .transpose()
        .map_err(json_error)?;
    sqlx::query(
        "UPDATE runs SET ended_at = ?, status = ?, effects = ?, error = ?, result = ?, \
         result_truncated = ?, trace_gaps = ?, replay = ?, replay_until = ?, \
         code_withheld = COALESCE(?, code_withheld), \
         code = CASE WHEN ? IS NULL THEN code ELSE NULL END \
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
    .bind(run.withhold_code)
    .bind(run.withhold_code)
    .bind(&id)
    .execute(&mut **tx)
    .await?;
    Ok(effects)
}

/// An operation's start row, as the writer stores it (args already conservatively redacted).
pub(crate) struct OperationStartRow<'a> {
    pub(crate) id: OperationId,
    pub(crate) parent: Option<OperationId>,
    pub(crate) session: SessionId,
    pub(crate) name: &'a str,
    pub(crate) kind: OperationKind,
    pub(crate) effect: Effect,
    pub(crate) page: Option<u32>,
    pub(crate) at: EpochMillis,
    pub(crate) args: &'a Value,
    pub(crate) args_truncated: bool,
}

pub(crate) async fn insert_operation(
    tx: &mut Transaction<'_, Sqlite>,
    op: OperationStartRow<'_>,
) -> Result<(), TraceError> {
    let inserted = sqlx::query(
        "INSERT INTO operations (run_id, seq, parent_id, session_id, name, kind, attempt, effect, \
         page_id, started_at, args, args_truncated) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(run_id, seq) DO NOTHING",
    )
    .bind(op.id.run.to_string())
    .bind(i64::from(op.id.seq))
    .bind(op.parent.map(|parent| parent.to_string()))
    .bind(op.session.to_string())
    .bind(op.name)
    .bind(op.kind.as_str())
    .bind(op.kind.attempt().map(i64::from))
    .bind(op.effect.as_str())
    .bind(op.page.map(i64::from))
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
         page_id = COALESCE(?, page_id), tab_id = ?, url = ?, title = ? \
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
    .bind(page.tab_id)
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
    let operations = sqlx::query(
        "DELETE FROM operations WHERE run_id IN \
         (SELECT id FROM runs WHERE started_at < ? AND ended_at IS NOT NULL)",
    )
    .bind(cutoff)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    let runs = sqlx::query("DELETE FROM runs WHERE started_at < ? AND ended_at IS NOT NULL")
        .bind(cutoff)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    let sessions = sqlx::query(
        "DELETE FROM sessions WHERE last_activity_at < ? \
         AND NOT EXISTS (SELECT 1 FROM runs r WHERE r.session_id = sessions.id)",
    )
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
        sessions,
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
        session: parse_id(&row.try_get::<String, _>("session_id")?)?,
        status: run_status(&status).ok_or_else(|| corrupt("status", &status))?,
        effects: effects
            .as_deref()
            .map(|text| run_effects(text).ok_or_else(|| corrupt("effects", text)))
            .transpose()?,
        error: error_column(row, "error")?,
        host_ref: row.try_get("host_ref")?,
        idempotency_key: row.try_get("idempotency_key")?,
        started_at: EpochMillis::from_millis(row.try_get("started_at")?),
        ended_at: time_column(row, "ended_at")?,
        operation_count: count_column(row, "operation_count")?,
        trace_gaps: count_column(row, "trace_gaps")?,
    })
}

fn session_summary(row: &SqliteRow) -> Result<SessionSummary, TraceError> {
    let privacy = match time_column(row, "private_since")? {
        Some(since) => Some(PrivacyRecord {
            since,
            reason: row
                .try_get::<Option<String>, _>("private_reason")?
                .as_deref()
                .and_then(PrivateReason::parse)
                .unwrap_or(PrivateReason::Host),
        }),
        None => None,
    };
    Ok(SessionSummary {
        id: parse_id(&row.try_get::<String, _>("id")?)?,
        client: row.try_get("client")?,
        started_at: EpochMillis::from_millis(row.try_get("started_at")?),
        ended_at: time_column(row, "ended_at")?,
        last_activity_at: EpochMillis::from_millis(row.try_get("last_activity_at")?),
        run_count: count_column(row, "run_count")?,
        privacy,
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
        session: parse_id(&row.try_get::<String, _>("session_id")?)?,
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
            tab_id: row.try_get("tab_id")?,
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
