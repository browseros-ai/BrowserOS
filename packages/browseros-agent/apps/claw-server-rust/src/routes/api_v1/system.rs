use super::error;
use crate::{AppState, error::CanonicalError, error::RequestId};
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
};
use claw_api::models::{
    HealthResponse, ShutdownResponse, SystemCapabilities, SystemInfo, TelemetryState,
    UpdateTelemetryRequest, system_capabilities::RecordingIngestVersion,
};

// The contract's health is pure liveness: `status` is a single-variant
// enum, so a reachable server can only answer "ok".
pub(super) async fn health() -> Json<HealthResponse> {
    Json(HealthResponse::default())
}

// Only signals; the runtime's shutdown owner drains sessions and stops
// the process.
pub(super) async fn shutdown(State(state): State<AppState>) -> Json<ShutdownResponse> {
    state.shutdown.request();
    Json(ShutdownResponse::default())
}

pub(super) async fn info(State(state): State<AppState>) -> Json<SystemInfo> {
    let mut info = SystemInfo::new(
        "BrowserClaw".to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
        state.config.local_server_url(),
    );
    let mut capabilities = SystemCapabilities::new();
    capabilities.recording_ingest_version = Some(RecordingIngestVersion::Variant2);
    capabilities.recording_ingest_max_bytes =
        Some(i64::try_from(claw_api::RECORDING_INGEST_MAX_BYTES).unwrap_or(i64::MAX));
    info.capabilities = Some(Box::new(capabilities));
    Json(info)
}

pub(super) async fn telemetry(State(state): State<AppState>) -> Json<TelemetryState> {
    Json(to_contract_state(state.telemetry.get_state().await))
}

pub(super) async fn update_telemetry(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    payload: Result<Json<UpdateTelemetryRequest>, JsonRejection>,
) -> Result<Json<TelemetryState>, CanonicalError> {
    let Json(payload) = payload.map_err(|_| {
        error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "consent must be a boolean",
        )
    })?;
    Ok(Json(to_contract_state(
        state.telemetry.set_consent(payload.consent).await,
    )))
}

fn to_contract_state(state: crate::telemetry::TelemetryState) -> TelemetryState {
    TelemetryState::new(state.distinct_id, state.enabled, state.consent)
}
