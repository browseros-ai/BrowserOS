//! `/chat*`: start a turn (UI message stream v1 SSE), the panels SSE (custom frames, 5 s heartbeat,
//! no `[DONE]`), run state, resume (204 / 409 / replay + tail), stop, and session delete.
//! Thin over `chat::ChatService`; every status and body is the TS route's (`api/routes/chat.ts`).
//!
//! Streams are subscribers: the response body is a cursor over the run's replay log, so a client
//! disconnect drops only that cursor and never cancels the run (`POST …/stop` and DELETE do).
//!
//! Handlers take `State<Arc<ChatService>>` (projected from `AppState` by `FromRef`) and the
//! request's [`RequestTrust`] from the request extensions, so they can be mounted on a test router
//! with a scripted service. Owned by P8.

use super::{errors, security::RequestTrust, wire::chat as wire};
use crate::{
    app::AppState,
    chat::{ChatRejection, ChatService, Resume, request},
};
use axum::{
    Extension, Json,
    body::Body,
    extract::{FromRef, Path, Query, State},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt as _;
use http::{HeaderMap, HeaderValue, StatusCode, header};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{convert::Infallible, sync::Arc};
use ui_message_stream::{ChunkStream, sse};

/// Bun's default request body cap: attachments travel inline as base64, so axum's 2 MB default
/// would reject image turns the TS server accepts.
const MAX_CHAT_BODY_BYTES: usize = 128 * 1024 * 1024;

impl FromRef<AppState> for Arc<ChatService> {
    fn from_ref(state: &AppState) -> Self {
        state.chat.clone()
    }
}

/// The request's `isTrustedAppRequest` verdict, computed once by the security middleware and
/// left in the extensions. Absent means untrusted (fail closed).
fn trust(extension: Option<Extension<RequestTrust>>) -> RequestTrust {
    extension.map(|Extension(trust)| trust).unwrap_or_default()
}

fn forbidden() -> Response {
    errors::error_string(StatusCode::FORBIDDEN, "Forbidden")
}

/// `POST /chat`: answers only after the engine is prepared; the body is the turn's stream.
pub async fn post_chat(
    State(chat): State<Arc<ChatService>>,
    trust_extension: Option<Extension<RequestTrust>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let value = match json_body(&headers, body).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let request = match request::parse_chat_request(&value) {
        Ok(request) => request,
        Err(issues) => return zod_issues_400(&issues),
    };
    match chat.start(request, trust(trust_extension)).await {
        Ok(chunks) => ui_message_stream_response(chunks),
        Err(rejection) => rejection_response(rejection),
    }
}

/// `GET /chat/panels`: the background page's routing table, as reconnectable SSE.
pub async fn panels(
    State(chat): State<Arc<ChatService>>,
    trust_extension: Option<Extension<RequestTrust>>,
) -> Response {
    if !trust(trust_extension).trusted_app {
        return forbidden();
    }
    let frames = chat.panel_assignments().map(|assignments| {
        let json = serde_json::to_string(&assignments).unwrap_or_else(|_| "{}".to_owned());
        Ok::<_, Infallible>(format!("data: {json}\n\n"))
    });
    let mut response = Response::new(Body::from_stream(frames));
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-transform"),
    );
    headers.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response
}

/// `DELETE /chat/panels/{tabId}`: `tabId` is read with JS `Number()` and must be a non-negative
/// integer.
pub async fn delete_panel(
    State(chat): State<Arc<ChatService>>,
    trust_extension: Option<Extension<RequestTrust>>,
    Path(tab_id): Path<String>,
) -> Response {
    if !trust(trust_extension).trusted_app {
        return forbidden();
    }
    let Some(tab_id) = js_number(&tab_id).filter(|id| id.fract() == 0.0 && *id >= 0.0) else {
        return errors::error_string(StatusCode::BAD_REQUEST, "Invalid tab id");
    };
    // Tab ids beyond u32 never have a panel, so removing one is a no-op success.
    if tab_id <= f64::from(u32::MAX) {
        chat.remove_panel_tab(tab_id as u32);
    }
    Json(server_api::SuccessResponse::new(true)).into_response()
}

