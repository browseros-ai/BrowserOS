//! `/chat`: start a turn and stream it, a second panel's state and replay, stop, release, and
//! the side-panel table. Every handler here is a thin caller of `Conversations`; the run is the
//! server's, so dropping a response detaches one subscriber and stops nothing. Chunk `runs`
//! fills the bodies.

use axum::extract::{DefaultBodyLimit, State};
use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::ApiError;
use super::dto;
use super::extract::{ChatBody, PathParams, QueryParams};
use crate::app::App;
use crate::conversations::NewTurn;

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
    State(_app): State<App>,
    ChatBody(body): ChatBody<dto::ChatRequest>,
) -> Result<Response, ApiError> {
    let _turn = NewTurn::from(body);
    Err(ApiError::not_implemented())
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
async fn watch_panels(State(_app): State<App>) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
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
    State(_app): State<App>,
    PathParams(_path): PathParams<dto::TabPath>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
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
    State(_app): State<App>,
    PathParams(_path): PathParams<dto::ConversationPath>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
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
    State(_app): State<App>,
    PathParams(_path): PathParams<dto::ConversationPath>,
    QueryParams(_query): QueryParams<dto::StreamQuery>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
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
    State(_app): State<App>,
    PathParams(_path): PathParams<dto::ConversationPath>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
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
    State(_app): State<App>,
    PathParams(_path): PathParams<dto::ConversationPath>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
}
