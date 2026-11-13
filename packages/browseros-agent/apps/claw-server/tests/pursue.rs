//! pursue through the MCP surface: who is offered it, and what a call that cannot run answers.
//! The decision loop itself is covered in browseros-decide with a fake browser; these drive the
//! server the way a connected agent does, with the billing gateway and the decisions worker
//! faked at their HTTP boundaries.

#[path = "support/fake_decisions.rs"]
mod fake_decisions;
#[path = "support/fake_gateway.rs"]
mod fake_gateway;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use claw_server::{
    AppState, build_router,
    config::{Behavior, Config, ServerMode},
    ids::SessionId,
    services::{
        billing::{Billing, Secret},
        decisions::Decisions,
    },
};
use fake_decisions::FakeDecisions;
use fake_gateway::{FakeGateway, KEY};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

struct TestApp {
    router: Router,
    state: AppState,
    worker: FakeDecisions,
    _gateway: FakeGateway,
    _dir: TempDir,
}

/// How the computer under test stands.
#[derive(Clone, Copy)]
struct Setup {
    mode: ServerMode,
    ultrafast: bool,
    worker: bool,
}

const ULTRAFAST_DESKTOP: Setup = Setup {
    mode: ServerMode::Desktop,
    ultrafast: true,
    worker: true,
};

async fn test_app(setup: Setup) -> anyhow::Result<TestApp> {
    let dir = tempfile::tempdir()?;
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49339,
        proxy_port: None,
        resources_dir: dir.path().join("resources"),
        browserclaw_dir: dir.path().join("browserclaw"),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
        native: Default::default(),
        mode: setup.mode,
        behavior: Behavior::defaults(setup.mode),
        behavior_ignored: Vec::new(),
    });
    let mut state = AppState::new_with_home(config, dir.path().join("home")).await?;
    let gateway = FakeGateway::start().await?;
    gateway.set_plan(Some(FakeGateway::active_plan()));
    let billing = Arc::new(
        Billing::open(
            Some(gateway.config()?),
            dir.path().join("browserclaw"),
            CancellationToken::new(),
        )
        .await,
    );
    if setup.ultrafast {
        billing
            .activate(Secret::new(KEY.to_string()), None)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    }
    state.billing = billing;
    let worker = FakeDecisions::start().await?;
    state.decisions = Arc::new(Decisions::new(
        setup.worker.then(|| worker.url()).transpose()?,
    ));
    state.sync_ultrafast_with_plan().await;
    Ok(TestApp {
        router: build_router(state.clone()),
        state,
        worker,
        _gateway: gateway,
        _dir: dir,
    })
}

async fn post(
    router: &Router,
    body: Value,
    session: Option<&str>,
) -> anyhow::Result<(StatusCode, HeaderMap, Value)> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    if let Some(session) = session {
        builder = builder
            .header("mcp-session-id", session)
            .header("mcp-protocol-version", "2025-06-18");
    }
    let response = router
        .clone()
        .oneshot(builder.body(Body::from(body.to_string()))?)
        .await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let text = std::str::from_utf8(&bytes)?;
    let data = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim))
        .find(|data| !data.is_empty());
    let value = match data {
        Some(data) => serde_json::from_str(data)?,
        None if text.is_empty() => Value::Null,
        None => serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string())),
    };
    Ok((status, headers, value))
}

async fn connect(app: &TestApp) -> anyhow::Result<String> {
    Ok(connect_reading_instructions(app).await?.0)
}

/// Connects as an agent does, returning the session and the instructions the server sent.
async fn connect_reading_instructions(app: &TestApp) -> anyhow::Result<(String, String)> {
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "Codex", "version": "1.0" }
        }
    });
    let (status, headers, body) = post(&app.router, initialize, None).await?;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let instructions = body["result"]["instructions"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let session = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing mcp-session-id"))?;
    let (status, _, _) = post(
        &app.router,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        Some(&session),
    )
    .await?;
    assert_eq!(status, StatusCode::ACCEPTED);
    for _ in 0..50 {
        if app
            .state
            .sessions
            .contains(&SessionId::new(session.clone()))
            .await
        {
            return Ok((session, instructions));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    anyhow::bail!("the session never registered")
}

async fn listed(app: &TestApp, session: &str) -> anyhow::Result<Vec<String>> {
    let (_, _, body) = post(
        &app.router,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
        Some(session),
    )
    .await?;
    Ok(body["result"]["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("tools not an array: {body:?}"))?
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_string))
        .collect())
}

async fn call_pursue(app: &TestApp, session: &str) -> anyhow::Result<Value> {
    let (status, _, body) = post(
        &app.router,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "pursue",
                "arguments": { "page": 1, "goal": "add Sensodyne Fresh Mint 150g to the cart" }
            }
        }),
        Some(session),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    Ok(body)
}

#[tokio::test]
async fn an_ultrafast_desktop_computer_is_offered_pursue() -> anyhow::Result<()> {
    let app = test_app(ULTRAFAST_DESKTOP).await?;
    let session = connect(&app).await?;
    assert!(
        listed(&app, &session)
            .await?
            .contains(&"pursue".to_string())
    );
    Ok(())
}

