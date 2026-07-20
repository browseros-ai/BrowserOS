//! In-process coverage of the canonical contract routes: drives the
//! full router over seeded app state via tower, no network or browser.
//! The TS twin is claw-server's `tests/routes/api-v1.test.ts`; the
//! cross-server suite in `contracts/claw-api/tests` layers real-HTTP
//! parity checks on top of both.

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use browseros_cdp::{CdpError, CdpEvent, SessionId as CdpSessionId};
use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection, TargetId};
use claw_server_rust::{
    AppState, build_router,
    capture::audit::{DispatchResultSummary, RecordToolDispatchInput},
    config::Config,
    identity::{ClientIdentity, ConversationIdentity},
    ids::{DispatchId, SessionId},
    sessions::Session,
    tabs::activity::{RecordToolInput, ScreencastFrame},
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

struct TestApp {
    router: Router,
    state: AppState,
    _dir: TempDir,
}

struct FixtureConnection {
    events: broadcast::Sender<CdpEvent>,
}

impl FixtureConnection {
    fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(1);
        Arc::new(Self { events })
    }
}

impl CdpConnection for FixtureConnection {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        _session: Option<&'a CdpSessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            match method {
                "Browser.getTabs" => Ok(json!({
                    "tabs": (1..=8).map(fixture_tab).collect::<Vec<_>>()
                })),
                "Browser.getTabInfo" => {
                    let tab_id = params.get("tabId").and_then(Value::as_i64);
                    let tab = (1..=8)
                        .map(fixture_tab)
                        .find(|tab| tab["tabId"].as_i64() == tab_id)
                        .ok_or_else(|| CdpError::Protocol {
                            code: -32000,
                            message: "tab not found".to_string(),
                        })?;
                    Ok(json!({ "tab": tab }))
                }
                _ => Ok(json!({})),
            }
        })
    }

    fn send_raw_json<'a>(
        &'a self,
        _method: &'a str,
        _params_json: &'a str,
        _session: Option<&'a CdpSessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async { Ok("{}".to_string()) })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn connection_epoch(&self) -> u64 {
        1
    }
}

fn fixture_tab(page_id: i64) -> Value {
    let (tab_id, target_id, url, title) = match page_id {
        7 => (
            101,
            "target-7".to_string(),
            "https://browseros.com".to_string(),
            "BrowserOS".to_string(),
        ),
        8 => (
            102,
            "target-8".to_string(),
            "https://example.com".to_string(),
            "Example".to_string(),
        ),
        _ => (
            page_id,
            format!("fixture-target-{page_id}"),
            format!("https://fixture.example/{page_id}"),
            format!("Fixture {page_id}"),
        ),
    };
    json!({
        "tabId": tab_id,
        "targetId": target_id,
        "url": url,
        "title": title,
        "isActive": page_id == 7,
        "isLoading": false,
        "loadProgress": 1.0,
        "isPinned": false,
        "isHidden": false,
        "windowId": 1,
        "index": page_id - 1
    })
}

async fn test_app() -> anyhow::Result<TestApp> {
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
        screencast_screenshot_fallback: true,
        dev_mode: false,
        auth_token: None,
    });
    let state = AppState::new_with_home(config, dir.path().join("home")).await?;
    let browser = BrowserSession::new(FixtureConnection::new(), BrowserSessionHooks::default());
    assert_eq!(browser.pages.list().await?.len(), 8);
    state.browser.set_session_for_testing(browser).await;
    Ok(TestApp {
        router: build_router(state.clone()),
        state,
        _dir: dir,
    })
}

async fn request(
    router: &Router,
    method: &str,
    uri: &str,
    content_type: Option<&str>,
    body: impl Into<Body>,
) -> anyhow::Result<(StatusCode, HeaderMap, Vec<u8>)> {
    request_with_headers(router, method, uri, content_type, &[], body).await
}

async fn request_with_headers(
    router: &Router,
    method: &str,
    uri: &str,
    content_type: Option<&str>,
    headers: &[(&str, &str)],
    body: impl Into<Body>,
) -> anyhow::Result<(StatusCode, HeaderMap, Vec<u8>)> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost");
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = router.clone().oneshot(builder.body(body.into())?).await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?.to_vec();
    Ok((status, headers, bytes))
}

fn json_body(bytes: &[u8]) -> anyhow::Result<Value> {
    Ok(serde_json::from_slice(bytes)?)
}

