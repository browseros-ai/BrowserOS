use super::{error, internal};
use crate::{
    AppState,
    error::{CanonicalError, RequestId},
};
use axum::{
    Extension,
    body::Body,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};

pub(super) async fn screenshot(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(dispatch_id): Path<String>,
) -> Result<Response, CanonicalError> {
    let dispatch_id = positive_i64(&request_id, &dispatch_id, "dispatchId")?;
    match state.screenshots.read(&dispatch_id.to_string()).await {
        // Dispatch screenshots are immutable after capture, unlike live preview frames.
        Ok(bytes) => Ok(jpeg_response(bytes, "public, max-age=86400, immutable")),
        Err(source) if source.status() == StatusCode::NOT_FOUND => Err(error(
            &request_id,
            StatusCode::NOT_FOUND,
            "screenshot_not_found",
            "dispatch screenshot not found",
        )),
        Err(source) => Err(internal(&request_id, source)),
    }
}

fn positive_i64(request_id: &RequestId, raw: &str, name: &str) -> Result<i64, CanonicalError> {
    raw.parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            error(
                request_id,
                StatusCode::BAD_REQUEST,
                "invalid_request",
                &format!("{name} must be positive"),
            )
        })
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
