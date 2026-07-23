//! Audit storage + retention endpoints and the retention sweep. The sweep is a
//! cross-service flow (audit db + screenshot files + recordings), so it lives
//! in the api/http layer rather than a single-service module: it deletes aged
//! audit rows, unlinks their screenshots, prunes recordings through the
//! existing cascade-correct retention, then reclaims disk.

use super::{error, internal};
use crate::{
    AppState,
    clock::now_epoch_ms,
    db::DATABASE_FILENAME,
    error::{AppResult, CanonicalError, RequestId},
    services::audit_settings::AuditRetention,
};
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
};
use claw_api::models::{
    AuditCleanupResult, AuditRetention as ApiRetention, AuditRetentionMode, AuditStorageState,
    AuditStorageUsage, SetAuditRetentionRequest,
};
use std::path::Path;

const MS_PER_DAY: i64 = 86_400_000;

/// GET /api/v1/audit/storage
pub(super) async fn storage(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<AuditStorageState>, CanonicalError> {
    let usage = audit_usage(&state)
        .await
        .map_err(|source| internal(&request_id, source))?;
    let retention = to_api_retention(state.audit_settings.get().await);
    Ok(Json(AuditStorageState::new(usage, retention)))
}

/// PUT /api/v1/audit/retention
pub(super) async fn set_retention(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    payload: Result<Json<SetAuditRetentionRequest>, JsonRejection>,
) -> Result<Json<ApiRetention>, CanonicalError> {
    let Json(payload) = payload.map_err(|_| {
        error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "expected an audit retention policy",
        )
    })?;
    let policy = policy_from_request(&payload).map_err(|message| {
        error(&request_id, StatusCode::BAD_REQUEST, "invalid_request", message)
    })?;
    let saved = state
        .audit_settings
        .set(policy)
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(to_api_retention(saved)))
}

/// POST /api/v1/audit/cleanup — apply the current policy immediately.
pub(super) async fn cleanup(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<AuditCleanupResult>, CanonicalError> {
    let report = sweep_audit_retention(&state, now_epoch_ms())
        .await
        .map_err(|source| internal(&request_id, source))?;
    let usage = audit_usage(&state)
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(AuditCleanupResult::new(
        report.sessions_deleted,
        report.screenshots_deleted,
        report.recordings_deleted,
        report.bytes_reclaimed,
        usage,
    )))
}

/// What one retention sweep removed.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SweepReport {
    pub sessions_deleted: i64,
    pub screenshots_deleted: i64,
    pub recordings_deleted: i64,
    pub bytes_reclaimed: i64,
}

/// Applies the current retention policy: delete aged audit rows + their
/// screenshots, prune recordings through the existing retention (which also
/// runs its orphan cleanup), then reclaim disk. Best-effort file unlinks; DB
/// deletes are transactional. Called by the scheduler and the cleanup endpoint.
pub(crate) async fn sweep_audit_retention(state: &AppState, now: i64) -> AppResult<SweepReport> {
    let policy = state.audit_settings.get().await;
    let db_before = db_file_bytes(state).await;
    let mut report = SweepReport::default();
    let mut screenshot_bytes_freed = 0i64;

    if let Some(days) = policy.days() {
        let cutoff = now.saturating_sub(i64::from(days).saturating_mul(MS_PER_DAY));
        let sessions = state.audit_log.sessions_older_than(cutoff).await?;
        if !sessions.is_empty() {
            let shots = state
                .audit_log
                .screenshot_dispatches_for_sessions(&sessions)
                .await?;
            state.audit_log.delete_sessions(&sessions).await?;
            report.sessions_deleted = i64::try_from(sessions.len()).unwrap_or(i64::MAX);
            for (session_id, dispatch_id) in &shots {
                if let Some(bytes) = remove_screenshot(state, session_id, *dispatch_id).await {
                    report.screenshots_deleted += 1;
                    screenshot_bytes_freed = screenshot_bytes_freed.saturating_add(bytes);
                }
            }
        }
    }

    // Recordings: drive the existing cascade-correct retention with the same
    // window (u64::MAX for keep-forever prunes only orphans, never claimed
    // streams). This unifies recording retention under the user policy.
    let recording_days = policy.days().map_or(u64::MAX, u64::from);
    let recordings = state.recordings.sweep_retention(recording_days, now).await?;
    report.recordings_deleted = i64::try_from(recordings.recordings_deleted).unwrap_or(i64::MAX);

    state.audit_log.reclaim_disk().await?;
    let db_after = db_file_bytes(state).await;
    report.bytes_reclaimed = db_before.saturating_sub(db_after).max(0) + screenshot_bytes_freed;
    Ok(report)
}

