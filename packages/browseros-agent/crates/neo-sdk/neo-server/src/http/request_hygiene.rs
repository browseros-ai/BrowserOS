//! What keeps web pages in Neo's own browser away from `/mcp`.

use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// claw-server's `mcp_request_hygiene`, copied. The agent port has no auth (in the cell only
/// mirrord reaches it), so this is what stops a page open in the browser Neo drives from
/// calling `run`:
/// - `OPTIONS` answers 204, as a preflight would, and goes no further;
/// - a request carrying `Origin` or `Sec-Fetch-Site` is 403: browser fetches carry one, native
///   MCP clients do not;
/// - a `POST`, `PUT` or `PATCH` (or a `DELETE` with a body) that is not JSON is 415.
pub(crate) async fn request_hygiene(request: Request, next: Next) -> Response {
    if request.method() == Method::OPTIONS {
        return StatusCode::NO_CONTENT.into_response();
    }
    let headers = request.headers();
    if headers.contains_key(header::ORIGIN) || headers.contains_key("sec-fetch-site") {
        return (
            StatusCode::FORBIDDEN,
            "browser requests are not served here",
        )
            .into_response();
    }
    let needs_json = match *request.method() {
        Method::POST | Method::PUT | Method::PATCH => true,
        Method::DELETE => headers.contains_key(header::CONTENT_TYPE),
        _ => false,
    };
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("application/json"));
    if needs_json && !is_json {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, "send application/json").into_response();
    }
    next.run(request).await
}
