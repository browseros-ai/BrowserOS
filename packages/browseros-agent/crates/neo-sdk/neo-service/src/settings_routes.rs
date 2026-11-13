//! The settings' routes: `GET`, `PUT` and `PATCH /v1/settings`.
//!
//! Only the host reaches them: they sit behind the host auth like every `/v1` route, on the host
//! listener, never on the agent's port, so a script or the model cannot change how Neo runs. On
//! the development TCP listener (`CLAW_HOST_TCP`) the token is the only check, and a script that
//! got the token could send a request from a page of the browser it drives, so these routes also
//! refuse every request a web page sends, as the agent port does (`request_hygiene` in the app).
//! `PUT` replaces what the host set and `PATCH` changes only the settings it names; both
//! answer once the change is saved and live (`Settings::change`).

use crate::api_error::ApiError;
use crate::service::ServiceState;
use crate::settings::ChangeMode;
use crate::wire::{self, SettingsBody};
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use neo_sdk_api as api;
use std::sync::Arc;

pub(crate) async fn get_settings(
    State(service): State<Arc<ServiceState>>,
    headers: HeaderMap,
) -> Result<Json<api::SettingsState>, ApiError> {
    refuse_pages(&headers)?;
    Ok(Json(wire::settings_state(&service.settings, None)?))
}

pub(crate) async fn put_settings(
    State(service): State<Arc<ServiceState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<api::SettingsState>, ApiError> {
    refuse_pages(&headers)?;
    change(&service, &body, ChangeMode::Replace).await
}

pub(crate) async fn patch_settings(
    State(service): State<Arc<ServiceState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<api::SettingsState>, ApiError> {
    refuse_pages(&headers)?;
    change(&service, &body, ChangeMode::Merge).await
}

/// 403 for a request a web page sent: every browser fetch, form and navigation carries `Origin`
/// or `Sec-Fetch-Site` (a same-origin one too), and the host's own clients send neither.
fn refuse_pages(headers: &HeaderMap) -> Result<(), ApiError> {
    if headers.contains_key(header::ORIGIN) || headers.contains_key("sec-fetch-site") {
        return Err(ApiError::forbidden(
            "a request from a web page is never served here",
        ));
    }
    Ok(())
}

async fn change(
    service: &ServiceState,
    body: &[u8],
    mode: ChangeMode,
) -> Result<Json<api::SettingsState>, ApiError> {
    let SettingsBody {
        values, unknown, ..
    } = wire::settings_request(body)?;
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
