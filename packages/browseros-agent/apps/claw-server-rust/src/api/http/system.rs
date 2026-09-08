use crate::{AppState, VERSION};
use axum::{Json, extract::State};
use claw_api::models::{
    HealthResponse, ShutdownResponse, SystemCapabilities, SystemDiagnostics, SystemInfo,
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
        "BrowserOS neo".to_string(),
        VERSION.to_string(),
        state.config.local_server_url(),
    );
    let mut capabilities = SystemCapabilities::new();
    capabilities.recording_ingest_version = Some(RecordingIngestVersion::Variant2);
    capabilities.recording_ingest_max_bytes =
        Some(i64::try_from(super::RECORDING_INGEST_MAX_BYTES).unwrap_or(i64::MAX));
    info.capabilities = Some(Box::new(capabilities));
    Json(info)
}

/// OS discovery can read files or invoke platform APIs; keep it off Tokio's executor.
/// Cache only process-stable public metadata, never environment or account details.
pub(super) async fn diagnostics() -> Json<SystemDiagnostics> {
    static DETAILS: std::sync::LazyLock<SystemDiagnostics> = std::sync::LazyLock::new(|| {
        let info = os_info::get();
        let mut details =
            SystemDiagnostics::new(VERSION.to_string(), info.os_type().to_string(), None);
        details.os_version = match info.version() {
            os_info::Version::Unknown => None,
            version => Some(version.to_string()),
        };
        details
    });
    let details = tokio::task::spawn_blocking(|| DETAILS.clone())
        .await
        .unwrap_or_else(|_| {
            SystemDiagnostics::new(VERSION.to_string(), std::env::consts::OS.to_string(), None)
        });
    Json(details)
}

#[cfg(test)]
mod diagnostics_tests {
    #[tokio::test]
    async fn only_public_metadata_is_serialized() -> Result<(), Box<dyn std::error::Error>> {
        let result = super::diagnostics().await.0;
        assert_eq!(result.version, crate::VERSION);
        assert!(!result.os.is_empty());
        let value = serde_json::to_value(result)?;
        let object = value.as_object().ok_or("diagnostics must be an object")?;
        assert!(
            object
                .keys()
                .all(|key| ["version", "os", "osVersion"].contains(&key.as_str()))
        );
        Ok(())
    }
}
