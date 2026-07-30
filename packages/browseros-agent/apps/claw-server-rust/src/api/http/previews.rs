use super::{error, internal, screenshots::jpeg_response};
use crate::{
    AppState,
    error::{CanonicalError, RequestId},
};
use axum::{
    Extension,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Response,
};
use std::collections::HashMap;

pub(super) async fn preview(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Response, CanonicalError> {
    let preferred_tab_id = match raw.get("browserTabId") {
        None => None,
        Some(value) => Some(value.parse::<i64>().map_err(|_| {
            error(
                &request_id,
                StatusCode::BAD_REQUEST,
                "invalid_browser_tab_id",
                "browserTabId must be an integer",
            )
        })?),
    };
    let bytes = state
        .visuals
        .capture(&session_id, preferred_tab_id)
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