fn live_session(session_id: &str) -> Arc<Session> {
    Session::new(
        SessionId::new(session_id),
        ClientIdentity::Ephemeral {
            slug: "codex".to_string(),
            label: "Codex".to_string(),
        },
        ConversationIdentity::new("codex", "research-browserclaw".to_string()),
        tokio::time::Instant::now(),
    )
}

async fn seed_dispatch(app: &TestApp, session_id: &str) -> anyhow::Result<i64> {
    Ok(app
        .state
        .audit
        .record_tool_dispatch(RecordToolDispatchInput {
            agent_id: "codex-research-browserclaw".to_string(),
            slug: "codex".to_string(),
            agent_label: "Codex".to_string(),
            session_id: session_id.to_string(),
            tool_name: "snapshot".to_string(),
            page_id: Some(7),
            target_id: Some("target-7".to_string()),
            url: None,
            title: None,
            raw_args: json!({}),
            duration_ms: 5,
            dispatch_id: DispatchId::new(),
            result: DispatchResultSummary {
                is_error: false,
                structured_content: json!({}),
                content: json!([]),
            },
        })
        .await?)
}

#[tokio::test]
async fn retired_rest_routes_are_unmounted() -> anyhow::Result<()> {
    let app = test_app().await?;
    for (method, path) in [
        ("GET", "/system/version"),
        ("GET", "/system/url"),
        ("GET", "/system/telemetry"),
        ("POST", "/system/telemetry"),
        ("POST", "/agents/agent-1/cancel"),
        ("GET", "/tabs/activity"),
        ("GET", "/connections"),
        ("POST", "/connections/NotAHarness/connect"),
        ("POST", "/connections/NotAHarness/disconnect"),
        ("GET", "/audit/dispatches"),
        ("GET", "/audit/tasks"),
        ("GET", "/audit/tasks/session-1"),
        ("GET", "/audit/screenshot/1"),
        ("GET", "/recordings/health"),
        ("POST", "/recordings/tabs/1/events"),
        ("GET", "/audit/replays/session-1"),
        ("GET", "/audit/replays/session-1/meta"),
    ] {
        let (status, _, bytes) = request(&app.router, method, path, None, Body::empty()).await?;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
        assert!(bytes.is_empty(), "{method} {path} reached a JSON handler");
    }
    Ok(())
}

#[tokio::test]
async fn canonical_control_settings_and_empty_lists() -> anyhow::Result<()> {
    let app = test_app().await?;
    let (status, _, bytes) =
        request(&app.router, "GET", "/system/health", None, Body::empty()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&bytes)?, json!({ "status": "ok" }));

    for (path, key) in [
        ("/api/v1/system", "product"),
        ("/api/v1/settings/telemetry", "distinctId"),
        ("/api/v1/sessions", "items"),
        ("/api/v1/tabs", "items"),
        ("/api/v1/connections", "items"),
    ] {
        let (status, _, bytes) = request(&app.router, "GET", path, None, Body::empty()).await?;
        assert_eq!(status, StatusCode::OK, "GET {path}: {bytes:?}");
        assert!(json_body(&bytes)?.get(key).is_some(), "GET {path}");
    }

    let (status, _, bytes) = request(
        &app.router,
        "PUT",
        "/api/v1/settings/telemetry",
        Some("application/json"),
        json!({ "consent": false }).to_string(),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&bytes)?["consent"], false);

    let (status, _, bytes) =
        request(&app.router, "POST", "/system/shutdown", None, Body::empty()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&bytes)?, json!({ "status": "ok" }));
    Ok(())
}

