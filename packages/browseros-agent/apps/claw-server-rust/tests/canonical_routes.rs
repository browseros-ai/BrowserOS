//! In-process coverage of the canonical contract routes: drives the
//! full router over seeded app state and a scripted browser via tower, with no network process.
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
    config::Config,
    db::audit_log::{DispatchResultSummary, RecordToolDispatchInput},
    identity::{ClientIdentity, ConversationIdentity},
    ids::{DispatchId, ProfileId, SessionId},
    services::cockpit::{RecordToolInput, ScreencastFrame},
    services::sessions::Session,
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tempfile::TempDir;
use tokio::sync::{Notify, broadcast};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

struct TestApp {
    router: Router,
    state: AppState,
    connection: Arc<FixtureConnection>,
    _dir: TempDir,
}

struct FixtureConnection {
    events: broadcast::Sender<CdpEvent>,
    tabs: tokio::sync::Mutex<Vec<Value>>,
    get_tabs_calls: AtomicUsize,
    fail_next_get_tabs: AtomicBool,
    next_get_tabs_gate: tokio::sync::Mutex<Option<Arc<GetTabsGate>>>,
}

struct GetTabsGate {
    entered: Notify,
    release: Notify,
}

impl GetTabsGate {
    async fn wait_until_entered(&self) {
        self.entered.notified().await;
    }

    fn release(&self) {
        self.release.notify_one();
    }
}

