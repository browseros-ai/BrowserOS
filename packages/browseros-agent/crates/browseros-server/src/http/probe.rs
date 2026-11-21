//! `POST /acpx/probe`: which models and reasoning levels a coding agent offers. A probe that
//! cannot run answers `200` with `error` set, so the new-agent dialog still opens.

use axum::extract::State;
use std::time::Duration;

use axum::Json;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::extract::Body;
use super::{ApiError, dto};
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
    State(app): State<App>,
    Body(body): Body<dto::ProbeRequest>,
) -> Result<Json<dto::AgentProbeBody>, ApiError> {
    let timeout_ms = body.timeout_ms.unwrap_or(120_000);
    if !(1_000..=120_000).contains(&timeout_ms) {
        return Err(ApiError::bad_request(
            "timeoutMs must be between 1000 and 120000.",
        ));
    }
    let timeout = Duration::from_millis(timeout_ms);
    Ok(Json(
        app.agent.probe(body.r#type.into(), timeout).await.into(),
    ))
}
