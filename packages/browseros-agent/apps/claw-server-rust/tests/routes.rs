use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use browseros_core::TargetId;
use claw_server_rust::{
    AppState, build_router,
    config::Config,
    domain::{AgentId, AgentRef, Session, SessionId, TabGroupColor},
    services::tab_activity::RecordToolInput,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_tungstenite::{WebSocketStream, accept_async, tungstenite::Message};
use tower::ServiceExt;

struct TestApp {
    router: Router,
    state: AppState,
    _dir: TempDir,
    _browser_task: Option<JoinHandle<()>>,
}

async fn test_app() -> anyhow::Result<TestApp> {
    test_app_with_cdp_port(49337, false).await
}

async fn test_app_with_cdp_port(cdp_port: u16, start_browser: bool) -> anyhow::Result<TestApp> {
    test_app_with_options(cdp_port, start_browser, true).await
}

async fn test_app_with_options(
    cdp_port: u16,
    start_browser: bool,
    screencast_screenshot_fallback: bool,
) -> anyhow::Result<TestApp> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("browserclaw");
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port,
        proxy_port: None,
        resources_dir: dir.path().join("resources"),
        browserclaw_dir: root.clone(),
        claw_dir: root,
        session_idle: Duration::from_secs(300),
        session_sweep_interval: Duration::from_secs(60),
        screencast_screenshot_fallback,
        dev_mode: false,
        auth_token: None,
    });
    let state = AppState::new_with_home(config, None, dir.path().join("home")).await?;
    let browser_task = if start_browser {
        Some(state.browser.start())
    } else {
        None
    };
    Ok(TestApp {
        router: build_router(state.clone()),
        state,
        _dir: dir,
        _browser_task: browser_task,
    })
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
        serde_json::from_slice(&bytes)?
    };
    Ok((status, value))
}

async fn request_json_with_headers(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> anyhow::Result<(StatusCode, HeaderMap, Value)> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost");
    let mut has_session_header = false;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("mcp-session-id") {
            has_session_header = true;
        }
        builder = builder.header(*name, *value);
    }
    if has_session_header {
        builder = builder.header("mcp-protocol-version", "2025-06-18");
    }
    let request_body = if let Some(body) = body {
        builder = builder
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = router.clone().oneshot(builder.body(request_body)?).await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let value = response_body_value(&headers, &bytes)?;
    Ok((status, headers, value))
}

async fn request_status(router: &Router, method: &str, uri: &str) -> anyhow::Result<StatusCode> {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost")
        .body(Body::empty())?;
    Ok(router.clone().oneshot(request).await?.status())
}

fn response_body_value(headers: &HeaderMap, bytes: &[u8]) -> anyhow::Result<Value> {
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    let body = std::str::from_utf8(bytes)?;
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"))
    {
        return sse_json(body);
    }
    serde_json::from_str(body).or_else(|_| Ok(Value::String(body.to_string())))
}

fn sse_json(body: &str) -> anyhow::Result<Value> {
    for line in body.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if !data.is_empty() {
                return Ok(serde_json::from_str(data)?);
            }
        }
    }
    Err(anyhow::anyhow!("SSE response had no JSON data: {body:?}"))
}

#[tokio::test]
async fn health_survives_cdp_down() -> anyhow::Result<()> {
    let app = test_app().await?;
    let (status, body) = request_json(&app.router, "GET", "/system/health", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["cdp"]["connected"], false);
    Ok(())
}

#[tokio::test]
async fn audit_empty_and_replay_gone() -> anyhow::Result<()> {
    let app = test_app().await?;
    let (status, dispatches) = request_json(&app.router, "GET", "/audit/dispatches", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        dispatches["rows"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("rows not array"))?
            .len(),
        0
    );
    assert!(dispatches["nextCursor"].is_null());

    let (status, body) = request_json(
        &app.router,
        "POST",
        "/audit/replay/missing/events",
        Some(json!({ "type": 3 })),
    )
    .await?;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(body["error"], "session not live");
    Ok(())
}