impl FixtureConnection {
    fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(1);
        Arc::new(Self {
            events,
            tabs: tokio::sync::Mutex::new((1..=8).map(fixture_tab).collect()),
            get_tabs_calls: AtomicUsize::new(0),
            fail_next_get_tabs: AtomicBool::new(false),
            next_get_tabs_gate: tokio::sync::Mutex::new(None),
        })
    }

    fn reset_get_tabs_calls(&self) {
        self.get_tabs_calls.store(0, Ordering::SeqCst);
    }

    fn get_tabs_calls(&self) -> usize {
        self.get_tabs_calls.load(Ordering::SeqCst)
    }

    fn fail_next_get_tabs(&self) {
        self.fail_next_get_tabs.store(true, Ordering::SeqCst);
    }

    async fn gate_next_get_tabs(&self) -> Arc<GetTabsGate> {
        let gate = Arc::new(GetTabsGate {
            entered: Notify::new(),
            release: Notify::new(),
        });
        *self.next_get_tabs_gate.lock().await = Some(gate.clone());
        gate
    }

    async fn remove_tab(&self, tab_id: i64) {
        self.tabs
            .lock()
            .await
            .retain(|tab| tab["tabId"].as_i64() != Some(tab_id));
    }

    async fn update_tab(&self, tab_id: i64, target_id: &str, url: &str, title: &str) {
        let mut tabs = self.tabs.lock().await;
        let Some(Value::Object(tab)) = tabs
            .iter_mut()
            .find(|tab| tab["tabId"].as_i64() == Some(tab_id))
        else {
            return;
        };
        tab.insert("targetId".to_string(), json!(target_id));
        tab.insert("url".to_string(), json!(url));
        tab.insert("title".to_string(), json!(title));
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
                "Browser.getTabs" => {
                    self.get_tabs_calls.fetch_add(1, Ordering::SeqCst);
                    let tabs = self.tabs.lock().await.clone();
                    if let Some(gate) = self.next_get_tabs_gate.lock().await.take() {
                        gate.entered.notify_one();
                        gate.release.notified().await;
                    }
                    if self.fail_next_get_tabs.swap(false, Ordering::SeqCst) {
                        return Err(CdpError::Protocol {
                            code: -32000,
                            message: "get tabs failed".to_string(),
                        });
                    }
                    Ok(json!({ "tabs": tabs }))
                }
                "Browser.getTabInfo" => {
                    let tab_id = params.get("tabId").and_then(Value::as_i64);
                    let tab = self
                        .tabs
                        .lock()
                        .await
                        .iter()
                        .find(|tab| tab["tabId"].as_i64() == tab_id)
                        .cloned()
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
    let connection = FixtureConnection::new();
    let browser = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
    assert_eq!(browser.pages.list().await?.len(), 8);
    connection.reset_get_tabs_calls();
    state.browser.set_session_for_testing(browser).await;
    Ok(TestApp {
        router: build_router(state.clone()),
        state,
        connection,
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

fn session_item<'a>(body: &'a Value, session_id: &str) -> anyhow::Result<&'a Value> {
    body["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["sessionId"] == session_id))
        .ok_or_else(|| anyhow::anyhow!("session {session_id} missing"))
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
        .audit_log
        .record_tool_dispatch(RecordToolDispatchInput {
            agent_id: "codex-research-browserclaw".to_string(),
            slug: "codex".to_string(),
            agent_label: "Codex".to_string(),
            session_id: session_id.to_string(),
            tool_name: "snapshot".to_string(),
            page_id: Some(7),
            tab_id: Some(101),
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
        ("GET", "/api/v1/tabs"),
        ("GET", "/api/v1/tabs/7/preview"),
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
        json!({ "hasData": false, "complete": true, "sizeBytes": 0, "tabs": [] })
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
    app.state.session_tabs.enqueue_claim_tab_for_session(
        101,
        Some("target-7".to_string()),
        "session-live".to_string(),
        session.convo_id().as_str().to_string(),
        0,
    );
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
    app.state.session_tabs.enqueue_claim_tab_for_session(
        102,
        Some("target-8".to_string()),
        "session-live".to_string(),
        session.convo_id().as_str().to_string(),
        0,
    );
    app.state.session_tabs.drain_writes().await;

    for document_id in [
        "33D25F3CF060E81B14070BC356FF187",
        "33D25F3CF060E81B14070BC356FF187Z",
        "018f47a7-1c2b-7def-8123-0123456789ab",
    ] {
        let malformed_headers = [
            ("x-recording-tab-id", "101"),
            ("x-recording-document-id", document_id),
            ("x-recording-batch-id", "malformed-document"),
        ];
        let (status, _, bytes) = request_with_headers(
            &app.router,
            "POST",
            "/api/v1/recordings/events",
            Some("application/x-ndjson"),
            &malformed_headers,
            "{\"ts\":50}\n",
        )
        .await?;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json_body(&bytes)?["code"], "invalid_request");
    }

    let events =
        "{\"ts\":100,\"data\":{\"id\":\"seven-a\"}}\n{\"ts\":200,\"data\":{\"id\":\"seven-b\"}}\n";
    let recording_headers = [
        ("x-recording-tab-id", "101"),
        (
            "x-recording-document-id",
            "33D25F3CF060E81B14070BC356FF1871",
        ),
        ("x-recording-batch-id", "batch-7"),
    ];
    let (status, _, bytes) = request_with_headers(
        &app.router,
        "POST",
        "/api/v1/recordings/events",
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
        "/api/v1/recordings/events",
        Some("application/x-ndjson"),
        &recording_headers,
        events,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&bytes)?, json!({ "accepted": 0 }));

    let second_headers = [
        ("x-recording-tab-id", "102"),
        (
            "x-recording-document-id",
            "8395FF2EF4A1D8579F1917B3B54ADECE",
        ),
        ("x-recording-batch-id", "batch-8"),
    ];
    let (status, _, bytes) = request_with_headers(
        &app.router,
        "POST",
        "/api/v1/recordings/events",
        Some("application/x-ndjson"),
        &second_headers,
        "{\"ts\":150,\"data\":{\"id\":\"eight\"}}\n",
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&bytes)?, json!({ "accepted": 1 }));

    let (status, _, bytes) = request(
        &app.router,
        "GET",
        "/api/v1/sessions/session-live/recording",
        None,
        Body::empty(),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let metadata = json_body(&bytes)?;
    assert_eq!(metadata["hasData"], true);
    assert_eq!(metadata["complete"], true);
    assert_eq!(metadata["tabs"].as_array().map(Vec::len), Some(2));

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
    assert!(events.contains("33D25F3CF060E81B14070BC356FF1871"));
    assert!(events.contains("8395FF2EF4A1D8579F1917B3B54ADECE"));
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
    let late_headers = [
        ("x-recording-tab-id", "101"),
        (
            "x-recording-document-id",
            "9E84CDCAB8762569B5B109D125F60147",
        ),
        ("x-recording-batch-id", "batch-late"),
    ];
    let (status, _, bytes) = request_with_headers(
        &app.router,
        "POST",
        "/api/v1/recordings/events",
        Some("application/x-ndjson"),
        &late_headers,
        "{\"ts\":300}\n",
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&bytes)?, json!({ "accepted": 1 }));

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
async fn recording_ingest_accepts_bodies_larger_than_ten_mebibytes() -> anyhow::Result<()> {
    let app = test_app().await?;
    let headers = [
        ("x-recording-tab-id", "101"),
        (
            "x-recording-document-id",
            "33D25F3CF060E81B14070BC356FF1871",
        ),
        ("x-recording-batch-id", "large-batch"),
    ];
    let body = format!("{{\"ts\":100}}\n{}", " ".repeat(10 * 1024 * 1024));

    let (status, _, bytes) = request_with_headers(
        &app.router,
        "POST",
        "/api/v1/recordings/events",
        Some("application/x-ndjson"),
        &headers,
        body,
    )
    .await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&bytes)?, json!({ "accepted": 1 }));
    Ok(())
}

struct LiveFixture {
    primary: Arc<Session>,
    second: Arc<Session>,
    zero_tab: Arc<Session>,
    screenshot_dispatch_id: i64,
}

async fn seed_live_fixture(app: &TestApp) -> anyhow::Result<LiveFixture> {
    let agents_dir = app.state.config.browserclaw_dir.join("agents");
    tokio::fs::create_dir_all(&agents_dir).await?;
    tokio::fs::write(
        agents_dir.join("profile-shared.json"),
        json!({
            "id": "profile-shared",
            "name": "Codex",
            "harness": "Codex",
            "loginMode": "profile",
            "selectedSites": [],
            "approvals": {},
            "aclRuleIds": [],
            "customAclRules": [],
            "slug": "codex",
            "mcpUrl": "http://127.0.0.1:9200/mcp",
            "status": "configured",
            "createdAt": "now",
            "updatedAt": "now"
        })
        .to_string(),
    )
    .await?;

    let primary = profiled_session("session-live", "research-browserclaw");
    let second = profiled_session("session-live-shared-profile", "compare-release-notes");
    let zero_tab = Session::new(
        SessionId::new("session-live-empty"),
        ClientIdentity::Ephemeral {
            slug: "claude-code".to_string(),
            label: "Claude Code".to_string(),
        },
        ConversationIdentity::new("claude-code", "waiting-for-first-tool".to_string()),
        tokio::time::Instant::now(),
    );
    for session in [&primary, &second, &zero_tab] {
        app.state.sessions.insert_for_testing(session.clone()).await;
        app.state
            .audit_log
            .record_session_start(
                session.id().as_str(),
                session.convo_id().as_str(),
                session.agent().slug(),
                session.agent().label(),
                session.agent().label(),
                "1.0",
            )
            .await?;
    }
    let screenshot_dispatch_id = seed_dispatch(app, primary.id().as_str()).await?;
    app.state
        .audit_log
        .mark_screenshot(screenshot_dispatch_id)
        .await?;
    app.state.tab_activity.set_now_for_testing(100);
    app.state
        .tab_activity
        .record_tool(RecordToolInput {
            target_id: TargetId::from("target-7".to_string()),
            tab_id: 101,
            page_id: 7,
            session_id: primary.id().as_str().to_string(),
            agent_id: primary.convo_id().as_str().to_string(),
            slug: "codex".to_string(),
            tool_name: "snapshot".to_string(),
        })
        .await;
    for (session, tab_id, target_id) in [(&primary, 101, "target-7"), (&primary, 102, "target-8")] {
        app.state.session_tabs.enqueue_claim_tab_for_session(
            tab_id,
            Some(target_id.to_string()),
            session.id().as_str().to_string(),
            session.convo_id().as_str().to_string(),
            0,
        );
    }
    app.state
        .previews
        .cache_frame(
            primary.id().as_str(),
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
        .write(&screenshot_dispatch_id.to_string(), &[0xff, 0xd8])
        .await?;
    app.connection.reset_get_tabs_calls();
    Ok(LiveFixture {
        primary,
        second,
        zero_tab,
        screenshot_dispatch_id,
    })
}

fn profiled_session(session_id: &str, name: &str) -> Arc<Session> {
    Session::new(
        SessionId::new(session_id),
        ClientIdentity::Profile {
            profile_id: ProfileId::new("profile-shared"),
            slug: "codex".to_string(),
            label: "Codex".to_string(),
        },
        ConversationIdentity::new("codex", name.to_string()),
        tokio::time::Instant::now(),
    )
}

#[tokio::test]
async fn live_projection_includes_zero_tab_and_same_profile_sessions() -> anyhow::Result<()> {
    let app = test_app().await?;
    let fixture = seed_live_fixture(&app).await?;

    let (status, _, bytes) = request(
        &app.router,
        "GET",
        "/api/v1/sessions?status=live&limit=1&cursor=999",
        None,
        Body::empty(),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(app.connection.get_tabs_calls(), 1);
    let snapshot = json_body(&bytes)?;
    assert!(snapshot.get("nextCursor").is_none());
    let items = snapshot["items"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("live items missing"))?;
    assert_eq!(items.len(), 3);
    let primary = items
        .iter()
        .find(|item| item["sessionId"] == fixture.primary.id().as_str())
        .ok_or_else(|| anyhow::anyhow!("primary live session missing"))?;
    let second = items
        .iter()
        .find(|item| item["sessionId"] == fixture.second.id().as_str())
        .ok_or_else(|| anyhow::anyhow!("same-profile live session missing"))?;
    let zero_tab = items
        .iter()
        .find(|item| item["sessionId"] == fixture.zero_tab.id().as_str())
        .ok_or_else(|| anyhow::anyhow!("zero-tab live session missing"))?;
    assert_eq!(primary["profileId"], "profile-shared");
    assert_eq!(second["profileId"], primary["profileId"]);
    assert_ne!(second["sessionId"], primary["sessionId"]);
    assert_eq!(primary["harness"], "Codex");
    assert_eq!(primary["color"], "#7A5AF8");
    assert_eq!(primary["live"]["state"], "active");
    assert_eq!(primary["live"]["browserTabs"][0]["browserTabId"], 101);
    assert_eq!(primary["live"]["browserTabs"][0]["toolCount"], 1);
    assert_eq!(primary["live"]["browserTabs"][0]["previewCapturedAt"], 123);
    assert_eq!(primary["live"]["browserTabs"][1]["browserTabId"], 102);
    assert_eq!(primary["live"]["browserTabs"][1]["toolCount"], 0);
    assert_eq!(primary["live"]["browserTabs"][1]["recentTools"], json!([]));
    assert!(
        primary["live"]["browserTabs"][1]
            .get("lastActivityAt")
            .is_none()
    );
    assert_eq!(
        zero_tab["live"],
        json!({ "state": "idle", "browserTabs": [] })
    );

    app.state.tab_activity.set_now_for_testing(30_101);
    let (_, _, bytes) = request(
        &app.router,
        "GET",
        "/api/v1/sessions?status=live&profileId=profile-shared",
        None,
        Body::empty(),
    )
    .await?;
    let filtered = json_body(&bytes)?;
    assert_eq!(filtered["items"].as_array().map(Vec::len), Some(2));
    assert!(
        filtered["items"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item["live"]["state"] == "idle"))
    );
    Ok(())
}

#[tokio::test]
async fn historical_session_queries_do_not_reconcile_browser_state() -> anyhow::Result<()> {
    let app = test_app().await?;
    seed_live_fixture(&app).await?;
    app.connection.reset_get_tabs_calls();

    for path in ["/api/v1/sessions", "/api/v1/sessions?status=done"] {
        let (status, _, _) = request(&app.router, "GET", path, None, Body::empty()).await?;
        assert_eq!(status, StatusCode::OK, "GET {path}");
    }
    assert_eq!(app.connection.get_tabs_calls(), 0);
    Ok(())
}

#[tokio::test]
async fn open_claims_reconcile_closed_and_reassigned_browser_tabs() -> anyhow::Result<()> {
    let app = test_app().await?;
    let fixture = seed_live_fixture(&app).await?;

    let (_, _, bytes) = request(
        &app.router,
        "GET",
        "/api/v1/sessions?status=live",
        None,
        Body::empty(),
    )
    .await?;
    let first = json_body(&bytes)?;
    let primary = first["items"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["sessionId"] == fixture.primary.id().as_str())
        })
        .ok_or_else(|| anyhow::anyhow!("primary session missing"))?;
    assert_eq!(
        primary["live"]["browserTabs"].as_array().map(Vec::len),
        Some(2)
    );

    app.connection.remove_tab(102).await;
    app.state.session_tabs.enqueue_claim_tab_for_session(
        101,
        Some("target-7".to_string()),
        fixture.second.id().as_str().to_string(),
        fixture.second.convo_id().as_str().to_string(),
        200,
    );
    let (_, _, bytes) = request(
        &app.router,
        "GET",
        "/api/v1/sessions?status=live",
        None,
        Body::empty(),
    )
    .await?;
    let second = json_body(&bytes)?;
    let items = second["items"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("live items missing"))?;
    let primary = items
        .iter()
        .find(|item| item["sessionId"] == fixture.primary.id().as_str())
        .ok_or_else(|| anyhow::anyhow!("primary session missing"))?;
    let reassigned = items
        .iter()
        .find(|item| item["sessionId"] == fixture.second.id().as_str())
        .ok_or_else(|| anyhow::anyhow!("new owner missing"))?;
    assert_eq!(primary["live"]["browserTabs"], json!([]));
    assert_eq!(reassigned["live"]["browserTabs"][0]["browserTabId"], 101);
    assert_eq!(reassigned["live"]["browserTabs"][0]["toolCount"], 0);
    Ok(())
}

