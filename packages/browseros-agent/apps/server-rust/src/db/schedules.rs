//! `scheduled_jobs` and `scheduled_job_runs`. The server only stores them (the extension runs the
//! jobs); run upserts prune each job to its newest 15 runs. A port of
//! `apps/server/src/lib/schedules/{schedule-store,run-store}.ts`.
//!
//! Upserts follow drizzle's `onConflictDoUpdate({ set: { ...row } })`: a key absent from the
//! request leaves the stored column alone, an explicit `null` clears it, `created_at` is only
//! written on insert. Hence the `Option<Option<_>>` fields below.

use super::{
    Database, DbError, DbResult,
    decode::Columns,
    entities::{scheduled_job_runs as runs, scheduled_jobs as jobs},
};
use crate::js;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, FromQueryResult,
    QueryFilter, QueryOrder, QueryResult, QuerySelect, sea_query::OnConflict,
};
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
    /// Parsed `ToolCallExecution[]`; `None` also when the stored text is unreadable (logged).
    pub tool_calls: Option<Value>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `PUT /scheduled-jobs/{id}` body after validation. `None` = absent (unchanged on update),
/// `Some(None)` = `null` (cleared).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpsertJob {
    pub name: String,
    pub query: String,
    pub schedule_type: String,
    pub schedule_time: Option<Option<String>>,
    pub schedule_interval: Option<Option<i64>>,
    /// Absent inserts the column default (`true`).
    pub enabled: Option<bool>,
    pub provider_id: Option<Option<String>>,
    pub last_run_at: Option<Option<i64>>,
    pub created_at: Option<i64>,
}

/// `PUT /scheduled-job-runs/{id}` body after validation; same field states as [`UpsertJob`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpsertRun {
    pub job_id: String,
    pub status: String,
    pub started_at: i64,
    pub completed_at: Option<Option<i64>>,
    pub result: Option<Option<String>>,
    pub final_result: Option<Option<String>>,
    pub execution_log: Option<Option<String>>,
    /// Stored as its JSON text.
    pub tool_calls: Option<Option<Value>>,
    pub error: Option<Option<String>>,
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ScheduleStore {
    db: Database,
}

