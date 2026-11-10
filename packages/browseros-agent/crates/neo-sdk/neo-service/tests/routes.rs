//! Neo: wire-level capture contracts with independent scripted page and capture sockets.

#![expect(
    clippy::unwrap_used,
    reason = "test fixtures fail immediately when setup or response decoding fails"
)]

use tokio::{task, time};

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, BodyDataStream, to_bytes};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use browseros_cdp::CdpError;
use bytes::Bytes;
use futures_util::StreamExt;
use neo_capture::testing::{ScriptedCdp, ScriptedTransport};
use neo_capture::{Frame, mjpeg};
use neo_service::NeoCapture;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

struct App {
    service: NeoCapture,
    router: Router,
    hub: Arc<ScriptedCdp>,
    pages: Arc<ScriptedCdp>,
    shutdown: CancellationToken,
}

impl App {
    fn new() -> Self {
        let hub = ScriptedCdp::new();
        let pages = ScriptedCdp::new();
        pages.set_reply(
            "Target.getTargetInfo",
            Ok(json!({"targetInfo": {
                "type": "page", "tabId": 42, "url": "https://example.test/a b\r\nX-Evil: 1"
            }})),
        );
        pages.set_reply(
            "Page.captureScreenshot",
            Ok(json!({"data": STANDARD.encode(b"JPEG")})),
        );
        let shutdown = CancellationToken::new();
        let service = NeoCapture::start_with(
            Arc::new(ScriptedTransport(Arc::clone(&hub))),
            Arc::new(ScriptedTransport(Arc::clone(&pages))),
            shutdown.clone(),
        );
        let router = service.router();
        Self {
            service,
            router,
            hub,
            pages,
            shutdown,
        }
    }

    async fn get(&self, path: &str) -> Response {
        get(&self.router, path).await
    }
}

async fn get(router: &Router, path: &str) -> Response {
    router
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn code(response: Response, status: StatusCode, expected: &str) {
    assert_eq!(response.status(), status, "HTTP error status");
    assert_eq!(
        response.headers()["content-type"],
        "application/json",
        "JSON error type"
    );
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["code"], expected, "stable error code");
    assert!(value["message"].is_string(), "errors explain the failure");
}

