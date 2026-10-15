//! MCP tool sources against real rmcp servers over loopback: the Streamable HTTP transport the
//! internal `/mcp` uses, transport detection, and the legacy HTTP+SSE transport (served here by a
//! small bridge from SSE/POST to an rmcp server on an in-memory pipe). Tool specs must match what
//! `@ai-sdk/mcp` builds, and calls must cancel on the server.

use axum::{
    Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::sse::{Event, Sse},
    routing::{get, post},
};
use browseros_agent::tools::{
    McpEndpoint, McpTransport, ToolKind, ToolSource, ToolSourceError, connect_mcp,
};
use futures_util::StreamExt;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, InitializeResult,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities,
    },
    serve_server,
    service::RequestContext,
    transport::streamable_http_server::{
        session::local::LocalSessionManager,
        tower::{StreamableHttpServerConfig, StreamableHttpService},
    },
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const LEASE_HEADER: &str = "x-browseros-internal-lease";

// ───── the test MCP server ─────

#[derive(Default)]
struct Observed {
    leases: Mutex<Vec<String>>,
    cancelled: Mutex<Vec<String>>,
    probes: Mutex<usize>,
}

#[derive(Clone)]
struct TestServer {
    observed: Arc<Observed>,
}

impl ServerHandler for TestServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("test-mcp", "0.0.0"))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        serde_json::from_value(json!({ "tools": [
            {
                "name": "echo",
                "description": "Echo the text",
                "inputSchema": { "type": "object", "properties": { "text": { "type": "string" } } },
                "annotations": { "title": "Echo Tool", "readOnlyHint": true },
            },
            { "name": "no_props", "title": "No Props", "inputSchema": { "type": "object" } },
            { "name": "slow", "description": "Waits", "inputSchema": { "type": "object", "properties": {} } },
            { "name": "flaky", "inputSchema": { "type": "object" } },
            { "name": "picture", "inputSchema": { "type": "object" } },
            { "name": "boom", "inputSchema": { "type": "object" } },
            {
                "name": "widget",
                "inputSchema": { "type": "object" },
                "_meta": { "ui": { "resourceUri": "ui://widget/main" } },
            },
        ]}))
        .map_err(|error| McpError::internal_error(error.to_string(), None))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let result = match request.name.as_ref() {
            "echo" => {
                let text = request
                    .arguments
                    .as_ref()
                    .and_then(|args| args.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                json!({ "content": [{ "type": "text", "text": text }], "isError": false })
            }
            "slow" => {
                tokio::select! {
                    () = context.ct.cancelled() => {
                        if let Ok(mut cancelled) = self.observed.cancelled.lock() {
                            cancelled.push("slow".to_owned());
                        }
                        return Err(McpError::internal_error("cancelled", None));
                    }
                    () = tokio::time::sleep(Duration::from_secs(30)) => json!({ "content": [] }),
                }
            }
            "flaky" => {
                json!({ "content": [{ "type": "text", "text": "bad input" }], "isError": true })
            }
            "picture" => json!({ "content": [
                { "type": "image", "data": "AAAA", "mimeType": "image/png" },
                { "type": "resource_link", "uri": "file:///x", "name": "x" },
            ]}),
            "boom" => return Err(McpError::invalid_params("boom failed", None)),
            other => {
                return Err(McpError::invalid_params(
                    format!("unknown tool {other}"),
                    None,
                ));
            }
        };
        let result: CallToolResult = serde_json::from_value(result)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        Ok(CallToolResponse::Complete(result))
    }
}

struct Server {
    base: String,
    observed: Arc<Observed>,
}

/// Streamable HTTP at `/mcp` and the legacy transport at `/sse` + `/messages`, on one port.
async fn start_server() -> Result<Server, Box<dyn std::error::Error>> {
    let observed = Arc::new(Observed::default());
    let handler = TestServer {
        observed: Arc::clone(&observed),
    };
    let streamable = {
        let handler = handler.clone();
        StreamableHttpService::new(
            move || Ok(handler.clone()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        )
    };
    let legacy = LegacyState {
        handler,
        sessions: Arc::new(Mutex::new(HashMap::new())),
    };
    let watch = Arc::clone(&observed);
    let app = Router::new()
        .nest_service("/mcp", streamable)
        .route("/sse", get(legacy_events))
        .route("/messages", post(legacy_post))
        .with_state(legacy)
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let watch = Arc::clone(&watch);
                async move {
                    if let Some(lease) = request
                        .headers()
                        .get(LEASE_HEADER)
                        .and_then(|v| v.to_str().ok())
                        && let Ok(mut leases) = watch.leases.lock()
                    {
                        leases.push(lease.to_owned());
                    }
                    if request.method() == axum::http::Method::POST
                        && request.uri().path() == "/sse"
                        && let Ok(mut probes) = watch.probes.lock()
                    {
                        *probes += 1;
                    }
                    next.run(request).await
                }
            },
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(Server { base, observed })
}