#[tokio::test]
async fn canonical_sessions_cancel_and_recordings() -> anyhow::Result<()> {
    let app = test_app().await?;
    let session = live_session("session-live");
    app.state.sessions.insert_for_testing(session.clone()).await;
    seed_dispatch(&app, "session-live").await?;

    let (status, _, bytes) = request(
        &app.router,
        "GET",
        "/api/v1/sessions/session-live",
        None,
        Body::empty(),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let detail = json_body(&bytes)?;
    assert_eq!(detail["session"]["name"], "research-browserclaw");
    assert_eq!(detail["dispatches"][0]["dispatchId"], 1);
    assert_eq!(detail["dispatches"][0]["hasScreenshot"], false);
    assert!(detail["dispatches"][0].get("agentId").is_none());
    assert!(detail["dispatches"][0].get("url").is_none());

    let (status, _, bytes) = request(
        &app.router,
        "POST",
        "/api/v1/sessions/session-live/cancel",
        None,
        Body::empty(),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&bytes)?, json!({ "cancelled": 0 }));

    let dispatch_token = CancellationToken::new();
    session
        .register_dispatch(DispatchId::new(), dispatch_token.clone())
        .await;
    let (status, _, bytes) = request(
        &app.router,
        "POST",
        "/api/v1/sessions/session-live/cancel",
        None,
        Body::empty(),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&bytes)?, json!({ "cancelled": 1 }));
    assert!(dispatch_token.is_cancelled());
    assert!(app.state.sessions.contains(session.id()).await);

    let (status, _, bytes) = request(
        &app.router,
        "GET",
        "/api/v1/sessions/session-live/recording",
        None,
        Body::empty(),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json_body(&bytes)?,
        json!({ "hasData": false, "sizeBytes": 0, "pageIds": [] })
    );

    app.state
        .tab_activity
        .record_tool(RecordToolInput {
            target_id: TargetId::from("target-7".to_string()),
            tab_id: 101,
            page_id: 7,
            session_id: "session-live".to_string(),
            agent_id: session.convo_id().as_str().to_string(),
            slug: "codex".to_string(),
            tool_name: "snapshot".to_string(),
        })
        .await;
    app.state
        .audit
        .claim_target_for_session("target-7", "session-live", session.convo_id().as_str(), 0)
        .await?;
    app.state
        .tab_activity
        .record_tool(RecordToolInput {
            target_id: TargetId::from("target-8".to_string()),
            tab_id: 102,
            page_id: 8,
            session_id: "session-live".to_string(),
            agent_id: session.convo_id().as_str().to_string(),
            slug: "codex".to_string(),
            tool_name: "snapshot".to_string(),
        })
        .await;
    app.state
        .audit
        .claim_target_for_session("target-8", "session-live", session.convo_id().as_str(), 0)
        .await?;

    let events =
        "{\"ts\":100,\"data\":{\"id\":\"seven-a\"}}\n{\"ts\":200,\"data\":{\"id\":\"seven-b\"}}\n";
    let recording_headers = [
        ("x-recording-tab-id", "101"),
        ("x-recording-page-id", "7"),
        ("x-recording-target-id", "target-7"),
        ("x-recording-batch-id", "batch-7"),
    ];
    let (status, _, bytes) = request_with_headers(
        &app.router,
        "POST",
        "/api/v1/sessions/session-live/recording/events",
        Some("application/x-ndjson"),
        &recording_headers,
        events,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&bytes)?, json!({ "accepted": 2 }));

    let (status, _, bytes) = request_with_headers(
        &app.router,
        "POST",
        "/api/v1/sessions/session-live/recording/events",
        Some("application/x-ndjson"),
        &recording_headers,
        events,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&bytes)?, json!({ "accepted": 0 }));

    let second_headers = [
        ("x-recording-tab-id", "102"),
        ("x-recording-page-id", "8"),
        ("x-recording-target-id", "target-8"),
        ("x-recording-batch-id", "batch-8"),
    ];
    let (status, _, bytes) = request_with_headers(
        &app.router,
        "POST",
        "/api/v1/sessions/session-live/recording/events",
        Some("application/x-ndjson"),
        &second_headers,
        "{\"ts\":150,\"data\":{\"id\":\"eight\"}}\n",
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&bytes)?, json!({ "accepted": 1 }));

    let stale_headers = [
        ("x-recording-tab-id", "102"),
        ("x-recording-page-id", "8"),
        ("x-recording-target-id", "target-7"),
    ];
    let (status, _, bytes) = request_with_headers(
        &app.router,
        "POST",
        "/api/v1/sessions/session-live/recording/events",
        Some("application/x-ndjson"),
        &stale_headers,
        "{\"ts\":175}\n",
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json_body(&bytes)?["code"], "recording_association_changed");

    let (status, headers, bytes) = request(
        &app.router,
        "GET",
        "/api/v1/sessions/session-live/recording/events",
        None,
        Body::empty(),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/x-ndjson")
    );
    let events = String::from_utf8(bytes)?;
    assert_eq!(events.matches("session-live").count(), 3);
    assert!(events.contains("\"targetId\":\"target-7\""));
    assert!(events.contains("\"targetId\":\"target-8\""));
    let seven_a = events
        .find("seven-a")
        .ok_or_else(|| anyhow::anyhow!("missing first target-7 event"))?;
    let eight = events
        .find("eight")
        .ok_or_else(|| anyhow::anyhow!("missing target-8 event"))?;
    let seven_b = events
        .find("seven-b")
        .ok_or_else(|| anyhow::anyhow!("missing second target-7 event"))?;
    assert!(seven_a < eight);
    assert!(eight < seven_b);

    assert!(
        app.state
            .sessions
            .remove(session.id(), "closed", Some("test"))
            .await?
    );
    let (status, _, bytes) = request(
        &app.router,
        "POST",
        "/api/v1/sessions/session-live/recording/events",
        Some("application/x-ndjson"),
        "{\"ts\":300}\n",
    )
    .await?;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(json_body(&bytes)?["code"], "session_ended");

    let (status, _, bytes) = request(
        &app.router,
        "POST",
        "/api/v1/sessions/session-live/cancel",
        None,
        Body::empty(),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json_body(&bytes)?["code"], "session_not_live");
    Ok(())
}

