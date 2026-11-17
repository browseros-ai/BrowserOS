//! The request id. Every response leaves with `x-request-id`, a fresh ULID, and every error
//! response gets its body here: the contract's `Error` with the same id in `requestId`. A host
//! quotes one id for one failure, and the log line for it carries the same id. Handlers return
//! an `ApiError` and never thread the id. It is the HTTP request's own id, unrelated to a
//! run's `requestId`.
//!
//! An error status that carries no `ApiError` (axum's own 405, a body over the size limit)
//! still gets the contract's body, so no framework text ever reaches a host.

use crate::api_error::ApiError;
use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use tracing::Instrument;
use ulid::Ulid;

/// The response header that carries the id.
pub(crate) const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// The middleware on every route (`axum::middleware::from_fn`). Logs inside the request run
/// in a span that carries the id.
pub(crate) async fn stamp(request: Request, next: Next) -> Response {
    let id = Ulid::new();
    let span = tracing::info_span!(
        "host_request",
        request_id = %id,
        method = %request.method(),
        path = request.uri().path(),
    );
    let response = next.run(request).instrument(span.clone()).await;
    span.in_scope(|| finish(response, id))
}

/// Stamps a response made outside the router: the host auth's refusals, which never reach it.
pub(crate) fn stamp_alone(response: Response) -> Response {
    finish(response, Ulid::new())
}

fn finish(response: Response, id: Ulid) -> Response {
    let id = id.to_string();
    let (mut parts, body) = response.into_parts();
    let error = parts
        .extensions
        .remove::<ApiError>()
        .or_else(|| ApiError::for_status(parts.status));
    let body = match error {
        None => body,
        Some(error) => {
            if let Some(cause) = &error.cause {
                tracing::error!(request_id = %id, code = error.code.as_str(), %cause, "host request failed");
            }
            let wire = neo_sdk_api::Error {
                code: error.code.as_str().to_string(),
                message: error.message.into_owned(),
                request_id: Some(id.clone()),
            };
            match serde_json::to_vec(&wire) {
                Ok(json) => {
                    parts.headers.remove(CONTENT_LENGTH);
                    parts
                        .headers
                        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
                    Body::from(json)
                }
                // A struct of strings always serializes; keep what the handler sent.
                Err(_) => body,
            }
        }
    };
    if let Ok(value) = HeaderValue::from_str(&id) {
        parts.headers.insert(REQUEST_ID, value);
    }
    Response::from_parts(parts, body)
}
