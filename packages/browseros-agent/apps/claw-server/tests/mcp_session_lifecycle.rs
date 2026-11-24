//! How long an MCP transport, a task and its tab group live once an agent goes quiet.
//! These run the real `/mcp` router with short clocks so each lifetime is observable.

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use browseros_core::NativeCapabilities;
use claw_server::{
    AppState, build_router,
    config::{Behavior, Config, NativeIdentity, ServerMode},
    ids::log_fingerprint,
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    io::Write,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tempfile::TempDir;
use tower::ServiceExt;

const LEGACY_VERSION: &str = "2025-11-25";

struct App {
    router: Router,
    state: AppState,
    _dir: TempDir,
}

struct Clocks {
    session_idle: Duration,
    session_complete_idle: Duration,
    mcp_transport_idle: Duration,
}

impl Default for Clocks {
    fn default() -> Self {
        Self {
            session_idle: Duration::from_secs(300),
            session_complete_idle: Duration::from_secs(300),
            mcp_transport_idle: Duration::from_secs(300),
        }
    }
}

async fn app(clocks: Clocks) -> anyhow::Result<App> {
    let dir = tempfile::tempdir()?;
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49337,
        proxy_port: None,
        resources_dir: dir.path().join("resources"),
        browserclaw_dir: dir.path().join("browserclaw"),
        session_idle: clocks.session_idle,
        session_complete_idle: clocks.session_complete_idle,
        mcp_transport_idle: clocks.mcp_transport_idle,
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
        native: NativeIdentity {
            capabilities: NativeCapabilities::from_browseros_version(Some("0.51.1.2")),
            asserted: false,
        },
        mode: ServerMode::Desktop,
        behavior: Behavior::defaults(ServerMode::Desktop),
        behavior_ignored: Vec::new(),
    });
    let state = AppState::new_with_home(config, dir.path().join("home")).await?;
    Ok(App {
        router: build_router(state.clone()),
        state,
        _dir: dir,
    })
}

async fn post(
    router: &Router,
    body: Value,
    headers: &[(&str, &str)],
) -> anyhow::Result<(StatusCode, HeaderMap, String)> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    for (key, value) in headers {
        request = request.header(*key, *value);
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(body.to_string()))?)
        .await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok((
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    ))
}

async fn legacy_transport(router: &Router) -> anyhow::Result<String> {
    let (status, headers, body) = post(
        router,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion": LEGACY_VERSION,
            "capabilities": {},
            "clientInfo": {"name":"opencode","version":"1.0"}}}),
        &[],
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let session = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("initialize returned no mcp-session-id"))?
        .to_string();
    let (status, _, body) = post(
        router,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        &transport_headers(&session),
    )
    .await?;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    Ok(session)
}

fn transport_headers(session: &str) -> [(&str, &str); 2] {
    [
        ("mcp-session-id", session),
        ("mcp-protocol-version", LEGACY_VERSION),
    ]
}

async fn ping(router: &Router, session: &str) -> anyhow::Result<StatusCode> {
    let (status, _, _) = post(
        router,
        json!({"jsonrpc":"2.0","id":2,"method":"ping"}),
        &transport_headers(session),
    )
    .await?;
    Ok(status)
}

async fn open_get_stream(router: &Router, session: &str) -> anyhow::Result<(StatusCode, Body)> {
    let request = Request::builder()
        .method("GET")
        .uri("/mcp")
        .header(header::HOST, "localhost")
        .header(header::ACCEPT, "text/event-stream")
        .header("mcp-session-id", session)
        .header("mcp-protocol-version", LEGACY_VERSION)
        .body(Body::empty())?;
    let response = router.clone().oneshot(request).await?;
    Ok((response.status(), response.into_body()))
}

#[tokio::test]
async fn the_transport_follows_its_own_idle_limit_not_the_session_idle() -> anyhow::Result<()> {
    let app = app(Clocks {
        session_idle: Duration::from_secs(300),
        mcp_transport_idle: Duration::from_millis(1_000),
        ..Clocks::default()
    })
    .await?;
    let session = legacy_transport(&app.router).await?;

    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(
            ping(&app.router, &session).await?,
            StatusCode::OK,
            "a request inside the limit keeps the transport"
        );
    }
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    assert_eq!(
        ping(&app.router, &session).await?,
        StatusCode::NOT_FOUND,
        "a request after the limit finds the transport gone"
    );
    Ok(())
}

