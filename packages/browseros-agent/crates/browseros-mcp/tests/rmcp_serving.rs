use axum::{
    body::{Body, to_bytes},
    http::{self, Request, StatusCode, header},
};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResult, ContentBlock, Implementation,
        InitializeRequestParams, InitializeResult, ServerCapabilities,
    },
    serve_server,
    service::{NotificationContext, RequestContext},
    transport::streamable_http_server::{
        session::local::LocalSessionManager,
        tower::{StreamableHttpServerConfig, StreamableHttpService},
    },
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf, duplex},
    sync::{Mutex, mpsc},
};
use tower::ServiceExt;

#[derive(Clone)]
struct ProbeService {
    state: Arc<ProbeState>,
    instance: usize,
}

struct ProbeState {
    instances: AtomicUsize,
    events: Mutex<Vec<ProbeEvent>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProbeEvent {
    instance: usize,
    kind: &'static str,
    session_id: Option<String>,
    client_name: Option<String>,
}

impl ProbeState {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            instances: AtomicUsize::new(0),
            events: Mutex::new(Vec::new()),
        })
    }

    fn next_service(self: &Arc<Self>) -> ProbeService {
        let instance = self.instances.fetch_add(1, Ordering::SeqCst) + 1;
        ProbeService {
            state: self.clone(),
            instance,
        }
    }

    async fn push(
        &self,
        instance: usize,
        kind: &'static str,
        session_id: Option<String>,
        client_name: Option<String>,
    ) {
        self.events.lock().await.push(ProbeEvent {
            instance,
            kind,
            session_id,
            client_name,
        });
    }

    async fn events(&self) -> Vec<ProbeEvent> {
        self.events.lock().await.clone()
    }
}

impl ProbeService {
    fn instance(&self) -> usize {
        self.instance
    }
}

impl Drop for ProbeService {
    fn drop(&mut self) {
        let state = self.state.clone();
        let instance = self.instance();
        tokio::spawn(async move {
            state.push(instance, "dropped", None, None).await;
        });
    }
}

impl ServerHandler for ProbeService {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("probe", "0.0.0"))
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        context.peer.set_peer_info(request.clone());
        self.state
            .push(
                self.instance(),
                "initialize",
                session_id_from_request_context(&context),
                Some(request.client_info.name),
            )
            .await;
        Ok(self.get_info())
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        self.state
            .push(
                self.instance(),
                "initialized",
                session_id_from_notification_context(&context),
                None,
            )
            .await;
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let session_id = session_id_from_request_context(&context);
        self.state
            .push(self.instance(), "call_tool", session_id.clone(), None)
            .await;
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "{}:{}",
            request.name,
            session_id.unwrap_or_else(|| "missing".to_string())
        ))]))
    }
}

fn session_id_from_request_context(context: &RequestContext<RoleServer>) -> Option<String> {
    context
        .extensions
        .get::<http::request::Parts>()
        .and_then(|parts| session_id_from_headers(&parts.headers))
}

fn session_id_from_notification_context(
    context: &NotificationContext<RoleServer>,
) -> Option<String> {
    context
        .extensions
        .get::<http::request::Parts>()
        .and_then(|parts| session_id_from_headers(&parts.headers))
}

fn session_id_from_headers(headers: &http::HeaderMap) -> Option<String> {
    headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn streamable_service(
    state: Arc<ProbeState>,
) -> StreamableHttpService<ProbeService, LocalSessionManager> {
    StreamableHttpService::new(
        move || Ok(state.next_service()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    )
}

async fn post_json(
    service: &StreamableHttpService<ProbeService, LocalSessionManager>,
    session_id: Option<&str>,
    body: Value,
) -> anyhow::Result<(StatusCode, http::HeaderMap, Value)> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    if let Some(session_id) = session_id {
        builder = builder
            .header("mcp-session-id", session_id)
            .header("mcp-protocol-version", "2025-06-18");
    }
    let response = service
        .clone()
        .oneshot(builder.body(Body::from(body.to_string()))?)
        .await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(Body::new(response.into_body()), usize::MAX).await?;
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        sse_json(&String::from_utf8(bytes.to_vec())?)?
    };
    Ok((status, headers, body))
}

async fn delete_session(
    service: &StreamableHttpService<ProbeService, LocalSessionManager>,
    session_id: &str,
) -> anyhow::Result<StatusCode> {
    let response = service
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/mcp")
                .header(header::HOST, "localhost")
                .header("mcp-session-id", session_id)
                .header("mcp-protocol-version", "2025-06-18")
                .body(Body::empty())?,
        )
        .await?;
    Ok(response.status())
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
    Err(anyhow::anyhow!("SSE response had no data line: {body:?}"))
}

