//! `scheduled_jobs` and `scheduled_job_runs`. The server only stores them (the extension runs the
//! jobs); run upserts prune each job to its newest 15 runs.

use super::{Database, DbError, DbResult, STUB};
use serde_json::Value;

/// Runs kept per job after every run upsert (TS parity).
pub const RUNS_KEPT_PER_JOB: usize = 15;

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledJobRow {
    pub id: String,
    pub name: String,
    pub query: String,
    pub schedule_type: String,
    pub schedule_time: Option<String>,
    pub schedule_interval: Option<i64>,
    pub enabled: bool,
    pub provider_id: Option<String>,
    pub last_run_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledRunRow {
    pub id: String,
    pub job_id: String,
    pub status: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub result: Option<String>,
    pub final_result: Option<String>,
    pub execution_log: Option<String>,
    pub tool_calls: Option<Value>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpsertJob {
    pub name: String,
    pub query: String,
    pub schedule_type: String,
    pub schedule_time: Option<String>,
    pub schedule_interval: Option<i64>,
    pub enabled: bool,
    pub provider_id: Option<String>,
    pub last_run_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpsertRun {
    pub job_id: String,
    pub status: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub result: Option<String>,
    pub final_result: Option<String>,
    pub execution_log: Option<String>,
    pub tool_calls: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScheduleStore {
    db: Database,
}

impl ScheduleStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn list_jobs(&self) -> DbResult<Vec<ScheduledJobRow>> {
        let _ = &self.db;
        Err(DbError::Unavailable(STUB))
    }

    pub async fn get_job(&self, id: &str) -> DbResult<Option<ScheduledJobRow>> {
        let _ = id;
        Err(DbError::Unavailable(STUB))
    }

    pub async fn upsert_job(&self, id: &str, input: UpsertJob) -> DbResult<ScheduledJobRow> {
        let _ = (id, input);
        Err(DbError::Unavailable(STUB))
    }

    pub async fn delete_job(&self, id: &str) -> DbResult<bool> {
        let _ = id;
        Err(DbError::Unavailable(STUB))
    }

    pub async fn list_runs(&self) -> DbResult<Vec<ScheduledRunRow>> {
        Err(DbError::Unavailable(STUB))
    }

    pub async fn get_run(&self, id: &str) -> DbResult<Option<ScheduledRunRow>> {
        let _ = id;
        Err(DbError::Unavailable(STUB))
    }

    /// Upserts then prunes the job to [`RUNS_KEPT_PER_JOB`], in one transaction.
    pub async fn upsert_run(&self, id: &str, input: UpsertRun) -> DbResult<ScheduledRunRow> {
        let _ = (id, input);
        Err(DbError::Unavailable(STUB))
    }

    pub async fn delete_run(&self, id: &str) -> DbResult<bool> {
        let _ = id;
        Err(DbError::Unavailable(STUB))
    }
}
