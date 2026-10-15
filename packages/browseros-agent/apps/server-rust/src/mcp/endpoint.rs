//! `POST /mcp` transport compatibility. TS's MCP SDK answers some requests itself before any
//! MCP server sees them, with JSON-RPC bodies the contract pins; rmcp answers the same cases
//! in plain text and in a different order. This layer reproduces the SDK's rejections, then
//! hands rmcp (stateless, single-JSON replies) only a request it will accept.
//!
//! Order, as TS composes lease check, `isLegacyRequest` and the SDK legs
//! (`mcp.ts:104-145`; `@modelcontextprotocol/server@2.1.0 dist/index.mjs:275-298, 693-725,
//! 1150-1185, 1443-1449`):
//!
//! ```text
//!  lease header present but unknown ──────────────► 401 -32001   (admission)
//!  body over 4 MiB (declared or read) ────────────► 415 if Content-Type is not JSON, else 413
//!  body stream fails ─────────────────────────────► 400 -32700 could not be read
//!  Accept lacks application/json + text/event-stream ► 406
//!  Content-Type not application/json ─────────────► 415
//!  body empty or not JSON ────────────────────────► 400 -32700 Invalid JSON
//!  batch over 100 / not one JSON-RPC message ─────► 400 -32600 / -32700
//!  rmcp: stateless JSON; its plain-text 500 ──────► 500 -32603 Internal server error
//! ```
//!
//! The SDK's modern (2026-07-28) leg skips the Accept check and orders 415 before 406; rmcp
//! enforces Accept on both eras, so this layer does too, with the SDK's body.

use crate::{
    http::errors::{Unhandled, UnhandledScope},
    telemetry::metrics::Metrics,
};
use axum::{
    body::{Body, Bytes},
    extract::Request,
    response::Response,
};
use browseros_mcp_host::{Admission, HostMcpService};
use futures_util::StreamExt;
use http::{HeaderMap, HeaderValue, StatusCode, header};
use rmcp::{
    model::ClientJsonRpcMessage,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, tower::StreamableHttpService,
    },
};
use serde_json::{Value, json};
use tower::ServiceExt;

/// rmcp's streamable-HTTP service over the shared host: one `HostMcpService` per request.
pub(super) type Transport = StreamableHttpService<HostMcpService, LocalSessionManager>;

/// The SDK's `DEFAULT_MAX_REQUEST_BODY_SIZE` (4 MiB).
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
/// The SDK's `MAX_BATCH_SIZE`.
const MAX_BATCH: usize = 100;

const JSON: &str = "application/json";
const EVENT_STREAM: &str = "text/event-stream";

pub(super) async fn post(
    admission: &dyn Admission,
    transport: &Transport,
    metrics: &Metrics,
    request: Request,
) -> Response {
    let (mut parts, body) = request.into_parts();
    // A refused lease never reaches MCP: TS validates it before the SDK reads the body.
    let leased = match admission.admit(&parts) {
        Ok(Some(caller)) => {
            parts.extensions.insert(caller);
            true
        }
        Ok(None) => false,
        Err(refusal) => return *refusal,
    };
    // TS counts every POST past the lease check, before the SDK's own rejections.
    metrics.capture("mcp.request", json!({ "leased": leased }));
    let json_content =
        is_json_content_type(joined(&parts.headers, header::CONTENT_TYPE).as_deref());
    let bytes = match read_body(&parts.headers, body).await {
        BodyRead::Complete(bytes) => bytes,
        BodyRead::TooLarge if !json_content => return unsupported_media_type(),
        BodyRead::TooLarge => {
            return rpc_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                -32000,
                &format!("Payload Too Large: Request body must not exceed {MAX_BODY_BYTES} bytes"),
            );
        }
        BodyRead::Unreadable => {
            return rpc_error(
                StatusCode::BAD_REQUEST,
                -32700,
                "Parse error: the request body could not be read",
            );
        }
    };
    let accept = joined(&parts.headers, header::ACCEPT).unwrap_or_default();
    if !(accept.contains(JSON) && accept.contains(EVENT_STREAM)) {
        return rpc_error(
            StatusCode::NOT_ACCEPTABLE,
            -32000,
            "Not Acceptable: Client must accept both application/json and text/event-stream",
        );
    }
    if !json_content {
        return unsupported_media_type();
    }
    if let Some(rejection) = message_rejection(&bytes) {
        return rejection;
    }
    // rmcp re-checks both headers by prefix/substring of the first value only; hand it the
    // canonical spellings the SDK checks above already accepted.
    parts
        .headers
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(JSON));
    parts.headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("application/json, text/event-stream"),
    );
    let response = match transport
        .clone()
        .oneshot(Request::from_parts(parts, Body::from(bytes)))
        .await
    {
        Ok(response) => response.map(Body::new),
        Err(never) => match never {},
    };
    if response.status() == StatusCode::INTERNAL_SERVER_ERROR && !is_json_response(&response) {
        let detail = axum::body::to_bytes(response.into_body(), MAX_BODY_BYTES)
            .await
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        tracing::error!(%detail, "Error handling MCP request");
        let mut response = rpc_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            -32603,
            "Internal server error",
        );
        // TS's `/mcp` catch reports to Sentry with its own scope before answering -32603.
        Unhandled {
            message: if detail.is_empty() {
                "Error handling MCP request".to_owned()
            } else {
                detail
            },
            scope: UnhandledScope::Mcp { leased },
        }
        .attach(&mut response);
        return response;
    }
    response
}

