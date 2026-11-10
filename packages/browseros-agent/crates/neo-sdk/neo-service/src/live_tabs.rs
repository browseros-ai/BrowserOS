//! Live views of one tab, served from the capture hub in `neo-capture` with a privacy
//! check on every frame: `GET /v1/tabs/{tabId}/screencast` (MJPEG) and `/video` (fragmented
//! AV1 MP4).
//!
//! **One tab per hub session.** The hub serves sessions, and claw-sdk has none. Each stream here
//! is a hub "session" named by its tab id, whose only tab is that tab ([`LiveTabs`]), and the
//! stream is pinned to it. The hub re-reads the session once a second: it picks up the tab's URL,
//! and it ends the stream when the tab closes or goes private.
//!
//! **A guard on every frame.** Once a second is too late for privacy: a tab that goes private
//! just after a re-read would keep streaming until the next one. So a stream takes a
//! [`CaptureGuard`] before the hub starts it, and a private tab is refused there with 409. Then
//! every frame and video fragment must pass `admit()` before it is written. The first one that
//! fails ends the body, so nothing after it is sent. `admit` runs after the pixels arrived, and a
//! pixel that could show a secret arrives after the flag's epoch bump (D6). So a frame that
//! passes was produced before the tab went private, even when it is written a moment after.
//!
//! ```text
//!  GET ─ tabId, quality ─▶ privacy.capture(tab) ─ None ─────────────▶ 409 page_private
//!                               │ guard
//!                               ▼
//!          CaptureHub screencast / video (session = tab) ─ not open ─▶ 404 tab_not_found
//!                               │                        └ failed ───▶ 503 browser_unavailable
//!                               ▼
//!           frame or fragment ─ guard.admit()? ─ no ─▶ the body ends
//!                               │ yes
//!                               ▼
//!          MJPEG part (X-Url masked) or MP4 bytes ─▶ body
//! ```

use crate::api_error::ApiError;
use crate::query;
use crate::service::ServiceState;
use axum::body::{Body, Bytes};
use axum::extract::{Path, RawQuery, State};
use axum::http::HeaderName;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use browseros_core::TabId;
use futures_util::future::BoxFuture;
use futures_util::{Stream, StreamExt};
use neo_capture::{CaptureFailure, CaptureResult, CaptureTab, Frame, SessionTabs, SessionView};
use neo_takeover::{BrowserTabs, PartError};
use neo_trace::{CaptureGuard, Privacy, SecretSet};
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
/// tab while it is open, live while the tab is not private.
pub(crate) struct LiveTabs {
    browser: BrowserTabs,
    privacy: Arc<Privacy>,
}

impl LiveTabs {
    pub(crate) fn new(browser: BrowserTabs, privacy: Arc<Privacy>) -> Self {
        Self { browser, privacy }
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
                live: self.privacy.is_private(tab).is_none(),
                tabs: vec![CaptureTab {
                    tab_id: tab.0,
                    target_id: page.target_id.0,
                    url: page.url,
                }],
            }))
        })
    }
}

/// `GET /v1/tabs/{tabId}/screencast`: the tab's frames as MJPEG parts, until it closes or goes
/// private.
pub(crate) async fn screencast(
    State(service): State<Arc<ServiceState>>,
    Path(tab): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<Response, ApiError> {
    let tab = query::tab_id(&tab)?;
    let quality = query::live(raw.as_deref())?;
    let guard = service
        .privacy
        .capture(tab)
        .ok_or_else(ApiError::page_private)?;
    let frames = service
        .live
        .screencast(&tab.0.to_string(), quality, Some(tab.0))
        .await?;
    let secrets = Arc::clone(service.privacy.secrets());
    let parts = admitted(frames, guard, tab)
        .map(move |frame| Ok::<_, Infallible>(mjpeg_part(&frame, &secrets)));
    Ok(streaming(
        &format!("multipart/x-mixed-replace; boundary={BOUNDARY}"),
        Body::from_stream(parts),
    ))
}

/// `GET /v1/tabs/{tabId}/video`: the tab as fragmented MP4, the initialization segment first,
/// until it closes or goes private.
pub(crate) async fn video(
    State(service): State<Arc<ServiceState>>,
    Path(tab): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<Response, ApiError> {
    let tab = query::tab_id(&tab)?;
    let quality = query::live(raw.as_deref())?;
    let guard = service
        .privacy
        .capture(tab)
        .ok_or_else(ApiError::page_private)?;
    let chunks = service
        .live
        .video(&tab.0.to_string(), quality, Some(tab.0))
        .await?;
    let body = Body::from_stream(admitted(chunks, guard, tab).map(Ok::<_, Infallible>));
    Ok(streaming("video/mp4", body))
}

/// The stream's items up to the first one its guard refuses, which ends it. The check runs as
/// each item is about to be written, after its pixels arrived (D6's epoch rule, per frame).
fn admitted<S: Stream>(stream: S, guard: CaptureGuard, tab: TabId) -> impl Stream<Item = S::Item> {
    stream.take_while(move |_| {
        let admitted = guard.admit();
        if !admitted {
            tracing::debug!(
                tab = tab.0,
                "a live view refused a frame: its tab went private"
            );
        }
        std::future::ready(admitted)
    })
}

/// One MJPEG part: the boundary, the part headers the contract documents, and the JPEG. The URL
/// is masked with the secrets known now, then made header-safe. Masking comes first so a secret
/// is matched in the form the page used.
fn mjpeg_part(frame: &Frame, secrets: &SecretSet) -> Bytes {
    let mut url = frame.url.clone();
    secrets.mask_str(&mut url);
    let head = format!(
        "--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nX-Tab-Id: {}\r\nX-Url: {}\r\nX-Frame-At: {}\r\n\r\n",
        frame.jpeg.len(),
        frame.tab_id,
        percent_encoding::utf8_percent_encode(&url, HEADER_UNSAFE),
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
    use neo_trace::{Registration, SecretSet, SecretSetConfig, SecretSource, SecretValue};

    #[test]
    fn a_part_masks_the_url_then_keeps_it_on_one_header_line() {
        let secrets = SecretSet::new(SecretSetConfig::default());
        assert!(matches!(
            secrets.register_from(
                SecretValue::new("hunter2-swordfish".into()),
                None,
                SecretSource::Page
            ),
            Registration::Registered
        ));
        let part = mjpeg_part(
            &Frame {
                jpeg: b"JPEG"[..].into(),
                tab_id: 42,
                url: "https://例.test/a b\r\nX-Evil: 1?token=hunter2-swordfish".into(),
                captured_at: 1_800_000_000_123,
            },
            &secrets,
        );
        assert_eq!(
            part.as_ref(),
            &b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: 4\r\nX-Tab-Id: 42\r\nX-Url: https://%E4%BE%8B.test/a%20b%0D%0AX-Evil:%201?token=[private]\r\nX-Frame-At: 1800000000123\r\n\r\nJPEG\r\n"[..]
        );
    }
}