/// The client → server half of one legacy session's pipe.
type SessionWriter = Arc<tokio::sync::Mutex<tokio::io::WriteHalf<tokio::io::DuplexStream>>>;

/// The legacy transport: each `GET /sse` starts an rmcp server session on an in-memory pipe and
/// announces `/messages?sessionId=…`; POSTed messages are written into the pipe and the server's
/// replies stream back as `message` events.
#[derive(Clone)]
struct LegacyState {
    handler: TestServer,
    sessions: Arc<Mutex<HashMap<String, SessionWriter>>>,
}

async fn legacy_events(
    State(state): State<LegacyState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let (client_side, server_side) = tokio::io::duplex(64 * 1024);
    let handler = state.handler.clone();
    tokio::spawn(async move {
        if let Ok(running) = serve_server(handler, server_side).await {
            let _ = running.waiting().await;
        }
    });
    let (reader, writer) = tokio::io::split(client_side);
    let session = format!("s{}", rand_id());
    if let Ok(mut sessions) = state.sessions.lock() {
        sessions.insert(session.clone(), Arc::new(tokio::sync::Mutex::new(writer)));
    }
    let endpoint = futures_util::stream::once(async move {
        Ok(Event::default()
            .event("endpoint")
            .data(format!("/messages?sessionId={session}")))
    });
    let messages =
        futures_util::stream::unfold(BufReader::new(reader).lines(), |mut lines| async move {
            let line = lines.next_line().await.ok()??;
            Some((Ok(Event::default().event("message").data(line)), lines))
        });
    Sse::new(endpoint.chain(messages))
}

async fn legacy_post(
    State(state): State<LegacyState>,
    Query(query): Query<HashMap<String, String>>,
    _headers: HeaderMap,
    body: String,
) -> StatusCode {
    let writer = query
        .get("sessionId")
        .and_then(|id| state.sessions.lock().ok()?.get(id).cloned());
    let Some(writer) = writer else {
        return StatusCode::NOT_FOUND;
    };
    let mut writer = writer.lock().await;
    let line = format!("{}\n", body.trim());
    if writer.write_all(line.as_bytes()).await.is_err() {
        return StatusCode::GONE;
    }
    StatusCode::ACCEPTED
}

fn rand_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::SeqCst)
}

fn endpoint(url: String, transport: McpTransport) -> Result<McpEndpoint, url::ParseError> {
    Ok(McpEndpoint {
        label: "browseros".to_owned(),
        url: url::Url::parse(&url)?,
        headers: vec![(
            "X-BrowserOS-Internal-Lease".to_owned(),
            "lease-123".to_owned(),
        )],
        transport,
    })
}

async fn connect(
    url: String,
    transport: McpTransport,
) -> Result<Arc<dyn ToolSource>, Box<dyn std::error::Error>> {
    Ok(connect_mcp(endpoint(url, transport)?).await?)
}

// ───── tests ─────

#[tokio::test]
async fn streamable_tools_have_the_ai_sdk_shape() -> TestResult {
    let server = start_server().await?;
    let source = connect(
        format!("{}/mcp?structured=1", server.base),
        McpTransport::StreamableHttp,
    )
    .await?;
    assert_eq!(source.label(), "browseros");
    let specs = source.tools().await?;
    let names: Vec<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "echo", "no_props", "slow", "flaky", "picture", "boom", "widget"
        ]
    );

    let echo = &specs[0];
    assert_eq!(echo.description, "Echo the text");
    assert_eq!(
        echo.input_schema,
        json!({ "type": "object", "properties": { "text": { "type": "string" } }, "additionalProperties": false })
    );
    assert_eq!(
        echo.kind,
        ToolKind::Dynamic {
            title: Some("Echo Tool".to_owned()),
            metadata: json!({
                "clientName": "ai-sdk-mcp-client",
                "toolName": "echo",
                "title": "Echo Tool",
                "annotations": { "title": "Echo Tool", "readOnlyHint": true },
            }),
        }
    );
    let no_props = &specs[1];
    assert_eq!(
        no_props.input_schema,
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    );
    assert_eq!(
        no_props.kind,
        ToolKind::Dynamic {
            title: Some("No Props".to_owned()),
            metadata: json!({ "clientName": "ai-sdk-mcp-client", "toolName": "no_props", "title": "No Props" }),
        }
    );
    let ToolKind::Dynamic { metadata, .. } = &specs[6].kind else {
        return Err("widget should be dynamic".into());
    };
    assert_eq!(
        metadata["app"],
        json!({ "resourceUri": "ui://widget/main", "mimeType": "text/html;profile=mcp-app" })
    );

    let leases = server
        .observed
        .leases
        .lock()
        .map(|l| l.clone())
        .unwrap_or_default();
    assert!(
        !leases.is_empty() && leases.iter().all(|lease| lease == "lease-123"),
        "{leases:?}"
    );
    source.close().await;
    Ok(())
}