/// `{"jsonrpc":"2.0","error":{code,message},"id":null}`, the SDK's transport error body.
fn rpc_error(status: StatusCode, code: i64, message: &str) -> Response {
    crate::http::errors::jsonrpc_error(status, code, message)
}

fn unsupported_media_type() -> Response {
    rpc_error(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        -32000,
        "Unsupported Media Type: Content-Type must be application/json",
    )
}

/// The SDK's refusal for a body that is not one JSON-RPC message rmcp can decode. rmcp has
/// no batch support (batching left MCP in 2025-06-18), so a batch the SDK would have served
/// is refused like a malformed message.
fn message_rejection(bytes: &Bytes) -> Option<Response> {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Some(rpc_error(
            StatusCode::BAD_REQUEST,
            -32700,
            "Parse error: Invalid JSON",
        ));
    };
    if value
        .as_array()
        .is_some_and(|batch| batch.len() > MAX_BATCH)
    {
        return Some(rpc_error(
            StatusCode::BAD_REQUEST,
            -32600,
            &format!("Invalid Request: Batch must not exceed {MAX_BATCH} messages"),
        ));
    }
    serde_json::from_value::<ClientJsonRpcMessage>(value)
        .is_err()
        .then(|| {
            rpc_error(
                StatusCode::BAD_REQUEST,
                -32700,
                "Parse error: Invalid JSON-RPC message",
            )
        })
}

enum BodyRead {
    Complete(Bytes),
    TooLarge,
    Unreadable,
}

/// Reads at most `MAX_BODY_BYTES`. A declared `Content-Length` over the limit is refused
/// without reading, as the SDK's `readRequestBody` does (`Number(header) > max`).
async fn read_body(headers: &HeaderMap, body: Body) -> BodyRead {
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<f64>().ok());
    if declared.is_some_and(|length| length > MAX_BODY_BYTES as f64) {
        return BodyRead::TooLarge;
    }
    let mut stream = body.into_data_stream();
    let mut collected = Vec::new();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return BodyRead::Unreadable;
        };
        if collected.len() + chunk.len() > MAX_BODY_BYTES {
            return BodyRead::TooLarge;
        }
        collected.extend_from_slice(&chunk);
    }
    BodyRead::Complete(Bytes::from(collected))
}

/// A header as Fetch's `Headers.get` reads it: repeated values joined with ", ".
fn joined(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers
        .get_all(name)
        .iter()
        .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
        .reduce(|joined, next| format!("{joined}, {next}"))
}

/// The SDK's `isJsonContentType`: the media type before any parameters is
/// `application/json`, case-insensitively.
pub fn is_json_content_type(header: Option<&str>) -> bool {
    header.is_some_and(|header| {
        header == JSON
            || header
                .split(';')
                .next()
                .is_some_and(|essence| essence.trim().eq_ignore_ascii_case(JSON))
    })
}

fn is_json_response(response: &Response) -> bool {
    is_json_content_type(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
    )
}