#[tokio::test]
async fn canonical_tabs_previews_screenshots_and_errors() -> anyhow::Result<()> {
    let app = test_app().await?;
    let session = live_session("session-live");
    app.state.sessions.insert_for_testing(session.clone()).await;
    let dispatch_id = seed_dispatch(&app, "session-live").await?;
    app.state
        .tab_activity
        .record_tool(RecordToolInput {
            target_id: TargetId::from("target-7".to_string()),
            tab_id: 101,
            page_id: 7,
            session_id: "session-live".to_string(),
            agent_id: session.convo_id().as_str().to_string(),
            slug: "codex".to_string(),
            tool_name: "snapshot".to_string(),
        })
        .await;
    app.state
        .screencast
        .cache_frame(
            7,
            "target-7",
            ScreencastFrame {
                jpeg_base64: "/9g=".to_string(),
                captured_at: 123,
            },
        )
        .await;
    app.state
        .screenshots
        .write(&dispatch_id.to_string(), &[0xff, 0xd8])
        .await?;

    let (status, _, bytes) =
        request(&app.router, "GET", "/api/v1/tabs", None, Body::empty()).await?;
    assert_eq!(status, StatusCode::OK);
    let tabs = json_body(&bytes)?;
    assert_eq!(tabs["items"][0]["tabId"], 101);
    assert_eq!(tabs["items"][0]["pageId"], 7);
    assert_eq!(tabs["items"][0]["targetId"], "target-7");
    assert_eq!(tabs["items"][0]["sessionId"], "session-live");
    assert_eq!(tabs["items"][0]["previewCapturedAt"], 123);
    assert!(tabs["items"][0].get("jpegBase64").is_none());

    for path in [
        "/api/v1/tabs/7/preview".to_string(),
        format!("/api/v1/dispatches/{dispatch_id}/screenshot"),
    ] {
        let (status, headers, bytes) =
            request(&app.router, "GET", &path, None, Body::empty()).await?;
        assert_eq!(status, StatusCode::OK, "GET {path}");
        assert_eq!(
            headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("image/jpeg")
        );
        assert_eq!(bytes, vec![0xff, 0xd8]);
    }

    for (method, path, expected_status, code) in [
        (
            "GET",
            "/api/v1/sessions?limit=0",
            StatusCode::BAD_REQUEST,
            "invalid_request",
        ),
        (
            "GET",
            "/api/v1/sessions/missing",
            StatusCode::NOT_FOUND,
            "session_not_found",
        ),
        (
            "POST",
            "/api/v1/sessions/missing/cancel",
            StatusCode::NOT_FOUND,
            "session_not_found",
        ),
        (
            "GET",
            "/api/v1/tabs/8/preview",
            StatusCode::NOT_FOUND,
            "preview_not_found",
        ),
        (
            "GET",
            "/api/v1/dispatches/999/screenshot",
            StatusCode::NOT_FOUND,
            "screenshot_not_found",
        ),
        (
            "PUT",
            "/api/v1/connections/Unknown",
            StatusCode::NOT_FOUND,
            "harness_not_found",
        ),
    ] {
        let (status, headers, bytes) =
            request(&app.router, method, path, None, Body::empty()).await?;
        assert_eq!(status, expected_status, "{method} {path}");
        let body = json_body(&bytes)?;
        assert_eq!(body["code"], code, "{method} {path}");
        assert_eq!(
            body["requestId"].as_str(),
            headers
                .get("x-request-id")
                .and_then(|value| value.to_str().ok()),
            "{method} {path}"
        );
    }
    Ok(())
}
