//! Whether to offer this installation a feedback call, and what came of it.
//!
//! # Why this is an endpoint rather than something the agent says
//!
//! Delivering the invitation through a tool response reaches every eligible installation,
//! but the server then cannot see whether it was rendered, read or acted on. An invitation
//! nobody can count cannot tell you whether the copy or the cohort is wrong, which is the
//! only thing it exists to find out. The cockpit reaches fewer people and reports all three.
//!
//! # Every refusal looks the same from outside
//!
//! Consent, cohort membership and whether this installation was already invited are all
//! answered as `eligible: false`. The caller is the cockpit, which has nothing to do with
//! the distinction, and saying which check failed would describe the cohort to anyone who
//! asks.

use super::{error, internal};
use crate::{
    AppState,
    db::feedback_invite::{InviteOutcome, MAX_INVITATION_ROUNDS},
    error::{AppResult, CanonicalError, RequestId},
    services::feedback_cohort::now_ms,
};
use axum::{
    Extension, Json,
    extract::{
        Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::StatusCode,
};
use claw_api::models::{FeedbackInvitation, FeedbackInviteOutcome, RecordFeedbackInviteRequest};
use serde::Deserialize;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InvitationQuery {
    #[serde(default)]
    supports_rounds: bool,
}

pub(super) async fn invitation(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    query: Result<Query<InvitationQuery>, QueryRejection>,
) -> Result<Json<FeedbackInvitation>, CanonicalError> {
    let Query(query) = query.map_err(|_| {
        error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "supportsRounds must be a boolean",
        )
    })?;
    decide(&state, query.supports_rounds)
        .await
        .map(Json)
        .map_err(|source| internal(&request_id, source))
}

pub(super) async fn respond(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    payload: Result<Json<RecordFeedbackInviteRequest>, JsonRejection>,
) -> Result<Json<FeedbackInvitation>, CanonicalError> {
    let Json(payload) = payload.map_err(|_| {
        error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "outcome must be one of shown, clicked or dismissed",
        )
    })?;
    let round = payload.round.unwrap_or(1);
    if !(1..=MAX_INVITATION_ROUNDS).contains(&round) {
        return Err(error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "round must be between 1 and 3",
        ));
    }
    record(&state, to_outcome(payload.outcome), round)
        .await
        .map_err(|source| internal(&request_id, source))?;
    decide(&state, payload.round.is_some())
        .await
        .map(Json)
        .map_err(|source| internal(&request_id, source))
}

async fn decide(state: &AppState, supports_rounds: bool) -> AppResult<FeedbackInvitation> {
    let Some(install_id) = invitable_install(state).await else {
        return Ok(not_eligible());
    };
    let Some(book_url) = state
        .feedback_cohort
        .invitation_url(&install_id, now_ms())
        .await
    else {
        return Ok(not_eligible());
    };
    let Some(round) = state
        .feedback_invites
        .offered_round(&install_id, now_ms())
        .await?
    else {
        return Ok(not_eligible());
    };
    if !supports_rounds && round != 1 {
        return Ok(not_eligible());
    }
    state
        .feedback_invites
        .remember_offer(&install_id, round)
        .await?;
    let mut invitation = FeedbackInvitation::new(true);
    invitation.book_url = Some(book_url);
    invitation.round = supports_rounds.then_some(round);
    Ok(invitation)
}

async fn record(state: &AppState, outcome: InviteOutcome, round: i32) -> AppResult<()> {
    let Some(install_id) = invitable_install(state).await else {
        return Ok(());
    };
    if !may_record(state, &install_id, round).await? {
        return Ok(());
    }
    state
        .feedback_invites
        .record(&install_id, outcome, now_ms(), round)
        .await
}

async fn may_record(state: &AppState, install_id: &str, round: i32) -> AppResult<bool> {
    if state
        .feedback_invites
        .recorded_round(install_id)
        .await?
        .is_some_and(|recorded| round <= recorded)
        || state
            .feedback_invites
            .was_offered(install_id, round)
            .await?
    {
        return Ok(true);
    }
    Ok(state
        .feedback_cohort
        .invitation_url(install_id, now_ms())
        .await
        .is_some())
}

/// The installation this request speaks for, if it may be contacted at all.
///
/// Consent gates this exactly as it gates the failure reports: an installation that opted
/// out of analytics has not agreed to be contacted either.
async fn invitable_install(state: &AppState) -> Option<String> {
    let analytics = state.analytics.get_state().await;
    if !analytics.consent {
        return None;
    }
    let install_id = analytics.distinct_id;
    (!install_id.trim().is_empty()).then_some(install_id)
}

fn not_eligible() -> FeedbackInvitation {
    FeedbackInvitation::new(false)
}

fn to_outcome(outcome: FeedbackInviteOutcome) -> InviteOutcome {
    match outcome {
        FeedbackInviteOutcome::Shown => InviteOutcome::Shown,
        FeedbackInviteOutcome::Clicked => InviteOutcome::Clicked,
        FeedbackInviteOutcome::Dismissed => InviteOutcome::Dismissed,
    }
}
