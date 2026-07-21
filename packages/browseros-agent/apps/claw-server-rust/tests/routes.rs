use axum::{
    Router,
    body::{Body, BodyDataStream, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use browseros_core::{PageId, TargetId, screenshot::ScreenshotCaptureOptions};
use claw_server_rust::{
    AppState, build_router,
    config::Config,
    db::audit_log::{DispatchResultSummary, RecordToolDispatchInput},
    identity::{ClientIdentity, ConversationIdentity},
    ids::{ConvoId, DispatchId, ProfileId, SessionId},
    services::sessions::Session,
    services::{cockpit::RecordToolInput, sessions::PageOwnership},
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
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tower::ServiceExt;

#[path = "support/contract_fixtures.rs"]
mod contract_fixtures;

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
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("browserclaw");
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port,
        proxy_port: None,
        resources_dir: dir.path().join("resources"),
        browserclaw_dir: root,
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
        auth_token: None,
    });
    let state = AppState::new_with_home(config, dir.path().join("home")).await?;
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

struct McpSseStream {
    body: BodyDataStream,
    pending: Vec<u8>,
}

impl McpSseStream {
    /// Opens the session's standalone SSE channel for server-initiated MCP requests.
    async fn open(router: &Router, session_id: &str) -> anyhow::Result<Self> {
        let request = Request::builder()
            .method("GET")
            .uri("/mcp")
            .header(header::HOST, "localhost")
            .header(header::ACCEPT, "text/event-stream")
            .header("mcp-session-id", session_id)
            .header("mcp-protocol-version", "2025-06-18")
            .body(Body::empty())?;
        let response = router.clone().oneshot(request).await?;
        assert_eq!(response.status(), StatusCode::OK);
        Ok(Self {
            body: response.into_body().into_data_stream(),
            pending: Vec::new(),
        })
    }

    async fn next_method(&mut self, method: &str) -> anyhow::Result<Value> {
        loop {
            while let Some(event) = take_sse_event(&mut self.pending) {
                let event = std::str::from_utf8(&event)?;
                let data = event
                    .lines()
                    .filter_map(|line| line.strip_prefix("data:"))
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                if data.is_empty() {
                    continue;
                }
                let value: Value = serde_json::from_str(&data)?;
                if value.get("method").and_then(Value::as_str) == Some(method) {
                    return Ok(value);
                }
            }
            let chunk = self
                .body
                .next()
                .await
                .ok_or_else(|| anyhow::anyhow!("MCP SSE stream ended before {method}"))??;
            self.pending.extend_from_slice(&chunk);
        }
    }
}

fn take_sse_event(pending: &mut Vec<u8>) -> Option<Vec<u8>> {
    let delimiter = pending
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, 4))
        .or_else(|| {
            pending
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| (index, 2))
        })?;
    let event = pending[..delimiter.0].to_vec();
    pending.drain(..delimiter.0 + delimiter.1);
    Some(event)
}

#[tokio::test]
async fn health_survives_cdp_down() -> anyhow::Result<()> {
    let app = test_app().await?;
    let (status, body) = request_json(&app.router, "GET", "/system/health", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "status": "ok" }));
    Ok(())
}

