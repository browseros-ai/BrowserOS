//! `/conversations`: the history list, one stored conversation, and deleting one. Reads and
//! the delete go through `Conversations`, the table's only writer, so a delete also releases a
//! live run first. Chunk `settings` fills the bodies.

use axum::extract::State;
use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::ApiError;
use super::dto;
use super::extract::PathParams;
use crate::app::App;

pub(super) fn routes() -> OpenApiRouter<App> {
    OpenApiRouter::new()
        .routes(routes!(list_conversations))
        .routes(routes!(get_conversation, delete_conversation))
}

/// Every conversation, newest first, without messages.
#[utoipa::path(
    get,
    path = "/conversations",
    tag = "conversations",
    operation_id = "listConversations",
    responses(
        (status = 200, description = "Summaries.", body = dto::ConversationList),
    )
)]
async fn list_conversations(State(_app): State<App>) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
}

/// One conversation with its transcript.
#[utoipa::path(
    get,
    path = "/conversations/{conversationId}",
    tag = "conversations",
    operation_id = "getConversation",
    params(dto::ConversationPath),
    responses(
        (status = 200, description = "The conversation with its transcript.", body = dto::ConversationResponse),
        (status = 400, description = "The conversation id is not a UUID.", body = dto::ApiErrorBody),
        (status = 404, description = "No such conversation.", body = dto::ApiErrorBody),
    )
)]
async fn get_conversation(
    State(_app): State<App>,
    PathParams(_path): PathParams<dto::ConversationPath>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
}

/// Delete the stored conversation and release its live state.
#[utoipa::path(
    delete,
    path = "/conversations/{conversationId}",
    tag = "conversations",
    operation_id = "deleteConversation",
    params(dto::ConversationPath),
    responses(
        (status = 200, description = "Deleted.", body = dto::Success),
        (status = 400, description = "The conversation id is not a UUID.", body = dto::ApiErrorBody),
        (status = 404, description = "No such conversation.", body = dto::ApiErrorBody),
    )
)]
async fn delete_conversation(
    State(_app): State<App>,
    PathParams(_path): PathParams<dto::ConversationPath>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
}
