use super::{error, internal};
use crate::{
    AppState, error::CanonicalError, error::RequestId, services::agent_settings::HumanHelp,
};
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
};
use claw_api::models::{
    AgentSettings, TelemetryState, UpdateAgentSettingsRequest, UpdateTelemetryRequest,
};

pub(super) async fn telemetry(State(state): State<AppState>) -> Json<TelemetryState> {
    Json(to_contract_state(state.analytics.get_state().await))
}

pub(super) async fn update_telemetry(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    payload: Result<Json<UpdateTelemetryRequest>, JsonRejection>,
) -> Result<Json<TelemetryState>, CanonicalError> {
    let Json(payload) = payload.map_err(|_| {
        error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "consent must be a boolean",
        )
    })?;
    let telemetry = state
        .analytics
        .set_consent(payload.consent)
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(to_contract_state(telemetry)))
}

pub(super) async fn agent(State(state): State<AppState>) -> Json<AgentSettings> {
    Json(to_contract_agent_settings(
        state.agent_settings.human_help(),
    ))
}

pub(super) async fn update_agent(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    payload: Result<Json<UpdateAgentSettingsRequest>, JsonRejection>,
) -> Result<Json<AgentSettings>, CanonicalError> {
    let Json(payload) = payload.map_err(|_| {
        error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "humanHelpEnabled must be a boolean",
        )
    })?;
    let human_help = HumanHelp::from_enabled(payload.human_help_enabled);
    state
        .agent_settings
        .set_human_help(human_help)
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(to_contract_agent_settings(human_help)))
}

fn to_contract_state(state: crate::analytics::TelemetryState) -> TelemetryState {
    TelemetryState::new(state.distinct_id, state.enabled, state.consent)
}

fn to_contract_agent_settings(human_help: HumanHelp) -> AgentSettings {
    AgentSettings::new(human_help.is_enabled())
}
