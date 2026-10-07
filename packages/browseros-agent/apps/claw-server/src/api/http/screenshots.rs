use super::{error, internal};
use crate::{
    AppState,
    error::{CanonicalError, RequestId},
};
use axum::{
    Extension, Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use claw_api::models::{SessionScreenshot, SessionScreenshotList};

pub(super) async fn list(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionScreenshotList>, CanonicalError> {
    let items = state
        .screenshots
        .list(&session_id)
        .await
        .map_err(|source| internal(&request_id, source))?
        .ok_or_else(|| {
            error(
                &request_id,
                StatusCode::NOT_FOUND,
                "session_not_found",
                "session not found",
            )
        })?
        .into_iter()
        .map(|row| SessionScreenshot::new(row.screenshot_id, row.captured_at, row.tool_name))
        .collect();
    Ok(Json(SessionScreenshotList::new(items)))
}

pub(super) async fn get(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path((session_id, screenshot_id)): Path<(String, String)>,
) -> Result<Response, CanonicalError> {
    let screenshot_id = positive_screenshot_id(&request_id, &screenshot_id)?;
    match state.screenshots.read(&session_id, screenshot_id).await {
        Ok(bytes) => Ok(jpeg_response(bytes, "public, max-age=31536000, immutable")),
        Err(source) if source.status() == StatusCode::NOT_FOUND => {
            tracing::warn!(request_id = %request_id.0, %session_id, screenshot_id, "session screenshot not found");
            Err(screenshot_not_found(&request_id))
        }
        Err(source) => Err(internal(&request_id, source)),
    }
}

fn positive_screenshot_id(request_id: &RequestId, raw: &str) -> Result<i64, CanonicalError> {
    raw.parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            error(
                request_id,
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "screenshotId must be positive",
            )
        })
}

fn screenshot_not_found(request_id: &RequestId) -> CanonicalError {
    error(
        request_id,
        StatusCode::NOT_FOUND,
        "screenshot_not_found",
        "session screenshot not found",
    )
}

pub(super) fn jpeg_response(bytes: Vec<u8>, cache_control: &'static str) -> Response {
    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    response
}
