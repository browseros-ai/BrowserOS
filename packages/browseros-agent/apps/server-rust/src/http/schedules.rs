//! `/scheduled-jobs*` and `/scheduled-job-runs*`: stores only (the extension runs jobs); cadence is
//! validated only when it changes; run upserts prune to 15 per job. Owned by P9.

use axum::response::Response;

pub async fn list_jobs() -> Response {
    super::not_implemented()
}

pub async fn get_job() -> Response {
    super::not_implemented()
}

pub async fn upsert_job() -> Response {
    super::not_implemented()
}

pub async fn delete_job() -> Response {
    super::not_implemented()
}

pub async fn list_runs() -> Response {
    super::not_implemented()
}

pub async fn get_run() -> Response {
    super::not_implemented()
}

pub async fn upsert_run() -> Response {
    super::not_implemented()
}

pub async fn delete_run() -> Response {
    super::not_implemented()
}