#[tokio::test]
async fn replay_tabs_tracks_only_live_agent_sessions() -> anyhow::Result<()> {
    let app = test_app().await?;
    app.state
        .tab_activity
        .record_tool(RecordToolInput {
            target_id: TargetId::from("target-live".to_string()),
            page_id: 7,
            url: "https://example.com/live".to_string(),
            title: "Live Tab".to_string(),
            agent_id: "agent-live".to_string(),
            slug: "codex".to_string(),
            tool_name: "tabs".to_string(),
        })
        .await;

    let (status, body) = request_json(&app.router, "GET", "/replay/tabs", None).await?;
    assert_eq!(status, StatusCode::OK, "initialize body: {body:?}");
    assert_eq!(body, json!({ "tabs": [] }));

    let session_id = SessionId::new("session-live");
    let session = test_session(session_id.clone(), "agent-live", "codex");
    app.state.sessions.insert_for_testing(session.clone()).await;

    let (status, body) = request_json(&app.router, "GET", "/replay/tabs", None).await?;
    assert_eq!(status, StatusCode::OK);
    let rows = body["tabs"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("tabs not array"))?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["sessionId"], "session-live");
    assert_eq!(rows[0]["tabPageId"], 7);
    assert_eq!(rows[0]["url"], "https://example.com/live");
    assert_eq!(rows[0]["title"], "Live Tab");
    assert!(rows[0]["groupColor"].is_null());

    app.state
        .sessions
        .ownership()
        .set_tab_group(
            session.agent().ownership_key(),
            Some("group-live".to_string()),
            Some(TabGroupColor::Purple),
        )
        .await;
    let (status, body) = request_json(&app.router, "GET", "/replay/tabs", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tabs"][0]["groupColor"], "purple");

    assert!(
        app.state
            .sessions
            .remove(&session_id, "closed", Some("test close"))
            .await?
    );
    let (status, body) = request_json(&app.router, "GET", "/replay/tabs", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "tabs": [] }));
    Ok(())
}

#[tokio::test]
async fn replay_tabs_keeps_first_live_session_per_agent_id() -> anyhow::Result<()> {
    let app = test_app().await?;
    app.state
        .tab_activity
        .record_tool(RecordToolInput {
            target_id: TargetId::from("target-duplicate".to_string()),
            page_id: 8,
            url: "https://example.com/duplicate".to_string(),
            title: "Duplicate".to_string(),
            agent_id: "agent-duplicate".to_string(),
            slug: "codex".to_string(),
            tool_name: "tabs".to_string(),
        })
        .await;
    app.state
        .sessions
        .insert_for_testing(test_session(
            SessionId::new("session-a"),
            "agent-duplicate",
            "codex",
        ))
        .await;
    app.state
        .sessions
        .insert_for_testing(test_session(
            SessionId::new("session-b"),
            "agent-duplicate",
            "codex",
        ))
        .await;

    let (status, body) = request_json(&app.router, "GET", "/replay/tabs", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tabs"][0]["sessionId"], "session-a");
    Ok(())
}

#[tokio::test]
async fn cancel_with_no_active_dispatches_returns_ts_shaped_404() -> anyhow::Result<()> {
    let app = test_app().await?;
    let (status, body) =
        request_json(&app.router, "POST", "/agents/idle-agent/cancel", None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        json!({
            "ok": false,
            "cancelled": 0,
            "reason": "no active dispatches for this agent"
        })
    );
    Ok(())
}

#[tokio::test]
async fn mcp_hygiene_rejects_browser_originated_requests() -> anyhow::Result<()> {
    let app = test_app().await?;

    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})),
        &[("origin", "http://evil.example")],
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, json!({ "error": "unsupported request" }));

    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "GET",
        "/mcp",
        None,
        &[("sec-fetch-site", "cross-site")],
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, json!({ "error": "unsupported request" }));

    // Hygiene applies to /mcp only: the same origin header is fine elsewhere.
    let (status, _headers, _body) =
        request_json_with_headers(&app.router, "GET", "/system/health", None, &[(
            "origin",
            "http://evil.example",
        )])
        .await?;
    assert_eq!(status, StatusCode::OK);

    // CORS preflight stays 204 like every other route (TS cors layer
    // answers OPTIONS before hygiene runs).
    let (status, _headers, _body) = request_json_with_headers(
        &app.router,
        "OPTIONS",
        "/mcp",
        None,
        &[("origin", "http://evil.example")],
    )
    .await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    Ok(())
}

#[tokio::test]
async fn mcp_hygiene_rejects_non_json_writes() -> anyhow::Result<()> {
    let app = test_app().await?;
    let request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from("hello"))?;
    let response = app.router.clone().oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let body: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(body, json!({ "error": "unsupported content type" }));
    Ok(())
}

