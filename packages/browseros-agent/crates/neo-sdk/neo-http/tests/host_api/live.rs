//! The live views as zuzu reads them: MJPEG parts by `Content-Length`, and MP4 bytes in order,
//! from the scripted capture connection. Streams stay pinned to the tab until it closes.

use crate::support::{BoxError, Harness, TestResult, harness, harness_with_capture};
use axum::body::{Body, BodyDataStream, Bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use browseros_core::CdpConnection;
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use neo_screencast::{CaptureFailure, CaptureResult, CaptureTransport};

use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

/// The tab every test watches. The scripted browser names its target `target-7`, and the first
/// capture session the scripted capture connection attaches is `capture-1`.
const TAB: i64 = 7;
const SESSION: &str = "capture-1";

/// A live response, read incrementally.
struct Live {
    status: StatusCode,
    headers: HeaderMap,
    body: BodyDataStream,
    /// Bytes received and not yet taken.
    buffer: Vec<u8>,
}

/// One MJPEG part: its bytes as sent, from the boundary to the CRLF after the JPEG.
struct Part {
    raw: Vec<u8>,
    headers: Vec<(String, String)>,
    jpeg: Vec<u8>,
}

impl Part {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

async fn open(h: &Harness, uri: &str) -> Result<Live, BoxError> {
    let request = Request::builder().uri(uri).body(Body::empty())?;
    let response = h.router.clone().oneshot(request).await?;
    Ok(Live {
        status: response.status(),
        headers: response.headers().clone(),
        body: response.into_body().into_data_stream(),
        buffer: Vec::new(),
    })
}

impl Live {
    fn header(&self, name: impl header::AsHeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    /// The headers every live response carries.
    fn assert_streaming(&self, content_type: &str) {
        assert_eq!(self.status, StatusCode::OK);
        assert_eq!(self.header(header::CONTENT_TYPE), Some(content_type));
        assert_eq!(self.header(header::CACHE_CONTROL), Some("no-store"));
        assert_eq!(self.header("x-accel-buffering"), Some("no"));
        assert!(self.header("x-request-id").is_some());
    }

    /// The next chunk, or `None` once the stream ended. A stream that neither yields nor ends
    /// within 5 s fails the test, so `None` always means "ended".
    async fn chunk(&mut self) -> Result<Option<Bytes>, BoxError> {
        match tokio::time::timeout(Duration::from_secs(5), self.body.next()).await {
            Ok(Some(chunk)) => Ok(Some(chunk?)),
            Ok(None) => Ok(None),
            Err(_) => Err("the stream neither yielded nor ended within 5 s".into()),
        }
    }

    /// The next MJPEG part, read by its `Content-Length` as zuzu's relay reads it.
    async fn part(&mut self) -> Result<Part, BoxError> {
        loop {
            if let Some(part) = self.take_part()? {
                return Ok(part);
            }
            let chunk = self.chunk().await?.ok_or("the stream ended")?;
            self.buffer.extend_from_slice(&chunk);
        }
    }

    fn take_part(&mut self) -> Result<Option<Part>, BoxError> {
        let Some(head_len) = self
            .buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
        else {
            return Ok(None);
        };
        let head = std::str::from_utf8(&self.buffer[..head_len])?;
        let mut lines = head.split("\r\n");
        assert_eq!(
            lines.next(),
            Some("--frame"),
            "a part starts at the boundary"
        );
        let headers = lines
            .map(|line| {
                line.split_once(": ")
                    .map(|(key, value)| (key.to_string(), value.to_string()))
                    .ok_or_else(|| format!("not a header line: {line:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let length: usize = headers
            .iter()
            .find(|(key, _)| key == "Content-Length")
            .ok_or("a part without Content-Length")?
            .1
            .parse()?;
        let jpeg_at = head_len + 4;
        let end = jpeg_at + length + 2;
        if self.buffer.len() < end {
            return Ok(None);
        }
        let raw = self.buffer.drain(..end).collect::<Vec<_>>();
        assert_eq!(&raw[end - 2..], b"\r\n", "a part ends with CRLF");
        Ok(Some(Part {
            jpeg: raw[jpeg_at..jpeg_at + length].to_vec(),
            raw,
            headers,
        }))
    }

    /// The next `len` bytes.
    async fn bytes(&mut self, len: usize) -> Result<Vec<u8>, BoxError> {
        while self.buffer.len() < len {
            let chunk = self.chunk().await?.ok_or("the stream ended")?;
            self.buffer.extend_from_slice(&chunk);
        }
        Ok(self.buffer.drain(..len).collect())
    }

    /// Everything still to come, once the stream has ended.
    async fn rest(&mut self) -> Result<Vec<u8>, BoxError> {
        let mut rest = std::mem::take(&mut self.buffer);
        while let Some(chunk) = self.chunk().await? {
            rest.extend_from_slice(&chunk);
        }
        Ok(rest)
    }
}

fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = u32::try_from(8 + body.len())
        .unwrap_or(u32::MAX)
        .to_be_bytes()
        .to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// A screencast of the open tab, past the browser starting it.
async fn watching(h: &Harness, uri: &str) -> Result<Live, BoxError> {
    h.browser.open_tab(TAB);
    let live = open(h, uri).await?;
    live.assert_streaming("multipart/x-mixed-replace; boundary=frame");
    h.capture.wait_for("Page.startScreencast", 1).await;
    Ok(live)
}

#[tokio::test]
async fn a_screencast_sends_each_frame_as_a_part_with_the_contract_headers() -> TestResult {
    let h = harness().await?;
    let mut live = watching(&h, "/v1/tabs/7/screencast?quality=low").await?;
    let attach = h.capture.sent("Target.attachToTarget");
    assert_eq!(
        attach.first().map(|command| &command.params),
        Some(&json!({ "targetId": "target-7", "flatten": true }))
    );
    let start = h.capture.sent("Page.startScreencast");
    assert_eq!(
        start
            .first()
            .map(|command| (&command.params["maxWidth"], &command.params["maxHeight"])),
        Some((&json!(640), &json!(400))),
        "captured at the low preset"
    );

    h.capture.emit_frame(SESSION, b"jpeg-1", 1_800_000_000.5);
    assert_eq!(
        live.part().await?.raw,
        b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: 6\r\nX-Tab-Id: 7\r\nX-Url: https://example.test/7\r\nX-Frame-At: 1800000000500\r\n\r\njpeg-1\r\n"
    );
    h.capture.emit_frame(SESSION, b"jpeg-two", 1_800_000_001.0);
    let second = live.part().await?;
    assert_eq!(second.jpeg, b"jpeg-two");
    assert_eq!(second.header("X-Frame-At"), Some("1800000001000"));
    Ok(())
}

#[tokio::test]
async fn a_screencast_keeps_its_url_and_frames_during_a_takeover() -> TestResult {
    let h = harness().await?;
    h.browser
        .set_url(TAB, "https://example.test/7?token=capture-test-value");
    let mut live = watching(&h, "/v1/tabs/7/screencast").await?;
    h.capture.emit_frame(SESSION, b"before", 1_800_000_000.0);
    assert_eq!(live.part().await?.jpeg, b"before");

    let takeover = h.post("/v1/takeovers", r#"{"tabId":7}"#).await?;
    assert_eq!(takeover.status, StatusCode::OK, "{}", takeover.body);
    h.capture.emit_frame(SESSION, b"during", 1_800_000_001.0);
    let part = live.part().await?;
    assert_eq!(part.jpeg, b"during");
    assert_eq!(
        part.header("X-Url"),
        Some("https://example.test/7?token=capture-test-value")
    );
    Ok(())
}

#[tokio::test]
async fn a_closed_tab_ends_its_screencast_and_is_then_not_found() -> TestResult {
    let h = harness().await?;
    let mut live = watching(&h, "/v1/tabs/7/screencast").await?;
    h.capture.emit_frame(SESSION, b"open", 1_800_000_000.0);
    assert_eq!(live.part().await?.jpeg, b"open");

    h.browser.close_tab(TAB);
    h.capture.emit_detached(SESSION);
    assert!(live.rest().await?.is_empty());
    h.get("/v1/tabs/7/screencast")
        .await?
        .assert_error(StatusCode::NOT_FOUND, "tab_not_found");
    Ok(())
}

#[tokio::test]
async fn a_video_sends_the_init_segment_then_each_fragment_during_a_takeover() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(TAB);
    let takeover = h.post("/v1/takeovers", r#"{"tabId":7}"#).await?;
    assert_eq!(takeover.status, StatusCode::OK, "{}", takeover.body);
    let mut live = open(&h, "/v1/tabs/7/video?quality=low").await?;
    live.assert_streaming("video/mp4");
    let start = h.capture.wait_for("Page.startScreenRecording", 1).await;
    assert_eq!(
        start.first().map(|command| &command.params),
        Some(&json!({ "maxWidth": 640, "maxHeight": 400, "frameRate": 10 }))
    );

    let init = [mp4_box(b"ftyp", b"av01"), mp4_box(b"moov", b"tracks")].concat();
    let first = [mp4_box(b"moof", b"1"), mp4_box(b"mdat", b"keyframe-1")].concat();
    h.capture
        .push_recording(SESSION, &[init.clone(), first.clone()].concat());
    assert_eq!(live.bytes(init.len()).await?, init);
    assert_eq!(live.bytes(first.len()).await?, first);
    Ok(())
}

#[tokio::test]
async fn a_closed_tab_ends_its_video() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(TAB);
    let mut live = open(&h, "/v1/tabs/7/video").await?;
    live.assert_streaming("video/mp4");
    h.capture.wait_for("Page.startScreenRecording", 1).await;
    let init = [mp4_box(b"ftyp", b"av01"), mp4_box(b"moov", b"tracks")].concat();
    h.capture.push_recording(SESSION, &init);
    assert_eq!(live.bytes(init.len()).await?, init);

    h.browser.close_tab(TAB);
    h.capture.emit_detached(SESSION);
    assert!(live.rest().await?.is_empty());
    h.get("/v1/tabs/7/video")
        .await?
        .assert_error(StatusCode::NOT_FOUND, "tab_not_found");
    Ok(())
}

#[tokio::test]
async fn an_unknown_tab_is_404_and_a_bad_request_is_400() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(TAB);
    for uri in ["/v1/tabs/8/screencast", "/v1/tabs/8/video"] {
        h.get(uri)
            .await?
            .assert_error(StatusCode::NOT_FOUND, "tab_not_found");
    }
    for uri in [
        "/v1/tabs/seven/screencast",
        "/v1/tabs/7/screencast?quality=medium",
        "/v1/tabs/7/video?quality=low&quality=high",
    ] {
        h.get(uri)
            .await?
            .assert_error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    assert!(h.capture.sent("Target.attachToTarget").is_empty());
    Ok(())
}

/// A capture connection Chromium refuses.
struct Refused;

impl CaptureTransport for Refused {
    fn connect(&self) -> BoxFuture<'_, CaptureResult<Arc<dyn CdpConnection>>> {
        Box::pin(async { Err(CaptureFailure::new("connection refused".into())) })
    }
}

#[tokio::test]
async fn a_capture_that_cannot_start_is_503() -> TestResult {
    let refused = harness_with_capture(Arc::new(Refused)).await?;
    refused.browser.open_tab(TAB);
    let unreachable = harness().await?;
    unreachable.browser.open_tab(TAB);
    unreachable.browser.become_unreachable();
    for h in [&refused, &unreachable] {
        for uri in ["/v1/tabs/7/screencast", "/v1/tabs/7/video"] {
            h.get(uri)
                .await?
                .assert_error(StatusCode::SERVICE_UNAVAILABLE, "browser_unavailable");
        }
    }
    Ok(())
}
