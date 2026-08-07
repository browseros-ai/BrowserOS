use super::{error, internal, screenshots::jpeg_response};
use crate::{
    AppState,
    error::{CanonicalError, RequestId},
};
use axum::{
    Extension,
    extract::{Path, State},
    http::StatusCode,
    response::Response,
};

pub(super) async fn preview(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Response, CanonicalError> {
    let bytes = state
        .visuals
        .capture(&session_id)
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