fn initialize_request(id: i64, client_name: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {
                "name": client_name,
                "version": "1"
            }
        }
    })
}

#[tokio::test]
async fn streamable_http_uses_per_session_factory_and_exposes_session_id_after_initialize()
-> anyhow::Result<()> {
    let state = ProbeState::new();
    let service = streamable_service(state.clone());

    let (status, headers, initialize) =
        post_json(&service, None, initialize_request(1, "client-a")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(initialize["id"], 1);
    let session_a = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("missing first session id"))?
        .to_string();

    let (status, _, _) = post_json(
        &service,
        Some(&session_a),
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, _, call) = post_json(
        &service,
        Some(&session_a),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "probe", "arguments": {} }
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(call["id"], 2);
    assert_eq!(
        call["result"]["content"][0]["text"],
        format!("probe:{session_a}")
    );

    let (status, headers, _) = post_json(&service, None, initialize_request(3, "client-b")).await?;
    assert_eq!(status, StatusCode::OK);
    let session_b = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("missing second session id"))?
        .to_string();
    assert_ne!(session_a, session_b);
    assert_eq!(state.instances.load(Ordering::SeqCst), 2);

    let events = state.events().await;
    assert!(events.contains(&ProbeEvent {
        instance: 1,
        kind: "initialize",
        session_id: None,
        client_name: Some("client-a".to_string()),
    }));
    assert!(events.contains(&ProbeEvent {
        instance: 1,
        kind: "initialized",
        session_id: Some(session_a.clone()),
        client_name: None,
    }));
    assert!(events.contains(&ProbeEvent {
        instance: 1,
        kind: "call_tool",
        session_id: Some(session_a),
        client_name: None,
    }));
    assert!(events.contains(&ProbeEvent {
        instance: 2,
        kind: "initialize",
        session_id: None,
        client_name: Some("client-b".to_string()),
    }));

    Ok(())
}

#[tokio::test]
async fn streamable_http_delete_drives_service_close_signal() -> anyhow::Result<()> {
    let state = ProbeState::new();
    let service = streamable_service(state.clone());
    let (_, headers, _) = post_json(&service, None, initialize_request(1, "client-a")).await?;
    let session_id = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("missing session id"))?
        .to_string();

    assert_eq!(
        delete_session(&service, &session_id).await?,
        StatusCode::ACCEPTED
    );

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if state
                .events()
                .await
                .iter()
                .any(|event| event.kind == "dropped")
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;

    Ok(())
}

#[tokio::test]
async fn stdio_transport_serves_the_same_server_handler() -> anyhow::Result<()> {
    let state = ProbeState::new();
    let service = state.next_service();
    let (client_side, server_side) = duplex(16 * 1024);
    let (server_read, server_write) = tokio::io::split(server_side);
    let server_task = tokio::spawn(async move {
        let running = serve_server(service, (server_read, server_write)).await?;
        running.waiting().await?;
        anyhow::Ok(())
    });

    let (client_read, client_write) = tokio::io::split(client_side);
    let (tx, mut rx) = mpsc::channel(8);
    let reader_task = tokio::spawn(read_stdio_responses(client_read, tx));
    let mut client_write = client_write;
    write_stdio(&mut client_write, initialize_request(1, "stdio-client")).await?;
    let initialize = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing initialize response"))?;
    assert_eq!(initialize["id"], 1);
    assert_eq!(initialize["result"]["serverInfo"]["name"], "probe");

    drop(client_write);
    server_task.abort();
    reader_task.abort();
    let _ = server_task.await;
    let _ = reader_task.await;

    let events = state.events().await;
    assert!(events.contains(&ProbeEvent {
        instance: 1,
        kind: "initialize",
        session_id: None,
        client_name: Some("stdio-client".to_string()),
    }));

    Ok(())
}

async fn write_stdio(
    writer: &mut WriteHalf<tokio::io::DuplexStream>,
    message: Value,
) -> anyhow::Result<()> {
    writer.write_all(message.to_string().as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

async fn read_stdio_responses(
    reader: ReadHalf<tokio::io::DuplexStream>,
    tx: mpsc::Sender<Value>,
) -> anyhow::Result<()> {
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        let parsed = serde_json::from_str::<Value>(&line)?;
        tx.send(parsed).await?;
    }
    Ok(())
}
