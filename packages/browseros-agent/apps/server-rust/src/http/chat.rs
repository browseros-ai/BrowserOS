//! `/chat*`: start a turn (UI message stream v1 SSE), the panels SSE (custom frames, 5 s heartbeat,
//! no `[DONE]`), run state, resume (204 / 409 / replay + tail), stop, and session delete.
//! Thin over `chat::ChatService`; every status and body is the TS route's (`api/routes/chat.ts`).
//!
//! Streams are subscribers: the response body is a cursor over the run's replay log, so a client
//! disconnect drops only that cursor and never cancels the run (`POST …/stop` and DELETE do).
//!
//! Handlers take `State<Arc<ChatService>>` (projected from `AppState` by `FromRef`) and the
//! security middleware's [`RequestTrust`], so they can be mounted on a test router with a scripted
//! service.

use super::Json;
use super::{
    errors,
    extract::{ZodJson, ZodUuid},
    security::RequestTrust,
    wire::chat as wire,
};
use crate::{
    app::AppState,
    chat::{ChatRejection, ChatService, Resume, request},
};
use axum::{
    body::Body,
    extract::{FromRef, Path, Query, State},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt as _;
use http::{HeaderValue, StatusCode, header};
use serde::Deserialize;
use serde_json::Value;
use std::{convert::Infallible, sync::Arc};
use ui_message_stream::{ChunkStream, sse};

impl FromRef<AppState> for Arc<ChatService> {
    fn from_ref(state: &AppState) -> Self {
        state.chat.clone()
    }
}

fn forbidden() -> Response {
    errors::error_string(StatusCode::FORBIDDEN, "Forbidden")
}

/// `POST /chat`: answers only after the engine is prepared; the body is the turn's stream.
/// The body is read the Hono way by `ZodJson` (non-JSON content type → `{}`, malformed → 500) and
/// validated by `parse_chat_request`, whose issues are already in zod's key order.
pub async fn post_chat(
    State(chat): State<Arc<ChatService>>,
    trust: RequestTrust,
    ZodJson(body): ZodJson<Value>,
) -> Response {
    let request = match request::parse_chat_request(&body) {
        Ok(request) => request,
        Err(issues) => return errors::zod_issues_400(&issues),
    };
    match chat.start(request, trust).await {
        Ok(chunks) => ui_message_stream_response(chunks),
        Err(rejection) => rejection_response(rejection),
    }
}

/// `GET /chat/panels`: the background page's routing table, as reconnectable SSE.
pub async fn panels(State(chat): State<Arc<ChatService>>, trust: RequestTrust) -> Response {
    if !trust.trusted_app {
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
    trust: RequestTrust,
    Path(tab_id): Path<String>,
) -> Response {
    if !trust.trusted_app {
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

/// `GET /chat/{conversationId}/state`: waits on the prepared gate (no timeout, TS parity). The id
/// is `safeParse`d in TS, so a bad one is `{"error":"Invalid conversation id"}`, not a zod body.
pub async fn state(
    State(chat): State<Arc<ChatService>>,
    Path(conversation_id): Path<String>,
) -> Response {
    let Some(ZodUuid(conversation_id)) = ZodUuid::parse(&conversation_id) else {
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
    ZodUuid(conversation_id): ZodUuid,
    Query(query): Query<ResumeQuery>,
) -> Response {
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
    ZodUuid(conversation_id): ZodUuid,
) -> Response {
    Json(server_api::StopChatResponse::new(
        chat.stop(conversation_id).await,
    ))
    .into_response()
}

/// `DELETE /chat/{conversationId}`: trusted-app check only for ACP conversations.
pub async fn delete_conversation_session(
    State(chat): State<Arc<ChatService>>,
    trust: RequestTrust,
    ZodUuid(conversation_id): ZodUuid,
) -> Response {
    if chat.is_acp_conversation(conversation_id) && !trust.trusted_app {
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

/// JS `Number(string)` for the forms a path segment can take: trimmed decimal (with exponent),
/// `0x`/`0o`/`0b` integers, `Infinity`; anything else is `NaN` (`None`). An all-space string is 0.
fn js_number(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(0.0);
    }
    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
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
        && unsigned
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_digit() || byte == b'.');
    if !valid {
        return None;
    }
    trimmed.parse::<f64>().ok()
}