async fn audit_usage(state: &AppState) -> AppResult<AuditStorageUsage> {
    let recording_bytes = state.recordings.recording_bytes_total().await?;
    let screenshot_bytes =
        dir_file_bytes(&state.config.browserclaw_dir.join("screenshots")).await;
    Ok(AuditStorageUsage::new(
        recording_bytes,
        screenshot_bytes,
        recording_bytes.saturating_add(screenshot_bytes),
    ))
}

/// Unlinks a screenshot file (nested path, then legacy flat), returning the
/// freed bytes when a file was removed.
async fn remove_screenshot(state: &AppState, session_id: &str, dispatch_id: i64) -> Option<i64> {
    for path in [
        state.screenshots.path_for(session_id, dispatch_id),
        state.screenshots.legacy_path_for(dispatch_id),
    ] {
        let size = tokio::fs::metadata(&path)
            .await
            .map(|meta| i64::try_from(meta.len()).unwrap_or(i64::MAX))
            .ok();
        match (size, tokio::fs::remove_file(&path).await) {
            (Some(bytes), Ok(())) => return Some(bytes),
            _ => continue,
        }
    }
    None
}

async fn db_file_bytes(state: &AppState) -> i64 {
    tokio::fs::metadata(state.config.browserclaw_dir.join(DATABASE_FILENAME))
        .await
        .map(|meta| i64::try_from(meta.len()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Sum of file sizes directly in `dir` and one level of subdirectories (the
/// nested `s-<session>/` screenshot layout). Best-effort; missing dir = 0.
async fn dir_file_bytes(dir: &Path) -> i64 {
    let mut total = 0i64;
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return 0;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(file_type) = entry.file_type().await else {
            continue;
        };
        if file_type.is_file() {
            if let Ok(meta) = entry.metadata().await {
                total = total.saturating_add(i64::try_from(meta.len()).unwrap_or(i64::MAX));
            }
        } else if file_type.is_dir() {
            total = total.saturating_add(subdir_file_bytes(&entry.path()).await);
        }
    }
    total
}

async fn subdir_file_bytes(dir: &Path) -> i64 {
    let mut total = 0i64;
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return 0;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Ok(meta) = entry.metadata().await
            && meta.is_file()
        {
            total = total.saturating_add(i64::try_from(meta.len()).unwrap_or(i64::MAX));
        }
    }
    total
}

fn to_api_retention(policy: AuditRetention) -> ApiRetention {
    match policy {
        AuditRetention::KeepForever => ApiRetention::new(AuditRetentionMode::KeepForever),
        AuditRetention::DeleteAfterDays { days } => {
            let mut retention = ApiRetention::new(AuditRetentionMode::DeleteAfterDays);
            retention.days = Some(i32::try_from(days).unwrap_or(i32::MAX));
            retention
        }
    }
}

fn policy_from_request(
    request: &SetAuditRetentionRequest,
) -> Result<AuditRetention, &'static str> {
    match request.mode {
        AuditRetentionMode::KeepForever => Ok(AuditRetention::KeepForever),
        AuditRetentionMode::DeleteAfterDays => {
            let days = request
                .days
                .ok_or("days is required when mode is deleteAfterDays")?;
            let days = u32::try_from(days).map_err(|_| "days must be a positive integer")?;
            if days < 1 {
                return Err("days must be at least 1");
            }
            Ok(AuditRetention::DeleteAfterDays { days })
        }
    }
}