/// `GET /chat/{conversationId}/state`: waits on the prepared gate (no timeout, TS parity).
pub async fn state(
    State(chat): State<Arc<ChatService>>,
    Path(conversation_id): Path<String>,
) -> Response {
    let Some(conversation_id) = request::parse_zod_uuid(&conversation_id) else {
        return errors::error_string(StatusCode::BAD_REQUEST, "Invalid conversation id");
    };
    let Some(snapshot) = chat.state(conversation_id).await else {
        return errors::error_string(StatusCode::NOT_FOUND, "Conversation not found");
    };
    (
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(wire::RunSnapshotBody::from(snapshot)),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct ResumeQuery {
    #[serde(rename = "runId")]
    run_id: Option<String>,
}

/// `GET /chat/{conversationId}/stream`: a disconnect drops only this subscriber.
pub async fn resume_stream(
    State(chat): State<Arc<ChatService>>,
    Path(conversation_id): Path<String>,
    Query(query): Query<ResumeQuery>,
) -> Response {
    let Some(conversation_id) = request::parse_zod_uuid(&conversation_id) else {
        return zod_issues_400(&[request::conversation_id_param_issue()]);
    };
    match chat.resume(conversation_id, query.run_id.as_deref()) {
        Resume::NoRun => StatusCode::NO_CONTENT.into_response(),
        // A newer turn won the state/stream race: the panel must hydrate again rather than
        // replay the new run onto an older snapshot.
        Resume::RunChanged => {
            errors::error_string(StatusCode::CONFLICT, "Conversation run changed")
        }
        Resume::Stream(chunks) => ui_message_stream_response(chunks),
    }
}

/// `POST /chat/{conversationId}/stop`: no trusted-app check (TS parity).
pub async fn stop(
    State(chat): State<Arc<ChatService>>,
    Path(conversation_id): Path<String>,
) -> Response {
    let Some(conversation_id) = request::parse_zod_uuid(&conversation_id) else {
        return zod_issues_400(&[request::conversation_id_param_issue()]);
    };
    Json(server_api::StopChatResponse::new(
        chat.stop(conversation_id).await,
    ))
    .into_response()
}

/// `DELETE /chat/{conversationId}`: trusted-app check only for ACP conversations.
pub async fn delete_conversation_session(
    State(chat): State<Arc<ChatService>>,
    trust_extension: Option<Extension<RequestTrust>>,
    Path(conversation_id): Path<String>,
) -> Response {
    let Some(conversation_id) = request::parse_zod_uuid(&conversation_id) else {
        return zod_issues_400(&[request::conversation_id_param_issue()]);
    };
    if chat.is_acp_conversation(conversation_id) && !trust(trust_extension).trusted_app {
        return forbidden();
    }
    let outcome = chat.delete(conversation_id).await;
    if outcome.deleted {
        return Json(wire::DeleteChatBody {
            success: true,
            message: format!("Session {conversation_id} deleted"),
            session_count: outcome.session_count,
        })
        .into_response();
    }
    errors::success_false(
        StatusCode::NOT_FOUND,
        &format!("Session {conversation_id} not found"),
    )
}

fn rejection_response(rejection: ChatRejection) -> Response {
    let status = match &rejection {
        ChatRejection::Forbidden => StatusCode::FORBIDDEN,
        ChatRejection::BadRequest(_) | ChatRejection::AgentTypeMismatch => StatusCode::BAD_REQUEST,
        ChatRejection::UnknownAgent => StatusCode::NOT_FOUND,
        ChatRejection::Busy => StatusCode::CONFLICT,
        ChatRejection::Internal(error) => return errors::chat_envelope_500(error),
    };
    errors::error_string(status, &rejection.to_string())
}

/// The AI SDK's `createUIMessageStreamResponse`: v1 headers, `data:` frames, `[DONE]` at the end.
fn ui_message_stream_response(chunks: ChunkStream) -> Response {
    let mut response = Response::new(Body::from_stream(sse::frames(chunks)));
    let headers = response.headers_mut();
    for (name, value) in sse::UI_STREAM_HEADERS {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

/// Hono's `zValidator('json')` body read: a missing or non-JSON content type validates `{}`;
/// malformed JSON is a 500 chat envelope (the TS `onError` does not special-case Hono's 400).
async fn json_body(headers: &HeaderMap, body: Body) -> Result<Value, Response> {
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(is_json_content_type);
    if !is_json {
        return Ok(json!({}));
    }
    let bytes = axum::body::to_bytes(body, MAX_CHAT_BODY_BYTES)
        .await
        .map_err(|_| errors::text_plain(StatusCode::PAYLOAD_TOO_LARGE, "Payload Too Large"))?;
    serde_json::from_slice(&bytes).map_err(|_| {
        errors::chat_envelope_500(&crate::chat::server_error(
            "Malformed JSON in request body",
        ))
    })
}

/// Hono's `jsonRegex`, case-insensitive:
/// `^application/([a-z-.]+\+)?json(;\s*[a-zA-Z0-9-]+=([^;]+))*$`.
fn is_json_content_type(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("application/") else {
        return false;
    };
    let (media, params) = match rest.split_once(';') {
        Some((media, params)) => (media, Some(params)),
        None => (rest, None),
    };
    let media_ok = media == "json"
        || media
            .strip_suffix("+json")
            .is_some_and(|suffix| {
                !suffix.is_empty()
                    && suffix
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte == b'-' || byte == b'.')
            });
    media_ok
        && params.is_none_or(|params| {
            params.split(';').all(|param| {
                param.trim_start().split_once('=').is_some_and(|(name, value)| {
                    !name.is_empty()
                        && name
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                        && !value.is_empty()
                })
            })
        })
}

/// 400 `{"success":false,"error":{"name":"ZodError","message":"<issues, 2-space JSON>"}}` with zod's
/// full issue objects (the `@hono/zod-validator` default hook).
fn zod_issues_400(issues: &[Value]) -> Response {
    let message = serde_json::to_string_pretty(issues).unwrap_or_default();
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "success": false, "error": { "name": "ZodError", "message": message } })),
    )
        .into_response()
}

/// JS `Number(string)` for the forms a path segment can take: trimmed decimal (with exponent),
/// `0x`/`0o`/`0b` integers, `Infinity`; anything else is `NaN` (`None`). An all-space string is 0.
fn js_number(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(0.0);
    }
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(digits) = trimmed.strip_prefix(prefix) {
            return u64::from_str_radix(digits, radix).ok().map(|n| n as f64);
        }
    }
    let unsigned = trimmed.trim_start_matches(['+', '-']);
    if unsigned == "Infinity" {
        return Some(if trimmed.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    let valid = trimmed.len() - unsigned.len() <= 1
        && !unsigned.is_empty()
        && unsigned
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'E' | b'+' | b'-'))
        && unsigned.bytes().next().is_some_and(|byte| byte.is_ascii_digit() || byte == b'.');
    if !valid {
        return None;
    }
    trimmed.parse::<f64>().ok()
}