#[tokio::test]
async fn mcp_initialize_list_guard_audit_and_delete() -> anyhow::Result<()> {
    let app = test_app().await?;
    let session_id = initialize_mcp(&app).await?;

    let list = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });
    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(list),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["result"]["tools"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("tools not array"))?
            .len(),
        16
    );

    let blocked = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "navigate",
            "arguments": { "page": 1, "url": "javascript:alert(1)" }
        }
    });
    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(blocked),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], true, "navigate body: {body:?}");
    assert!(
        body["result"]["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing text"))?
            .contains("javascript")
    );

    let (status, dispatches) = request_json(&app.router, "GET", "/audit/dispatches", None).await?;
    assert_eq!(status, StatusCode::OK);
    let rows = dispatches["rows"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("rows not array"))?;
    assert_eq!(rows.len(), 1);
    assert!(rows[0]["dispatchId"].as_str().is_some());

    let (status, _headers, _body) = request_json_with_headers(
        &app.router,
        "DELETE",
        "/mcp",
        None,
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, _headers, stale) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(json!({"jsonrpc":"2.0","id":4,"method":"tools/list","params":{}})),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        stale
            .as_str()
            .is_some_and(|body| body.contains("Session not found"))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_initialize_with_elicitation_capability_stays_nonblocking() -> anyhow::Result<()> {
    let app = test_app().await?;
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": { "elicitation": {} },
            "clientInfo": { "name": "Claude Code", "version": "1.0" }
        }
    });
    let (status, headers, body) =
        request_json_with_headers(&app.router, "POST", "/mcp", Some(initialize), &[]).await?;
    assert_eq!(status, StatusCode::OK, "initialize body: {body:?}");
    let session_id = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("missing mcp-session-id"))?
        .to_string();
    send_initialized(&app.router, &session_id).await?;

    // The naming task fires in the background; the handshake and tool
    // surface must stay responsive regardless of the pending elicitation.
    let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} });
    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(list),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert!(body["result"]["tools"].as_array().is_some_and(|tools| !tools.is_empty()));

    wait_for_session_registration(&app, &session_id).await?;
    let session = app
        .state
        .sessions
        .lookup(&SessionId::new(session_id.clone()))
        .await
        .ok_or_else(|| anyhow::anyhow!("session not minted"))?;
    assert_eq!(session.session_label().await, None);
    Ok(())
}

#[tokio::test]
async fn mcp_tabs_new_roundtrips_through_mock_cdp() -> anyhow::Result<()> {
    let mock = MockCdp::start().await?;
    let app = test_app_with_cdp_port(mock.cdp_port, false).await?;
    app.state.browser.connect_once_for_testing().await?;
    wait_for_cdp_connected(&app.router).await?;
    let session_id = initialize_mcp(&app).await?;

    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(tabs_new_request(10)),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], false, "tabs_new body: {body:?}");
    assert_eq!(body["result"]["structuredContent"]["page"], 1);

    let tabs_list = json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "tools/call",
        "params": { "name": "tabs", "arguments": { "action": "list" } }
    });
    let (status, _headers, list_body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(tabs_list),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list_body["result"]["isError"], false);
    assert_eq!(
        list_body["result"]["structuredContent"]["pages"][0]["page"],
        1
    );

    let (status, dispatches) = request_json(&app.router, "GET", "/audit/dispatches", None).await?;
    assert_eq!(status, StatusCode::OK);
    let rows = dispatches["rows"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("rows not array"))?;
    assert!(
        rows.iter()
            .any(|row| row["toolName"] == "tabs" && row["dispatchId"].is_string())
    );

    // Fallback screenshots default on: the tabs-new dispatch persisted a frame.
    let mut screenshot_statuses = Vec::new();
    for row in rows {
        let dispatch_id = row["dispatchId"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("dispatchId not string"))?;
        screenshot_statuses.push(
            request_status(
                &app.router,
                "GET",
                &format!("/audit/screenshot/{dispatch_id}"),
            )
            .await?,
        );
    }
    assert!(
        screenshot_statuses.contains(&StatusCode::OK),
        "no dispatch had a persisted screenshot: {screenshot_statuses:?}"
    );
    drop(mock);
    Ok(())
}

