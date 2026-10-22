//! Local operator control for access grants, separate from native compatibility and user
//! preferences. No agent tool can grant itself Pro; browser pages cannot call this writer.
use super::{error, internal};
use crate::{
    AppState,
    error::{CanonicalError, RequestId},
};
use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header},
};
use claw_api::models::FeatureFlags;

pub(super) async fn get(State(state): State<AppState>) -> Json<FeatureFlags> {
    Json(FeatureFlags {
        pro: state.features.snapshot().pro,
    })
}

pub(super) async fn update(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<FeatureFlags>, JsonRejection>,
) -> Result<Json<FeatureFlags>, CanonicalError> {
    // The listener is local, but its ordinary CORS policy permits web apps. This temporary
    // operator override must refuse browser-originated writes even after a CORS preflight.
    // Replace this writer with authenticated entitlement updates when paid Pro launches.
    if headers.contains_key(header::ORIGIN) || headers.contains_key("sec-fetch-site") {
        return Err(error(
            &request_id,
            StatusCode::FORBIDDEN,
            "forbidden",
            "Feature access can only be changed by a local operator",
        ));
    }
    let Json(flags) = payload.map_err(|_| {
        error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "pro must be a boolean",
        )
    })?;
    state
        .features
        .update(browseros_core::FeatureFlags { pro: flags.pro })
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(flags))
}
