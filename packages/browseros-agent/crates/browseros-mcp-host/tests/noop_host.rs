//! A second host needs nothing claw-specific: a `HostCtx` on the no-op adapters (no
//! database, no analytics, no cockpit) mounted with `mcp_router` serves the full tool
//! surface over streamable HTTP, runs the shared local tool, and routes catalog tools
//! through the dispatch pipeline.

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use browseros_core::NativeCapabilities;
use browseros_mcp_host::{
    HostCore, HostCoreConfig, HostCtx, McpRouteConfig, mcp_router, sessions::Session,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

const PROTOCOL: &str = "2025-06-18";

async fn noop_host() -> anyhow::Result<(Arc<HostCtx>, tempfile::TempDir)> {
    let dir = tempfile::tempdir()?;
    let core = HostCore::open(HostCoreConfig {
        // Nothing listens here: the host must serve MCP before any browser attaches.
        cdp_port: 9,
        data_root: dir.path().join("host"),
        native: NativeCapabilities::from_browseros_version(Some("0.51.1.3")),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(600),
        session_sweep_interval: Duration::from_secs(60),
    })
    .await?;
    Ok((HostCtx::builder(core).build(), dir))
}

async fn post(
    router: &Router,
    session: Option<&str>,
    body: Value,
) -> anyhow::Result<(StatusCode, HeaderMap, Value)> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    if let Some(session) = session {
        request = request
            .header("mcp-session-id", session)
            .header("mcp-protocol-version", PROTOCOL);
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(body.to_string()))?)
        .await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let text = std::str::from_utf8(&bytes)?;
    let value = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .find(|data| !data.is_empty())
        .map_or_else(
            || {
                if text.trim().is_empty() {
                    Ok(Value::Null)
                } else {
                    serde_json::from_str(text)
                }
            },
            serde_json::from_str,
        )?;
    Ok((status, headers, value))
}

fn text_of(result: &Value) -> String {
    result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

async fn live_session(host: &HostCtx, id: &str) -> anyhow::Result<Arc<Session>> {
    // The transport registers the session on `notifications/initialized`, which rmcp
    // delivers asynchronously after answering it.
    for _ in 0..200 {
        if let Some(session) = host
            .sessions()
            .lookup(&browseros_mcp_host::ids::SessionId::new(id))
            .await
        {
            return Ok(session);
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    anyhow::bail!("session {id} never started")
}

#[tokio::test]
async fn a_noop_host_lists_the_catalog_and_serves_local_and_catalog_tools() -> anyhow::Result<()> {
    let (host, _dir) = noop_host().await?;
    let router = mcp_router(host.clone(), McpRouteConfig::default());

    let (status, headers, init) = post(
        &router,
        None,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL,
                "capabilities": {},
                "clientInfo": { "name": "Codex", "version": "1.0" }
            }
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{init}");
    assert_eq!(init["result"]["serverInfo"]["name"], "browseros");
    assert_eq!(
        init["result"]["instructions"].as_str(),
        Some(browseros_mcp::browser_mcp_instructions(host.native()).as_str())
    );
    let session_id = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("missing mcp-session-id"))?
        .to_string();
    let (status, _, _) = post(
        &router,
        Some(&session_id),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await?;
    assert_eq!(status, StatusCode::ACCEPTED);
    let session = live_session(&host, &session_id).await?;

    let (status, _, list) = post(
        &router,
        Some(&session_id),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{list}");
    let tools = list["result"]["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("tools missing: {list}"))?;
    let names = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();
    let mut expected = browseros_mcp::catalog()
        .iter()
        .filter(|tool| tool.available(host.native()))
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    expected.push("name_session");
    assert_eq!(names, expected);
    assert!(names.contains(&"run") && names.contains(&"browser_target"));
    for tool in tools {
        assert_eq!(
            tool["inputSchema"]["properties"]["session"]["type"], "string",
            "{} lacks the session argument",
            tool["name"]
        );
    }

    // The shared local tool works with no journal, analytics or browser behind it.
    let (_, _, named) = post(
        &router,
        Some(&session_id),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "name_session", "arguments": { "name": "Invoice Sync" } }
        }),
    )
    .await?;
    assert_eq!(named["result"]["isError"], false, "{named}");
    assert!(text_of(&named["result"]).starts_with("renamed to codex/invoice-sync"));
    assert_eq!(session.label().await, "invoice-sync");

    // A catalog tool reaches the dispatch pipeline, whose guard explains the missing
    // browser in this host's product name.
    let (_, _, tabs) = post(
        &router,
        Some(&session_id),
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": { "name": "tabs", "arguments": { "action": "list" } }
        }),
    )
    .await?;
    assert_eq!(tabs["result"]["isError"], true, "{tabs}");
    let text = text_of(&tabs["result"]);
    assert!(text.contains("no link to the browser yet"), "{text}");
    assert!(text.contains("If BrowserOS is not running"), "{text}");
    Ok(())
}