#[tokio::test]
async fn secure_preview_is_owned_fail_closed_and_cache_only() -> anyhow::Result<()> {
    let app = test_app().await?;
    let fixture = seed_live_fixture(&app).await?;
    assert_eq!(app.state.previews.last_read_at_for_testing(), 0);

    let preview_path = format!(
        "/api/v1/sessions/{}/browser-tabs/101/preview",
        fixture.primary.id().as_str()
    );
    let (status, headers, bytes) =
        request(&app.router, "GET", &preview_path, None, Body::empty()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/jpeg");
    assert_eq!(bytes, vec![0xff, 0xd8]);
    assert_eq!(app.state.previews.last_read_at_for_testing(), 0);

    let failures = [
        format!(
            "/api/v1/sessions/{}/browser-tabs/101/preview",
            fixture.second.id().as_str()
        ),
        "/api/v1/sessions/missing/browser-tabs/101/preview".to_string(),
        format!(
            "/api/v1/sessions/{}/browser-tabs/999/preview",
            fixture.primary.id().as_str()
        ),
    ];
    let mut failure_shape = None;
    for path in failures {
        let (status, _, bytes) = request(&app.router, "GET", &path, None, Body::empty()).await?;
        assert_eq!(status, StatusCode::NOT_FOUND, "GET {path}");
        let body = json_body(&bytes)?;
        let shape = (body["code"].clone(), body["message"].clone());
        assert_eq!(shape.0, "preview_not_found");
        assert_eq!(shape.1, "browser tab preview not found");
        assert_eq!(failure_shape.get_or_insert_with(|| shape.clone()), &shape);
    }

    app.state.session_tabs.enqueue_claim_tab_for_session(
        101,
        Some("target-7".to_string()),
        fixture.second.id().as_str().to_string(),
        fixture.second.convo_id().as_str().to_string(),
        200,
    );
    let (status, _, bytes) =
        request(&app.router, "GET", &preview_path, None, Body::empty()).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json_body(&bytes)?["code"], "preview_not_found");

    app.connection
        .update_tab(101, "target-reused", "https://example.com/reused", "Reused")
        .await;
    let reassigned_path = format!(
        "/api/v1/sessions/{}/browser-tabs/101/preview",
        fixture.second.id().as_str()
    );
    let (status, _, bytes) =
        request(&app.router, "GET", &reassigned_path, None, Body::empty()).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json_body(&bytes)?["code"], "preview_not_found");

    let (status, _, _) = request(
        &app.router,
        "GET",
        "/api/v1/sessions?status=live",
        None,
        Body::empty(),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert!(app.state.previews.last_read_at_for_testing() > 0);

    let screenshot_path = format!(
        "/api/v1/dispatches/{}/screenshot",
        fixture.screenshot_dispatch_id
    );
    let (status, headers, bytes) =
        request(&app.router, "GET", &screenshot_path, None, Body::empty()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/jpeg");
    assert_eq!(bytes, vec![0xff, 0xd8]);

    for (path, expected_status, code) in [
        (
            "/api/v1/sessions?limit=0",
            StatusCode::BAD_REQUEST,
            "invalid_request",
        ),
        (
            "/api/v1/sessions/session-live/browser-tabs/0/preview",
            StatusCode::BAD_REQUEST,
            "invalid_request",
        ),
        (
            "/api/v1/dispatches/999/screenshot",
            StatusCode::NOT_FOUND,
            "screenshot_not_found",
        ),
    ] {
        let (status, _, bytes) = request(&app.router, "GET", path, None, Body::empty()).await?;
        assert_eq!(status, expected_status, "GET {path}");
        assert_eq!(json_body(&bytes)?["code"], code, "GET {path}");
    }
    Ok(())
}

#[tokio::test]
async fn preview_rejects_disconnected_session_after_browser_reconciliation() -> anyhow::Result<()> {
    let app = test_app().await?;
    let fixture = seed_live_fixture(&app).await?;
    let gate = app.connection.gate_next_get_tabs().await;
    let router = app.router.clone();
    let preview_path = format!(
        "/api/v1/sessions/{}/browser-tabs/101/preview",
        fixture.primary.id().as_str()
    );
    let preview =
        tokio::spawn(
            async move { request(&router, "GET", &preview_path, None, Body::empty()).await },
        );

    gate.wait_until_entered().await;
    assert!(
        app.state
            .sessions
            .remove(fixture.primary.id(), "closed", Some("test disconnect"))
            .await?
    );
    app.state.session_tabs.enqueue_claim_tab_for_session(
        101,
        Some("target-7".to_string()),
        fixture.primary.id().as_str().to_string(),
        fixture.primary.convo_id().as_str().to_string(),
        200,
    );
    app.state.session_tabs.drain_writes().await;
    assert!(!app.state.sessions.contains(fixture.primary.id()).await);
    assert!(
        app.state
            .session_tabs
            .open_session_tab(fixture.primary.id().as_str(), 101)
            .await?
            .is_some()
    );

    gate.release();
    let (status, _, bytes) = preview.await??;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json_body(&bytes)?["code"], "preview_not_found");
    assert_ne!(bytes, vec![0xff, 0xd8]);
    Ok(())
}