#[tokio::test]
async fn pursue_is_not_offered_without_the_plan_a_worker_or_on_a_hosted_server()
-> anyhow::Result<()> {
    for setup in [
        Setup {
            ultrafast: false,
            ..ULTRAFAST_DESKTOP
        },
        Setup {
            worker: false,
            ..ULTRAFAST_DESKTOP
        },
        Setup {
            mode: ServerMode::Hosted,
            ..ULTRAFAST_DESKTOP
        },
    ] {
        let app = test_app(setup).await?;
        let session = connect(&app).await?;
        assert!(
            !listed(&app, &session)
                .await?
                .contains(&"pursue".to_string())
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_lapsed_plan_hands_back_to_run_without_asking_the_worker() -> anyhow::Result<()> {
    let app = test_app(Setup {
        ultrafast: false,
        ..ULTRAFAST_DESKTOP
    })
    .await?;
    let session = connect(&app).await?;

    let body = call_pursue(&app, &session).await?;

    assert_eq!(body["result"]["isError"], false, "{body:?}");
    let reply = &body["result"]["structuredContent"];
    assert_eq!(reply["outcome"], "handed_back");
    assert_eq!(reply["continueWith"], "run");
    assert!(
        reply["said"]
            .as_str()
            .unwrap_or_default()
            .contains("not on this plan")
    );
    assert!(app.worker.requests().is_empty());
    Ok(())
}

#[tokio::test]
async fn with_no_browser_connected_pursue_hands_back_before_asking_anything() -> anyhow::Result<()>
{
    let app = test_app(ULTRAFAST_DESKTOP).await?;
    let session = connect(&app).await?;

    let body = call_pursue(&app, &session).await?;

    assert_eq!(body["result"]["isError"], false, "{body:?}");
    let reply = &body["result"]["structuredContent"];
    assert_eq!(reply["outcome"], "handed_back");
    assert_eq!(reply["decisions"], 0);
    assert!(app.worker.requests().is_empty());
    Ok(())
}

#[tokio::test]
async fn a_hosted_server_refuses_pursue_by_name() -> anyhow::Result<()> {
    let app = test_app(Setup {
        mode: ServerMode::Hosted,
        ..ULTRAFAST_DESKTOP
    })
    .await?;
    let session = connect(&app).await?;

    let body = call_pursue(&app, &session).await?;

    assert_eq!(body["result"]["isError"], true, "{body:?}");
    assert!(
        body["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("Use run")
    );
    Ok(())
}

#[tokio::test]
async fn once_pursue_has_handed_a_task_back_it_does_not_take_it_again() -> anyhow::Result<()> {
    let app = test_app(ULTRAFAST_DESKTOP).await?;
    let session = connect(&app).await?;
    let other = connect(&app).await?;
    app.state
        .sessions
        .lookup(&SessionId::new(session.clone()))
        .await
        .ok_or_else(|| anyhow::anyhow!("no session"))?
        .mark_pursue_handed_back();

    let body = call_pursue(&app, &session).await?;
    let reply = &body["result"]["structuredContent"];
    assert_eq!(body["result"]["isError"], false, "{body:?}");
    assert_eq!(reply["outcome"], "handed_back");
    assert_eq!(reply["status"], "already_handed_back");
    assert_eq!(reply["continueWith"], "run");

    let elsewhere = call_pursue(&app, &other).await?;
    assert_ne!(
        elsewhere["result"]["structuredContent"]["status"], "already_handed_back",
        "another task still gets its own pursue"
    );
    assert!(app.worker.requests().is_empty());
    Ok(())
}

#[tokio::test]
async fn naming_a_new_task_offers_pursue_again() -> anyhow::Result<()> {
    let app = test_app(ULTRAFAST_DESKTOP).await?;
    let session = connect(&app).await?;
    let (status, _, body) = post(
        &app.router,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": { "name": "name_session", "arguments": { "name": "cart errand" } }
        }),
        Some(&session),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    app.state
        .sessions
        .lookup(&SessionId::new(session.clone()))
        .await
        .ok_or_else(|| anyhow::anyhow!("no session"))?
        .mark_pursue_handed_back();
    let locked = call_pursue(&app, &session).await?;
    assert_eq!(
        locked["result"]["structuredContent"]["status"],
        "already_handed_back"
    );

    let (status, _, body) = post(
        &app.router,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": { "name": "name_session", "arguments": { "name": "flight search" } }
        }),
        Some(&session),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let fresh = call_pursue(&app, &session).await?;
    assert_ne!(
        fresh["result"]["structuredContent"]["status"], "already_handed_back",
        "{fresh:?}"
    );
    Ok(())
}

#[tokio::test]
async fn agents_are_taught_pursue_only_when_it_is_offered() -> anyhow::Result<()> {
    let offered = test_app(ULTRAFAST_DESKTOP).await?;
    let (_, instructions) = connect_reading_instructions(&offered).await?;
    assert!(
        instructions.contains("Start each task that does something on a site with pursue"),
        "{instructions}"
    );
    assert!(
        !instructions.contains("Reach for run first"),
        "{instructions}"
    );

    let lapsed = test_app(Setup {
        ultrafast: false,
        ..ULTRAFAST_DESKTOP
    })
    .await?;
    let (_, instructions) = connect_reading_instructions(&lapsed).await?;
    assert!(!instructions.contains("pursue"));
    assert!(instructions.contains("Reach for run first"));
    Ok(())
}
