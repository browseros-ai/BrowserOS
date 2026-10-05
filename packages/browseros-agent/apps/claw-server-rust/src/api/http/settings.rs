use super::{error, internal};
use crate::services::jev_settings::{Budgets, Credential, JevSettings};
use crate::{AppState, error::CanonicalError, error::RequestId};
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
};
use claw_api::models::{
    JevBudgets, JevModeState, TelemetryState, UpdateJevBudgetsRequest, UpdateJevCredentialRequest,
    UpdateJevModeRequest, UpdateTelemetryRequest,
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

fn to_contract_state(state: crate::analytics::TelemetryState) -> TelemetryState {
    TelemetryState::new(state.distinct_id, state.enabled, state.consent)
}

pub(super) async fn jev_mode(State(state): State<AppState>) -> Json<JevModeState> {
    Json(to_jev_state(state.jev_settings.get().await))
}

/// Stores a credential only after it has answered one real request.
///
/// Saying "saved" teaches the user nothing: a wrong credential, a blocked
/// network and a suspended account all look identical until something is
/// actually asked. One trivial question settles all three in a few hundred
/// milliseconds, and the provider's own message is what the user sees.
pub(super) async fn update_jev_credential(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    payload: Result<Json<UpdateJevCredentialRequest>, JsonRejection>,
) -> Result<Json<JevModeState>, CanonicalError> {
    let Json(payload) = payload.map_err(|_| {
        error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "credential must be a string",
        )
    })?;
    let credential = Credential::new(payload.credential);
    if credential.is_empty() {
        return Err(error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "credential must not be empty",
        ));
    }
    if let Err(reason) = browseros_policy::Decider::new(credential.expose())
        .check()
        .await
    {
        return Err(error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "credential_rejected",
            &reason,
        ));
    }
    let settings = state
        .jev_settings
        .set_credential(credential)
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(to_jev_state(settings)))
}

pub(super) async fn update_jev_mode(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    payload: Result<Json<UpdateJevModeRequest>, JsonRejection>,
) -> Result<Json<JevModeState>, CanonicalError> {
    let Json(payload) = payload.map_err(|_| {
        error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "paused must be a boolean",
        )
    })?;
    let settings = state
        .jev_settings
        .set_paused(payload.paused)
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(to_jev_state(settings)))
}

pub(super) async fn update_jev_budgets(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    payload: Result<Json<UpdateJevBudgetsRequest>, JsonRejection>,
) -> Result<Json<JevModeState>, CanonicalError> {
    let Json(payload) = payload.map_err(|_| {
        error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "maxSteps and maxSeconds must be whole numbers",
        )
    })?;
    let budgets = Budgets {
        max_steps: u32::try_from(payload.max_steps)
            .unwrap_or(u32::MAX)
            .clamp(1, 200),
        max_seconds: u32::try_from(payload.max_seconds)
            .unwrap_or(u32::MAX)
            .clamp(5, 600),
    };
    let settings = state
        .jev_settings
        .set_budgets(budgets)
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(to_jev_state(settings)))
}

/// Forgets the credential. A separate action from pausing on purpose.
pub(super) async fn delete_jev_credential(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<JevModeState>, CanonicalError> {
    let settings = state
        .jev_settings
        .clear()
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(to_jev_state(settings)))
}

fn to_jev_state(settings: JevSettings) -> JevModeState {
    JevModeState::new(
        !settings.credential.is_empty(),
        settings.paused,
        settings.is_active(),
        // The credential never travels back, only enough of its tail to tell
        // the user which one is stored.
        settings.credential.fingerprint(),
        JevBudgets::new(
            i64::from(settings.budgets.max_steps),
            i64::from(settings.budgets.max_seconds),
        ),
    )
}