#[tokio::test]
async fn system_shutdown_preserves_contract_body_and_defers_runtime_teardown() -> anyhow::Result<()>
{
    let app = test_app().await?;
    app.state
        .sessions
        .insert_for_testing(test_session(
            SessionId::new("shutdown-session"),
            "shutdown-agent",
            "shutdown-convo",
        ))
        .await;
    let request = Request::builder()
        .method("POST")
        .uri("/system/shutdown")
        .header(header::HOST, "localhost")
        .body(Body::empty())?;

    let response = app.router.clone().oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    assert_eq!(bytes.as_ref(), br#"{"status":"ok"}"#);
    tokio::time::timeout(Duration::from_secs(1), app.state.shutdown.requested()).await?;
    assert_eq!(app.state.sessions.count().await, 1);

    let retry = Request::builder()
        .method("POST")
        .uri("/system/shutdown")
        .header(header::HOST, "localhost")
        .body(Body::empty())?;
    let retry_response = app.router.clone().oneshot(retry).await?;
    let retry_bytes = to_bytes(retry_response.into_body(), usize::MAX).await?;
    assert_eq!(retry_bytes.as_ref(), br#"{"status":"ok"}"#);
    assert_eq!(app.state.sessions.shutdown().await?, 1);
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
    let (status, _headers, _body) = request_json_with_headers(
        &app.router,
        "GET",
        "/system/health",
        None,
        &[("origin", "http://evil.example")],
    )
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
        17
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

    let dispatches = app
        .state
        .audit_log
        .list_dispatches(Default::default())
        .await?;
    assert!(
        dispatches.rows.is_empty(),
        "guard rejections must skip audit effects"
    );

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
async fn mcp_name_session_lists_and_renames_while_disconnected() -> anyhow::Result<()> {
    let app = test_app().await?;
    let session_id = initialize_mcp(&app).await?;
    let headers = [("mcp-session-id", session_id.as_str())];

    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} })),
        &headers,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let tool = body["result"]["tools"]
        .as_array()
        .and_then(|tools| tools.iter().find(|tool| tool["name"] == "name_session"))
        .ok_or_else(|| anyhow::anyhow!("name_session missing"))?;
    assert_eq!(
        tool["description"],
        "Rename this browser session: a small lowercase 2-3 word label for what this session is doing, e.g. \"invoice processing\". Tabs are grouped as <client>/<name>. Call again to rename."
    );
    assert_eq!(
        tool["inputSchema"],
        json!({
            "type": "object",
            "properties": { "name": { "type": "string", "maxLength": 64 } },
            "required": ["name"]
        })
    );
    assert_eq!(
        tool["annotations"],
        json!({
            "title": "Name session",
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": true
        })
    );
    assert!(
        app.state
            .audit_log
            .list_tasks(Default::default())
            .await?
            .tasks
            .is_empty()
    );

    let session = app
        .state
        .sessions
        .lookup(&SessionId::new(session_id.clone()))
        .await
        .ok_or_else(|| anyhow::anyhow!("session not minted"))?;
    let generated = session.generated_label().to_string();
    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(name_session_request(3, "  Invoice Processing!!!  ")),
        &headers,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], false);
    assert_eq!(
        body["result"]["content"][0]["text"],
        format!("renamed to codex/invoice-processing (was codex/{generated})")
    );
    assert_eq!(session.label().await, "invoice-processing");
    let first_task = app
        .state
        .audit_log
        .get_task(session_id.as_str())
        .await?
        .ok_or_else(|| anyhow::anyhow!("name_session dispatch missing"))?;
    assert_eq!(first_task.summary.dispatch_count, 1);
    assert_eq!(first_task.summary.tool_sequence, ["name_session"]);
    assert_eq!(first_task.dispatches[0].tool_name, "name_session");
    assert_eq!(
        first_task.dispatches[0].args_json.as_deref(),
        Some(r#"{"name":"  Invoice Processing!!!  "}"#)
    );
    let (status, live) =
        request_json(&app.router, "GET", "/api/v1/sessions?status=live", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(live["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(live["items"][0]["sessionId"], session_id);
    assert_eq!(live["items"][0]["dispatchCount"], 1);
    assert_eq!(live["items"][0]["toolSequence"], json!(["name_session"]));

    let (_status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(name_session_request(4, "Quarterly Reporting")),
        &headers,
    )
    .await?;
    assert_eq!(
        body["result"]["content"][0]["text"],
        "renamed to codex/quarterly-reporting (was codex/invoice-processing)"
    );
    assert_eq!(
        app.state
            .audit_log
            .get_task_summary(session_id.as_str())
            .await?
            .map(|task| (task.dispatch_count, task.tool_sequence)),
        Some((
            2,
            vec!["name_session".to_string(), "name_session".to_string()]
        ))
    );

    for (id, invalid, message) in [
        (
            5,
            "!!!".to_string(),
            "name must contain a usable session name",
        ),
        (6, "x".repeat(65), "name must be at most 64 characters"),
    ] {
        let (_status, _headers, body) = request_json_with_headers(
            &app.router,
            "POST",
            "/mcp",
            Some(name_session_request(id, &invalid)),
            &headers,
        )
        .await?;
        assert_eq!(body["result"]["isError"], true);
        assert_eq!(body["result"]["content"][0]["text"], message);
        assert_eq!(session.label().await, "quarterly-reporting");
    }
    assert_eq!(
        app.state
            .audit_log
            .get_task_summary(session_id.as_str())
            .await?
            .map(|task| task.dispatch_count),
        Some(2)
    );
    Ok(())
}

#[tokio::test]
async fn mcp_session_naming_appends_five_tips_without_elicitation() -> anyhow::Result<()> {
    let mock = MockCdp::start().await?;
    let app = test_app_with_cdp_port(mock.cdp_port, false).await?;
    app.state.browser.connect_once_for_testing().await?;
    wait_for_cdp_connected(&app).await?;
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
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

    wait_for_session_registration(&app, &session_id).await?;
    let session = app
        .state
        .sessions
        .lookup(&SessionId::new(session_id.clone()))
        .await
        .ok_or_else(|| anyhow::anyhow!("session not minted"))?;
    let generated = session.generated_label().to_string();
    assert_eq!(session.label().await, generated);

    let mut stream = McpSseStream::open(&app.router, &session_id).await?;
    let tip = format!(
        "Tip: this session is \"claude/{generated}\" — rename it with name_session name=\"<2-3 word task label>\""
    );
    for id in 3..=7 {
        let (status, _headers, body) = request_json_with_headers(
            &app.router,
            "POST",
            "/mcp",
            Some(tabs_new_request(id)),
            &[("mcp-session-id", &session_id)],
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["isError"], false, "tabs_new body: {body:?}");
        assert_eq!(
            body["result"]["content"]
                .as_array()
                .and_then(|content| content.last())
                .and_then(|block| block["text"].as_str()),
            Some(tip.as_str())
        );
    }
    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(tabs_new_request(8)),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], false, "tabs_new body: {body:?}");
    assert!(!body.to_string().contains("Tip: this session is"));
    match tokio::time::timeout(
        Duration::from_millis(250),
        stream.next_method("elicitation/create"),
    )
    .await
    {
        Err(_) => {}
        Ok(Ok(request)) => anyhow::bail!("unexpected elicitation: {request:?}"),
        Ok(Err(error)) => return Err(error),
    }
    assert!(!mock.group_updates.lock().await.is_empty());
    drop(mock);
    Ok(())
}

#[tokio::test]
async fn mcp_tabs_new_roundtrips_through_mock_cdp() -> anyhow::Result<()> {
    let mock = MockCdp::start().await?;
    let app = test_app_with_cdp_port(mock.cdp_port, false).await?;
    app.state.browser.connect_once_for_testing().await?;
    wait_for_cdp_connected(&app).await?;
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
    assert!(body["result"].get("structuredContent").is_none());
    assert!(body["result"].get("_meta").is_none());

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
    assert!(list_body["result"].get("structuredContent").is_none());
    assert!(
        list_body["result"]["content"][0]["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("Your tabs:\n[1] https://example.com"))
    );

    let wait = json!({
        "jsonrpc": "2.0",
        "id": 12,
        "method": "tools/call",
        "params": {
            "name": "wait",
            "arguments": { "page": 1, "for": "time", "value": 1 }
        }
    });
    let (status, _headers, wait_body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(wait),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(wait_body["result"]["isError"], false);

    let dispatches = app
        .state
        .audit_log
        .list_dispatches(Default::default())
        .await?;
    let rows = &dispatches.rows;
    assert!(
        rows.iter()
            .any(|row| row.tool_name == "tabs" && row.dispatch_id.is_some())
    );
    let screenshot = rows
        .iter()
        .find(|row| row.has_screenshot)
        .ok_or_else(|| anyhow::anyhow!("no dispatch had a session screenshot"))?;
    assert_eq!(
        tokio::fs::read(app.state.screenshots.path_for(&session_id, screenshot.id)).await?,
        b"jpeg"
    );
    assert!(
        tokio::fs::metadata(app.state.screenshots.legacy_path_for(screenshot.id))
            .await
            .is_err()
    );

    drop(mock);
    Ok(())
}

#[tokio::test]
async fn same_name_mcp_sessions_have_distinct_groups_and_reject_cross_page_access()
-> anyhow::Result<()> {
    let mock = MockCdp::start().await?;
    let app = test_app_with_cdp_port(mock.cdp_port, false).await?;
    app.state.browser.connect_once_for_testing().await?;
    wait_for_cdp_connected(&app).await?;
    let session_a_id = initialize_mcp(&app).await?;
    let session_b_id = initialize_mcp(&app).await?;
    let session_a = app
        .state
        .sessions
        .lookup(&SessionId::new(session_a_id.clone()))
        .await
        .ok_or_else(|| anyhow::anyhow!("first session missing"))?;
    let session_b = app
        .state
        .sessions
        .lookup(&SessionId::new(session_b_id.clone()))
        .await
        .ok_or_else(|| anyhow::anyhow!("second session missing"))?;
    assert_ne!(session_a.convo_id(), session_b.convo_id());

    for (id, session_id) in [(10, &session_a_id), (11, &session_b_id)] {
        let (status, _headers, body) = request_json_with_headers(
            &app.router,
            "POST",
            "/mcp",
            Some(tabs_new_request(id)),
            &[("mcp-session-id", session_id)],
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["isError"], false, "tabs new: {body:?}");
    }

    let ownership = app.state.sessions.ownership();
    let (group_a, group_b) =
        wait_for_distinct_session_groups(&ownership, session_a.convo_id(), session_b.convo_id())
            .await?;
    assert_ne!(group_a, group_b);
    let pages_a = ownership.owned_pages(session_a.convo_id()).await;
    let pages_b = ownership.owned_pages(session_b.convo_id()).await;
    assert_eq!(pages_a.len(), 1);
    assert_eq!(pages_b.len(), 1);
    assert!(pages_a.is_disjoint(&pages_b));
    let page_a = pages_a
        .first()
        .map(|page| page.0)
        .ok_or_else(|| anyhow::anyhow!("first session page missing"))?;
    let page_b = pages_b
        .first()
        .map(|page| page.0)
        .ok_or_else(|| anyhow::anyhow!("second session page missing"))?;

    for (id, name, arguments) in [
        (12, "snapshot", json!({ "page": page_b })),
        (
            13,
            "navigate",
            json!({ "page": page_b, "url": "https://example.org" }),
        ),
    ] {
        let (status, _headers, body) = request_json_with_headers(
            &app.router,
            "POST",
            "/mcp",
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": { "name": name, "arguments": arguments }
            })),
            &[("mcp-session-id", &session_a_id)],
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["isError"], true, "cross-page body: {body:?}");
        assert_eq!(
            body["result"]["content"][0]["text"],
            format!(
                "page {page_b} is not owned by this agent; call `tabs new` to open a fresh page and use the returned page id."
            )
        );
    }
    let (_status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(json!({
            "jsonrpc": "2.0",
            "id": 14,
            "method": "tools/call",
            "params": { "name": "snapshot", "arguments": { "page": page_a } }
        })),
        &[("mcp-session-id", &session_b_id)],
    )
    .await?;
    assert_eq!(body["result"]["isError"], true);
    drop(mock);
    Ok(())
}

