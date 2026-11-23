//! The private MCP endpoint `/mcp/{task}`, through the real router over the browser double:
//! who it refuses, the whole handshake for an armed task, the armed scope reaching the call,
//! and dropping `Armed` stopping a call in flight. The same endpoint over the real engine is in
//! `browser_live.rs`.

mod support;

use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use browseros_server::browser::{Access, Scope, ScriptedReply};
use serde_json::{Value, json};
use support::{TestOptions, TestServer, body_text};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const TASK: &str = "5b0f3f7e-6c3a-4c1e-9d55-0b8a2f6e1a01";

fn scope(access: Access) -> Scope {
    Scope {
        task: TASK.into(),
        access,
    }
}

async fn with_replies(
    replies: Vec<ScriptedReply>,
) -> Result<TestServer, Box<dyn std::error::Error>> {
    Ok(TestServer::with(TestOptions {
        browser_replies: Some(replies),
        ..TestOptions::default()
    })
    .await?)
}

/// A request as a native MCP client sends it: loopback `Host`, no `Origin`.
fn mcp(task: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(Method::POST)
        .uri(format!("/mcp/{task}"))
        .header(header::HOST, "127.0.0.1:9200")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-11-25")
}

fn post(task: &str, body: &Value) -> Result<Request<Body>, axum::http::Error> {
    mcp(task).body(Body::from(body.to_string()))
}

/// The JSON-RPC messages of a response, whether it came as JSON or as an event stream.
async fn messages(response: axum::http::Response<Body>) -> Vec<Value> {
    let text = body_text(response).await;
    if let Ok(single) = serde_json::from_str::<Value>(&text) {
        return vec![single];
    }
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|data| serde_json::from_str(data.trim_start()).ok())
        .collect()
}

async fn reply_to(server: &TestServer, body: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let id = body["id"].clone();
    let response = server.send(post(TASK, &body)?).await;
    assert_eq!(response.status(), StatusCode::OK, "{}", body["method"]);
    messages(response)
        .await
        .into_iter()
        .find(|message| message["id"] == id)
        .ok_or_else(|| format!("no reply to {}", body["method"]).into())
}

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "browser-mcp-test", "version": "1" }
        }
    })
}

#[tokio::test]
async fn an_unarmed_task_is_refused() -> TestResult {
    let server = with_replies(Vec::new()).await?;
    let response = server.send(post(TASK, &initialize())?).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn a_request_from_a_web_page_is_refused_even_for_an_armed_task() -> TestResult {
    let server = with_replies(Vec::new()).await?;
    let _armed = server.app.browser.arm(scope(Access::Full));
    for (name, value) in [
        (header::ORIGIN.as_str(), "https://example.com"),
        ("sec-fetch-site", "cross-site"),
    ] {
        let request = mcp(TASK)
            .header(name, value)
            .body(Body::from(initialize().to_string()))?;
        let response = server.send(request).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{name}");
    }
    Ok(())
}

#[tokio::test]
async fn an_armed_task_completes_the_handshake_and_runs_under_the_armed_scope() -> TestResult {
    let server = with_replies(vec![ScriptedReply::text("2")]).await?;
    let browser = server.browser.clone().ok_or("no browser double")?;
    let armed = server
        .app
        .browser
        .arm(scope(Access::Read))
        .ok_or("the double arms")?;
    assert_eq!(armed.url(), format!("http://127.0.0.1:0/mcp/{TASK}"));

    let initialized = reply_to(&server, initialize()).await?;
    assert_eq!(initialized["result"]["serverInfo"]["name"], "browseros");
    assert!(initialized["result"]["capabilities"]["tools"].is_object());

    let notified = server
        .send(post(
            TASK,
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        )?)
        .await;
    assert_eq!(notified.status(), StatusCode::ACCEPTED);

    let pong = reply_to(
        &server,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }),
    )
    .await?;
    assert_eq!(pong["result"], json!({}));

    let listed = reply_to(
        &server,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list", "params": {} }),
    )
    .await?;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool["name"].as_str())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(names, vec!["run"]);

    let called = reply_to(
        &server,
        json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "run", "arguments": { "code": "return 1 + 1" } }
        }),
    )
    .await?;
    assert_eq!(called["result"]["content"][0]["text"], "2");
    assert_eq!(called["result"]["isError"], false);

    let calls = browser.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].scope,
        scope(Access::Read),
        "the armed scope, not the agent's"
    );
    assert_eq!(calls[0].args, json!({ "code": "return 1 + 1" }));
    drop(armed);
    Ok(())
}

#[tokio::test]
async fn extra_run_arguments_return_a_correctable_tool_error() -> TestResult {
    let server = with_replies(Vec::new()).await?;
    let _armed = server.app.browser.arm(scope(Access::Full));
    let reply = reply_to(
        &server,
        json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": {
                "name": "run",
                "arguments": {"code": "await page.goto(url)", "tabId": 123, "url": "https://example.com"}
            }
        }),
    ).await?;
    assert_eq!(reply["result"]["isError"], true);
    let error = reply["result"]["content"][0]["text"]
        .as_str()
        .ok_or("no error text")?;
    assert!(error.contains("\"tabId\", \"url\""), "{error}");
    assert!(error.contains("only code and optional timeout"), "{error}");
    assert!(error.contains("inside the JavaScript in code"), "{error}");
    assert!(
        server
            .browser
            .as_ref()
            .ok_or("no browser double")?
            .calls()
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn dropping_armed_stops_a_call_in_flight_and_closes_the_endpoint() -> TestResult {
    let server = with_replies(vec![ScriptedReply::UntilCancelled]).await?;
    let browser = server.browser.clone().ok_or("no browser double")?;
    let armed = server
        .app
        .browser
        .arm(scope(Access::Full))
        .ok_or("the double arms")?;
    assert!(browser.is_armed(TASK));
    let call = json!({
        "jsonrpc": "2.0", "id": 9, "method": "tools/call",
        "params": { "name": "run", "arguments": { "code": "await new Promise(() => {})" } }
    });
    let request = post(TASK, &call)?;
    let router = server.router.clone();
    let in_flight = tokio::spawn(async move {
        use tower::ServiceExt;
        let response = router.oneshot(request).await?;
        Ok::<_, std::convert::Infallible>(messages(response).await)
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while browser.calls().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;

    drop(armed);
    let replies = tokio::time::timeout(Duration::from_secs(5), in_flight).await???;
    let reply = replies
        .iter()
        .find(|message| message["id"] == 9)
        .ok_or("no reply to the call")?;
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(reply["result"]["content"][0]["text"], "Stopped");
    assert!(!browser.is_armed(TASK));

    let after = server.send(post(TASK, &initialize())?).await;
    assert_eq!(after.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn without_an_engine_nothing_can_be_armed() -> TestResult {
    let server = TestServer::start().await?;
    assert!(server.app.browser.arm(scope(Access::Full)).is_none());
    assert!(server.app.browser.tools().is_empty());
    assert!(!server.app.browser.connected());
    let response = server.send(post(TASK, &initialize())?).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}
