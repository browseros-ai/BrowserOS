//! Persisted conversation history CRUD. Legacy import is an atomic insert-if-absent, separate
//! from live chat/session deletion: importing old extension state must never clobber newer
//! server messages, and deleting stored history must not cancel a running turn.

use axum::{
    Json,
    extract::{FromRequest, Path, Request, State},
    response::{IntoResponse, Response},
};
use server_api::models as api;

use super::wire::crud::{self, CrudJson};
use crate::{AppState, db};

pub async fn list(State(state): State<AppState>) -> Response {
    match state.conversations.list().await {
        Ok(rows) => Json(api::ConversationList::new(
            rows.into_iter().map(crud::conversation_summary).collect(),
        ))
        .into_response(),
        Err(error) => crud::db_error(error),
    }
}

pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let id = match crud::uuid_param(&id, "conversationId") {
        Ok(id) => id,
        Err(response) => return response,
    };
    match state.conversations.get(id).await {
        Ok(Some(row)) => match crud::conversation_detail(row) {
            Ok(row) => Json(api::ConversationResponse::new(row)).into_response(),
            Err(response) => response,
        },
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
        Err(response) => return response,
    };
    let CrudJson(input) =
        match CrudJson::<api::ImportConversationRequest>::from_request(request, &state).await {
            Ok(input) => input,
            Err(response) => return response,
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
        last_messaged_at: input.last_messaged_at,
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
    let id = match crud::uuid_param(&id, "conversationId") {
        Ok(id) => id,
        Err(response) => return response,
    };
    crud::deleted(state.conversations.delete(id).await, "Unknown conversation")
}
