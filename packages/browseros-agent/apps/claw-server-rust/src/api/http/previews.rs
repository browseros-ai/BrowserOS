use super::{dispatches::jpeg_response, error, internal};
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
use base64::{Engine as _, engine::general_purpose::STANDARD};

pub(super) async fn preview(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path((session_id, browser_tab_id)): Path<(String, String)>,
) -> Result<Response, CanonicalError> {
    let browser_tab_id = positive_browser_tab_id(&request_id, &browser_tab_id)?;
    let frame = state
        .cockpit
        .preview(&session_id, browser_tab_id)
        .await
        .map_err(|source| internal(&request_id, source))?
        .ok_or_else(|| preview_not_found(&request_id))?;
    let bytes = STANDARD.decode(frame.jpeg_base64).map_err(|source| {
        tracing::error!(request_id = %request_id.0, error = %source, "cached preview is invalid");
        error(
            &request_id,
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal server error",
        )
    })?;
    // An empty cached frame is absent from the preview resource; the cockpit uses the same
    // placeholder as it does before the first capture.
    if bytes.is_empty() {
        return Err(preview_not_found(&request_id));
    }
    // The next screencast frame supersedes this one, so clients must revalidate every read.
    Ok(jpeg_response(bytes, "private, max-age=0, must-revalidate"))
}

fn positive_browser_tab_id(
    request_id: &RequestId,
    browser_tab_id: &str,
) -> Result<i64, CanonicalError> {
    browser_tab_id
        .parse::<i64>()
        .ok()
        .filter(|browser_tab_id| *browser_tab_id > 0)
        .ok_or_else(|| {
            error(
                request_id,
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "browserTabId must be positive",
            )
        })
}

fn preview_not_found(request_id: &RequestId) -> CanonicalError {
    error(
        request_id,
        StatusCode::NOT_FOUND,
        "preview_not_found",
        "browser tab preview not found",
    )
}