async fn next(body: &mut BodyDataStream) -> Bytes {
    time::timeout(Duration::from_secs(2), body.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn still_bytes_headers_formats_and_validation() {
    let app = App::new();
    for (query, content_type, params) in [
        ("", "image/jpeg", json!({"format":"jpeg","quality":80})),
        ("?type=png", "image/png", json!({"format":"png"})),
        (
            "?quality=0&ignored=yes",
            "image/jpeg",
            json!({"format":"jpeg","quality":0}),
        ),
        (
            "?quality=100",
            "image/jpeg",
            json!({"format":"jpeg","quality":100}),
        ),
    ] {
        let response = app.get(&format!("/v1/pages/a/screenshot{query}")).await;
        assert_eq!(response.status(), StatusCode::OK, "valid screenshot");
        assert_eq!(
            response.headers()["content-type"],
            content_type,
            "image format"
        );
        assert_eq!(response.headers()["x-tab-id"], "42", "BrowserOS identity");
        assert_eq!(
            response.headers()["x-url"],
            "https://example.test/a%20b%0D%0AX-Evil:%201",
            "header-safe URL"
        );
        assert_eq!(
            response.headers()["cache-control"],
            "no-store",
            "fresh captures only"
        );
        assert_eq!(
            response.headers()["x-accel-buffering"],
            "no",
            "proxy buffering disabled"
        );
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap(),
            b"JPEG"[..],
            "unaltered image bytes"
        );
        assert_eq!(
            app.pages
                .sent("Page.captureScreenshot")
                .last()
                .unwrap()
                .params,
            params,
            "CDP screenshot parameters"
        );
    }
    for query in [
        "type=gif",
        "type=png&quality=5",
        "quality=101",
        "quality=-1",
        "quality=oops",
    ] {
        code(
            app.get(&format!("/v1/pages/a/screenshot?{query}")).await,
            StatusCode::BAD_REQUEST,
            "invalid_request",
        )
        .await;
    }
    assert!(
        app.pages
            .sent("Emulation.setFocusEmulationEnabled")
            .is_empty(),
        "stills never emulate"
    );
    assert!(
        app.hub.sent("Target.attachToTarget").is_empty(),
        "stills never borrow the hub socket"
    );
    app.service.close().await;
}

#[tokio::test]
async fn strict_guard_rejects_any_browser_header_on_every_route_without_cors() {
    let app = App::new();
    for route in ["screenshot", "screencast", "video"] {
        for (name, value) in [
            ("origin", "https://evil.test"),
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-site", "none"),
            ("sec-fetch-site", ""),
        ] {
            let response = app
                .router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/v1/pages/a/{route}"))
                        .header(name, value)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(
                !response
                    .headers()
                    .contains_key("access-control-allow-origin"),
                "no CORS grant"
            );
            code(response, StatusCode::FORBIDDEN, "forbidden").await;
        }
    }
    assert!(
        app.pages.sent("Target.getTargetInfo").is_empty(),
        "guard runs before browser work"
    );
    assert_eq!(
        app.get("/v1/pages/a/screenshot").await.status(),
        StatusCode::OK,
        "headerless native viewer admitted"
    );
    app.service.close().await;
}

#[tokio::test]
async fn missing_pages_and_browser_failures_have_stable_errors() {
    let app = App::new();
    app.pages.set_reply(
        "Target.getTargetInfo",
        Err(CdpError::Protocol {
            code: -32602,
            message: "No target with given id found".into(),
        }),
    );
    for route in ["screenshot", "screencast", "video"] {
        code(
            app.get(&format!("/v1/pages/a/{route}")).await,
            StatusCode::NOT_FOUND,
            "page_not_found",
        )
        .await;
    }
    app.pages.set_reply(
        "Target.getTargetInfo",
        Ok(json!({"targetInfo":{"type":"worker"}})),
    );
    code(
        app.get("/v1/pages/a/screenshot").await,
        StatusCode::NOT_FOUND,
        "page_not_found",
    )
    .await;
    app.pages
        .set_reply("Target.getTargetInfo", Err(CdpError::ConnectionLost));
    code(
        app.get("/v1/pages/a/screenshot").await,
        StatusCode::SERVICE_UNAVAILABLE,
        "browser_unavailable",
    )
    .await;
    app.service.close().await;
}

#[tokio::test(start_paused = true)]
async fn still_deadline_releases_its_non_emulating_session() {
    let app = App::new();
    app.pages.block("Page.captureScreenshot", None);
    let start = time::Instant::now();
    code(
        app.get("/v1/pages/a/screenshot").await,
        StatusCode::GATEWAY_TIMEOUT,
        "capture_timeout",
    )
    .await;
    assert_eq!(
        start.elapsed(),
        Duration::from_secs(3),
        "single still deadline"
    );
    assert_eq!(
        app.pages.wait_for("Target.detachFromTarget", 1).await.len(),
        1,
        "timed-out session detached"
    );
    assert!(
        app.pages
            .sent("Emulation.setFocusEmulationEnabled")
            .is_empty(),
        "timeout cleanup cannot clear run focus"
    );
    app.service.close().await;
}

#[tokio::test(start_paused = true)]
async fn stalled_presence_does_not_block_another_page_and_is_not_detached() {
    let app = App::new();
    app.pages
        .block("Emulation.setFocusEmulationEnabled", Some("capture-1"));
    let router = app.router.clone();
    let first = tokio::spawn(async move { get(&router, "/v1/pages/a/screencast").await });
    assert_eq!(
        app.pages
            .wait_for("Emulation.setFocusEmulationEnabled", 1)
            .await
            .len(),
        1,
        "first presence awaits renderer"
    );
    let second = app.get("/v1/pages/b/screencast").await;
    assert_eq!(
        second.status(),
        StatusCode::OK,
        "another page opens despite blocked renderer"
    );
    let mut body = second.into_body().into_data_stream();
    app.hub.emit_frame("capture-1", b"live page b", 1.0);
    assert!(
        next(&mut body).await.ends_with(b"live page b\r\n"),
        "other page delivers frames"
    );
    code(
        first.await.unwrap(),
        StatusCode::GATEWAY_TIMEOUT,
        "capture_timeout",
    )
    .await;
    assert!(
        app.pages.sent("Target.detachFromTarget").is_empty(),
        "a presence timeout must never detach"
    );
    assert_eq!(
        app.pages.sent("Target.attachToTarget").len(),
        2,
        "one presence per page"
    );
    app.shutdown.cancel();
    assert!(body.next().await.is_none(), "shutdown ends other page body");
    app.service.close().await;
}

#[tokio::test(start_paused = true)]
async fn shared_presence_outlives_viewers_and_mjpeg_matches_legacy() {
    let app = App::new();
    let (one, two) = tokio::join!(
        app.get("/v1/pages/a/screencast?quality=low"),
        app.get("/v1/pages/a/screencast?quality=low")
    );
    assert_eq!(one.status(), StatusCode::OK, "first viewer opens");
    assert_eq!(two.status(), StatusCode::OK, "second viewer opens");
    assert_eq!(
        one.headers()["content-type"],
        mjpeg::CONTENT_TYPE,
        "legacy multipart type"
    );
    assert_eq!(
        app.pages.sent("Target.attachToTarget").len(),
        1,
        "concurrent viewers share presence"
    );
    assert_eq!(
        app.pages.sent("Emulation.setFocusEmulationEnabled").len(),
        1,
        "one emulation setup"
    );
    assert_eq!(
        app.pages.sent("Emulation.setFocusEmulationEnabled")[0].params,
        json!({"enabled":true}),
        "presence only enables"
    );
    let mut one = one.into_body().into_data_stream();
    let mut two = two.into_body().into_data_stream();
    app.hub.emit_frame("capture-1", b"JPEG", 1.0);
    let expected = mjpeg::part(&Frame {
        jpeg: b"JPEG"[..].into(),
        tab_id: 42,
        url: "https://example.test/a b\r\nX-Evil: 1".into(),
        captured_at: 1000,
    });
    assert_eq!(
        next(&mut one).await,
        expected,
        "first part is byte-compatible"
    );
    assert_eq!(
        next(&mut two).await,
        expected,
        "second viewer shares newest frame"
    );
    drop(one);
    drop(two);
    task::yield_now().await;
    time::advance(Duration::from_secs(6)).await;
    assert!(
        app.pages.sent("Target.detachFromTarget").is_empty(),
        "viewer departure preserves presence"
    );
    assert_eq!(
        app.hub.wait_for("Page.stopScreencast", 1).await.len(),
        1,
        "hub stops after grace"
    );
    assert!(
        app.hub
            .sent("Emulation.setFocusEmulationEnabled")
            .is_empty(),
        "capture sessions never emulate"
    );
    app.service.close().await;
}

#[tokio::test(start_paused = true)]
async fn presence_detach_and_link_replacement_end_bodies_and_next_viewer_reattaches() {
    let app = App::new();
    for reconnect in [false, true] {
        let mut body = app
            .get("/v1/pages/a/screencast")
            .await
            .into_body()
            .into_data_stream();
        let presence = app
            .pages
            .sent("Emulation.setFocusEmulationEnabled")
            .last()
            .unwrap()
            .session
            .clone()
            .unwrap();
        if reconnect {
            app.pages.reconnect();
            time::advance(Duration::from_secs(1)).await;
        } else {
            app.pages.emit_detached(&presence);
        }
        assert!(
            time::timeout(Duration::from_secs(2), body.next())
                .await
                .unwrap()
                .is_none(),
            "presence death ends body independently of hub"
        );
    }
    let mut body = app
        .get("/v1/pages/a/screencast")
        .await
        .into_body()
        .into_data_stream();
    assert_eq!(
        app.pages.sent("Target.attachToTarget").len(),
        3,
        "new generation re-establishes presence"
    );
    app.shutdown.cancel();
    assert!(body.next().await.is_none(), "host shutdown ends open body");
    app.service.close().await;
}

#[tokio::test(start_paused = true)]
async fn video_yields_initialization_then_fragments_and_ends_on_presence_loss() {
    let app = App::new();
    let response = app.get("/v1/pages/a/video?quality=low").await;
    assert_eq!(response.status(), StatusCode::OK, "video opens");
    assert_eq!(
        response.headers()["content-type"],
        "video/mp4",
        "fragmented MP4 response"
    );
    let mut body = response.into_body().into_data_stream();
    let mut mp4 = Vec::new();
    for kind in [b"ftyp", b"moov", b"moof", b"mdat", b"moof"] {
        mp4.extend_from_slice(&8_u32.to_be_bytes());
        mp4.extend_from_slice(kind);
    }
    app.hub.push_recording("capture-1", &mp4);
    assert!(
        next(&mut body).await.windows(4).any(|b| b == b"ftyp"),
        "init segment first"
    );
    assert!(
        next(&mut body).await.windows(4).any(|b| b == b"moof"),
        "keyframe-led fragment follows"
    );
    app.pages.reconnect();
    time::advance(Duration::from_secs(1)).await;
    assert!(
        time::timeout(Duration::from_secs(2), body.next())
            .await
            .unwrap()
            .is_none(),
        "video stops on page-link loss"
    );
    app.service.close().await;
}

#[tokio::test(start_paused = true)]
async fn closing_the_service_ends_bodies_and_releases_presence_without_cancelling_the_host() {
    let app = App::new();
    let mut body = app
        .get("/v1/pages/a/screencast")
        .await
        .into_body()
        .into_data_stream();
    app.service.close().await;
    assert!(
        body.next().await.is_none(),
        "close ends dependent response bodies"
    );
    assert!(
        !app.shutdown.is_cancelled(),
        "close does not cancel sibling host services"
    );
    assert_eq!(
        app.pages.sent("Target.detachFromTarget").len(),
        1,
        "close releases persistent presence"
    );
}

#[tokio::test(start_paused = true)]
async fn capture_generation_loss_ends_body_with_healthy_presence_and_a_new_request_recovers() {
    let app = App::new();
    let response = app.get("/v1/pages/a/screencast?quality=low").await;
    assert_eq!(response.status(), StatusCode::OK, "initial capture opens");
    let mut body = response.into_body().into_data_stream();
    let first = app.hub.sent("Page.startScreencast")[0]
        .session
        .clone()
        .unwrap();
    app.hub.emit_frame(&first, b"before break", 1.0);
    assert!(
        next(&mut body).await.ends_with(b"before break\r\n"),
        "initial body delivers a frame"
    );

    // Keep old-worker cleanup pending to exercise the gap between socket generation loss
    // and capture completion. A new HTTP request must not join that stale capture lease.
    app.hub.block("Page.stopScreencast", Some(&first));
    app.hub.reconnect();
    time::advance(Duration::from_secs(1)).await;
    let ended = time::timeout(Duration::from_secs(2), body.next()).await;
    assert!(
        matches!(ended, Ok(None)),
        "capture-only generation loss must end the current HTTP body, got {ended:?}"
    );
    assert_eq!(
        app.pages.sent("Emulation.setFocusEmulationEnabled").len(),
        1,
        "page presence stays healthy"
    );
    assert!(
        app.pages.sent("Target.detachFromTarget").is_empty(),
        "capture loss never detaches page presence"
    );

    let response = app.get("/v1/pages/a/screencast?quality=low").await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "fresh request opens a working capture"
    );
    let mut next_body = response.into_body().into_data_stream();
    let starts = app.hub.wait_for("Page.startScreencast", 2).await;
    let current = starts.last().unwrap().session.as_ref().unwrap();
    assert_ne!(current, &first, "fresh request uses a new capture session");
    app.hub.emit_frame(current, b"after break", 2.0);
    assert!(
        next(&mut next_body).await.ends_with(b"after break\r\n"),
        "new response captures after reconnect"
    );
    assert_eq!(
        app.pages.sent("Target.attachToTarget").len(),
        1,
        "healthy presence is reused across hub generations"
    );
    app.shutdown.cancel();
    assert!(
        next_body.next().await.is_none(),
        "shutdown still ends the recovered body"
    );
    app.service.close().await;
}