#[tokio::test]
async fn canonical_cancel_endpoint_aborts_in_flight_dispatch() -> anyhow::Result<()> {
    let mock = MockCdp::start().await?;
    let app = test_app_with_cdp_port(mock.cdp_port, false).await?;
    app.state.browser.connect_once_for_testing().await?;
    wait_for_cdp_connected(&app).await?;
    let session_id = initialize_mcp(&app).await?;
    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(tabs_new_request(20)),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert!(body["result"].get("structuredContent").is_none());

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
            &format!("/api/v1/sessions/{session_id}/cancel"),
            None,
        )
        .await?;
        if status == StatusCode::OK && body["cancelled"] == 1 {
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
        wait_body["result"]["content"][0]["text"],
        "Operation cancelled by the User"
    );
    assert!(wait_body["result"].get("structuredContent").is_none());

    let dispatches = app
        .state
        .audit_log
        .list_dispatches(Default::default())
        .await?;
    let cancellation_meta = dispatches
        .rows
        .iter()
        .find(|row| row.tool_name == "wait")
        .and_then(|row| row.result_meta.as_deref())
        .ok_or_else(|| anyhow::anyhow!("missing cancellation audit row"))?;
    let cancellation_meta: Value = serde_json::from_str(cancellation_meta)?;
    assert_eq!(cancellation_meta["isError"], true);
    assert_eq!(
        cancellation_meta["structuredKeys"],
        json!(["cancellationKind", "cancellationReason"])
    );
    drop(mock);
    Ok(())
}

