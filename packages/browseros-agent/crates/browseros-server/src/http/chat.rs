//! `/chat`: start a turn and stream it, a second panel's state and replay, stop, release, and
//! the side-panel table. Every handler here is a thin caller of `Conversations`; the run is the
//! server's, so dropping a response detaches one subscriber and stops nothing.

use std::convert::Infallible;

use axum::Json;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::{Stream, StreamExt as _, stream};
use tokio::sync::broadcast;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::ApiError;
use super::dto;
use super::extract::{ChatBody, PathParams, QueryParams};
use crate::app::App;
use crate::conversations::{Attach, Busy, NewTurn};
use crate::turn::{self, ChatError, ChatErrorCode, Frame};

/// The largest `POST /chat` body: attachments travel inline as base64 data URLs (a 5 MiB image
/// is about 6.7 MiB on the wire, and a message may carry several). axum's default is 2 MiB.
const CHAT_BODY_LIMIT: usize = 64 * 1024 * 1024;

pub(super) fn routes() -> OpenApiRouter<App> {
    OpenApiRouter::new()
        .routes(routes!(start_turn))
        .routes(routes!(watch_panels))
        .routes(routes!(forget_panel))
        .routes(routes!(get_run_state))
        .routes(routes!(resume_stream))
        .routes(routes!(stop_turn))
        .routes(routes!(release_conversation))
        // Only `POST /chat` carries a body; the limit reaches no other route.
        .layer(DefaultBodyLimit::max(CHAT_BODY_LIMIT))
}

/// Start one turn and stream it.
///
/// Starts a server-owned run for the conversation and returns a subscriber view of it. Closing
/// this response does not stop the run; `stopTurn` does. A turn that cannot run (unknown
/// provider, no provider selected, not signed in, an agent that will not start) is a `200`
/// stream whose last chunk is `error`; only a body that does not parse is a `400`. The body is
/// the AI SDK UI message stream v1 for `ai@7.0.114`: each event is `data: <UiMessageChunk>`,
/// and the stream ends with `data: [DONE]`.
#[utoipa::path(
    post,
    path = "/chat",
    tag = "chat",
    operation_id = "startTurn",
    request_body = dto::ChatRequest,
    responses(
        (status = 200, description = "The run's chunks from the first one, then live.",
            content_type = "text/event-stream", body = dto::UiMessageChunk,
            headers(("x-vercel-ai-ui-message-stream" = String, description = "Always `v1`."))),
        (status = 400, description = "The body did not parse.", body = dto::ChatErrorEnvelope),
        (status = 409, description = "A turn is already running for this conversation.", body = dto::ChatErrorEnvelope),
    )
)]
async fn start_turn(
    State(app): State<App>,
    ChatBody(body): ChatBody<dto::ChatRequest>,
) -> Result<Response, ApiError> {
    // Boundary: the wire body becomes a domain `NewTurn` here. Nothing below sees a wire type.
    let turn = NewTurn::from(body);
    // From here the run belongs to `conversations`, not to this request: dropping the response
    // detaches one subscriber and stops nothing.
    match app.conversations.send(turn).await {
        Ok(subscription) => Ok(ui_stream(subscription)),
        Err(Busy) => Err(ApiError::chat(StatusCode::CONFLICT, busy())),
    }
}

/// The UI stream response: r4's five headers, one `data:` line per frame, then `[DONE]`. Frames
/// are compact JSON, so none holds a newline that would split its event.
fn ui_stream(frames: impl Stream<Item = Frame> + Send + 'static) -> Response {
    let events = frames
        .map(|frame| Event::default().data(frame.data()))
        .chain(stream::once(async { Event::default().data(turn::DONE) }))
        .map(Ok::<_, Infallible>);
    // The array's headers replace the ones `Sse` sets, so each is sent once.
    (turn::STREAM_HEADERS, Sse::new(events)).into_response()
}

