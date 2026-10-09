use super::{error, internal, screenshots::jpeg_response};
use crate::{
    AppState,
    error::{CanonicalError, RequestId},
};
use axum::{
    Extension,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::Response,
};
use serde::Deserialize;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PreviewParams {
    browser_tab_id: Option<i64>,
}

pub(super) async fn preview(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    params: Result<Query<PreviewParams>, QueryRejection>,
) -> Result<Response, CanonicalError> {
    let Query(params) = params.map_err(|_| {
        error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "invalid preview query parameters",
        )
    })?;
    if params.browser_tab_id.is_some_and(|tab_id| tab_id <= 0) {
        return Err(error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "browserTabId must be positive",
        ));
    }
    let bytes = state
        .visuals
        .capture(&session_id, params.browser_tab_id)
        .await
        .map_err(|source| internal(&request_id, source))?
        .ok_or_else(|| preview_not_found(&request_id))?;
    Ok(jpeg_response(bytes, "private, no-store"))
}

fn preview_not_found(request_id: &RequestId) -> CanonicalError {
    error(
        request_id,
        StatusCode::NOT_FOUND,
        "preview_not_found",
        "session preview not found",
    )
}