#[tokio::test]
async fn canonical_live_sessions_enrich_through_profile_identity() -> anyhow::Result<()> {
    let mock = MockCdp::start().await?;
    mock.add_tab(101, "target-exact", 1).await;
    mock.add_tab(102, "target-fallback", 1).await;
    let app = test_app_with_cdp_port(mock.cdp_port, false).await?;
    app.state.browser.connect_once_for_testing().await?;
    let browser = app
        .state
        .browser
        .session()
        .await
        .ok_or_else(|| anyhow::anyhow!("missing browser session"))?;
    assert_eq!(browser.pages.list().await?.len(), 2);
    let agents_dir = app.state.config.browserclaw_dir.join("agents");
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

    let stored_session = Session::new(
        SessionId::new("stored-session"),
        ClientIdentity::Profile {
            profile_id: ProfileId::new("stored-agent"),
            slug: "mcp".to_string(),
            label: "Stored Agent".to_string(),
        },
        ConversationIdentity::new("mcp", "agile-alpaca".to_string()),
        tokio::time::Instant::now(),
    );
    let ephemeral_session =
        test_session(SessionId::new("ephemeral-session"), "bright-beaver", "mcp");
    app.state
        .sessions
        .insert_for_testing(stored_session.clone())
        .await;
    app.state
        .sessions
        .insert_for_testing(ephemeral_session.clone())
        .await;
    record_session_with_dispatch(&app, &stored_session).await?;
    record_session_with_dispatch(&app, &ephemeral_session).await?;

    app.state
        .tab_activity
        .record_tool(RecordToolInput {
            target_id: TargetId::from("target-exact".to_string()),
            tab_id: 101,
            page_id: 1,
            session_id: stored_session.id().as_str().to_string(),
            agent_id: stored_session.convo_id().as_str().to_string(),
            slug: "mcp".to_string(),
            tool_name: "tabs".to_string(),
        })
        .await;
    app.state.session_tabs.enqueue_claim_tab_for_session(
        101,
        Some("target-exact".to_string()),
        stored_session.id().as_str().to_string(),
        stored_session.convo_id().as_str().to_string(),
        1,
    );
    app.state
        .tab_activity
        .record_tool(RecordToolInput {
            target_id: TargetId::from("target-fallback".to_string()),
            tab_id: 102,
            page_id: 2,
            session_id: ephemeral_session.id().as_str().to_string(),
            agent_id: ephemeral_session.convo_id().as_str().to_string(),
            slug: "mcp".to_string(),
            tool_name: "tabs".to_string(),
        })
        .await;
    app.state.session_tabs.enqueue_claim_tab_for_session(
        102,
        Some("target-fallback".to_string()),
        ephemeral_session.id().as_str().to_string(),
        ephemeral_session.convo_id().as_str().to_string(),
        1,
    );

    let (status, body) =
        request_json(&app.router, "GET", "/api/v1/sessions?status=live", None).await?;
    assert_eq!(status, StatusCode::OK);
    let rows = body["items"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("sessions not array"))?;
    let exact = rows
        .iter()
        .find(|row| row["sessionId"] == stored_session.id().as_str())
        .ok_or_else(|| anyhow::anyhow!("missing exact session"))?;
    assert_eq!(exact["label"], "Stored Agent");
    assert_eq!(exact["harness"], "Codex");
    assert_eq!(exact["live"]["browserTabs"][0]["browserTabId"], 101);

    let fallback = rows
        .iter()
        .find(|row| row["sessionId"] == ephemeral_session.id().as_str())
        .ok_or_else(|| anyhow::anyhow!("missing fallback session"))?;
    assert_eq!(fallback["label"], "mcp");
    assert!(fallback["harness"].is_null());
    assert_eq!(fallback["live"]["browserTabs"][0]["browserTabId"], 102);
    Ok(())
}

