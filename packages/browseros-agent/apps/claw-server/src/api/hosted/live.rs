//! The live-view routes, `/screencast` (MJPEG) and `/video` (fragmented MP4). Both are thin:
//! parse `quality` and `tab`, open the stream from [`CaptureHub`], and frame it for HTTP. What
//! is captured, shared, followed and when it stops is decided in `services::capture`.
//!
//! [`CaptureHub`]: crate::services::capture::CaptureHub

use super::{HostedError, integer};
use crate::{
    AppState,
    error::RequestId,
    services::capture::{CaptureError, Frame, Quality},
};
use axum::{
    Extension,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use bytes::{BufMut, Bytes, BytesMut};
use futures_util::StreamExt;
use std::{collections::HashMap, convert::Infallible};

/// The multipart boundary between frames; `<img>` and MJPEG clients read it from the header.
const BOUNDARY: &str = "frame";

pub(super) async fn screencast(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Response, HostedError> {
    let (quality, tab) = params(&request_id, &raw)?;
    let frames = state
        .capture
        .screencast(&session_id, quality, tab)
        .await
        .map_err(|error| HostedError::capture(&request_id, error))?;
    let body = Body::from_stream(frames.map(|frame| Ok::<_, Infallible>(multipart_part(&frame))));
    Ok(streaming(
        &format!("multipart/x-mixed-replace; boundary={BOUNDARY}"),
        body,
    ))
}

pub(super) async fn video(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Response, HostedError> {
    let (quality, tab) = params(&request_id, &raw)?;
    let chunks = state
        .capture
        .video(&session_id, quality, tab)
        .await
        .map_err(|error| HostedError::capture(&request_id, error))?;
    Ok(streaming(
        "video/mp4",
        Body::from_stream(chunks.map(Ok::<_, Infallible>)),
    ))
}

fn params(
    request_id: &RequestId,
    raw: &HashMap<String, String>,
) -> Result<(Quality, Option<i64>), HostedError> {
    let quality = match raw.get("quality") {
        None => Quality::High,
        Some(value) => Quality::parse(value)
            .ok_or_else(|| HostedError::invalid(request_id, "`quality` must be low or high"))?,
    };
    Ok((quality, integer(request_id, raw, "tab", 0, i64::MAX)?))
}

fn streaming(content_type: &str, body: Body) -> Response {
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(content_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    // Every frame is new; nothing in between may cache or buffer the stream.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    *response.status_mut() = StatusCode::OK;
    response
}

/// One MJPEG part: the boundary, the part headers the spec documents, the JPEG.
fn multipart_part(frame: &Frame) -> Bytes {
    let head = format!(
        "--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nX-Tab-Id: {}\r\nX-Url: {}\r\nX-Frame-At: {}\r\n\r\n",
        frame.jpeg.len(),
        frame.tab_id,
        header_safe(&frame.url),
        frame.captured_at,
    );
    let mut part = BytesMut::with_capacity(head.len() + frame.jpeg.len() + 2);
    part.put_slice(head.as_bytes());
    part.put_slice(&frame.jpeg);
    part.put_slice(b"\r\n");
    part.freeze()
}

/// A URL as a part-header value: printable ASCII kept, anything else percent-encoded, so a
/// page URL can never end the header early or break a client's parser.
fn header_safe(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for byte in url.bytes() {
        if byte.is_ascii_graphic() {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

impl HostedError {
    fn capture(request_id: &RequestId, error: CaptureError) -> Self {
        match error {
            CaptureError::SessionNotFound => Self::not_found(request_id),
            CaptureError::TabNotFound => Self::new(
                request_id,
                StatusCode::NOT_FOUND,
                "tab_not_found",
                "tab is not open in this session",
            ),
            CaptureError::SessionNotLive => Self::new(
                request_id,
                StatusCode::CONFLICT,
                "session_not_live",
                "session has ended",
            ),
            CaptureError::NoOpenTab => Self::new(
                request_id,
                StatusCode::CONFLICT,
                "no_open_tab",
                "session has no open tab",
            ),
            CaptureError::Browser(source) => Self::internal(request_id, source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Frame, header_safe, multipart_part};

    #[test]
    fn a_part_carries_the_documented_headers_around_the_jpeg() {
        let part = multipart_part(&Frame {
            jpeg: b"JPEG"[..].into(),
            tab_id: 42,
            url: "https://example.com/a b\r\nX-Evil: 1".to_string(),
            captured_at: 1_800_000_000_123,
        });

        assert_eq!(
            part.as_ref(),
            &b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: 4\r\nX-Tab-Id: 42\r\nX-Url: https://example.com/a%20b%0D%0AX-Evil:%201\r\nX-Frame-At: 1800000000123\r\n\r\nJPEG\r\n"[..]
        );
    }

    #[test]
    fn non_ascii_urls_are_percent_encoded() {
        assert_eq!(
            header_safe("https://例.com/é"),
            "https://%E4%BE%8B.com/%C3%A9"
        );
    }
}
