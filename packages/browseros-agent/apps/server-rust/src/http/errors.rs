//! One constructor per existing error body dialect (contract `responses.yaml`, `common.yaml`).
//! Handlers never hand-build error JSON, so each dialect stays byte-identical everywhere it is
//! used. Tests pin every body against the contract fixtures.

use super::Json;
use super::extract::ZodIssue;
use axum::response::{IntoResponse, Response};
use browseros_agent::llm::ChatError;
use http::{StatusCode, header};
use serde_json::{Value, json};

/// `{"error":"…"}`.
pub fn error_string(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

/// 400 `{"success":false,"error":{"name":"ZodError","message":"<pretty JSON array of issues>"}}`.
pub fn zod_400(issues: &[ZodIssue]) -> Response {
    let issues: Vec<Value> = issues.iter().map(ZodIssue::wire_value).collect();
    zod_issues_400(&issues)
}

/// The same 400 for issues already rendered in zod's own key order (`/chat`'s union schema needs
/// orders `ZodIssue::wire_value` cannot produce: code-first `invalid_type`, `custom`, `invalid_key`).
pub fn zod_issues_400(issues: &[Value]) -> Response {
    let message = serde_json::to_string_pretty(issues).unwrap_or_default();
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "success": false, "error": { "name": "ZodError", "message": message } })),
    )
        .into_response()
}

/// HTTP 500 chat envelope; an upstream statusCode remains in the body for the client card.
pub fn chat_envelope_500(error: &ChatError) -> Response {
    let mut body = serde_json::to_value(error).unwrap_or_else(|_| json!({}));
    if let Some(object) = body.as_object_mut() {
        object.insert("name".to_owned(), json!("InternalServerError"));
        object.insert(
            "statusCode".to_owned(),
            json!(error.status_code.unwrap_or(500)),
        );
    }
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": body })),
    )
        .into_response()
}

/// 403 from the trusted-origin guard (nested `HttpAgentError` envelope).
pub fn forbidden_origin() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "error": {
            "name": "ForbiddenOrigin",
            "message": "Origin not allowed",
            "code": "FORBIDDEN_ORIGIN",
            "statusCode": 403,
        }})),
    )
        .into_response()
}

/// Hono's malformed-JSON HTTPException falls through the generic error handler.
pub fn malformed_json() -> Response {
    chat_envelope_500(&ChatError {
        code: browseros_agent::llm::ChatErrorCode::Unknown,
        title: "Something went wrong".into(),
        message: "Malformed JSON in request body".into(),
        retryable: true,
        provider: None,
        status_code: None,
        docs_url: None,
        retry_after_seconds: None,
        details: Some("Malformed JSON in request body".into()),
    })
}

/// 403 from `/mcp`'s browser-fetch rejection.
pub fn forbidden_browser_request() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({"error": {
            "name": "ForbiddenBrowserRequest",
            "message": "Browser requests are not allowed on this endpoint",
            "code": "FORBIDDEN_BROWSER_REQUEST",
            "statusCode": 403
        }})),
    )
        .into_response()
}

/// `{"success":false,"message":"…"}`.
pub fn success_false(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(json!({ "success": false, "message": message })),
    )
        .into_response()
}

/// `{"message":"…"}`.
pub fn message_only(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "message": message }))).into_response()
}

pub fn text_plain(status: StatusCode, body: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=UTF-8")],
        body.to_owned(),
    )
        .into_response()
}

/// `/acpx/probe` failures: `{"error":{"code":"wrapper_error","message":"…"}}`.
pub fn probe_wrapper_error(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": { "code": "wrapper_error", "message": message } })),
    )
        .into_response()
}

/// JSON-RPC error with `id: null` (`/mcp` lease rejection is 401 / -32001).
pub fn jsonrpc_error(status: StatusCode, code: i64, message: &str) -> Response {
    (
        status,
        Json(
            json!({ "jsonrpc": "2.0", "error": { "code": code, "message": message }, "id": null }),
        ),
    )
        .into_response()
}
