//! Deserializes every contract fixture through the generated `claw_api` models
//! so schema and Rust wire types cannot drift independently.

use axum::{http::StatusCode, response::IntoResponse};
use claw_api::models::{
    ApiError, AppendRecordingEventsResponse, CancelSessionResponse, Connection, ConnectionList,
    HealthResponse, RecordingMetadata, SessionDetail, SessionList, ShutdownResponse, SystemInfo,
    TelemetryState,
};
use claw_server_rust::error::{CanonicalError, RequestId};
use serde::de::DeserializeOwned;
use std::{fs, path::PathBuf};

fn fixture<T: DeserializeOwned>(name: &str) -> anyhow::Result<T> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/claw-api/fixtures")
        .join(name);
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

#[test]
fn generated_dtos_deserialize_shared_fixtures() -> anyhow::Result<()> {
    let _: HealthResponse = fixture("health.json")?;
    let _: ShutdownResponse = fixture("shutdown.json")?;
    let _: SystemInfo = fixture("system-info.json")?;
    let _: TelemetryState = fixture("telemetry-state.json")?;
    let _: SessionList = fixture("session-list.json")?;
    let _: SessionDetail = fixture("session-detail.json")?;
    let _: CancelSessionResponse = fixture("cancel-session.json")?;
    let _: RecordingMetadata = fixture("recording-metadata.json")?;
    let _: AppendRecordingEventsResponse = fixture("append-recording-events.json")?;
    let _: Connection = fixture("connection.json")?;
    let _: ConnectionList = fixture("connection-list.json")?;
    let _: ApiError = fixture("api-error.json")?;
    let _: ApiError = fixture("api-error-minimal.json")?;
    Ok(())
}

#[tokio::test]
async fn canonical_error_carries_the_middleware_request_id() -> anyhow::Result<()> {
    let response = CanonicalError::new(
        StatusCode::NOT_FOUND,
        "session_not_found",
        "Session was not found.",
        Some(&RequestId("request-1".to_string())),
    )
    .into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    let actual: ApiError = serde_json::from_slice(&bytes)?;
    assert_eq!(actual.code, "session_not_found");
    assert_eq!(actual.message, "Session was not found.");
    assert_eq!(actual.request_id.as_deref(), Some("request-1"));
    Ok(())
}
