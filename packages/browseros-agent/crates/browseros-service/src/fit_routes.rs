//! The fit's routes: `POST /v1/tabs/{tabId}/fit`, `GET /v1/fits/{fitId}`,
//! `POST /v1/fits/{fitId}/navigate` and `POST /v1/fits/{fitId}/restore`.
//!
//! The owner's page polls `GET` for keyboard boxes and its browser bar, so it answers from the
//! fits' table and never waits on the browser. `POST .../fit` answers once the fit's plan ran:
//! 503 `display_unavailable` when Xvnc could not be resized, which leaves everything as it
//! was, and 503 `browser_unavailable` when the browser could not be reached. The browser bar is
//! the host's own call, so no lock refuses it.

use crate::api_error::ApiError;
use crate::service::ServiceState;
use crate::{query, wire};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use claw_sdk_api as api;
use std::sync::Arc;

pub(crate) async fn fit_tab(
    State(service): State<Arc<ServiceState>>,
    Path(tab): Path<String>,
    body: Bytes,
) -> Result<Json<api::Fit>, ApiError> {
    let tab = query::tab_id(&tab)?;
    let viewer = wire::viewer_request(&body)?;
    // Before the fit, so a reconcile whose listing missed the tab keeps it (`tab_stamps`).
    service.stamps.stamp(tab).await;
    let fit = service.fits.fit(tab, viewer).await?;
    Ok(Json(wire::fit(&fit)))
}

pub(crate) async fn get_fit(
    State(service): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::Fit>, ApiError> {
    let id = query::fit_id(&id)?;
    service
        .fits
        .view(id)
        .map(|fit| Json(wire::fit(&fit)))
        .ok_or_else(|| ApiError::not_found("no fit has this id"))
}

pub(crate) async fn navigate_fit(
    State(service): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<api::NavigateResult>, ApiError> {
    let id = query::fit_id(&id)?;
    let action = wire::navigate_request(&body)?;
    let outcome = service.fits.navigate(id, action).await?;
    Ok(Json(wire::navigate_result(outcome)))
}

pub(crate) async fn restore_fit(
    State(service): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::Fit>, ApiError> {
    let id = query::fit_id(&id)?;
    Ok(Json(wire::fit(&service.fits.restore(id).await?)))
}
