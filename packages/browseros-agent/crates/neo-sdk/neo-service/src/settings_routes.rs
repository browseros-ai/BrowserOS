//! The settings' routes: `GET`, `PUT` and `PATCH /v1/settings`.
//!
//! Only the host reaches them: they sit behind the host auth like every `/v1` route, on the host
//! listener, never on the agent's port, so a script or the model cannot change how Neo runs.
//! `PUT` replaces what the host set and `PATCH` changes only the settings it names; both
//! answer once the change is saved and live (`Settings::change`).

use crate::api_error::ApiError;
use crate::service::ServiceState;
use crate::settings::ChangeMode;
use crate::wire;
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use neo_sdk_api as api;
use std::sync::Arc;

pub(crate) async fn get_settings(
    State(service): State<Arc<ServiceState>>,
) -> Result<Json<api::SettingsState>, ApiError> {
    Ok(Json(wire::settings_state(&service.settings, None)?))
}

pub(crate) async fn put_settings(
    State(service): State<Arc<ServiceState>>,
    body: Bytes,
) -> Result<Json<api::SettingsState>, ApiError> {
    change(&service, &body, ChangeMode::Replace).await
}

pub(crate) async fn patch_settings(
    State(service): State<Arc<ServiceState>>,
    body: Bytes,
) -> Result<Json<api::SettingsState>, ApiError> {
    change(&service, &body, ChangeMode::Merge).await
}

async fn change(
    service: &ServiceState,
    body: &[u8],
    mode: ChangeMode,
) -> Result<Json<api::SettingsState>, ApiError> {
    let (values, unknown) = wire::settings_request(body)?;
    for name in &unknown {
        tracing::warn!(
            name,
            "a host set a setting this server does not know; skipped"
        );
    }
    service
        .settings
        .change(values, mode)
        .await
        .map_err(|error| ApiError::internal(format!("saving the settings: {error}")))?;
    Ok(Json(wire::settings_state(
        &service.settings,
        Some(unknown),
    )?))
}
