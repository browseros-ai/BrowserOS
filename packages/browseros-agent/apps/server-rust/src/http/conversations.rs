//! Persisted conversation history CRUD. Legacy import is an atomic insert-if-absent, separate
//! from live chat/session deletion: importing old extension state must never clobber newer
//! server messages, and deleting stored history must not cancel a running turn.

use super::Json;
use axum::{
    extract::{FromRequest, Path, Request, State},
    response::{IntoResponse, Response},
};
use serde_json::json;
use server_api::models as api;

use super::wire::crud::{self, CrudJson};
use crate::{AppState, db};

pub async fn list(State(state): State<AppState>) -> Response {
    match state.conversations.list_stored().await {
        Ok(rows) => {
            let summaries = rows
                .into_iter()
                .map(crud::stored_conversation_summary)
                .collect::<Vec<_>>();
            Json(json!({"conversations": summaries})).into_response()
        }
        Err(error) => crud::db_error(error),
    }
}

pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    // Preserve TS validation without normalizing a case-sensitive persisted identifier.
    if let Err(response) = crud::uuid_param(&id, "conversationId") {
        return response.into_response();
    }
    match state.conversations.get_stored(&id).await {
        Ok(Some(row)) => {
            Json(json!({"conversation": crud::stored_conversation_detail(row)})).into_response()
        }
        Ok(None) => crud::not_found("Unknown conversation"),
        Err(error) => crud::db_error(error),
    }
}

pub async fn import(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let id = match crud::uuid_param(&id, "conversationId") {
        Ok(id) => id,
        Err(response) => return response.into_response(),
    };
    let CrudJson(input) =
        match CrudJson::<api::ImportConversationRequest>::from_request(request, &state).await {
            Ok(input) => input,
            Err(response) => return response.into_response(),
        };
    let messages = match input
        .messages
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(messages) => messages,
        Err(error) => return crud::internal_error(&error.to_string()),
    };
    let input = db::ImportConversation {
        messages,
        origin: input.origin,
        // The store must retain this metadata and raw messages for the legacy import path.
        target_type: match input.target_type.unwrap_or_default() {
            api::ImportConversationRequestTargetType::Browseros => db::TargetType::BrowserOs,
            api::ImportConversationRequestTargetType::Claude => db::TargetType::Claude,
            api::ImportConversationRequestTargetType::Codex => db::TargetType::Codex,
        },
        agent_id: input.agent_id,
        last_messaged_at: input.last_messaged_at.map(crud::store_integer),
    };
    match state.conversations.import_if_absent(id, input).await {
        Ok(row) => {
            let imported = row.is_some();
            Json(api::ImportConversationResponse::new(
                row.map(crud::conversation_summary),
                imported,
            ))
            .into_response()
        }
        Err(error) => crud::db_error(error),
    }
}

pub async fn delete(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    if let Err(response) = crud::uuid_param(&id, "conversationId") {
        return response.into_response();
    }
    crud::deleted(
        state.conversations.delete(&id).await,
        "Unknown conversation",
    )
}
