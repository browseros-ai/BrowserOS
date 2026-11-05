//! The v1 takeover flow's routes: `POST /v1/takeovers`, `GET /v1/takeovers/{takeoverId}` and
//! `POST /v1/takeovers/{takeoverId}/release`, over `browseros_takeover::Takeovers`.
//!
//! zuzu starts a takeover when its own poll sees the owner open the link, and opens the answer's
//! `display` (`/v1/display?mode=control&release=<id>`) for the owner through its relay. It then
//! long-polls `GET ...?wait=60&state=active` until the takeover reads `released`: by its own
//! `POST .../release` when the owner is done, by the TTL 10 s before `expiresAt`, or by the tab
//! closing. Start answers once the tab is private, locked, held off from spectators, armed and
//! fitted. A start not done by the time its TTL release is due, 10 s before `expiresAt`, gives
//! back what it took and answers the contract's `503 browser_unavailable`, never an active
//! takeover. Release answers once the agent's lock lifted, the last step of the give-back.
//!
//! The owner expects this flow to change (D13). These routes and `Takeovers` are replaced
//! together; the parts' own routes stay as they are.

use crate::api_error::ApiError;
use crate::service::ServiceState;
use crate::{query, wire};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use claw_sdk_api as api;
use std::sync::Arc;
use tokio::time::Instant;

pub(crate) async fn start_takeover(
    State(service): State<Arc<ServiceState>>,
    body: Bytes,
) -> Result<Json<api::Takeover>, ApiError> {
    let request = wire::takeover_request(&body)?;
    // Before the start makes the tab private, locks it and fits it, so a reconcile whose listing
    // missed the tab keeps all three (`tab_stamps`).
    service.stamps.stamp(request.tab).await;
    let takeover = service.takeovers.start(request).await?;
    Ok(Json(wire::takeover(&takeover)))
}

pub(crate) async fn get_takeover(
    State(service): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<Json<api::Takeover>, ApiError> {
    let id = query::takeover_id(&id)?;
    let wait = query::takeover(raw.as_deref())?;
    service
        .takeovers
        .wait(id, Instant::now() + wait)
        .await
        .map(|takeover| Json(wire::takeover(&takeover)))
        .ok_or_else(|| ApiError::not_found("no takeover has this id"))
}

pub(crate) async fn release_takeover(
    State(service): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<api::Takeover>, ApiError> {
    let id = query::takeover_id(&id)?;
    let reason = wire::release_request(&body)?;
    Ok(Json(wire::takeover(
        &service.takeovers.release(id, reason).await?,
    )))
}
