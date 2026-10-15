//! `/mcp`: `GET` answers status JSON (no SSE); `POST`/`DELETE` become the lease-admission layer in
//! front of the shared `browseros-mcp-host` rmcp service (P15). Browser fetches are rejected.

use axum::response::Response;

pub async fn status_json() -> Response {
    super::not_implemented()
}

pub async fn dispatch() -> Response {
    super::not_implemented()
}

pub async fn delete_session() -> Response {
    super::not_implemented()
}
