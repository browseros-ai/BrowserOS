//! ACP agent CRUD over the unified provider table. Config is validated/parsed for this
//! projection; successful updates and deletes discard live sessions so the next turn cannot
//! continue using a deleted command, stale credentials, or old persistent ACP state.

use axum::{
    Json,
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use http::StatusCode;
use server_api::models as api;

use super::wire::crud::{self, CrudJson};
use crate::{AppState, db};

pub async fn list(State(state): State<AppState>) -> Response {
    match state.agents.list().await {
        // The store owns updatedAt DESC ordering; sorting again would change tie behavior.
        Ok(rows) => Json(api::AgentList::new(
            rows.into_iter().map(crud::agent).collect(),
        ))
        .into_response(),
        Err(error) => crud::db_error(error),
    }
}

pub async fn create(
    State(state): State<AppState>,
    CrudJson(input): CrudJson<api::CreateAgentRequest>,
) -> Response {
    let (kind, name, model_id, reasoning_effort, working_directory, custom_config) = match input {
        api::CreateAgentRequest::CreateBuiltinAgentRequest(input) => (
            match input.r#type {
                api::CreateBuiltinAgentRequestType::Claude => db::AgentKind::Claude,
                api::CreateBuiltinAgentRequestType::Codex => db::AgentKind::Codex,
            },
            input.name,
            input.model_id,
            input.reasoning_effort,
            input.working_directory,
            None,
        ),
        api::CreateAgentRequest::CreateCustomAgentRequest(input) => {
            let custom_config = match serde_json::to_string(&input.custom_config) {
                Ok(config) => config,
                Err(error) => return crud::internal_error(&error.to_string()),
            };
            (
                db::AgentKind::Custom,
                input.name,
                input.model_id,
                input.reasoning_effort,
                input.working_directory,
                Some(custom_config),
            )
        }
    };
    match state
        .agents
        .create(db::CreateAgent {
            kind,
            name,
            model_id,
            reasoning_effort,
            working_directory: working_directory.map(Into::into),
            custom_config,
        })
        .await
    {
        Ok(row) => (
            StatusCode::CREATED,
            Json(api::AgentResponse::new(crud::agent(row))),
        )
            .into_response(),
        Err(error) => crud::db_error(error),
    }
}

pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let id = match crud::uuid_param(&id, "agentId") {
        Ok(id) => id,
        Err(response) => return response.into_response(),
    };
    match state.agents.get(id).await {
        Ok(Some(row)) => Json(api::AgentResponse::new(crud::agent(row))).into_response(),
        Ok(None) => crud::not_found("Unknown agent"),
        Err(error) => crud::db_error(error),
    }
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: axum::extract::Request,
) -> Response {
    // TS validates the path before parsing JSON; a malformed body must not turn an invalid
    // UUID's 400 into a 500. Keep this ordering explicit at the body-consuming seam.
    let id = match crud::uuid_param(&id, "agentId") {
        Ok(id) => id,
        Err(response) => return response.into_response(),
    };
    let CrudJson(input) = match <CrudJson<api::UpdateAgentRequest> as axum::extract::FromRequest<
        AppState,
    >>::from_request(request, &state)
    .await
    {
        Ok(input) => input,
        Err(response) => return response.into_response(),
    };
    match state.agents.get(id).await {
        Ok(Some(row)) if input.custom_config.is_some() && row.kind != db::AgentKind::Custom => {
            return super::errors::error_string(
                StatusCode::BAD_REQUEST,
                "customConfig is only allowed for custom agents",
            );
        }
        Ok(Some(_)) => {}
        Ok(None) => return crud::not_found("Unknown agent"),
        Err(error) => return crud::db_error(error),
    }
    let custom_config = match input
        .custom_config
        .map(|value| serde_json::to_string(&value))
        .transpose()
    {
        Ok(value) => value.map(Some),
        Err(error) => return crud::internal_error(&error.to_string()),
    };
    let patch = db::UpdateAgent {
        name: input.name,
        model_id: input.model_id,
        reasoning_effort: input.reasoning_effort,
        working_directory: input.working_directory.map(|value| value.map(Into::into)),
        custom_config,
    };
    match state.agents.update(id, patch).await {
        Ok(Some(row)) => {
            // Persistence precedes runtime invalidation. Once the HTTP write succeeds, every
            // future turn must be spawned from the saved config, even if no session exists.
            state.acp.close_all_for_agent(id, true).await;
            Json(api::AgentResponse::new(crud::agent(row))).into_response()
        }
        Ok(None) => crud::not_found("Unknown agent"),
        Err(error) => crud::db_error(error),
    }
}

pub async fn delete(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let id = match crud::uuid_param(&id, "agentId") {
        Ok(id) => id,
        Err(response) => return response.into_response(),
    };
    match state.agents.delete(id).await {
        Ok(true) => {
            state.acp.close_all_for_agent(id, true).await;
            Json(api::SuccessResponse::new(true)).into_response()
        }
        Ok(false) => crud::not_found("Unknown agent"),
        Err(error) => crud::db_error(error),
    }
}
