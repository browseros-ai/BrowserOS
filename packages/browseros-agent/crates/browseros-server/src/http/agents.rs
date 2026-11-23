//! `/agents`: the coding-agent rows (Claude Code, Codex). `GET /agents` must answer `200` even
//! when empty, because the extension's composer waits on it before every send.

use axum::Json;
use axum::extract::State;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::ApiError;
use super::dto;
use super::extract::{Body, PathParams};
use crate::app::App;

pub(super) fn routes() -> OpenApiRouter<App> {
    OpenApiRouter::new()
        .routes(routes!(list_agents, create_agent))
        .routes(routes!(update_agent, delete_agent))
}

/// Coding agents. Must answer 200 even when empty; the composer waits on it.
#[utoipa::path(
    get,
    path = "/agents",
    tag = "agents",
    operation_id = "listAgents",
    responses(
        (status = 200, description = "Agents of a type this server runs.", body = dto::AgentList),
    )
)]
async fn list_agents(State(app): State<App>) -> Result<Json<dto::AgentList>, ApiError> {
    let agents = app.store.agents().list().await?;
    Ok(Json(dto::AgentList {
        agents: agents.into_iter().map(dto::AgentBody::from).collect(),
    }))
}

/// Create a coding agent; the server makes the id.
#[utoipa::path(
    post,
    path = "/agents",
    tag = "agents",
    operation_id = "createAgent",
    request_body = dto::AgentInput,
    responses(
        (status = 201, description = "Created; the server made the id.", body = dto::AgentResponse),
        (status = 400, description = "The body is not an agent.", body = dto::ApiErrorBody),
    )
)]
async fn create_agent(
    State(app): State<App>,
    Body(body): Body<dto::AgentInput>,
) -> Result<(axum::http::StatusCode, Json<dto::AgentResponse>), ApiError> {
    let new = crate::store::NewAgent::try_from(body).map_err(ApiError::bad_request)?;
    let agent = app.store.agents().create(new).await?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(dto::AgentResponse {
            agent: agent.into(),
        }),
    ))
}

/// Edit a coding agent.
#[utoipa::path(
    put,
    path = "/agents/{agentId}",
    tag = "agents",
    operation_id = "updateAgent",
    params(dto::AgentPath),
    request_body = dto::AgentPatchBody,
    responses(
        (status = 200, description = "Updated.", body = dto::AgentResponse),
        (status = 400, description = "The body is not an agent patch.", body = dto::ApiErrorBody),
        (status = 404, description = "No such agent.", body = dto::ApiErrorBody),
    )
)]
async fn update_agent(
    State(app): State<App>,
    PathParams(path): PathParams<dto::AgentPath>,
    Body(body): Body<dto::AgentPatchBody>,
) -> Result<Json<dto::AgentResponse>, ApiError> {
    let patch = crate::store::AgentPatch::try_from(body).map_err(ApiError::bad_request)?;
    let agent = app
        .store
        .agents()
        .update(&path.agent_id, patch)
        .await?
        .ok_or_else(|| ApiError::not_found("Unknown agent"))?;
    Ok(Json(dto::AgentResponse {
        agent: agent.into(),
    }))
}

/// Delete a coding agent.
#[utoipa::path(
    delete,
    path = "/agents/{agentId}",
    tag = "agents",
    operation_id = "deleteAgent",
    params(dto::AgentPath),
    responses(
        (status = 200, description = "Deleted.", body = dto::Success),
        (status = 404, description = "No such agent.", body = dto::ApiErrorBody),
    )
)]
async fn delete_agent(
    State(app): State<App>,
    PathParams(path): PathParams<dto::AgentPath>,
) -> Result<Json<dto::Success>, ApiError> {
    if !app.store.agents().delete(&path.agent_id).await? {
        return Err(ApiError::not_found("Unknown agent"));
    }
    Ok(Json(dto::Success { success: true }))
}
