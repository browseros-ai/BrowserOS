use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    routing::any,
};
use claw_server::{
    AppState,
    analytics::{AnalyticsService, events::SERVER_STARTED},
    build_router,
    config::{Config, ServerMode},
};
use serde_json::{Value, json};
use std::{
    env,
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;
use uuid::Uuid;

const TEST_CASE: &str = "CLAW_TELEMETRY_TEST_CASE";
const TEST_ROOT: &str = "CLAW_TELEMETRY_TEST_ROOT";

#[test]
fn telemetry_routes_round_trip_persist_and_honor_the_env_gate() -> anyhow::Result<()> {
    if let Ok(case) = env::var(TEST_CASE) {
        return tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(run_child_case(&case));
    }

    let root = tempfile::tempdir()?;
    // Each child configures the real app in its own process. The collector outlives them,
    // and each child drains delivery before the parent compares its final request count.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let requests = Arc::new(AtomicUsize::new(0));
    let (host, collector) = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let host = format!("http://{}", listener.local_addr()?);
        let requests = requests.clone();
        let app = Router::new().fallback(any(move || {
            requests.fetch_add(1, Ordering::SeqCst);
            async { StatusCode::OK }
        }));
        let collector = tokio::spawn(async move { axum::serve(listener, app).await });
        Ok::<_, anyhow::Error>((host, collector))
    })?;
    run_child(root.path(), "roundtrip", "1", &host)?;
    let desktop_requests = requests.load(Ordering::SeqCst);
    assert!(
        desktop_requests > 0,
        "desktop delivers to the local collector"
    );
    run_child(root.path(), "gate-off", "false", &host)?;
    run_child(root.path(), "hosted", "1", &host)?;
    assert_eq!(requests.load(Ordering::SeqCst), desktop_requests);
    collector.abort();
    Ok(())
}

fn run_child(root: &Path, case: &str, analytics_enabled: &str, host: &str) -> anyhow::Result<()> {
    let output = Command::new(env::current_exe()?)
        .arg("--exact")
        .arg("telemetry_routes_round_trip_persist_and_honor_the_env_gate")
        .arg("--nocapture")
        .env(TEST_CASE, case)
        .env(TEST_ROOT, root)
        .env("HOME", root)
        .env("CLAW_POSTHOG_KEY", "test-posthog-key")
        .env("CLAW_POSTHOG_HOST", host)
        .env("CLAW_ANALYTICS_ENABLED", analytics_enabled)
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "telemetry child {case} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

async fn run_child_case(case: &str) -> anyhow::Result<()> {
    let root = env::var_os(TEST_ROOT)
        .map(std::path::PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("missing telemetry test root"))?;
    match case {
        "roundtrip" => roundtrip_case(&root).await,
        "gate-off" => gate_off_case(&root, ServerMode::Desktop).await,
        "hosted" => gate_off_case(&root, ServerMode::Hosted).await,
        _ => anyhow::bail!("unknown telemetry test case {case}"),
    }
}

async fn roundtrip_case(root: &Path) -> anyhow::Result<()> {
    let (router, _) = test_router(root, ServerMode::Desktop).await?;
    let (status, initial) =
        request_json(&router, "GET", "/api/v1/settings/telemetry", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(initial["enabled"], true);
    assert_eq!(initial["consent"], true);
    let distinct_id = initial["distinctId"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing distinctId"))?;
    Uuid::parse_str(distinct_id)?;

    let (status, _) = request_json(
        &router,
        "PUT",
        "/api/v1/settings/telemetry",
        Some(json!({ "consent": "yes" })),
    )
    .await?;
    assert!(status.is_client_error());

    let (status, disabled) = request_json(
        &router,
        "PUT",
        "/api/v1/settings/telemetry",
        Some(json!({ "consent": false })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(disabled["distinctId"], distinct_id);
    assert_eq!(disabled["enabled"], false);
    assert_eq!(disabled["consent"], false);
    drop(router);

    let analytics_path = root.join("analytics.json");
    let raw = std::fs::read_to_string(&analytics_path)?;
    assert!(raw.ends_with('\n'));
    let persisted: Value = serde_json::from_str(&raw)?;
    assert_eq!(persisted.as_object().map(serde_json::Map::len), Some(2));
    assert_eq!(persisted["enabled"], false);
    assert_eq!(persisted["distinctId"], distinct_id);
    assert!(!root.join("installation.json").exists());

    let (restarted, analytics) = test_router(root, ServerMode::Desktop).await?;
    let (status, after_restart) =
        request_json(&restarted, "GET", "/api/v1/settings/telemetry", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after_restart, disabled);
    let (status, enabled) = request_json(
        &restarted,
        "PUT",
        "/api/v1/settings/telemetry",
        Some(json!({ "consent": true })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(enabled["enabled"], true);
    assert_eq!(enabled["consent"], true);
    analytics.capture(SERVER_STARTED, json!({}));
    analytics.shutdown().await;
    Ok(())
}

async fn gate_off_case(root: &Path, mode: ServerMode) -> anyhow::Result<()> {
    let (router, analytics) = test_router(root, mode).await?;
    analytics.capture(SERVER_STARTED, json!({}));
    let (status, state) = request_json(&router, "GET", "/api/v1/settings/telemetry", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state["enabled"], false);
    assert_eq!(state["consent"], true);
    let persisted: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("analytics.json"))?)?;
    assert_eq!(state["distinctId"], persisted["distinctId"]);
    assert_eq!(
        persisted,
        json!({ "enabled": true, "distinctId": state["distinctId"] })
    );
    assert!(!root.join("installation.json").exists());
    let (status, unchanged) = request_json(
        &router,
        "PUT",
        "/api/v1/settings/telemetry",
        Some(json!({ "consent": true })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        unchanged, state,
        "consent cannot override deployment policy"
    );
    analytics.capture(SERVER_STARTED, json!({}));
    // Drain delivery before the parent checks its collector; no timing guess is needed.
    analytics.shutdown().await;
    Ok(())
}

async fn test_router(
    root: &Path,
    mode: ServerMode,
) -> anyhow::Result<(Router, Arc<AnalyticsService>)> {
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49337,
        proxy_port: None,
        resources_dir: root.join("resources"),
        browserclaw_dir: root.to_path_buf(),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
        native: Default::default(),
        mode,
        behavior: Default::default(),
        behavior_ignored: Vec::new(),
    });
    let state = AppState::new_with_home(config, root.join("home")).await?;
    let analytics = state.analytics.clone();
    Ok((build_router(state), analytics))
}

async fn request_json(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost");
    let request_body = if let Some(body) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = router.clone().oneshot(builder.body(request_body)?).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    Ok((status, value))
}
