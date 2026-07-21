//! Canonical BrowserClaw HTTP API and shared request middleware.

use super::mcp::streamable_http_service;
use crate::{
    AppState,
    error::{AppError, CanonicalError, RequestId},
};
use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use std::time::Instant;
use tracing::{Instrument, info_span};
use ulid::Ulid;

mod connections;
mod dispatches;
mod previews;
mod recordings;
mod replay;
mod sessions;
mod settings;
mod system;

pub(super) const RECORDING_INGEST_MAX_BYTES: usize = 16 * 1024 * 1024;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/system/health", get(system::health))
        .route("/system/shutdown", post(system::shutdown))
        .route("/api/v1/system", get(system::info))
        .route(
            "/api/v1/settings/telemetry",
            get(settings::telemetry).put(settings::update_telemetry),
        )
        .route("/api/v1/sessions", get(sessions::list))
        .route("/api/v1/sessions/{session_id}", get(sessions::get))
        .route(
            "/api/v1/sessions/{session_id}/browser-tabs/{browser_tab_id}/preview",
            get(previews::preview),
        )
        .route(
            "/api/v1/sessions/{session_id}/cancel",
            post(sessions::cancel),
        )
        .route(
            "/api/v1/sessions/{session_id}/recording",
            get(replay::recording),
        )
        .route(
            "/api/v1/sessions/{session_id}/recording/events",
            get(replay::download_events),
        )
        .route(
            "/api/v1/recordings/events",
            post(recordings::append_document_events)
                .layer(DefaultBodyLimit::max(RECORDING_INGEST_MAX_BYTES)),
        )
        .route(
            "/api/v1/dispatches/{dispatch_id}/screenshot",
            get(dispatches::screenshot),
        )
        .route("/api/v1/connections", get(connections::list))
        .route(
            "/api/v1/connections/{harness}",
            put(connections::connect).delete(connections::disconnect),
        )
        .nest_service(
            "/mcp",
            Router::new()
                .fallback_service(streamable_http_service(state))
                .layer(middleware::from_fn(mcp_request_hygiene)),
        )
        .fallback(route_fallback)
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

/// Enforces the header conventions native MCP clients follow (parity with
/// the TS server's mcp-request-hygiene middleware). A browser-page fetch
/// against the loopback MCP endpoint always carries `origin` or
/// `sec-fetch-site`; native MCP clients never do.
async fn mcp_request_hygiene(req: Request, next: Next) -> Response {
    // The nested /mcp service shadows the router's `/{*path}` preflight
    // route, and the TS server's cors layer answers OPTIONS before its
    // hygiene runs — mirror both so preflight stays 204 here too.
    if *req.method() == Method::OPTIONS {
        return StatusCode::NO_CONTENT.into_response();
    }
    let headers = req.headers();
    if headers.contains_key(header::ORIGIN) || headers.contains_key("sec-fetch-site") {
        return AppError::forbidden("unsupported request").into_response();
    }
    let needs_json = match *req.method() {
        Method::POST | Method::PUT | Method::PATCH => true,
        // rmcp's DELETE /mcp session teardown carries no body and no
        // content-type; the TS server never sees that shape (its clients
        // always send application/json), so exempt only that case.
        Method::DELETE => headers.contains_key(header::CONTENT_TYPE),
        _ => false,
    };
    if needs_json {
        let is_json = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("application/json"));
        if !is_json {
            return AppError::unsupported_media_type("unsupported content type").into_response();
        }
    }
    next.run(req).await
}

pub async fn request_context(mut req: Request, next: Next) -> Response {
    let request_id = RequestId(Ulid::new().to_string());
    req.extensions_mut().insert(request_id.clone());
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let reject_recording_origin =
        path == "/api/v1/recordings/events" && !trusted_recording_origin(req.headers());
    let span = info_span!("http_request", request_id = %request_id.0, %method, %path);
    async move {
        let start = Instant::now();
        let mut response = if reject_recording_origin {
            CanonicalError::new(
                StatusCode::FORBIDDEN,
                "forbidden",
                "recording ingest is restricted to BrowserClaw",
                Some(&request_id),
            )
            .into_response()
        } else {
            next.run(req).await
        };
        // One structured line per failed request; sub-400 traffic stays
        // unlogged on purpose (claw-app polls several endpoints).
        let status = response.status().as_u16();
        if status >= 400 {
            let duration_ms = start.elapsed().as_millis() as u64;
            if status >= 500 {
                tracing::error!(%method, %path, status, duration_ms, "request failed");
            } else {
                tracing::warn!(%method, %path, status, duration_ms, "request failed");
            }
        }
        let headers = response.headers_mut();
        if !reject_recording_origin {
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_ORIGIN,
                HeaderValue::from_static("*"),
            );
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("GET,POST,PUT,PATCH,DELETE,OPTIONS"),
            );
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static(
                    "accept,content-type,authorization,mcp-session-id,mcp-protocol-version,last-event-id,x-recording-batch-id,x-recording-tab-id,x-recording-document-id,x-recording-has-gap",
                ),
            );
        }
        if let Ok(value) = HeaderValue::from_str(&request_id.0) {
            headers.insert("x-request-id", value);
        }
        response
    }
    .instrument(span)
    .await
}

/// Stable origin derived from claw-app's manifest signing key.
const BROWSERCLAW_EXTENSION_ORIGIN: &str = "chrome-extension://pjimfkbpehlcllblajnpfamdfjhhlgkc";

fn trusted_recording_origin(headers: &axum::http::HeaderMap) -> bool {
    match headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        None => true,
        Some(BROWSERCLAW_EXTENSION_ORIGIN) => true,
        Some("null") => {
            headers
                .get("sec-fetch-site")
                .and_then(|value| value.to_str().ok())
                == Some("none")
        }
        Some(_) => false,
    }
}

async fn route_fallback(request: Request) -> StatusCode {
    if *request.method() == Method::OPTIONS {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}
