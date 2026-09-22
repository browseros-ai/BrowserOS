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
    db::feedback_invite::InviteOutcome,
    error::{AppResult, CanonicalError, RequestId},
    services::feedback_cohort::now_ms,
};
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
};
use claw_api::models::{FeedbackInvitation, FeedbackInviteOutcome, RecordFeedbackInviteRequest};

pub(super) async fn invitation(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<FeedbackInvitation>, CanonicalError> {
    decide(&state)
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
    record(&state, to_outcome(payload.outcome))
        .await
        .map_err(|source| internal(&request_id, source))?;
    // Recording anything spends the invitation, so the answer after any outcome is the
    // same one a fresh page load would get.
    Ok(Json(not_eligible()))
}

async fn decide(state: &AppState) -> AppResult<FeedbackInvitation> {
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
    if state.feedback_invites.already_invited(&install_id).await? {
        return Ok(not_eligible());
    }
    let mut invitation = FeedbackInvitation::new(true);
    invitation.book_url = Some(book_url);
    Ok(invitation)
}

async fn record(state: &AppState, outcome: InviteOutcome) -> AppResult<()> {
    let Some(install_id) = invitable_install(state).await else {
        return Ok(());
    };
    if !may_record(state, &install_id).await? {
        return Ok(());
    }
    state
        .feedback_invites
        .record(&install_id, outcome, now_ms())
        .await
}

/// Whether this installation may have an outcome written for it.
///
/// An outcome must only ever spend an invitation the installation was actually offered.
/// Anything able to reach the loopback server can POST here, so without this an unrelated
/// page or local process could burn an installation's single invitation before it had ever
/// been shown one, and the row is permanent.
///
/// A row that already exists is updated without consulting the cohort, on purpose: that
/// invitation was granted by an earlier decision, and a refresh between the card appearing
/// and the reader answering must not discard what they did.
async fn may_record(state: &AppState, install_id: &str) -> AppResult<bool> {
    if state.feedback_invites.already_invited(install_id).await? {
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
