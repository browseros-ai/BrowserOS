//! The hosted API: `/v1`, a read-only record of agent sessions for the backend that runs a
//! hosted BrowserOS neo (`BROWSERCLAW_MODE=hosted`, the Docker image).
//!
//! `contracts/hosted-api/openapi.yaml` is the published contract, and every response body is
//! a `hosted_api` model generated from it, so the served JSON cannot drift from the spec.
//! Nothing here may use the cockpit's `claw_api` types: the cockpit contract changes freely,
//! this one only additively (`tests/dependency_boundaries.rs` enforces the split).
//!
//! This module is the HTTP adapter. It parses query parameters, calls [`SessionRecords`],
//! and maps failures into the spec's `Error` body. What a session record contains, and when
//! it is complete, is decided in `records`.

mod records;

use crate::{AppState, error::AppError, error::RequestId};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use records::{GetOptions, ListQuery, SessionRecords};
use std::{collections::HashMap, time::Duration};

/// The `/v1` routes. `app::build_router` mounts them only in hosted mode, so a desktop
/// server answers `/v1/*` with the router's plain 404.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/sessions", get(list))
        .route("/v1/sessions/{session_id}", get(session))
        .route("/v1/sessions/{session_id}/recording", get(recording))
}

async fn list(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<hosted_api::SessionList>, HostedError> {
    let query = ListQuery {
        client: raw.get("client").cloned(),
        since: integer(&request_id, &raw, "since", 0, i64::MAX)?,
        until: integer(&request_id, &raw, "until", 0, i64::MAX)?,
        search: raw.get("q").cloned(),
        cursor: cursor(&request_id, &raw)?,
        limit: integer(&request_id, &raw, "limit", 1, 100)?,
    };
    SessionRecords::new(&state)
        .list(query)
        .await
        .map(Json)
        .map_err(|source| HostedError::internal(&request_id, source))
}

async fn session(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<hosted_api::Session>, HostedError> {
    let options = GetOptions {
        screenshots: includes_screenshots(&request_id, &raw)?,
        wait: Duration::from_secs(
            integer(&request_id, &raw, "wait", 0, 60)?
                .and_then(|seconds| u64::try_from(seconds).ok())
                .unwrap_or(0),
        ),
    };
    SessionRecords::new(&state)
        .get(&session_id, options)
        .await
        .map_err(|source| HostedError::internal(&request_id, source))?
        .map(Json)
        .ok_or_else(|| HostedError::not_found(&request_id))
}

async fn recording(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<hosted_api::Recording>, HostedError> {
    SessionRecords::new(&state)
        .recording(&session_id)
        .await
        .map_err(|source| HostedError::internal(&request_id, source))?
        .map(Json)
        .ok_or_else(|| HostedError::not_found(&request_id))
}

/// `include` is a comma-separated list. Unknown values are rejected rather than ignored, so
/// a typo does not silently return a response without the data the caller asked for.
fn includes_screenshots(
    request_id: &RequestId,
    raw: &HashMap<String, String>,
) -> Result<bool, HostedError> {
    let Some(include) = raw.get("include") else {
        return Ok(false);
    };
    let mut screenshots = false;
    for value in include.split(',').map(str::trim) {
        match value {
            "screenshots" => screenshots = true,
            _ => {
                return Err(HostedError::invalid(
                    request_id,
                    &format!("unknown `include` value {value:?}"),
                ));
            }
        }
    }
    Ok(screenshots)
}

/// Cursors are opaque to clients; today one is the last item's `tasks.cursor_id`.
fn cursor(
    request_id: &RequestId,
    raw: &HashMap<String, String>,
) -> Result<Option<i64>, HostedError> {
    let Some(value) = raw.get("cursor") else {
        return Ok(None);
    };
    match value.parse::<i64>() {
        Ok(cursor) if cursor > 0 => Ok(Some(cursor)),
        _ => Err(HostedError::invalid(request_id, "invalid cursor")),
    }
}

fn integer(
    request_id: &RequestId,
    raw: &HashMap<String, String>,
    key: &str,
    minimum: i64,
    maximum: i64,
) -> Result<Option<i64>, HostedError> {
    let Some(value) = raw.get(key) else {
        return Ok(None);
    };
    let value = value
        .parse::<i64>()
        .map_err(|_| HostedError::invalid(request_id, &format!("`{key}` must be an integer")))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(HostedError::invalid(
            request_id,
            &format!("`{key}` must be between {minimum} and {maximum}"),
        ));
    }
    Ok(Some(value))
}

/// A failed `/v1` request: the spec's `Error` body with its HTTP status.
struct HostedError {
    status: StatusCode,
    body: hosted_api::Error,
}

impl HostedError {
    fn new(request_id: &RequestId, status: StatusCode, code: &str, message: &str) -> Self {
        let mut body = hosted_api::Error::new(code.to_string(), message.to_string());
        body.request_id = Some(request_id.0.clone());
        Self { status, body }
    }

    fn invalid(request_id: &RequestId, message: &str) -> Self {
        Self::new(
            request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            message,
        )
    }

    fn not_found(request_id: &RequestId) -> Self {
        Self::new(
            request_id,
            StatusCode::NOT_FOUND,
            "session_not_found",
            "session not found",
        )
    }

    /// Logs the cause and returns a generic 500, so storage details never reach the wire.
    fn internal(request_id: &RequestId, source: AppError) -> Self {
        tracing::error!(request_id = %request_id.0, error = %source, "hosted API route failed");
        Self::new(
            request_id,
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal server error",
        )
    }
}

impl IntoResponse for HostedError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}
