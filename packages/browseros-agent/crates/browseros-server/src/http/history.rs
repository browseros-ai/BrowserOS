//! `/conversations`: the history list, one stored conversation, and deleting one. Reads and
//! the delete go through `Conversations`, the table's only writer, so a delete also releases a
//! live run first.

use axum::Json;
use axum::extract::State;
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
async fn list_conversations(
    State(app): State<App>,
) -> Result<Json<dto::ConversationList>, ApiError> {
    let conversations = app.conversations.list().await?;
    Ok(Json(dto::ConversationList {
        conversations: conversations
            .into_iter()
            .map(dto::ConversationSummaryBody::from)
            .collect(),
    }))
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
    State(app): State<App>,
    PathParams(path): PathParams<dto::ConversationPath>,
) -> Result<Json<dto::ConversationResponse>, ApiError> {
    let conversation = require_conversation(app.conversations.get(path.conversation_id).await?)?;
    Ok(Json(dto::ConversationResponse {
        conversation: conversation.into(),
    }))
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
    State(app): State<App>,
    PathParams(path): PathParams<dto::ConversationPath>,
) -> Result<Json<dto::Success>, ApiError> {
    let deleted = app.conversations.delete(path.conversation_id).await?;
    require_conversation(deleted.then_some(dto::Success { success: true })).map(Json)
}

fn require_conversation<T>(value: Option<T>) -> Result<T, ApiError> {
    value.ok_or_else(|| ApiError::not_found("Unknown conversation"))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    use super::require_conversation;

    #[tokio::test]
    async fn a_missing_conversation_is_a_json_404() -> anyhow::Result<()> {
        let response = require_conversation::<()>(None)
            .err()
            .ok_or_else(|| anyhow::anyhow!("missing conversation was treated as found"))?
            .into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body)?,
            serde_json::json!({ "error": "Unknown conversation" })
        );
        Ok(())
    }
}