#[tokio::test]
async fn preview_rejects_ownership_transfer_during_frame_lookup() -> anyhow::Result<()> {
    let app = test_app().await?;
    let fixture = seed_live_fixture(&app).await?;
    let gate = app.state.previews.gate_next_frame_read_for_testing();
    let router = app.router.clone();
    let preview_path = format!(
        "/api/v1/sessions/{}/browser-tabs/101/preview",
        fixture.primary.id().as_str()
    );
    let preview =
        tokio::spawn(
            async move { request(&router, "GET", &preview_path, None, Body::empty()).await },
        );

    gate.wait_until_entered().await;
    assert_eq!(app.connection.get_tabs_calls(), 1);
    app.state.session_tabs.enqueue_claim_tab_for_session(
        101,
        Some("target-7".to_string()),
        fixture.second.id().as_str().to_string(),
        fixture.second.convo_id().as_str().to_string(),
        200,
    );
    app.state.session_tabs.drain_writes().await;
    assert!(
        app.state
            .session_tabs
            .open_session_tab(fixture.primary.id().as_str(), 101)
            .await?
            .is_none()
    );
    assert!(
        app.state
            .session_tabs
            .open_session_tab(fixture.second.id().as_str(), 101)
            .await?
            .is_some()
    );

    gate.release();
    let (status, _, bytes) = preview.await??;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json_body(&bytes)?["code"], "preview_not_found");
    assert_ne!(bytes, vec![0xff, 0xd8]);
    Ok(())
}