fn busy() -> ChatError {
    ChatError::new(
        ChatErrorCode::Unknown,
        "Already answering",
        "An answer is still running in this conversation. Wait for it or stop it, then send again.",
    )
}

/// Which tab's side panel shows which run.
///
/// Endless server-sent events. Every event is the whole table, sent on connect, on every
/// change, and every 5 seconds as a heartbeat. The extension's background worker is the only
/// reader.
#[utoipa::path(
    get,
    path = "/chat/panels",
    tag = "chat",
    operation_id = "watchPanels",
    responses(
        (status = 200, description = "A stream of full tables.",
            content_type = "text/event-stream", body = dto::PanelAssignments),
    )
)]
async fn watch_panels(State(app): State<App>) -> Result<Response, ApiError> {
    // Subscribe before reading the first table, so nothing published in between is missed.
    let receiver = app.conversations.panels();
    let first = app.conversations.current_panels();
    let conversations = app.conversations.clone();
    let tables = stream::unfold((receiver, Some(first)), move |(mut receiver, first)| {
        let conversations = conversations.clone();
        async move {
            if let Some(first) = first {
                return Some((first, (receiver, None)));
            }
            let table = match receiver.recv().await {
                Ok(table) => table,
                // This stream fell more than the backlog behind. The extension needs every
                // intermediate table (a tab `running` with a new `autoOpenId` is what opens a
                // panel), so it is not dropped silently: the current table goes out, and the
                // next heartbeat or change follows.
                Err(broadcast::error::RecvError::Lagged(_)) => conversations.current_panels(),
                Err(broadcast::error::RecvError::Closed) => return None,
            };
            Some((table, (receiver, None)))
        }
    });
    let events =
        tables.map(|table| Event::default().json_data(dto::PanelAssignments::from(table.as_ref())));
    Ok(Sse::new(events).into_response())
}

/// Forget a closed tab.
#[utoipa::path(
    delete,
    path = "/chat/panels/{tabId}",
    tag = "chat",
    operation_id = "forgetPanel",
    params(dto::TabPath),
    responses(
        (status = 200, description = "Forgotten, or already unknown.", body = dto::Success),
        (status = 400, description = "The tab id is not a non-negative integer.", body = dto::ApiErrorBody),
    )
)]
async fn forget_panel(
    State(app): State<App>,
    PathParams(path): PathParams<dto::TabPath>,
) -> Result<Response, ApiError> {
    app.conversations.forget_panel(path.tab_id);
    Ok(Json(dto::Success::OK).into_response())
}

/// The current run of a conversation, for a panel that did not start it.
#[utoipa::path(
    get,
    path = "/chat/{conversationId}/state",
    tag = "chat",
    operation_id = "getRunState",
    params(dto::ConversationPath),
    responses(
        (status = 200, description = "While `status` is `running`, `messages` is the history up to and including this turn's user message (the stream supplies the answer); once the run has ended, it is the whole transcript including the sealed answer.",
            body = dto::RunStateBody,
            headers(("cache-control" = String, description = "Always `no-store`."))),
        (status = 400, description = "The conversation id is not a UUID.", body = dto::ApiErrorBody),
        (status = 404, description = "No run is known for this conversation.", body = dto::ApiErrorBody),
    )
)]
async fn get_run_state(
    State(app): State<App>,
    PathParams(path): PathParams<dto::ConversationPath>,
) -> Result<Response, ApiError> {
    let state = app
        .conversations
        .state(path.conversation_id)
        .ok_or_else(|| ApiError::not_found("Conversation run not found"))?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(dto::RunStateBody::from(state)),
    )
        .into_response())
}

