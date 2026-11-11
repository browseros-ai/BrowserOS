//! Live views of one tab through the `neo-capture` hub: MJPEG at `/screencast` and
//! fragmented AV1 MP4 at `/video`.
//!
//! Each stream uses its tab id as the hub session id and stays pinned to that tab. The hub
//! re-reads the session once a second to refresh the URL and end the stream if the tab closes.

use crate::api_error::ApiError;
use crate::query;
use crate::service::ServiceState;
use axum::body::{Body, Bytes};
use axum::extract::{Path, RawQuery, State};
use axum::http::HeaderName;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use browseros_core::TabId;
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use neo_capture::{CaptureFailure, CaptureResult, CaptureTab, Frame, SessionTabs, SessionView};
use neo_takeover::{BrowserTabs, PartError};
use percent_encoding::{AsciiSet, CONTROLS};
use std::convert::Infallible;
use std::sync::Arc;

/// The multipart boundary between frames. MJPEG clients read it from `Content-Type`.
const BOUNDARY: &str = "frame";

/// Every byte that is not printable ASCII: the C0 controls, space and DEL, plus every non-ASCII
/// byte, which `utf8_percent_encode` always encodes. CR or LF in a URL could otherwise end the
/// `X-Url` line early and break a client's part parser.
const HEADER_UNSAFE: &AsciiSet = &CONTROLS.add(b' ');

const X_ACCEL_BUFFERING: HeaderName = HeaderName::from_static("x-accel-buffering");

/// The capture hub's view of claw-sdk: a "session" id is a tab id, and that session is the one
/// tab while it is open.
pub(crate) struct LiveTabs {
    browser: BrowserTabs,
}

impl LiveTabs {
    pub(crate) fn new(browser: BrowserTabs) -> Self {
        Self { browser }
    }
}

impl SessionTabs for LiveTabs {
    fn view<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, CaptureResult<Option<SessionView>>> {
        Box::pin(async move {
            let Ok(tab) = session_id.parse().map(TabId) else {
                return Ok(None);
            };
            // An error here keeps an open stream waiting for the next re-read, and refuses a
            // new one with 503: a browser that does not answer once has not closed the tab.
            let page = match self.browser.page(tab).await {
                Ok(page) => page,
                Err(PartError::TabNotFound(_)) => return Ok(None),
                Err(error) => return Err(CaptureFailure::new(error.to_string())),
            };
            Ok(Some(SessionView {
                live: true,
                tabs: vec![CaptureTab {
                    tab_id: tab.0,
                    target_id: page.target_id.0,
                    url: page.url,
                }],
            }))
        })
    }
}

/// `GET /v1/tabs/{tabId}/screencast`: the tab's frames as MJPEG parts, until it closes.
pub(crate) async fn screencast(
    State(service): State<Arc<ServiceState>>,
    Path(tab): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<Response, ApiError> {
    let tab = query::tab_id(&tab)?;
    let quality = query::live(raw.as_deref())?;
    let frames = service
        .live
        .screencast(&tab.0.to_string(), quality, Some(tab.0))
        .await?;
    let parts = frames.map(|frame| Ok::<_, Infallible>(mjpeg_part(&frame)));
    Ok(streaming(
        &format!("multipart/x-mixed-replace; boundary={BOUNDARY}"),
        Body::from_stream(parts),
    ))
}

/// `GET /v1/tabs/{tabId}/video`: the tab as fragmented MP4, the initialization segment first,
/// until it closes.
pub(crate) async fn video(
    State(service): State<Arc<ServiceState>>,
    Path(tab): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<Response, ApiError> {
    let tab = query::tab_id(&tab)?;
    let quality = query::live(raw.as_deref())?;
    let chunks = service
        .live
        .video(&tab.0.to_string(), quality, Some(tab.0))
        .await?;
    let body = Body::from_stream(chunks.map(Ok::<_, Infallible>));
    Ok(streaming("video/mp4", body))
}

/// One MJPEG part: the boundary, the part headers the contract documents, and the JPEG.
fn mjpeg_part(frame: &Frame) -> Bytes {
    let head = format!(
        "--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nX-Tab-Id: {}\r\nX-Url: {}\r\nX-Frame-At: {}\r\n\r\n",
        frame.jpeg.len(),
        frame.tab_id,
        percent_encoding::utf8_percent_encode(&frame.url, HEADER_UNSAFE),
        frame.captured_at,
    );
    let mut part = Vec::with_capacity(head.len() + frame.jpeg.len() + 2);
    part.extend_from_slice(head.as_bytes());
    part.extend_from_slice(&frame.jpeg);
    part.extend_from_slice(b"\r\n");
    Bytes::from(part)
}

/// A live response. Every frame is new, so nothing on the way may cache or buffer it.
fn streaming(content_type: &str, body: Body) -> Response {
    (
        [
            (CONTENT_TYPE, content_type),
            (CACHE_CONTROL, "no-store"),
            (X_ACCEL_BUFFERING, "no"),
        ],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::mjpeg_part;
    use neo_capture::Frame;

    #[test]
    fn a_part_keeps_the_url_on_one_header_line() {
        let part = mjpeg_part(&Frame {
            jpeg: b"JPEG"[..].into(),
            tab_id: 42,
            url: "https://例.test/a b\r\nX-Evil: 1?token=hunter2-swordfish".into(),
            captured_at: 1_800_000_000_123,
        });
        assert_eq!(
            part.as_ref(),
            &b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: 4\r\nX-Tab-Id: 42\r\nX-Url: https://%E4%BE%8B.test/a%20b%0D%0AX-Evil:%201?token=hunter2-swordfish\r\nX-Frame-At: 1800000000123\r\n\r\nJPEG\r\n"[..]
        );
    }
}
