//! `/agents`: the coding-agent rows (Claude Code, Codex). `GET /agents` must answer `200` even
//! when empty, because the extension's composer waits on it before every send. Chunk
//! `settings` fills the bodies.

use axum::extract::State;
use axum::response::Response;
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
async fn list_agents(State(_app): State<App>) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
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
    State(_app): State<App>,
    Body(body): Body<dto::AgentInput>,
) -> Result<Response, ApiError> {
    let _new = crate::store::NewAgent::try_from(body).map_err(ApiError::bad_request)?;
    Err(ApiError::not_implemented())
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
    State(_app): State<App>,
    PathParams(_path): PathParams<dto::AgentPath>,
    Body(body): Body<dto::AgentPatchBody>,
) -> Result<Response, ApiError> {
    let _patch = crate::store::AgentPatch::try_from(body).map_err(ApiError::bad_request)?;
    Err(ApiError::not_implemented())
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
    State(_app): State<App>,
    PathParams(_path): PathParams<dto::AgentPath>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
}
