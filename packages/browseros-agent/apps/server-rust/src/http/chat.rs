//! `/chat*`: start a turn (UI message stream v1 SSE), the panels SSE (custom frames, 5 s heartbeat,
//! no `[DONE]`), run state, resume (204 / 409 / replay + tail), stop, and session delete.
//! Thin over `chat::ChatService`. Owned by P8.

use axum::response::Response;

/// `POST /chat`: answers only after the engine is prepared; the body is the turn's stream.
pub async fn post_chat() -> Response {
    super::not_implemented()
}

pub async fn panels() -> Response {
    super::not_implemented()
}

pub async fn delete_panel() -> Response {
    super::not_implemented()
}

/// `GET /chat/{conversationId}/state`: waits on the prepared gate.
pub async fn state() -> Response {
    super::not_implemented()
}

/// `GET /chat/{conversationId}/stream`: a disconnect drops only this subscriber.
pub async fn resume_stream() -> Response {
    super::not_implemented()
}

pub async fn stop() -> Response {
    super::not_implemented()
}

/// `DELETE /chat/{conversationId}`: trusted-app check only for ACP conversations.
pub async fn delete_conversation_session() -> Response {
    super::not_implemented()
}
