//! The hosted live-view routes, `/v1/sessions/{id}/screencast` and `/video`, through the full
//! router: a scripted browser stands in for Chromium and a static table for the session's tabs,
//! so these check the HTTP contract (status, headers, framing, errors) the spec publishes.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use claw_server::{
    AppState, build_router,
    config::{Config, ServerMode},
    services::capture::{
        CaptureHub, CaptureTab, SessionView,
        testing::{ScriptedCdp, ScriptedTransport, StaticSessionTabs},
    },
};
use futures_util::StreamExt;
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

struct TestApp {
    router: Router,
    browser: Arc<ScriptedCdp>,
    tabs: Arc<StaticSessionTabs>,
    _dir: TempDir,
}

async fn app() -> anyhow::Result<TestApp> {
    let dir = tempfile::tempdir()?;
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49337,
        proxy_port: None,
        resources_dir: dir.path().join("resources"),
        browserclaw_dir: dir.path().join("browserclaw"),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
        native: Default::default(),
        mode: ServerMode::Hosted,
    });
    let mut state = AppState::new_with_home(config, dir.path().join("home")).await?;
    let browser = ScriptedCdp::new();
    let tabs = Arc::new(StaticSessionTabs::default());
    state.capture = CaptureHub::new(
        tabs.clone(),
        Arc::new(ScriptedTransport(browser.clone())),
        CancellationToken::new(),
    );
    Ok(TestApp {
        router: build_router(state),
        browser,
        tabs,
        _dir: dir,
    })
}

fn live(tab_ids: &[i64]) -> SessionView {
    SessionView {
        live: true,
        tabs: tab_ids
            .iter()
            .map(|&tab_id| CaptureTab {
                tab_id,
                target_id: format!("target-{tab_id}"),
                url: format!("https://news.ycombinator.com/item?id={tab_id}"),
            })
            .collect(),
    }
}

async fn open(router: &Router, uri: &str) -> anyhow::Result<axum::response::Response> {
    let request = Request::builder()
        .uri(uri)
        .header(header::HOST, "localhost")
        .body(Body::empty())?;
    Ok(router.clone().oneshot(request).await?)
}

async fn error(router: &Router, uri: &str) -> anyhow::Result<(StatusCode, Value)> {
    let response = open(router, uri).await?;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

/// Reads body chunks until `want` bytes arrived, or fails after 5 s.
async fn read_exactly(
    body: &mut axum::body::BodyDataStream,
    want: usize,
) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    while out.len() < want {
        let chunk = tokio::time::timeout(Duration::from_secs(5), body.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("body ended after {} bytes", out.len()))??;
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

#[tokio::test]
async fn screencast_streams_multipart_jpeg_frames_with_tab_headers() -> anyhow::Result<()> {
    let app = app().await?;
    app.tabs.set("s1", live(&[5]));

    let response = open(&app.router, "/v1/sessions/s1/screencast?quality=low").await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("multipart/x-mixed-replace; boundary=frame")
    );
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    let start = app.browser.wait_for("Page.startScreencast", 1).await;
    assert_eq!(start[0].params["maxWidth"], 480);

    app.browser
        .emit_frame("capture-1", b"JPEGDATA", 1_800_000_000.25);
    let expected = b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: 8\r\nX-Tab-Id: 5\r\nX-Url: https://news.ycombinator.com/item?id=5\r\nX-Frame-At: 1800000000250\r\n\r\nJPEGDATA\r\n";
    let mut body = response.into_body().into_data_stream();
    assert_eq!(read_exactly(&mut body, expected.len()).await?, expected);

    // The stream closes with the session.
    app.tabs.set(
        "s1",
        SessionView {
            live: false,
            tabs: Vec::new(),
        },
    );
    let ended = tokio::time::timeout(Duration::from_secs(5), body.next()).await?;
    assert!(ended.is_none());
    Ok(())
}

#[tokio::test]
async fn video_streams_the_mp4_init_segment_then_fragments() -> anyhow::Result<()> {
    let app = app().await?;
    app.tabs.set("s1", live(&[5]));
    let mp4_box = |kind: &[u8; 4], body: &[u8]| {
        let mut out = u32::try_from(8 + body.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes()
            .to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    };
    let init = [mp4_box(b"ftyp", b"av01"), mp4_box(b"moov", b"av1")].concat();
    let fragment = [mp4_box(b"moof", b"1"), mp4_box(b"mdat", b"key")].concat();

    let response = open(&app.router, "/v1/sessions/s1/video?tab=5").await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("video/mp4")
    );
    let start = app.browser.wait_for("Page.startScreenRecording", 1).await;
    assert_eq!(start[0].params["frameRate"], 15);
    app.browser
        .push_recording("capture-1", &[init.clone(), fragment.clone()].concat());

    let mut body = response.into_body().into_data_stream();
    let expected = [init, fragment].concat();
    assert_eq!(read_exactly(&mut body, expected.len()).await?, expected);
    Ok(())
}

#[tokio::test]
async fn live_views_report_what_there_is_to_watch() -> anyhow::Result<()> {
    let app = app().await?;
    app.tabs.set(
        "ended",
        SessionView {
            live: false,
            tabs: Vec::new(),
        },
    );
    app.tabs.set("idle", live(&[]));
    app.tabs.set("open", live(&[5]));

    for kind in ["screencast", "video"] {
        for (uri, status, code) in [
            (
                format!("/v1/sessions/missing/{kind}"),
                StatusCode::NOT_FOUND,
                "session_not_found",
            ),
            (
                format!("/v1/sessions/open/{kind}?tab=9"),
                StatusCode::NOT_FOUND,
                "tab_not_found",
            ),
            (
                format!("/v1/sessions/ended/{kind}"),
                StatusCode::CONFLICT,
                "session_not_live",
            ),
            (
                format!("/v1/sessions/idle/{kind}"),
                StatusCode::CONFLICT,
                "no_open_tab",
            ),
            (
                format!("/v1/sessions/open/{kind}?quality=medium"),
                StatusCode::BAD_REQUEST,
                "invalid_request",
            ),
            (
                format!("/v1/sessions/open/{kind}?tab=abc"),
                StatusCode::BAD_REQUEST,
                "invalid_request",
            ),
        ] {
            let (actual, body) = error(&app.router, &uri).await?;
            assert_eq!(actual, status, "{uri}");
            assert_eq!(body["code"], code, "{uri}");
            assert!(body["requestId"].is_string(), "{uri}");
        }
    }
    assert!(app.browser.sent("Target.attachToTarget").is_empty());
    Ok(())
}
