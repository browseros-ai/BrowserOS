use super::{error, internal};
use crate::{
    AppState,
    config::TokenSource,
    error::CanonicalError,
    error::RequestId,
    services::agent_settings::{HumanHelp, MAX_JEV_TOKEN},
};
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
};
use claw_api::models::{
    AgentSettings, JevTokenSource, SaveJevTokenRequest, TelemetryState, UpdateAgentSettingsRequest,
    UpdateTelemetryRequest,
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
    Json(settings_of(&state))
}

pub(super) async fn update_agent(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    payload: Result<Json<UpdateAgentSettingsRequest>, JsonRejection>,
) -> Result<Json<AgentSettings>, CanonicalError> {
    let invalid_settings = || {
        error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "humanHelpEnabled and taskCleanupTipEnabled must be booleans",
        )
    };
    let Json(payload) = payload.map_err(|_| invalid_settings())?;
    if payload.human_help_enabled.is_none() && payload.task_cleanup_tip_enabled.is_none() {
        return Err(invalid_settings());
    }
    let human_help = payload.human_help_enabled.map(HumanHelp::from_enabled);
    state
        .agent_settings
        .update(human_help, payload.task_cleanup_tip_enabled)
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(settings_of(&state)))
}

/// Stores the decision model credential, which offers the pursue tool.
///
/// Its own route rather than a field on `update_agent`, so that saving a switch can never clear
/// a credential. The reply carries the source and never the token.
pub(super) async fn save_jev_token(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    payload: Result<Json<SaveJevTokenRequest>, JsonRejection>,
) -> Result<Json<AgentSettings>, CanonicalError> {
    let Json(payload) = payload.map_err(|_| {
        error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "token must be a string",
        )
    })?;
    if payload.token.trim().is_empty() || payload.token.trim().len() > MAX_JEV_TOKEN {
        return Err(error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            &format!("token must be between 1 and {MAX_JEV_TOKEN} characters"),
        ));
    }
    state
        .agent_settings
        .set_jev_token(&payload.token)
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(settings_of(&state)))
}

/// Forgets the stored credential. `JEV_TOKEN`, if set, becomes the credential again.
pub(super) async fn remove_jev_token(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<AgentSettings>, CanonicalError> {
    state
        .agent_settings
        .clear_jev_token()
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(settings_of(&state)))
}

fn to_contract_state(state: crate::analytics::TelemetryState) -> TelemetryState {
    TelemetryState::new(state.distinct_id, state.enabled, state.consent)
}

/// The settings as the client sees them, read from the store rather than from whatever the
/// handler happened to be given, so a reply cannot disagree with what the server will do next.
fn settings_of(state: &AppState) -> AgentSettings {
    AgentSettings::new(
        state.agent_settings.human_help().is_enabled(),
        state.agent_settings.task_cleanup_tip_enabled(),
        to_contract_source(state.agent_settings.jev_token_source()),
    )
}

fn to_contract_source(source: TokenSource) -> JevTokenSource {
    match source {
        TokenSource::Settings => JevTokenSource::Settings,
        TokenSource::Environment => JevTokenSource::Environment,
        TokenSource::None => JevTokenSource::None,
    }
}
