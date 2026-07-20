//! Canonical contract routes: the Rust implementation of the shared
//! BrowserClaw OpenAPI contract (`contracts/claw-api`), speaking the
//! generated `claw_api` types end to end. The TS claw-server implements
//! the same surface, and the cross-server contract suite runs the same
//! cases against both. Besides `/api/v1/*` the contract also owns
//! `/system/health` and `/system/shutdown`; these are the server's full
//! REST surface.

use crate::{
    AppState,
    error::{AppError, CanonicalError, RequestId},
};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::StatusCode,
    routing::{get, post, put},
};
use claw_api::RECORDING_INGEST_MAX_BYTES;

mod connections;
mod sessions;
mod system;
mod tabs;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/system/health", get(system::health))
        .route("/system/shutdown", post(system::shutdown))
        .route("/api/v1/system", get(system::info))
        .route(
            "/api/v1/settings/telemetry",
            get(system::telemetry).put(system::update_telemetry),
        )
        .route("/api/v1/sessions", get(sessions::list))
        .route("/api/v1/sessions/{session_id}", get(sessions::get))
        .route(
            "/api/v1/sessions/{session_id}/cancel",
            post(sessions::cancel),
        )
        .route(
            "/api/v1/sessions/{session_id}/recording",
            get(sessions::recording),
        )
        .route(
            "/api/v1/sessions/{session_id}/recording/events",
            get(sessions::download_events)
                .post(sessions::append_legacy_events)
                .layer(DefaultBodyLimit::max(RECORDING_INGEST_MAX_BYTES)),
        )
        .route(
            "/api/v1/recordings/events",
            post(sessions::append_document_events)
                .layer(DefaultBodyLimit::max(RECORDING_INGEST_MAX_BYTES)),
        )
        .route("/api/v1/tabs", get(tabs::list))
        .route("/api/v1/tabs/{page_id}/preview", get(tabs::preview))
        .route(
            "/api/v1/dispatches/{dispatch_id}/screenshot",
            get(tabs::screenshot),
        )
        .route("/api/v1/connections", get(connections::list))
        .route(
            "/api/v1/connections/{harness}",
            put(connections::connect).delete(connections::disconnect),
        )
}

pub(super) fn error(
    request_id: &RequestId,
    status: StatusCode,
    code: &str,
    message: &str,
) -> CanonicalError {
    CanonicalError::new(status, code, message, Some(request_id))
}

pub(super) fn internal(request_id: &RequestId, source: AppError) -> CanonicalError {
    tracing::error!(request_id = %request_id.0, error = %source, "canonical route failed");
    error(
        request_id,
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "internal server error",
    )
}
