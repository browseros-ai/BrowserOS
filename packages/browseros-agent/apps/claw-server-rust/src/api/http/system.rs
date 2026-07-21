use crate::AppState;
use axum::{Json, extract::State};
use claw_api::models::{
    HealthResponse, ShutdownResponse, SystemCapabilities, SystemInfo,
    system_capabilities::RecordingIngestVersion,
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
        Some(i64::try_from(super::RECORDING_INGEST_MAX_BYTES).unwrap_or(i64::MAX));
    info.capabilities = Some(Box::new(capabilities));
    Json(info)
}
