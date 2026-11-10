//! Neo: capture HTTP policy. Parse at the boundary and keep all browser work behind deadlines.

use std::fmt::Display;
use tokio::time;

use std::collections::HashMap;
use std::convert::Infallible;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router, middleware};
use futures_util::StreamExt;
use neo_capture::{CaptureError, CaptureFailure, Quality, mjpeg};
use serde_json::json;

use crate::{NeoCapture, pages, still};

/// Neo: one deadline covers page lookup, presence setup and the hub's stream open.
const STREAM_TIMEOUT: Duration = Duration::from_secs(5);
/// Neo: viewport JPEG default agrees with the run tool's default quality.
const JPEG_QUALITY: u8 = 80;

pub(crate) fn router(capture: NeoCapture) -> Router {
    Router::new()
        .route("/v1/pages/{page}/screenshot", get(screenshot))
        .route("/v1/pages/{page}/screencast", get(screencast))
        .route("/v1/pages/{page}/video", get(video))
        .layer(middleware::from_fn(guard))
        .with_state(capture)
}

async fn guard(request: Request, next: Next) -> Response {
    // Identical to /mcp, including Sec-Fetch-Site: none. No CORS is granted here.
    if request.headers().contains_key(header::ORIGIN)
        || request.headers().contains_key("sec-fetch-site")
    {
        return Failure(
            StatusCode::FORBIDDEN,
            "forbidden",
            "browser requests are forbidden".into(),
        )
        .into_response();
    }
    next.run(request).await
}

async fn screenshot(
    State(capture): State<NeoCapture>,
    Path(page): Path<String>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Response, Failure> {
    let format = raw.get("type").map_or("jpeg", String::as_str);
    if !matches!(format, "jpeg" | "png") {
        return Err(Failure::invalid("`type` must be jpeg or png"));
    }
    let quality = match (format, raw.get("quality")) {
        ("png", Some(_)) => return Err(Failure::invalid("`quality` is only valid with jpeg")),
        ("png", None) => None,
        (_, None) => Some(JPEG_QUALITY),
        (_, Some(value)) => Some(
            value
                .parse::<u8>()
                .ok()
                .filter(|q| *q <= 100)
                .ok_or_else(|| Failure::invalid("`quality` must be between 0 and 100"))?,
        ),
    };
    let (bytes, info) = still::screenshot(&capture, &page, format, quality).await?;
    let content_type = if format == "png" {
        "image/png"
    } else {
        "image/jpeg"
    };
    let mut response = success(content_type, Body::from(bytes));
    response.headers_mut().insert(
        "x-tab-id",
        HeaderValue::from_str(&info.tab_id.to_string()).map_err(Failure::browser)?,
    );
    response.headers_mut().insert(
        "x-url",
        HeaderValue::from_str(&mjpeg::header_safe(&info.url)).map_err(Failure::browser)?,
    );
    Ok(response)
}

#[derive(Clone, Copy)]
enum StreamKind {
    Screencast,
    Video,
}

async fn screencast(
    State(capture): State<NeoCapture>,
    Path(page): Path<String>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Response, Failure> {
    stream(&capture, &page, &raw, StreamKind::Screencast).await
}

async fn video(
    State(capture): State<NeoCapture>,
    Path(page): Path<String>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Response, Failure> {
    stream(&capture, &page, &raw, StreamKind::Video).await
}

async fn stream(
    capture: &NeoCapture,
    page: &str,
    raw: &HashMap<String, String>,
    kind: StreamKind,
) -> Result<Response, Failure> {
    let quality = match raw.get("quality") {
        None => Quality::High,
        Some(value) => Quality::parse(value)
            .ok_or_else(|| Failure::invalid("`quality` must be low or high"))?,
    };
    time::timeout(STREAM_TIMEOUT, async {
        let generation = capture.0.pages.generation().await?;
        let info = generation
            .lookup(page)
            .await?
            .ok_or_else(Failure::missing)?;
        let presence = generation.presence.ensure(&generation.link, page).await?;
        // A hub session uses a different socket. It cannot see page-link loss itself, so both
        // bodies must follow presence's generation/target cancellation, even with buffered data.
        let body = match kind {
            StreamKind::Screencast => {
                let frames = capture
                    .0
                    .hub
                    .screencast(page, quality, Some(info.tab_id))
                    .await?;
                // Legacy hub streams follow/retry after capture-socket loss. A Neo response
                // belongs to its initial generation and must reach EOF so its viewer reconnects,
                // even when the separate page-presence socket remains healthy.
                let capture_ended = frames.generation_ended();
                Body::from_stream(
                    frames
                        .take_until(capture_ended.cancelled_owned())
                        .take_until(presence.cancelled_owned())
                        .take_until(capture.0.shutdown.clone().cancelled_owned())
                        .map(|frame| Ok::<_, Infallible>(mjpeg::part(&frame))),
                )
            }
            StreamKind::Video => {
                let chunks = capture
                    .0
                    .hub
                    .video(page, quality, Some(info.tab_id))
                    .await?;
                Body::from_stream(
                    chunks
                        .take_until(presence.cancelled_owned())
                        .take_until(capture.0.shutdown.clone().cancelled_owned())
                        .map(Ok::<_, Infallible>),
                )
            }
        };
        Ok(success(
            match kind {
                StreamKind::Screencast => mjpeg::CONTENT_TYPE,
                StreamKind::Video => "video/mp4",
            },
            body,
        ))
    })
    .await
    .map_err(|_| Failure::timeout())?
}

fn success(content_type: &'static str, body: Body) -> Response {
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

/// Stable HTTP status/code/message mapping at the host boundary, without legacy app errors.
pub(crate) struct Failure(StatusCode, &'static str, String);

impl Failure {
    fn invalid(message: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, "invalid_request", message.into())
    }
    pub(crate) fn missing() -> Self {
        Self(
            StatusCode::NOT_FOUND,
            "page_not_found",
            "page not found".into(),
        )
    }
    pub(crate) fn browser(error: impl Display) -> Self {
        Self(
            StatusCode::SERVICE_UNAVAILABLE,
            "browser_unavailable",
            error.to_string(),
        )
    }
    pub(crate) fn timeout() -> Self {
        Self(
            StatusCode::GATEWAY_TIMEOUT,
            "capture_timeout",
            "capture timed out".into(),
        )
    }
}

impl From<CaptureFailure> for Failure {
    fn from(error: CaptureFailure) -> Self {
        if pages::missing(&error) {
            Self::missing()
        } else {
            Self::browser(error)
        }
    }
}

impl From<CaptureError> for Failure {
    fn from(error: CaptureError) -> Self {
        match error {
            CaptureError::Browser(error) => error.into(),
            CaptureError::SessionNotFound
            | CaptureError::SessionNotLive
            | CaptureError::TabNotFound
            | CaptureError::NoOpenTab => Self::missing(),
        }
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"code": self.1, "message": self.2}))).into_response()
    }
}