#[tokio::test]
async fn an_open_get_stream_does_not_keep_the_transport_alive() -> anyhow::Result<()> {
    let app = app(Clocks {
        mcp_transport_idle: Duration::from_millis(1_000),
        ..Clocks::default()
    })
    .await?;
    let session = legacy_transport(&app.router).await?;
    let (status, body) = open_get_stream(&app.router, &session).await?;
    assert_eq!(status, StatusCode::OK);
    let stream = tokio::spawn(async move {
        let mut chunks = body.into_data_stream();
        while let Some(Ok(_)) = chunks.next().await {}
    });

    tokio::time::timeout(Duration::from_secs(5), stream).await??;
    assert_eq!(ping(&app.router, &session).await?, StatusCode::NOT_FOUND);
    let (status, _) = open_get_stream(&app.router, &session).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn a_transport_outlives_its_finished_task() -> anyhow::Result<()> {
    // A transport idle far above the session idle keeps answering after the task's own
    // clocks would have fired, which is what spares a returning client the 404.
    let app = app(Clocks {
        session_idle: Duration::from_millis(200),
        session_complete_idle: Duration::from_millis(200),
        mcp_transport_idle: Duration::from_secs(60),
    })
    .await?;
    let session = legacy_transport(&app.router).await?;
    tokio::time::sleep(Duration::from_millis(600)).await;
    app.state.sessions.sweep_idle().await?;
    assert_eq!(ping(&app.router, &session).await?, StatusCode::OK);
    Ok(())
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        String::from_utf8_lossy(&bytes).into_owned()
    }

    async fn wait_for(&self, needle: &str) -> String {
        for _ in 0..50 {
            let text = self.text();
            if text.contains(needle) {
                return text;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.text()
    }
}

impl Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// Process-wide, because the tests in this file run on parallel threads and a thread-local
// subscriber misses events whose callsite another thread cached first.
fn captured_info_logs() -> &'static Logs {
    static LOGS: OnceLock<Logs> = OnceLock::new();
    LOGS.get_or_init(|| {
        let logs = Logs::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(move || writer.clone())
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
        logs
    })
}

#[tokio::test]
async fn transport_and_session_ends_are_logged_by_fingerprint_only() -> anyhow::Result<()> {
    let logs = captured_info_logs();
    let app = app(Clocks {
        mcp_transport_idle: Duration::from_millis(1_000),
        ..Clocks::default()
    })
    .await?;
    let session = legacy_transport(&app.router).await?;
    let fingerprint = log_fingerprint(&session);

    tokio::time::sleep(Duration::from_millis(1_600)).await;
    assert_eq!(ping(&app.router, &session).await?, StatusCode::NOT_FOUND);
    let text = logs.wait_for("claw session ended").await;
    assert!(
        text.contains("mcp transport session ended") && text.contains(&fingerprint),
        "{text}"
    );
    assert!(
        text.contains("mcp request for an ended transport session"),
        "{text}"
    );
    assert!(
        text.contains("claw session ended") && text.contains("transport closed"),
        "{text}"
    );

    let never_issued = "never-issued-transport";
    assert_eq!(
        ping(&app.router, never_issued).await?,
        StatusCode::NOT_FOUND
    );
    let text = logs.wait_for("unknown transport session").await;
    assert!(
        text.contains("mcp request for an unknown transport session")
            && text.contains(&log_fingerprint(never_issued)),
        "{text}"
    );

    // rmcp's own "create new session" line and the existing session-start line still carry the
    // full id; the lines this server adds for transport and task ends must not.
    for line in text.lines().filter(|line| {
        [
            "mcp transport session ended",
            "transport session; the client must re-initialize",
            "claw session ended",
        ]
        .iter()
        .any(|message| line.contains(message))
    }) {
        assert!(
            !line.contains(&session) && !line.contains(never_issued),
            "a full transport id was logged: {line}"
        );
    }
    Ok(())
}