#[tokio::test]
async fn transferred_session_cannot_read_prior_owner_frame() -> anyhow::Result<()> {
    let app = test_app().await?;
    let fixture = seed_live_fixture(&app).await?;
    app.state.session_tabs.enqueue_claim_tab_for_session(
        101,
        Some("target-7".to_string()),
        fixture.second.id().as_str().to_string(),
        fixture.second.convo_id().as_str().to_string(),
        200,
    );
    app.state.session_tabs.drain_writes().await;

    let (status, _, bytes) = request(
        &app.router,
        "GET",
        "/api/v1/sessions?status=live",
        None,
        Body::empty(),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let body = json_body(&bytes)?;
    let second = session_item(&body, fixture.second.id().as_str())?;
    assert_eq!(second["live"]["browserTabs"][0]["browserTabId"], 101);
    assert!(
        second["live"]["browserTabs"][0]
            .get("previewCapturedAt")
            .is_none()
    );

    let preview_path = format!(
        "/api/v1/sessions/{}/browser-tabs/101/preview",
        fixture.second.id().as_str()
    );
    let (status, _, bytes) =
        request(&app.router, "GET", &preview_path, None, Body::empty()).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let body = json_body(&bytes)?;
    assert_eq!(body["code"], "preview_not_found");
    assert_eq!(body["message"], "browser tab preview not found");
    Ok(())
}

#[tokio::test]
async fn preview_rejects_target_rebind_during_frame_lookup() -> anyhow::Result<()> {
    let app = test_app().await?;
    let fixture = seed_live_fixture(&app).await?;
    let gate = app.state.previews.gate_next_frame_read_for_testing();
    let router = app.router.clone();
    let preview_path = format!(
        "/api/v1/sessions/{}/browser-tabs/101/preview",
        fixture.primary.id().as_str()
    );
    let preview =
        tokio::spawn(
            async move { request(&router, "GET", &preview_path, None, Body::empty()).await },
        );

    gate.wait_until_entered().await;
    assert_eq!(app.connection.get_tabs_calls(), 1);
    app.connection
        .update_tab(101, "target-rebound", "https://example.com/new", "New")
        .await;
    gate.release();

    let (status, _, bytes) = preview.await??;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let body = json_body(&bytes)?;
    assert_eq!(body["code"], "preview_not_found");
    assert_eq!(body["message"], "browser tab preview not found");
    Ok(())
}

#[tokio::test]
async fn preview_rejects_transfer_during_final_page_reconciliation() -> anyhow::Result<()> {
    let app = test_app().await?;
    let fixture = seed_live_fixture(&app).await?;
    let frame_gate = app.state.previews.gate_next_frame_read_for_testing();
    let router = app.router.clone();
    let preview_path = format!(
        "/api/v1/sessions/{}/browser-tabs/101/preview",
        fixture.primary.id().as_str()
    );
    let preview =
        tokio::spawn(
            async move { request(&router, "GET", &preview_path, None, Body::empty()).await },
        );

    frame_gate.wait_until_entered().await;
    let pages_gate = app.connection.gate_next_get_tabs().await;
    frame_gate.release();
    pages_gate.wait_until_entered().await;
    app.state.session_tabs.enqueue_claim_tab_for_session(
        101,
        Some("target-7".to_string()),
        fixture.second.id().as_str().to_string(),
        fixture.second.convo_id().as_str().to_string(),
        200,
    );
    app.state.session_tabs.drain_writes().await;
    assert!(
        app.state
            .session_tabs
            .open_session_tab(fixture.primary.id().as_str(), 101)
            .await?
            .is_none()
    );
    assert!(
        app.state
            .session_tabs
            .open_session_tab(fixture.second.id().as_str(), 101)
            .await?
            .is_some()
    );
    pages_gate.release();

    let (status, _, bytes) = preview.await??;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let body = json_body(&bytes)?;
    assert_eq!(body["code"], "preview_not_found");
    assert_eq!(body["message"], "browser tab preview not found");
    Ok(())
}

#[tokio::test]
async fn browser_failure_hides_tabs_without_erasing_activity() -> anyhow::Result<()> {
    let app = test_app().await?;
    let fixture = seed_live_fixture(&app).await?;
    app.connection.fail_next_get_tabs();

    let (status, _, bytes) = request(
        &app.router,
        "GET",
        "/api/v1/sessions?status=live",
        None,
        Body::empty(),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let unavailable = json_body(&bytes)?;
    assert_eq!(unavailable["items"].as_array().map(Vec::len), Some(3));
    assert!(unavailable["items"].as_array().is_some_and(|items| {
        items
            .iter()
            .all(|item| item["live"]["state"] == "idle" && item["live"]["browserTabs"] == json!([]))
    }));

    let (status, _, bytes) = request(
        &app.router,
        "GET",
        "/api/v1/sessions?status=live",
        None,
        Body::empty(),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let recovered = json_body(&bytes)?;
    let primary = session_item(&recovered, fixture.primary.id().as_str())?;
    assert_eq!(primary["live"]["state"], "active");
    assert_eq!(primary["live"]["browserTabs"][0]["browserTabId"], 101);
    assert_eq!(primary["live"]["browserTabs"][0]["toolCount"], 1);
    Ok(())
}

#[tokio::test]
async fn live_projection_revalidates_transfer_after_reconciliation() -> anyhow::Result<()> {
    let app = test_app().await?;
    let fixture = seed_live_fixture(&app).await?;
    let gate = app.connection.gate_next_get_tabs().await;
    let router = app.router.clone();
    let list = tokio::spawn(async move {
        request(
            &router,
            "GET",
            "/api/v1/sessions?status=live",
            None,
            Body::empty(),
        )
        .await
    });

    gate.wait_until_entered().await;
    app.state.session_tabs.enqueue_claim_tab_for_session(
        101,
        Some("target-7".to_string()),
        fixture.second.id().as_str().to_string(),
        fixture.second.convo_id().as_str().to_string(),
        200,
    );
    app.state.session_tabs.drain_writes().await;
    gate.release();

    let (status, _, bytes) = list.await??;
    assert_eq!(status, StatusCode::OK);
    let body = json_body(&bytes)?;
    for session in [&fixture.primary, &fixture.second] {
        let item = session_item(&body, session.id().as_str())?;
        assert!(
            item["live"]["browserTabs"]
                .as_array()
                .is_some_and(|tabs| tabs.iter().all(|tab| tab["browserTabId"] != 101))
        );
    }
    Ok(())
}

#[tokio::test]
async fn live_projection_drops_disconnect_during_reconciliation() -> anyhow::Result<()> {
    let app = test_app().await?;
    let fixture = seed_live_fixture(&app).await?;
    let gate = app.connection.gate_next_get_tabs().await;
    let router = app.router.clone();
    let list = tokio::spawn(async move {
        request(
            &router,
            "GET",
            "/api/v1/sessions?status=live",
            None,
            Body::empty(),
        )
        .await
    });

    gate.wait_until_entered().await;
    assert!(
        app.state
            .sessions
            .remove(fixture.primary.id(), "closed", Some("test disconnect"))
            .await?
    );
    app.state.session_tabs.enqueue_claim_tab_for_session(
        101,
        Some("target-7".to_string()),
        fixture.primary.id().as_str().to_string(),
        fixture.primary.convo_id().as_str().to_string(),
        200,
    );
    app.state.session_tabs.drain_writes().await;
    gate.release();

    let (status, _, bytes) = list.await??;
    assert_eq!(status, StatusCode::OK);
    let body = json_body(&bytes)?;
    assert!(session_item(&body, fixture.primary.id().as_str()).is_err());
    Ok(())
}
