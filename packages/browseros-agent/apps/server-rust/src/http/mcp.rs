//! `/mcp` routes (contract `mcp.yaml`). Browser fetches never get here: `security::ingress`
//! answers any request carrying `Sec-Fetch-Site` with 403 first, and counts in-flight POSTs
//! for `/status`. POST is `mcp::McpServer`'s lease admission + transport in front of the
//! shared host; GET is a status blob, not SSE.

use super::errors;
use crate::AppState;
use axum::{
    Json,
    extract::{Request, State},
    response::{IntoResponse, Response},
};
use http::StatusCode;
use serde_json::json;

pub async fn status_json() -> Response {
    Json(json!({
        "status": "ok",
        "message": "MCP server is running. Use POST to interact.",
    }))
    .into_response()
}

pub async fn dispatch(State(state): State<AppState>, request: Request) -> Response {
    state.mcp.post(request).await
}

/// TS registers no DELETE (its legacy transport is stateless), so Hono answered its plain
/// 404. Served by a handler rather than the router fallback so the contract still lists it.
pub async fn delete_session() -> Response {
    errors::text_plain(StatusCode::NOT_FOUND, "404 Not Found")
}