impl ScheduleStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Rowid order (no `ORDER BY`, as in TS).
    pub async fn list_jobs(&self) -> DbResult<Vec<ScheduledJobRow>> {
        Ok(jobs::Entity::find()
            .into_model::<ScheduledJobRow>()
            .all(self.db.connection())
            .await?)
    }

    pub async fn get_job(&self, id: &str) -> DbResult<Option<ScheduledJobRow>> {
        Ok(jobs::Entity::find_by_id(id.to_owned())
            .into_model::<ScheduledJobRow>()
            .one(self.db.connection())
            .await?)
    }

    /// Fails (a foreign key error, 500 at the route) when `provider_id` names no provider.
    pub async fn upsert_job(&self, id: &str, input: UpsertJob) -> DbResult<ScheduledJobRow> {
        let now = js::now_ms();
        let mut row = jobs::ActiveModel {
            id: Set(id.to_owned()),
            name: Set(input.name),
            query: Set(input.query),
            schedule_type: Set(input.schedule_type),
            created_at: Set(input.created_at.unwrap_or(now)),
            updated_at: Set(now),
            ..Default::default()
        };
        let mut updated = vec![
            jobs::Column::Name,
            jobs::Column::Query,
            jobs::Column::ScheduleType,
            jobs::Column::UpdatedAt,
        ];
        if let Some(value) = input.schedule_time {
            row.schedule_time = Set(value);
            updated.push(jobs::Column::ScheduleTime);
        }
        if let Some(value) = input.schedule_interval {
            row.schedule_interval = Set(value);
            updated.push(jobs::Column::ScheduleInterval);
        }
        if let Some(value) = input.enabled {
            row.enabled = Set(value);
            updated.push(jobs::Column::Enabled);
        }
        if let Some(value) = input.provider_id {
            row.provider_id = Set(value);
            updated.push(jobs::Column::ProviderId);
        }
        if let Some(value) = input.last_run_at {
            row.last_run_at = Set(value);
            updated.push(jobs::Column::LastRunAt);
        }

        let transaction = self.db.begin_write().await?;
        jobs::Entity::insert(row)
            .on_conflict(
                OnConflict::column(jobs::Column::Id)
                    .update_columns(updated)
                    .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        let saved = jobs::Entity::find_by_id(id.to_owned())
            .into_model::<ScheduledJobRow>()
            .one(&transaction)
            .await?
            .ok_or_else(|| DbError::Query(format!("scheduled job {id} vanished after upsert")))?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Deletes the job; its runs go with it (ON DELETE CASCADE).
    pub async fn delete_job(&self, id: &str) -> DbResult<bool> {
        let transaction = self.db.begin_write().await?;
        let deleted = jobs::Entity::delete_by_id(id.to_owned())
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(deleted.rows_affected > 0)
    }

    /// Newest `started_at` first.
    pub async fn list_runs(&self) -> DbResult<Vec<ScheduledRunRow>> {
        Ok(runs::Entity::find()
            .order_by_desc(runs::Column::StartedAt)
            .into_model::<ScheduledRunRow>()
            .all(self.db.connection())
            .await?)
    }

    pub async fn get_run(&self, id: &str) -> DbResult<Option<ScheduledRunRow>> {
        Ok(runs::Entity::find_by_id(id.to_owned())
            .into_model::<ScheduledRunRow>()
            .one(self.db.connection())
            .await?)
    }

    /// Upserts then prunes the job to [`RUNS_KEPT_PER_JOB`], in one transaction. Fails (500 at
    /// the route) when `job_id` names no job.
    pub async fn upsert_run(&self, id: &str, input: UpsertRun) -> DbResult<ScheduledRunRow> {
        let now = js::now_ms();
        let mut row = runs::ActiveModel {
            id: Set(id.to_owned()),
            job_id: Set(input.job_id),
            status: Set(input.status),
            started_at: Set(input.started_at),
            created_at: Set(input.created_at.unwrap_or(now)),
            updated_at: Set(now),
            ..Default::default()
        };
        let mut updated = vec![
            runs::Column::JobId,
            runs::Column::Status,
            runs::Column::StartedAt,
            runs::Column::UpdatedAt,
        ];
        if let Some(value) = input.completed_at {
            row.completed_at = Set(value);
            updated.push(runs::Column::CompletedAt);
        }
        for (column, value, slot) in [
            (runs::Column::Result, input.result, &mut row.result),
            (
                runs::Column::FinalResult,
                input.final_result,
                &mut row.final_result,
            ),
            (
                runs::Column::ExecutionLog,
                input.execution_log,
                &mut row.execution_log,
            ),
            (runs::Column::Error, input.error, &mut row.error),
        ] {
            if let Some(value) = value {
                *slot = Set(value);
                updated.push(column);
            }
        }
        if let Some(value) = input.tool_calls {
            let text = value
                .map(|calls| serde_json::to_string(&calls))
                .transpose()
                .map_err(|error| DbError::Query(format!("run tool calls: {error}")))?;
            row.tool_calls = Set(text);
            updated.push(runs::Column::ToolCalls);
        }

        let transaction = self.db.begin_write().await?;
        runs::Entity::insert(row)
            .on_conflict(
                OnConflict::column(runs::Column::Id)
                    .update_columns(updated)
                    .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        let saved = runs::Entity::find_by_id(id.to_owned())
            .into_model::<ScheduledRunRow>()
            .one(&transaction)
            .await?
            .ok_or_else(|| DbError::Query(format!("run {id} vanished after upsert")))?;
        prune(&transaction, &saved.job_id).await?;
        transaction.commit().await?;
        Ok(saved)
    }

    pub async fn delete_run(&self, id: &str) -> DbResult<bool> {
        let transaction = self.db.begin_write().await?;
        let deleted = runs::Entity::delete_by_id(id.to_owned())
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(deleted.rows_affected > 0)
    }
}

/// Drops all but the newest [`RUNS_KEPT_PER_JOB`] runs of a job (newest by `started_at`).
async fn prune(connection: &impl ConnectionTrait, job_id: &str) -> Result<u64, DbErr> {
    let stale: Vec<String> = runs::Entity::find()
        .select_only()
        .column(runs::Column::Id)
        .filter(runs::Column::JobId.eq(job_id))
        .order_by_desc(runs::Column::StartedAt)
        .into_tuple::<String>()
        .all(connection)
        .await?
        .into_iter()
        .skip(RUNS_KEPT_PER_JOB)
        .collect();
    if stale.is_empty() {
        return Ok(0);
    }
    let deleted = runs::Entity::delete_many()
        .filter(runs::Column::Id.is_in(stale))
        .exec(connection)
        .await?;
    Ok(deleted.rows_affected)
}

impl FromQueryResult for ScheduledJobRow {
    fn from_query_result(row: &QueryResult, prefix: &str) -> Result<Self, DbErr> {
        let columns = Columns::new(row, prefix);
        Ok(Self {
            id: columns.text("id")?,
            name: columns.text("name")?,
            query: columns.text("query")?,
            schedule_type: columns.text("schedule_type")?,
            schedule_time: columns.opt_text("schedule_time")?,
            schedule_interval: columns.opt_int("schedule_interval")?,
            enabled: columns.boolean("enabled")?,
            provider_id: columns.opt_text("provider_id")?,
            last_run_at: columns.opt_int("last_run_at")?,
            created_at: columns.int("created_at")?,
            updated_at: columns.int("updated_at")?,
        })
    }
}

impl FromQueryResult for ScheduledRunRow {
    fn from_query_result(row: &QueryResult, prefix: &str) -> Result<Self, DbErr> {
        let columns = Columns::new(row, prefix);
        let id = columns.text("id")?;
        let tool_calls =
            columns
                .opt_text("tool_calls")?
                .and_then(|text| match serde_json::from_str(&text) {
                    Ok(calls) => Some(calls),
                    Err(error) => {
                        tracing::warn!(run_id = %id, %error, "ignoring unreadable run tool calls");
                        None
                    }
                });
        Ok(Self {
            job_id: columns.text("job_id")?,
            status: columns.text("status")?,
            started_at: columns.int("started_at")?,
            completed_at: columns.opt_int("completed_at")?,
            result: columns.opt_text("result")?,
            final_result: columns.opt_text("final_result")?,
            execution_log: columns.opt_text("execution_log")?,
            tool_calls,
            error: columns.opt_text("error")?,
            created_at: columns.int("created_at")?,
            updated_at: columns.int("updated_at")?,
            id,
        })
    }
}
