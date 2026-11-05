//! The takeover parts' routes. For now the lock: `POST /v1/locks`, `GET /v1/locks/{lockId}` and
//! `DELETE /v1/locks/{lockId}`.
//!
//! The routes write the same `Locks` the app's lock policy reads (`Service::locks`), so a lock
//! refuses the agent from the moment it is in the table, before `POST` answers. `POST` answers
//! once the tab's in-flight writes settled, or after 2 s. A takeover's lock is never released
//! here: `DELETE` on it is 409 `takeover_lock`, and the host releases the takeover instead
//! (decision 29).

use crate::api_error::ApiError;
use crate::service::ServiceState;
use crate::{query, wire};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use browseros_takeover::LockBinding;
use claw_sdk_api as api;
use std::sync::Arc;

pub(crate) async fn create_lock(
    State(service): State<Arc<ServiceState>>,
    body: Bytes,
) -> Result<Json<api::Lock>, ApiError> {
    let (tab, reads, ttl) = wire::lock_request(&body)?;
    // A lock on a tab that is not open would refuse nothing now, and could refuse a tab that
    // later opens with the same id: Chromium numbers tabs from 1 again after a restart.
    service.browser.page(tab).await?;
    let lock = service.locks.lock(tab, reads, ttl, LockBinding::Host).await;
    Ok(Json(wire::lock(&lock)))
}

pub(crate) async fn get_lock(
    State(service): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::Lock>, ApiError> {
    let id = query::lock_id(&id)?;
    service
        .locks
        .get(id)
        .map(|lock| Json(wire::lock(&lock)))
        .ok_or_else(|| ApiError::not_found("no lock has this id"))
}

pub(crate) async fn release_lock(
    State(service): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::Lock>, ApiError> {
    let id = query::lock_id(&id)?;
    Ok(Json(wire::lock(&service.locks.unlock_host(id)?)))
}