#[tokio::test]
async fn screencast_fallback_flag_disables_fallback_screenshots() -> anyhow::Result<()> {
    let mock = MockCdp::start().await?;
    let app = test_app_with_options(mock.cdp_port, false, false).await?;
    app.state.browser.connect_once_for_testing().await?;
    wait_for_cdp_connected(&app.router).await?;
    let session_id = initialize_mcp(&app).await?;

    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(tabs_new_request(30)),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], false, "tabs_new body: {body:?}");

    let (status, dispatches) = request_json(&app.router, "GET", "/audit/dispatches", None).await?;
    assert_eq!(status, StatusCode::OK);
    let rows = dispatches["rows"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("rows not array"))?;
    assert_eq!(rows.len(), 1);
    let dispatch_id = rows[0]["dispatchId"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("dispatchId not string"))?;
    let status = request_status(
        &app.router,
        "GET",
        &format!("/audit/screenshot/{dispatch_id}"),
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    drop(mock);
    Ok(())
}

#[tokio::test]
async fn cancel_endpoint_aborts_in_flight_dispatch() -> anyhow::Result<()> {
    let mock = MockCdp::start().await?;
    let app = test_app_with_cdp_port(mock.cdp_port, false).await?;
    app.state.browser.connect_once_for_testing().await?;
    wait_for_cdp_connected(&app.router).await?;
    let session_id = initialize_mcp(&app).await?;
    let session = app
        .state
        .sessions
        .lookup(&SessionId::new(session_id.clone()))
        .await
        .ok_or_else(|| anyhow::anyhow!("missing test session"))?;
    let agent_id = session.agent().agent_id().as_str().to_string();

    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(tabs_new_request(20)),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["structuredContent"]["page"], 1);

    let wait_call = json!({
        "jsonrpc": "2.0",
        "id": 21,
        "method": "tools/call",
        "params": {
            "name": "wait",
            "arguments": { "page": 1, "for": "time", "value": 60000 }
        }
    });
    let router = app.router.clone();
    let wait_session_id = session_id.clone();
    let wait_task = tokio::spawn(async move {
        request_json_with_headers(
            &router,
            "POST",
            "/mcp",
            Some(wait_call),
            &[("mcp-session-id", &wait_session_id)],
        )
        .await
    });

    let mut cancelled = None;
    for _ in 0..50 {
        let (status, body) = request_json(
            &app.router,
            "POST",
            &format!("/agents/{agent_id}/cancel"),
            None,
        )
        .await?;
        if status == StatusCode::OK {
            cancelled = Some(body);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let body = cancelled.ok_or_else(|| anyhow::anyhow!("cancel route never observed dispatch"))?;
    assert_eq!(body["cancelled"], 1);

    let (status, _headers, wait_body) = wait_task.await??;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(wait_body["result"]["isError"], true);
    assert_eq!(
        wait_body["result"]["structuredContent"]["cancellationKind"],
        "cockpit.operator-cancelled"
    );
    drop(mock);
    Ok(())
}

#[tokio::test]
async fn tabs_activity_enriches_by_agent_id_only() -> anyhow::Result<()> {
    let app = test_app().await?;
    let agents_dir = app.state.config.claw_dir.join("agents");
    tokio::fs::create_dir_all(&agents_dir).await?;
    tokio::fs::write(
        agents_dir.join("stored-agent.json"),
        json!({
            "id": "stored-agent",
            "name": "Stored Agent",
            "harness": "Codex",
            "loginMode": "profile",
            "selectedSites": [],
            "approvals": {},
            "aclRuleIds": [],
            "customAclRules": [],
            "slug": "mcp",
            "mcpUrl": "http://127.0.0.1:9200/mcp",
            "status": "configured",
            "createdAt": "now",
            "updatedAt": "now"
        })
        .to_string(),
    )
    .await?;

    app.state
        .tab_activity
        .record_tool(RecordToolInput {
            target_id: TargetId::from("target-exact".to_string()),
            page_id: 1,
            url: "https://example.com/exact".to_string(),
            title: "Exact".to_string(),
            agent_id: "stored-agent".to_string(),
            slug: "mcp".to_string(),
            tool_name: "tabs".to_string(),
        })
        .await;
    app.state
        .tab_activity
        .record_tool(RecordToolInput {
            target_id: TargetId::from("target-fallback".to_string()),
            page_id: 2,
            url: "https://example.com/fallback".to_string(),
            title: "Fallback".to_string(),
            agent_id: "ephemeral-agent".to_string(),
            slug: "mcp".to_string(),
            tool_name: "tabs".to_string(),
        })
        .await;

    let (status, body) = request_json(&app.router, "GET", "/tabs/activity", None).await?;
    assert_eq!(status, StatusCode::OK);
    let rows = body["tabs"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("tabs not array"))?;
    let exact = rows
        .iter()
        .find(|row| row["targetId"] == "target-exact")
        .ok_or_else(|| anyhow::anyhow!("missing exact tab"))?;
    assert_eq!(exact["agentLabel"], "Stored Agent");
    assert_eq!(exact["harness"], "Codex");

    let fallback = rows
        .iter()
        .find(|row| row["targetId"] == "target-fallback")
        .ok_or_else(|| anyhow::anyhow!("missing fallback tab"))?;
    assert_eq!(fallback["agentLabel"], "mcp");
    assert!(fallback["harness"].is_null());
    Ok(())
}

#[tokio::test]
async fn tabs_activity_serves_pushed_screencast_frames_with_acks() -> anyhow::Result<()> {
    let mock = MockCdp::start().await?;
    mock.add_tab(1, "target-1", 1).await;
    mock.add_tab(2, "target-2", 2).await;
    let app = test_app_with_cdp_port(mock.cdp_port, false).await?;
    app.state.browser.connect_once_for_testing().await?;
    wait_for_cdp_connected(&app.router).await?;

    for (page_id, target_id, agent_id) in [(1, "target-1", "agent-a"), (2, "target-2", "agent-b")] {
        app.state
            .tab_activity
            .record_tool(RecordToolInput {
                target_id: TargetId::from(target_id.to_string()),
                page_id,
                url: format!("https://example.com/{target_id}"),
                title: target_id.to_string(),
                agent_id: agent_id.to_string(),
                slug: "codex".to_string(),
                tool_name: "tabs".to_string(),
            })
            .await;
    }

    let screencast_task = app
        .state
        .screencast
        .clone()
        .start(app.state.browser.clone(), app.state.tab_activity.clone());

    // Frame 2 for each target is only released by the mock after the server
    // acks frame 1 with the right integer sessionId on the right target
    // session — seeing cast2 frames proves the full route→store→ack loop
    // for two concurrent screencasts.
    let mut last_frames: Vec<(String, Option<Value>)> = Vec::new();
    let mut done = false;
    for _ in 0..100 {
        let (status, body) = request_json(&app.router, "GET", "/tabs/activity", None).await?;
        assert_eq!(status, StatusCode::OK);
        let rows = body["tabs"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("tabs not array"))?;
        last_frames = rows
            .iter()
            .map(|row| {
                (
                    row["targetId"].as_str().unwrap_or_default().to_string(),
                    (!row["screencast"].is_null()).then(|| row["screencast"].clone()),
                )
            })
            .collect();
        let frame_data = |target: &str| {
            last_frames
                .iter()
                .find(|(target_id, _)| target_id == target)
                .and_then(|(_, frame)| frame.as_ref())
                .and_then(|frame| frame["jpegBase64"].as_str())
                .map(str::to_string)
        };
        if frame_data("target-1").as_deref() == Some("cast2-target-1")
            && frame_data("target-2").as_deref() == Some("cast2-target-2")
        {
            done = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        done,
        "pushed frames never reached /tabs/activity; last: {last_frames:?}"
    );

    for (_, frame) in &last_frames {
        let frame = frame
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing screencast frame"))?;
        assert!(frame["capturedAt"].as_i64().is_some_and(|at| at > 0));
        let data = frame["jpegBase64"].as_str().unwrap_or_default();
        assert!(
            data.starts_with("cast2-"),
            "unexpected frame payload: {data}"
        );
    }

    let acks = mock.acks.lock().await.clone();
    assert!(
        acks.contains(&("session-target-1".to_string(), 1001)),
        "missing ack for target-1 frame 1: {acks:?}"
    );
    assert!(
        acks.contains(&("session-target-2".to_string(), 2001)),
        "missing ack for target-2 frame 1: {acks:?}"
    );
    // The frame pushed on the foreign envelope session must never be
    // stored or acked (its ack ids end in 9).
    assert!(
        acks.iter().all(|(_, ack_id)| ack_id % 1000 != 9),
        "foreign-session frame was acked: {acks:?}"
    );

    screencast_task.abort();
    drop(mock);
    Ok(())
}

fn test_session(session_id: SessionId, agent_id: &str, slug: &str) -> Arc<Session> {
    Session::new(
        session_id,
        AgentRef::Ephemeral {
            agent_id: AgentId::new(agent_id),
            slug: slug.to_string(),
            label: slug.to_string(),
        },
        tokio::time::Instant::now(),
    )
}

async fn initialize_mcp(app: &TestApp) -> anyhow::Result<String> {
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
    let (status, headers, body) =
        request_json_with_headers(&app.router, "POST", "/mcp", Some(initialize), &[]).await?;
    assert_eq!(status, StatusCode::OK, "initialize body: {body:?}");
    assert_eq!(
        body["result"]["serverInfo"]["name"],
        "browseros-claw-server"
    );
    let session_id = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing mcp-session-id"))?;
    send_initialized(&app.router, &session_id).await?;
    wait_for_session_registration(app, &session_id).await?;
    Ok(session_id)
}

fn tabs_new_request(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "tabs",
            "arguments": { "action": "new", "url": "https://example.com" }
        }
    })
}