#[tokio::test]
async fn live_projection_filters_external_close() -> anyhow::Result<()> {
    let mock = MockCdp::start().await?;
    mock.add_tab(101, "target-old", 1).await;
    let app = test_app_with_cdp_port(mock.cdp_port, false).await?;
    app.state.browser.connect_once_for_testing().await?;
    let browser = app
        .state
        .browser
        .session()
        .await
        .ok_or_else(|| anyhow::anyhow!("missing browser session"))?;
    assert_eq!(browser.pages.list().await?.len(), 1);
    let session = test_session(SessionId::new("session-live"), "codex-agent", "codex");
    app.state.sessions.insert_for_testing(session.clone()).await;
    record_session_with_dispatch(&app, &session).await?;
    app.state
        .tab_activity
        .record_tool(RecordToolInput {
            target_id: TargetId::from("target-old".to_string()),
            tab_id: 101,
            page_id: 1,
            session_id: session.id().as_str().to_string(),
            agent_id: session.convo_id().as_str().to_string(),
            slug: "codex".to_string(),
            tool_name: "snapshot".to_string(),
        })
        .await;
    app.state.session_tabs.enqueue_claim_tab_for_session(
        101,
        Some("target-old".to_string()),
        session.id().as_str().to_string(),
        session.convo_id().as_str().to_string(),
        1,
    );
    let (status, body) =
        request_json(&app.router, "GET", "/api/v1/sessions?status=live", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["items"][0]["live"]["browserTabs"][0]["browserTabId"],
        101
    );

    mock.remove_tab(101).await;
    let recording_request = Request::builder()
        .method("POST")
        .uri("/api/v1/sessions/session-live/recording/events")
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .header("x-recording-tab-id", "101")
        .header("x-recording-page-id", "1")
        .header("x-recording-target-id", "target-old")
        .body(Body::from("{\"ts\":1}\n"))?;
    let recording_response = app.router.clone().oneshot(recording_request).await?;
    assert_eq!(recording_response.status(), StatusCode::METHOD_NOT_ALLOWED);

    let (status, body) =
        request_json(&app.router, "GET", "/api/v1/sessions?status=live", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"][0]["live"]["browserTabs"], json!([]));

    Ok(())
}

#[tokio::test]
async fn live_projection_refreshes_metadata_and_preview_uses_current_target() -> anyhow::Result<()>
{
    let mock = MockCdp::start().await?;
    mock.add_tab(101, "target-old", 1).await;
    let app = test_app_with_cdp_port(mock.cdp_port, false).await?;
    app.state.browser.connect_once_for_testing().await?;
    let session = test_session(SessionId::new("session-live"), "codex-agent", "codex");
    app.state.sessions.insert_for_testing(session.clone()).await;
    record_session_with_dispatch(&app, &session).await?;
    app.state
        .tab_activity
        .record_tool(RecordToolInput {
            target_id: TargetId::from("target-old".to_string()),
            tab_id: 101,
            page_id: 1,
            session_id: session.id().as_str().to_string(),
            agent_id: session.convo_id().as_str().to_string(),
            slug: "codex".to_string(),
            tool_name: "navigate".to_string(),
        })
        .await;
    app.state.session_tabs.enqueue_claim_tab_for_session(
        101,
        Some("target-old".to_string()),
        session.id().as_str().to_string(),
        session.convo_id().as_str().to_string(),
        1,
    );

    mock.update_tab(
        101,
        "target-old",
        "https://example.com/after-navigation",
        "After navigation",
    )
    .await;
    let (status, body) =
        request_json(&app.router, "GET", "/api/v1/sessions?status=live", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["items"][0]["live"]["browserTabs"][0]["url"],
        "https://example.com/after-navigation"
    );
    assert_eq!(
        body["items"][0]["live"]["browserTabs"][0]["title"],
        "After navigation"
    );
    mock.update_tab(
        101,
        "target-new",
        "https://example.com/reused",
        "Replacement tab",
    )
    .await;
    let captures_before = mock.captures.lock().await.len();
    let browser = app
        .state
        .browser
        .session()
        .await
        .ok_or_else(|| anyhow::anyhow!("missing browser session"))?;
    assert!(
        browser
            .screenshot_for_target(
                PageId(1),
                &TargetId::from("target-old".to_string()),
                ScreenshotCaptureOptions::default(),
            )
            .await?
            .is_none()
    );
    assert_eq!(mock.captures.lock().await.len(), captures_before);
    let (status, body) =
        request_json(&app.router, "GET", "/api/v1/sessions?status=live", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["items"][0]["live"]["browserTabs"][0]["browserTabId"],
        101
    );
    assert_eq!(body["items"][0]["live"]["browserTabs"][0]["toolCount"], 0);
    assert_eq!(
        request_status(&app.router, "GET", "/api/v1/sessions/session-live/preview",).await?,
        StatusCode::OK
    );

    app.state
        .tab_activity
        .record_tool(RecordToolInput {
            target_id: TargetId::from("target-new".to_string()),
            tab_id: 101,
            page_id: 1,
            session_id: session.id().as_str().to_string(),
            agent_id: session.convo_id().as_str().to_string(),
            slug: "codex".to_string(),
            tool_name: "navigate".to_string(),
        })
        .await;
    let (status, body) =
        request_json(&app.router, "GET", "/api/v1/sessions?status=live", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"][0]["live"]["browserTabs"][0]["toolCount"], 1);
    assert_eq!(
        request_status(&app.router, "GET", "/api/v1/sessions/session-live/preview",).await?,
        StatusCode::OK
    );
    Ok(())
}

#[tokio::test]
async fn session_previews_capture_each_sessions_owned_target() -> anyhow::Result<()> {
    let mock = MockCdp::start().await?;
    mock.add_tab(101, "target-1", 1).await;
    mock.add_tab(102, "target-2", 2).await;
    let app = test_app_with_cdp_port(mock.cdp_port, false).await?;
    app.state.browser.connect_once_for_testing().await?;
    wait_for_cdp_connected(&app).await?;

    for (page_id, target_id, agent_id) in [(1, "target-1", "agent-a"), (2, "target-2", "agent-b")] {
        let session = test_session(
            SessionId::new(format!("session-{target_id}")),
            agent_id,
            "codex",
        );
        app.state.sessions.insert_for_testing(session.clone()).await;
        record_session_with_dispatch(&app, &session).await?;
        app.state
            .tab_activity
            .record_tool(RecordToolInput {
                target_id: TargetId::from(target_id.to_string()),
                tab_id: i64::from(page_id) + 100,
                page_id,
                session_id: format!("session-{target_id}"),
                agent_id: agent_id.to_string(),
                slug: "codex".to_string(),
                tool_name: "tabs".to_string(),
            })
            .await;
        app.state.session_tabs.enqueue_claim_tab_for_session(
            i64::from(page_id) + 100,
            Some(target_id.to_string()),
            session.id().as_str().to_string(),
            session.convo_id().as_str().to_string(),
            1,
        );
    }

    assert_eq!(
        request_status(&app.router, "GET", "/api/v1/sessions?status=live").await?,
        StatusCode::OK
    );
    for session_id in ["session-target-1", "session-target-2"] {
        assert_eq!(
            request_status(
                &app.router,
                "GET",
                &format!("/api/v1/sessions/{session_id}/preview"),
            )
            .await?,
            StatusCode::OK
        );
    }

    let captures = mock.captures.lock().await.clone();
    for session_id in ["session-target-1", "session-target-2"] {
        let (_, params) = captures
            .iter()
            .find(|(session, _)| session == session_id)
            .ok_or_else(|| anyhow::anyhow!("missing screenshot capture for {session_id}"))?;
        assert_eq!(
            params,
            &json!({
                "format": "jpeg",
                "fromSurface": true,
                "captureBeyondViewport": false,
                "quality": 50
            })
        );
    }

    drop(mock);
    Ok(())
}

fn test_session(session_id: SessionId, agent_id: &str, slug: &str) -> Arc<Session> {
    let generated_label = agent_id
        .strip_prefix(&format!("{slug}-"))
        .unwrap_or(agent_id)
        .to_string();
    Session::new(
        session_id,
        ClientIdentity::Ephemeral {
            slug: slug.to_string(),
            label: slug.to_string(),
        },
        ConversationIdentity::new(slug, generated_label),
        tokio::time::Instant::now(),
    )
}

async fn record_session_with_dispatch(app: &TestApp, session: &Session) -> anyhow::Result<()> {
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
    app.state
        .audit_log
        .record_tool_dispatch(RecordToolDispatchInput {
            agent_id: session.convo_id().as_str().to_string(),
            slug: session.agent().slug().to_string(),
            agent_label: session.agent().label().to_string(),
            session_id: session.id().as_str().to_string(),
            tool_name: "snapshot".to_string(),
            page_id: None,
            tab_id: None,
            target_id: None,
            url: None,
            title: None,
            raw_args: json!({}),
            duration_ms: 1,
            dispatch_id: DispatchId::new(),
            result: DispatchResultSummary {
                is_error: false,
                structured_content: json!({}),
                content: json!([]),
            },
        })
        .await?;
    Ok(())
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
    assert_eq!(body["result"]["serverInfo"]["name"], "browserclaw");
    assert_eq!(body["result"]["serverInfo"]["title"], "BrowserClaw");
    assert!(
        body["result"]["instructions"].as_str().is_some_and(
            |instructions| instructions.starts_with("BrowserClaw — the browser for agents")
        )
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

fn name_session_request(id: u64, name: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "name_session",
            "arguments": { "name": name }
        }
    })
}

/// The initialized notification is acknowledged before the service finishes
/// registering the session, so an immediate tool call can race the registry.
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

async fn wait_for_cdp_connected(app: &TestApp) -> anyhow::Result<()> {
    for _ in 0..120 {
        if app.state.browser.state().connected {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(anyhow::anyhow!("mock CDP did not connect"))
}

async fn wait_for_distinct_session_groups(
    ownership: &Arc<PageOwnership>,
    key_a: &ConvoId,
    key_b: &ConvoId,
) -> anyhow::Result<(String, String)> {
    for _ in 0..100 {
        let group_a = ownership.tab_group_ref(key_a).await;
        let group_b = ownership.tab_group_ref(key_b).await;
        if let (Some(group_a), Some(group_b)) = (group_a, group_b) {
            return Ok((group_a, group_b));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    anyhow::bail!("session tab groups were not created")
}

struct MockCdp {
    cdp_port: u16,
    tabs: Arc<tokio::sync::Mutex<Vec<MockTab>>>,
    captures: Arc<tokio::sync::Mutex<Vec<(String, Value)>>>,
    group_updates: Arc<tokio::sync::Mutex<Vec<Value>>>,
    tasks: Vec<JoinHandle<()>>,
}

impl MockCdp {
    async fn start() -> anyhow::Result<Self> {
        let tabs = Arc::new(tokio::sync::Mutex::new(Vec::<MockTab>::new()));
        let captures = Arc::new(tokio::sync::Mutex::new(Vec::<(String, Value)>::new()));
        let group_updates = Arc::new(tokio::sync::Mutex::new(Vec::<Value>::new()));
        let ws_listener = TcpListener::bind("127.0.0.1:0").await?;
        let ws_addr = ws_listener.local_addr()?;
        let ws_tabs = tabs.clone();
        let ws_captures = captures.clone();
        let ws_group_updates = group_updates.clone();
        let ws_task = tokio::spawn(async move {
            loop {
                let Ok((stream, _addr)) = ws_listener.accept().await else {
                    break;
                };
                let tabs = ws_tabs.clone();
                let captures = ws_captures.clone();
                let group_updates = ws_group_updates.clone();
                tokio::spawn(async move {
                    let _ = handle_mock_ws(stream, tabs, captures, group_updates).await;
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
            captures,
            group_updates,
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

    async fn remove_tab(&self, tab_id: i64) {
        self.tabs.lock().await.retain(|tab| tab.tab_id != tab_id);
    }

    async fn update_tab(&self, tab_id: i64, target_id: &str, url: &str, title: &str) {
        if let Some(tab) = self
            .tabs
            .lock()
            .await
            .iter_mut()
            .find(|tab| tab.tab_id == tab_id)
        {
            tab.target_id = target_id.to_string();
            tab.url = url.to_string();
            tab.title = title.to_string();
        }
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
    captures: Arc<tokio::sync::Mutex<Vec<(String, Value)>>>,
    group_updates: Arc<tokio::sync::Mutex<Vec<Value>>>,
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
        if method == "Page.captureScreenshot" {
            captures
                .lock()
                .await
                .push((session.clone().unwrap_or_default(), params));
        } else if method == "Browser.updateTabGroup" {
            group_updates.lock().await.push(params);
        }
        let response = match result {
            Ok(result) => json!({ "id": id, "result": result }),
            Err(message) => json!({ "id": id, "error": { "code": -32000, "message": message } }),
        };
        ws.send(Message::Text(response.to_string().into())).await?;
    }
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
            let group_id = format!(
                "group-{}",
                tabs.iter()
                    .filter_map(|tab| tab.group_id.as_deref())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    + 1
            );
            let tab_ids = params
                .get("tabIds")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for id in &tab_ids {
                if let Some(tab_id) = id.as_i64()
                    && let Some(tab) = tabs.iter_mut().find(|tab| tab.tab_id == tab_id)
                {
                    tab.group_id = Some(group_id.clone());
                }
            }
            Ok(json!({
                "group": {
                    "groupId": group_id,
                    "windowId": 1,
                    "title": params.get("title").and_then(Value::as_str).unwrap_or("group"),
                    "color": "blue",
                    "collapsed": false,
                    "tabIds": tab_ids
                }
            }))
        }
        "Browser.addTabsToGroup" => {
            let group_id = params
                .get("groupId")
                .and_then(Value::as_str)
                .unwrap_or("group-1")
                .to_string();
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
                    tab.group_id = Some(group_id.clone());
                }
            }
            let grouped_ids = tabs
                .iter()
                .filter(|tab| tab.group_id.as_deref() == Some(group_id.as_str()))
                .map(|tab| tab.tab_id)
                .collect::<Vec<_>>();
            Ok(json!({
                "group": {
                    "groupId": group_id,
                    "windowId": 1,
                    "title": "group",
                    "color": "blue",
                    "collapsed": false,
                    "tabIds": grouped_ids
                }
            }))
        }
        "Browser.updateTabGroup" => {
            let group_id = params
                .get("groupId")
                .and_then(Value::as_str)
                .unwrap_or("group-1");
            let tabs = tabs.lock().await;
            let grouped_ids = tabs
                .iter()
                .filter(|tab| tab.group_id.as_deref() == Some(group_id))
                .map(|tab| tab.tab_id)
                .collect::<Vec<_>>();
            Ok(json!({
                "group": {
                    "groupId": group_id,
                    "windowId": 1,
                    "title": params.get("title").and_then(Value::as_str).unwrap_or("group"),
                    "color": params.get("color").and_then(Value::as_str).unwrap_or("blue"),
                    "collapsed": params.get("collapsed").and_then(Value::as_bool).unwrap_or(false),
                    "tabIds": grouped_ids
                }
            }))
        }
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