#[tokio::test]
async fn calls_return_the_raw_result_and_the_model_output() -> TestResult {
    let server = start_server().await?;
    let source = connect(format!("{}/mcp", server.base), McpTransport::StreamableHttp).await?;
    let token = CancellationToken::new;

    let echo = source
        .call("echo", json!({ "text": "hi" }), token())
        .await?;
    assert_eq!(
        echo.ui,
        json!({ "content": [{ "type": "text", "text": "hi" }], "isError": false })
    );
    assert_eq!(
        serde_json::to_value(&echo.model)?,
        json!({ "type": "content", "value": [{ "type": "text", "text": "hi" }] })
    );

    // An `isError` result is an ordinary output (the wire shows `output.isError`).
    let flaky = source.call("flaky", json!({}), token()).await?;
    assert_eq!(flaky.ui["isError"], json!(true));

    // `mcpToModelOutput`: images become inline `file` data, other items their JSON text.
    let picture = source.call("picture", json!({}), token()).await?;
    let model = serde_json::to_value(&picture.model)?;
    assert_eq!(
        model["value"][0],
        json!({ "type": "file", "mediaType": "image/png", "data": { "type": "data", "data": "AAAA" } })
    );
    assert_eq!(model["value"][1]["type"], json!("text"));
    let linked: Value =
        serde_json::from_str(model["value"][1]["text"].as_str().unwrap_or_default())?;
    assert_eq!(linked["type"], json!("resource_link"));

    let Err(error) = source.call("boom", json!({}), token()).await else {
        return Err("a JSON-RPC error fails the call".into());
    };
    assert_eq!(error.0, "boom failed");

    source.close().await;
    assert!(
        source
            .call("echo", json!({ "text": "late" }), token())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn cancelling_a_call_notifies_the_server() -> TestResult {
    let server = start_server().await?;
    let source = connect(format!("{}/mcp", server.base), McpTransport::StreamableHttp).await?;
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        stopper.cancel();
    });
    let started = std::time::Instant::now();
    assert!(source.call("slow", json!({}), cancel).await.is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
    for _ in 0..100 {
        if server
            .observed
            .cancelled
            .lock()
            .map(|c| !c.is_empty())
            .unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        server
            .observed
            .cancelled
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default(),
        ["slow"]
    );
    source.close().await;
    Ok(())
}

/// A server that refuses the `initialize` POST is probed as legacy SSE and spoken to over the
/// deprecated transport; one that answers it is Streamable HTTP. Results are cached per URL.
#[tokio::test]
async fn detection_picks_the_transport_and_legacy_sse_works() -> TestResult {
    let server = start_server().await?;
    let legacy = connect(format!("{}/sse", server.base), McpTransport::Detect).await?;
    assert_eq!(legacy.tools().await?.len(), 7);
    let echo = legacy
        .call(
            "echo",
            json!({ "text": "over sse" }),
            CancellationToken::new(),
        )
        .await?;
    assert_eq!(echo.ui["content"][0]["text"], json!("over sse"));
    let Err(error) = legacy
        .call("boom", json!({}), CancellationToken::new())
        .await
    else {
        return Err("boom should fail over SSE too".into());
    };
    assert_eq!(error.0, "boom failed");
    legacy.close().await;

    // Second connect: the probe answer is cached, so no new probe reaches the server.
    let probes_before = server
        .observed
        .probes
        .lock()
        .map(|p| *p)
        .unwrap_or_default();
    assert_eq!(probes_before, 1);
    let again = connect(format!("{}/sse", server.base), McpTransport::Detect).await?;
    assert_eq!(
        server
            .observed
            .probes
            .lock()
            .map(|p| *p)
            .unwrap_or_default(),
        1
    );
    again.close().await;

    let streamable = connect(format!("{}/mcp", server.base), McpTransport::Detect).await?;
    let echo = streamable
        .call(
            "echo",
            json!({ "text": "detected" }),
            CancellationToken::new(),
        )
        .await?;
    assert_eq!(echo.ui["content"][0]["text"], json!("detected"));
    streamable.close().await;
    Ok(())
}

#[tokio::test]
async fn unreachable_servers_are_unavailable() -> TestResult {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    match connect_mcp(endpoint(
        format!("http://127.0.0.1:{port}/mcp"),
        McpTransport::StreamableHttp,
    )?)
    .await
    {
        Err(ToolSourceError::Unavailable { label, .. }) => assert_eq!(label, "browseros"),
        Err(other) => return Err(format!("expected unavailable, got {other}").into()),
        Ok(_) => return Err("connecting to a closed port succeeded".into()),
    }
    Ok(())
}

/// A server that accepts the connection and never answers hits the 15 s connect budget.
#[tokio::test(start_paused = true)]
async fn silent_servers_time_out() -> TestResult {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/mcp", listener.local_addr()?);
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    match connect_mcp(endpoint(url, McpTransport::StreamableHttp)?).await {
        Err(ToolSourceError::Timeout { label, seconds }) => {
            assert_eq!((label.as_str(), seconds), ("browseros", 15));
        }
        Err(other) => return Err(format!("expected a timeout, got {other}").into()),
        Ok(_) => return Err("a silent server connected".into()),
    }
    Ok(())
}