/// The initialized notification is acknowledged with 202 before the
/// session_started hook finishes registering the session, so a tools/call
/// fired immediately after can race the registry. Tests wait it out.
async fn wait_for_session_registration(app: &TestApp, session_id: &str) -> anyhow::Result<()> {
    for _ in 0..50 {
        if app
            .state
            .sessions
            .contains(&SessionId::new(session_id.to_string()))
            .await
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    anyhow::bail!("initialized notification did not register session {session_id}");
}

async fn send_initialized(router: &Router, session_id: &str) -> anyhow::Result<()> {
    let (status, _headers, _body) = request_json_with_headers(
        router,
        "POST",
        "/mcp",
        Some(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        })),
        &[("mcp-session-id", session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::ACCEPTED);
    Ok(())
}

async fn wait_for_cdp_connected(router: &Router) -> anyhow::Result<()> {
    let mut last = Value::Null;
    for _ in 0..120 {
        let (status, body) = request_json(router, "GET", "/system/health", None).await?;
        if status == StatusCode::OK && body["cdp"]["connected"] == true {
            return Ok(());
        }
        last = body;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(anyhow::anyhow!(
        "mock CDP did not connect; last health: {last}"
    ))
}

struct MockCdp {
    cdp_port: u16,
    tabs: Arc<tokio::sync::Mutex<Vec<MockTab>>>,
    acks: Arc<tokio::sync::Mutex<Vec<(String, i64)>>>,
    tasks: Vec<JoinHandle<()>>,
}

impl MockCdp {
    async fn start() -> anyhow::Result<Self> {
        let tabs = Arc::new(tokio::sync::Mutex::new(Vec::<MockTab>::new()));
        let acks = Arc::new(tokio::sync::Mutex::new(Vec::<(String, i64)>::new()));
        let ws_listener = TcpListener::bind("127.0.0.1:0").await?;
        let ws_addr = ws_listener.local_addr()?;
        let ws_tabs = tabs.clone();
        let ws_acks = acks.clone();
        let ws_task = tokio::spawn(async move {
            loop {
                let Ok((stream, _addr)) = ws_listener.accept().await else {
                    break;
                };
                let tabs = ws_tabs.clone();
                let acks = ws_acks.clone();
                tokio::spawn(async move {
                    let _ = handle_mock_ws(stream, tabs, acks).await;
                });
            }
        });

        let http_listener = TcpListener::bind("127.0.0.1:0").await?;
        let cdp_port = http_listener.local_addr()?.port();
        let http_task = tokio::spawn(async move {
            loop {
                let Ok((stream, _addr)) = http_listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let _ = handle_mock_http(stream, ws_addr.port()).await;
                });
            }
        });

        Ok(Self {
            cdp_port,
            tabs,
            acks,
            tasks: vec![ws_task, http_task],
        })
    }

    async fn add_tab(&self, tab_id: i64, target_id: &str, window_id: i64) {
        self.tabs.lock().await.push(MockTab {
            tab_id,
            target_id: target_id.to_string(),
            url: format!("https://example.com/{target_id}"),
            title: format!("Tab {tab_id}"),
            group_id: None,
            window_id,
        });
    }
}

