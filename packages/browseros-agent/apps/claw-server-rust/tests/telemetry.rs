use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use claw_server_rust::{AppState, build_router, config::Config};
use serde_json::{Value, json};
use std::{env, path::Path, process::Command, sync::Arc, time::Duration};
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
    run_child(root.path(), "roundtrip", "1")?;
    run_child(root.path(), "gate-off", "false")?;
    Ok(())
}

fn run_child(root: &Path, case: &str, analytics_enabled: &str) -> anyhow::Result<()> {
    let output = Command::new(env::current_exe()?)
        .arg("--exact")
        .arg("telemetry_routes_round_trip_persist_and_honor_the_env_gate")
        .arg("--nocapture")
        .env(TEST_CASE, case)
        .env(TEST_ROOT, root)
        .env("HOME", root)
        .env("CLAW_POSTHOG_KEY", "test-posthog-key")
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
        "gate-off" => gate_off_case(&root).await,
        _ => anyhow::bail!("unknown telemetry test case {case}"),
    }
}

async fn roundtrip_case(root: &Path) -> anyhow::Result<()> {
    let router = test_router(root).await?;
    let (status, initial) = request_json(&router, "GET", "/system/telemetry", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(initial["enabled"], true);
    assert_eq!(initial["consent"], true);
    let distinct_id = initial["distinctId"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing distinctId"))?;
    Uuid::parse_str(distinct_id)?;

    let (status, _) = request_json(
        &router,
        "POST",
        "/system/telemetry",
        Some(json!({ "consent": "yes" })),
    )
    .await?;
    assert!(status.is_client_error());

    let (status, disabled) = request_json(
        &router,
        "POST",
        "/system/telemetry",
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
    assert_eq!(persisted["distinctId"], distinct_id);
    assert_eq!(persisted["enabled"], false);

    let restarted = test_router(root).await?;
    let (status, after_restart) =
        request_json(&restarted, "GET", "/system/telemetry", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after_restart, disabled);
    let (status, enabled) = request_json(
        &restarted,
        "POST",
        "/system/telemetry",
        Some(json!({ "consent": true })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(enabled["enabled"], true);
    assert_eq!(enabled["consent"], true);
    Ok(())
}

async fn gate_off_case(root: &Path) -> anyhow::Result<()> {
    let router = test_router(root).await?;
    let (status, state) = request_json(&router, "GET", "/system/telemetry", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state["enabled"], false);
    assert_eq!(state["consent"], true);
    let persisted: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("analytics.json"))?)?;
    assert_eq!(state["distinctId"], persisted["distinctId"]);
    Ok(())
}

async fn test_router(root: &Path) -> anyhow::Result<Router> {
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49337,
        proxy_port: None,
        resources_dir: root.join("resources"),
        browserclaw_dir: root.to_path_buf(),
        claw_dir: root.to_path_buf(),
        session_idle: Duration::from_secs(300),
        session_sweep_interval: Duration::from_secs(60),
        screencast_screenshot_fallback: true,
        dev_mode: false,
        auth_token: None,
    });
    let state = AppState::new_with_home(config, None, root.join("home")).await?;
    Ok(build_router(state))
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
