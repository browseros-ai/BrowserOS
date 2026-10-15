//! Schedule and execution-history persistence; the extension owns actual alarm execution.
//! Cadence validation gates only new/changed cadence so maintenance writes can still update
//! legacy invalid jobs. Every run write delegates pruning to the store's write transaction.

use axum::{
    Json,
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use http::StatusCode;
use server_api::models as api;

use super::wire::crud::{self, CrudJson};
use crate::{AppState, db};

pub async fn list_jobs(State(state): State<AppState>) -> Response {
    match state.schedules.list_jobs().await {
        Ok(rows) => Json(api::ScheduledJobList::new(
            rows.into_iter().map(crud::job).collect(),
        ))
        .into_response(),
        Err(error) => crud::db_error(error),
    }
}

pub async fn get_job(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.schedules.get_job(&id).await {
        Ok(Some(row)) => Json(api::ScheduledJobResponse::new(crud::job(row))).into_response(),
        Ok(None) => crud::not_found("Unknown scheduled job"),
        Err(error) => crud::db_error(error),
    }
}

pub async fn upsert_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
    CrudJson(input): CrudJson<api::UpsertScheduledJobRequest>,
) -> Response {
    let existing = match state.schedules.get_job(&id).await {
        Ok(row) => row,
        Err(error) => return crud::db_error(error),
    };
    if cadence_changed(existing.as_ref(), &input)
        && let Some(problem) = cadence_problem(&input)
    {
        return super::errors::error_string(StatusCode::BAD_REQUEST, &problem);
    }
    let input = db::UpsertJob {
        name: input.name,
        query: input.query,
        schedule_type: schedule_type(input.schedule_type).to_owned(),
        schedule_time: input.schedule_time,
        schedule_interval: input
            .schedule_interval
            .map(|value| value.map(crud::store_integer)),
        enabled: input.enabled,
        provider_id: input.provider_id,
        last_run_at: input
            .last_run_at
            .map(|value| value.map(crud::store_integer)),
        created_at: input.created_at.map(crud::store_integer),
    };
    match state.schedules.upsert_job(&id, input).await {
        Ok(row) => Json(api::ScheduledJobResponse::new(crud::job(row))).into_response(),
        // FK errors intentionally remain the server's 500 envelope, not a new 400 dialect.
        Err(error) => crud::db_error(error),
    }
}

pub async fn delete_job(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    crud::deleted(
        state.schedules.delete_job(&id).await,
        "Unknown scheduled job",
    )
}

pub async fn list_runs(State(state): State<AppState>) -> Response {
    match state.schedules.list_runs().await {
        // startedAt DESC comes from the store, including SQLite's ordering of tied rows.
        Ok(rows) => {
            match rows
                .into_iter()
                .map(crud::run)
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(rows) => Json(api::ScheduledJobRunList::new(rows)).into_response(),
                Err(response) => response.into_response(),
            }
        }
        Err(error) => crud::db_error(error),
    }
}

pub async fn get_run(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.schedules.get_run(&id).await {
        Ok(Some(row)) => match crud::run(row) {
            Ok(row) => Json(api::ScheduledJobRunResponse::new(row)).into_response(),
            Err(response) => response.into_response(),
        },
        Ok(None) => crud::not_found("Unknown run"),
        Err(error) => crud::db_error(error),
    }
}

pub async fn upsert_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
    CrudJson(input): CrudJson<api::UpsertScheduledJobRunRequest>,
) -> Response {
    let tool_calls = match input
        .tool_calls
        .map(|calls| calls.map(serde_json::to_value).transpose())
        .transpose()
    {
        Ok(calls) => calls,
        Err(error) => return crud::internal_error(&error.to_string()),
    };
    let input = db::UpsertRun {
        job_id: input.job_id,
        status: match input.status {
            api::UpsertScheduledJobRunRequestStatus::Running => "running",
            api::UpsertScheduledJobRunRequestStatus::Completed => "completed",
            api::UpsertScheduledJobRunRequestStatus::Failed => "failed",
        }
        .to_owned(),
        started_at: crud::store_integer(input.started_at),
        completed_at: input
            .completed_at
            .map(|value| value.map(crud::store_integer)),
        result: input.result,
        final_result: input.final_result,
        execution_log: input.execution_log,
        tool_calls,
        error: input.error,
        created_at: input.created_at.map(crud::store_integer),
    };
    // The store returns the saved row even if this write prunes that same old run. Pruning
    // only on insert would let repeated completion writes grow a job past the 15-row cap.
    match state.schedules.upsert_run(&id, input).await {
        Ok(row) => match crud::run(row) {
            Ok(row) => Json(api::ScheduledJobRunResponse::new(row)).into_response(),
            Err(response) => response.into_response(),
        },
        Err(error) => crud::db_error(error),
    }
}

pub async fn delete_run(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    crud::deleted(state.schedules.delete_run(&id).await, "Unknown run")
}

fn schedule_type(kind: api::UpsertScheduledJobRequestScheduleType) -> &'static str {
    match kind {
        api::UpsertScheduledJobRequestScheduleType::Daily => "daily",
        api::UpsertScheduledJobRequestScheduleType::Hourly => "hourly",
        api::UpsertScheduledJobRequestScheduleType::Minutes => "minutes",
    }
}

fn cadence_changed(
    existing: Option<&db::ScheduledJobRow>,
    next: &api::UpsertScheduledJobRequest,
) -> bool {
    existing.is_none_or(|row| {
        row.schedule_type != schedule_type(next.schedule_type)
            || row.schedule_time.as_ref() != next.schedule_time.as_ref().and_then(Option::as_ref)
            || row.schedule_interval.map(|value| value as f64) != next.schedule_interval.flatten()
    })
}

fn cadence_problem(input: &api::UpsertScheduledJobRequest) -> Option<String> {
    if input.schedule_type == api::UpsertScheduledJobRequestScheduleType::Daily {
        let valid = input
            .schedule_time
            .as_ref()
            .and_then(Option::as_deref)
            .is_some_and(|time| {
                let time = time.as_bytes();
                time.len() == 5
                    && time[2] == b':'
                    && time
                        .iter()
                        .enumerate()
                        .all(|(i, b)| i == 2 || b.is_ascii_digit())
                    && (time[0] == b'0' || time[0] == b'1' || (time[0] == b'2' && time[1] <= b'3'))
                    && time[3] <= b'5'
            });
        return (!valid).then(|| "daily jobs require scheduleTime as HH:MM (24-hour)".to_owned());
    }
    let valid = input
        .schedule_interval
        .flatten()
        .is_some_and(|value| value.fract() == 0.0 && (1.0..=60.0).contains(&value));
    (!valid).then(|| {
        format!(
            "{} jobs require an integer scheduleInterval between 1 and 60",
            schedule_type(input.schedule_type)
        )
    })
}