impl Drop for MockCdp {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[derive(Clone)]
struct MockTab {
    tab_id: i64,
    target_id: String,
    url: String,
    title: String,
    group_id: Option<String>,
    window_id: i64,
}

async fn handle_mock_http(mut stream: TcpStream, ws_port: u16) -> anyhow::Result<()> {
    let mut buffer = [0_u8; 1024];
    let read = stream.read(&mut buffer).await?;
    let request = std::str::from_utf8(&buffer[..read]).unwrap_or_default();
    let (status, body) = if request.starts_with("GET /json/version ") {
        (
            "200 OK",
            json!({
                "Browser": "BrowserOS Mock",
                "Protocol-Version": "1.3",
                "webSocketDebuggerUrl": format!("ws://127.0.0.1:{ws_port}/devtools/browser/mock")
            })
            .to_string(),
        )
    } else {
        ("404 Not Found", json!({ "error": "not found" }).to_string())
    };
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

async fn handle_mock_ws(
    stream: TcpStream,
    tabs: Arc<tokio::sync::Mutex<Vec<MockTab>>>,
    acks: Arc<tokio::sync::Mutex<Vec<(String, i64)>>>,
) -> anyhow::Result<()> {
    let mut ws = accept_async(stream).await?;
    while let Some(message) = ws.next().await {
        let message = message?;
        let text = match message {
            Message::Text(text) => text.to_string(),
            Message::Binary(bytes) => String::from_utf8(bytes.to_vec()).unwrap_or_default(),
            Message::Close(_) => return Ok(()),
            Message::Ping(bytes) => {
                ws.send(Message::Pong(bytes)).await?;
                continue;
            }
            Message::Pong(_) | Message::Frame(_) => continue,
        };
        let request: Value = serde_json::from_str(&text)?;
        let id = request
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("missing CDP id"))?;
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing CDP method"))?;
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let session = request
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let result = handle_mock_cdp_method(method, params.clone(), tabs.clone()).await;
        let response = match result {
            Ok(result) => json!({ "id": id, "result": result }),
            Err(message) => json!({ "id": id, "error": { "code": -32000, "message": message } }),
        };
        ws.send(Message::Text(response.to_string().into())).await?;
        match method {
            // A real browser pushes frames only after startScreencast; ours
            // pushes one frame on a foreign session (must be dropped by the
            // server) and then frame 1 on the requesting session.
            "Page.startScreencast" => {
                if let Some(session) = &session
                    && let Some(tab) = find_tab_for_session(&tabs, session).await
                {
                    let base = tab.tab_id * 1000;
                    send_screencast_frame(&mut ws, "session-bogus", "foreign", base + 9).await?;
                    send_screencast_frame(
                        &mut ws,
                        session,
                        &format!("cast1-{}", tab.target_id),
                        base + 1,
                    )
                    .await?;
                }
            }
            // Frame 2 is released only by a correct ack of frame 1 — the CDP
            // backpressure contract the server must honor.
            "Page.screencastFrameAck" => {
                if let Some(session) = &session
                    && let Some(ack_id) = params.get("sessionId").and_then(Value::as_i64)
                {
                    acks.lock().await.push((session.clone(), ack_id));
                    if ack_id % 1000 == 1
                        && let Some(tab) = find_tab_for_session(&tabs, session).await
                    {
                        send_screencast_frame(
                            &mut ws,
                            session,
                            &format!("cast2-{}", tab.target_id),
                            tab.tab_id * 1000 + 2,
                        )
                        .await?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

async fn find_tab_for_session(
    tabs: &Arc<tokio::sync::Mutex<Vec<MockTab>>>,
    session: &str,
) -> Option<MockTab> {
    let target_id = session.strip_prefix("session-")?;
    tabs.lock()
        .await
        .iter()
        .find(|tab| tab.target_id == target_id)
        .cloned()
}

async fn send_screencast_frame(
    ws: &mut WebSocketStream<TcpStream>,
    session: &str,
    data: &str,
    ack_id: i64,
) -> anyhow::Result<()> {
    let event = json!({
        "method": "Page.screencastFrame",
        "params": {
            "data": data,
            "metadata": {
                "offsetTop": 0.0,
                "pageScaleFactor": 1.0,
                "deviceWidth": 1280.0,
                "deviceHeight": 800.0,
                "scrollOffsetX": 0.0,
                "scrollOffsetY": 0.0
            },
            "sessionId": ack_id
        },
        "sessionId": session
    });
    ws.send(Message::Text(event.to_string().into())).await?;
    Ok(())
}

async fn handle_mock_cdp_method(
    method: &str,
    params: Value,
    tabs: Arc<tokio::sync::Mutex<Vec<MockTab>>>,
) -> Result<Value, String> {
    match method {
        "Browser.getVersion" => Ok(json!({ "product": "BrowserOS Mock" })),
        "Browser.getTabs" => {
            let tabs = tabs.lock().await;
            Ok(json!({ "tabs": tabs.iter().map(tab_json).collect::<Vec<_>>() }))
        }
        "Browser.createTab" => {
            let mut tabs = tabs.lock().await;
            let tab_id = i64::try_from(tabs.len()).unwrap_or(0) + 1;
            let tab = MockTab {
                tab_id,
                target_id: format!("target-{tab_id}"),
                url: params
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("about:blank")
                    .to_string(),
                title: format!("Tab {tab_id}"),
                group_id: None,
                window_id: 1,
            };
            tabs.push(tab.clone());
            Ok(json!({ "tab": tab_json(&tab) }))
        }
        "Browser.getActiveTab" => {
            let tabs = tabs.lock().await;
            let window_id = params.get("windowId").and_then(Value::as_i64);
            let tab = tabs
                .iter()
                .find(|tab| Some(tab.window_id) == window_id)
                .cloned();
            match tab {
                Some(tab) => Ok(json!({ "tab": tab_json(&tab) })),
                None => Ok(json!({})),
            }
        }
        "Browser.getTabInfo" => {
            let tabs = tabs.lock().await;
            let tab_id = params.get("tabId").and_then(Value::as_i64);
            let target_id = params.get("targetId").and_then(Value::as_str);
            let tab = tabs
                .iter()
                .find(|tab| Some(tab.tab_id) == tab_id || Some(tab.target_id.as_str()) == target_id)
                .cloned()
                .ok_or_else(|| "tab not found".to_string())?;
            Ok(json!({ "tab": tab_json(&tab) }))
        }
        "Target.attachToTarget" => {
            let target_id = params
                .get("targetId")
                .and_then(Value::as_str)
                .unwrap_or("target");
            Ok(json!({ "sessionId": format!("session-{target_id}") }))
        }
        "Page.enable"
        | "DOM.enable"
        | "Runtime.enable"
        | "Accessibility.enable"
        | "Runtime.runIfWaitingForDebugger"
        | "Target.setAutoAttach" => Ok(json!({})),
        "Page.captureScreenshot" => Ok(json!({ "data": "anBlZw==" })),
        "Browser.createTabGroup" => {
            let mut tabs = tabs.lock().await;
            let tab_ids = params
                .get("tabIds")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for id in &tab_ids {
                if let Some(tab_id) = id.as_i64()
                    && let Some(tab) = tabs.iter_mut().find(|tab| tab.tab_id == tab_id)
                {
                    tab.group_id = Some("group-1".to_string());
                }
            }
            Ok(json!({
                "group": {
                    "groupId": "group-1",
                    "windowId": 1,
                    "title": params.get("title").and_then(Value::as_str).unwrap_or("group"),
                    "color": "blue",
                    "collapsed": false,
                    "tabIds": tab_ids
                }
            }))
        }
        "Browser.addTabsToGroup" => Ok(json!({
            "group": {
                "groupId": params.get("groupId").and_then(Value::as_str).unwrap_or("group-1"),
                "windowId": 1,
                "title": "group",
                "color": "blue",
                "collapsed": false,
                "tabIds": params.get("tabIds").cloned().unwrap_or_else(|| json!([]))
            }
        })),
        _ => Ok(json!({})),
    }
}

fn tab_json(tab: &MockTab) -> Value {
    let mut value = json!({
        "tabId": tab.tab_id,
        "targetId": tab.target_id,
        "url": tab.url,
        "title": tab.title,
        "isActive": true,
        "isLoading": false,
        "loadProgress": 1.0,
        "isPinned": false,
        "isHidden": false,
        "windowId": tab.window_id,
        "index": tab.tab_id - 1
    });
    if let (Value::Object(object), Some(group_id)) = (&mut value, &tab.group_id) {
        object.insert("groupId".to_string(), json!(group_id));
    }
    value
}