/// Replay a run from its first chunk, then follow it live.
#[utoipa::path(
    get,
    path = "/chat/{conversationId}/stream",
    tag = "chat",
    operation_id = "resumeStream",
    params(dto::ConversationPath, dto::StreamQuery),
    responses(
        (status = 200, description = "Same framing as `startTurn`. A finished run replays and ends.",
            content_type = "text/event-stream", body = dto::UiMessageChunk),
        (status = 204, description = "No run is known for this conversation, or its replay log is gone."),
        (status = 400, description = "The conversation id or `runId` is not a UUID.", body = dto::ApiErrorBody),
        (status = 409, description = "A newer run replaced `runId`. Re-read the state.", body = dto::ApiErrorBody),
    )
)]
async fn resume_stream(
    State(app): State<App>,
    PathParams(path): PathParams<dto::ConversationPath>,
    QueryParams(query): QueryParams<dto::StreamQuery>,
) -> Result<Response, ApiError> {
    match app.conversations.attach(path.conversation_id, query.run_id) {
        // Replay and live are one code path: a subscription is a cursor that starts at frame 0.
        Attach::Run(subscription) => Ok(ui_stream(subscription)),
        Attach::NoRun => Ok(StatusCode::NO_CONTENT.into_response()),
        Attach::Changed => Err(ApiError::conflict("Conversation run changed")),
    }
}

/// Stop the running turn. The only call that cancels a run.
#[utoipa::path(
    post,
    path = "/chat/{conversationId}/stop",
    tag = "chat",
    operation_id = "stopTurn",
    params(dto::ConversationPath),
    responses(
        (status = 200, description = "`stopped` is false when nothing was running.", body = dto::StopResult),
        (status = 400, description = "The conversation id is not a UUID.", body = dto::ApiErrorBody),
    )
)]
async fn stop_turn(
    State(app): State<App>,
    PathParams(path): PathParams<dto::ConversationPath>,
) -> Result<Response, ApiError> {
    // Answers at once: the run unwinds on its own task and its stream ends with `abort`.
    let stopped = app.conversations.stop(path.conversation_id);
    Ok(Json(dto::StopResult { stopped }).into_response())
}

/// Release what the server holds live for a conversation.
///
/// Stops its run, forgets the run record and its panel assignments. The stored conversation is
/// untouched (`deleteConversation` removes that). Idempotent.
#[utoipa::path(
    delete,
    path = "/chat/{conversationId}",
    tag = "chat",
    operation_id = "releaseConversation",
    params(dto::ConversationPath),
    responses(
        (status = 200, description = "Released, or nothing was held.", body = dto::Success),
        (status = 400, description = "The conversation id is not a UUID.", body = dto::ApiErrorBody),
    )
)]
async fn release_conversation(
    State(app): State<App>,
    PathParams(path): PathParams<dto::ConversationPath>,
) -> Result<Response, ApiError> {
    app.conversations.release(path.conversation_id).await;
    Ok(Json(dto::Success::OK).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn::{MessageId, Transcript, TurnEnd};

    /// The two renderings of one error must be the same JSON: the panel's error card parses a
    /// `409` body and an `error` chunk's `errorText` with one function.
    #[test]
    fn the_error_chunk_carries_the_response_body_envelope() -> Result<(), serde_json::Error> {
        let mut error = ChatError::new(ChatErrorCode::AuthFailed, "Signed out", "Sign in again.");
        error.provider = Some("chatgpt-pro".into());
        error.status_code = Some(401);
        error.docs_url = Some("https://docs.browseros.com".into());
        error.retry_after_seconds = Some(3);
        error.details = Some("token expired".into());

        let mut transcript = Transcript::new(MessageId("m".into()));
        transcript.finish(&TurnEnd::Failed(error.clone()));
        let last = transcript
            .frames(0)
            .last()
            .map(Frame::data)
            .unwrap_or_default();
        let chunk: serde_json::Value = serde_json::from_str(last)?;
        let from_stream: serde_json::Value =
            serde_json::from_str(chunk["errorText"].as_str().unwrap_or_default())?;

        let from_body = serde_json::to_value(dto::ChatErrorEnvelope::from(error))?;
        assert_eq!(from_stream, from_body);
        Ok(())
    }
}
