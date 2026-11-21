//! `POST /acpx/probe`: which models and reasoning levels a coding agent offers. A probe that
//! cannot run answers `200` with `error` set, so the new-agent dialog still opens. Chunk `acp`
//! fills the body.

use axum::extract::State;
use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::ApiError;
use super::dto;
use super::extract::Body;
use crate::app::App;

pub(super) fn routes() -> OpenApiRouter<App> {
    OpenApiRouter::new().routes(routes!(probe_agent))
}

/// Ask an agent which models and reasoning levels it offers.
///
/// Starts the agent's adapter, reads what it advertises, and stops it. It does not prove the
/// agent is signed in. A probe that cannot run answers 200 with `error` set and the defaults
/// `models: []`, `reasoning: null`, `supportsConfigOption: false`, `agentInfo: null`,
/// `protocolVersion: 0`; the dialog still lets the agent be created. (The old server answered
/// such a probe with `500 {error: {code, message}}`; the dialog renders either.)
#[utoipa::path(
    post,
    path = "/acpx/probe",
    tag = "agents",
    operation_id = "probeAgent",
    request_body = dto::ProbeRequest,
    responses(
        (status = 200, description = "What the agent advertised.", body = dto::AgentProbeBody),
        (status = 400, description = "The body did not parse.", body = dto::ApiErrorBody),
    )
)]
async fn probe_agent(
    State(_app): State<App>,
    Body(_body): Body<dto::ProbeRequest>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
}
