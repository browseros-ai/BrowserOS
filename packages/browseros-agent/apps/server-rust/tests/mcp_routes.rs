//! `/mcp` against TS behaviour: the routes as `build_router` mounts them (status JSON, plain
//! 404 DELETE, browser-fetch 403, any Host, SDK transport rejections) and the shared host
//! with server-rust's policy (lease admission, read-only catalog, run guard, structured
//! opt-in, side-panel association, connector gate, scheduled background pages) over the
//! host crate's synthetic browser peer.

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::Request,
    routing::post,
};
use browseros_mcp_host::{
    AdmittedCaller, HostCore, HostCoreConfig, LocalToolProvider, SessionSpec,
    identity::ClientInfo,
    seams::LocalToolCall,
    test_support::{self, ProfileConnection},
};
use futures_util::future::BoxFuture;
use http::{HeaderMap, StatusCode, header};
use rmcp::model::{CallToolResult, ContentBlock, Tool};
use serde_json::{Value, json};
use server_rust::{
    AppState,
    chat::{ActiveRun, BackgroundPages, TabResolver},
    config::{Config, EnvConfig},
    klavis::ScopeResolver,
    mcp::{
        HostPages, HostTabs, LeaseScope, LeaseSource, LeaseSpec, MAX_BODY_BYTES, McpQuery,
        McpServer, NO_ACTIVE_RUN, RunRegistry, ToolLeases, connector_scope,
        conversation_of_session,
    },
};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;
use uuid::Uuid;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const LEGACY: &str = "2025-06-18";
const READ_ONLY_CATALOG: [&str; 9] = [
    "tabs",
    "history",
    "snapshot",
    "diff",
    "read",
    "grep",
    "screenshot",
    "pdf",
    "wait",
];

// ─── Harness ───

/// A run registry the test drives: which conversations are running, and every
/// side-panel association the host asked for.
#[derive(Default)]
struct Runs {
    active: Mutex<HashMap<Uuid, ActiveRun>>,
    associated: Mutex<Vec<(Uuid, Vec<u32>)>>,
}

impl Runs {
    fn start(&self, conversation_id: Uuid) -> ActiveRun {
        let run = ActiveRun {
            conversation_id,
            run_id: Uuid::new_v4(),
            panels_visible: true,
            tab_group: None,
        };
        self.lock_active().insert(conversation_id, run.clone());
        run
    }

    fn lock_active(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, ActiveRun>> {
        self.active.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn associated(&self) -> Vec<(Uuid, Vec<u32>)> {
        self.associated
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

impl RunRegistry for Runs {
    fn active_run(&self, conversation_id: Uuid) -> Option<ActiveRun> {
        self.lock_active().get(&conversation_id).cloned()
    }

    fn associate_tabs(&self, run: &ActiveRun, tab_ids: &[u32]) -> bool {
        self.associated
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((run.run_id, tab_ids.to_vec()));
        true
    }
}

/// A Klavis-shaped provider: it asks the server's `connector_scope` for the caller's policy
/// on every call, as Klavis's own provider does, and echoes what it was told.
struct Connectors(ScopeResolver);

impl LocalToolProvider for Connectors {
    fn tools<'a>(&'a self, _caller: Option<&'a AdmittedCaller>) -> BoxFuture<'a, Vec<Tool>> {
        Box::pin(async {
            let Value::Object(schema) = json!({ "type": "object", "properties": {} }) else {
                unreachable!();
            };
            vec![Tool::new(
                "connector_mcp_servers",
                "List managed connectors.",
                Arc::new(schema),
            )]
        })
    }

    fn call<'a>(&'a self, name: &'a str, call: LocalToolCall<'a>) -> BoxFuture<'a, CallToolResult> {
        Box::pin(async move {
            let scope = (self.0)(call.caller);
            if let Some(rejection) = scope.rejection {
                return CallToolResult::error(vec![ContentBlock::text(rejection)]);
            }
            CallToolResult::success(vec![ContentBlock::text(format!(
                "{name} served {} for {:?}",
                call.session.id(),
                scope.selected_server_names
            ))])
        })
    }
}

struct Fixture {
    router: Router,
    server: Arc<McpServer>,
    leases: Arc<ToolLeases>,
    runs: Arc<Runs>,
    _dir: tempfile::TempDir,
}

/// The `/mcp` POST handler over a host on the synthetic browser (or none), driven by `Runs`.
async fn fixture(browser: Option<Arc<ProfileConnection>>) -> Result<Fixture> {
    let dir = tempfile::tempdir()?;
    let core = HostCore::open(HostCoreConfig {
        cdp_port: 9,
        data_root: dir.path().join("mcp-host"),
        native: test_support::current_native(),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(600),
        session_sweep_interval: Duration::from_secs(60),
    })
    .await?;
    let runs = Arc::new(Runs::default());
    let leases = ToolLeases::with_registry(runs.clone());
    let connectors = Connectors(connector_scope(leases.runs().clone()));
    let server = Arc::new(McpServer::on_core(
        core,
        leases.clone(),
        Some(Arc::new(connectors)),
    ));
    if let Some(browser) = browser {
        let session = test_support::current_session(browser);
        session.pages.list().await?;
        server
            .host()
            .browser()
            .set_session_for_testing(session)
            .await;
    }
    let handler = server.clone();
    let router = Router::new().route(
        "/mcp",
        post(move |request: Request| {
            let server = handler.clone();
            async move { server.post(request).await }
        }),
    );
    Ok(Fixture {
        router,
        server,
        leases,
        runs,
        _dir: dir,
    })
}

fn spec(conversation_id: Uuid, read_only: bool) -> LeaseSpec {
    LeaseSpec {
        conversation_id,
        read_only,
        source: LeaseSource::Chat,
        client_slug: "browseros",
        task_label: "Compare flight prices".to_owned(),
        auto_group: true,
        browser_context: None,
    }
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
    text: String,
}

/// A 2025-era MCP POST as the engines' loopback clients send it.
fn request(uri: &str, lease: Option<&str>, body: impl Into<Body>) -> Result<Request> {
    let mut builder = Request::post(uri)
        .header(header::HOST, "127.0.0.1:9200")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("mcp-protocol-version", LEGACY);
    if let Some(lease) = lease {
        builder = builder.header("X-BrowserOS-Internal-Lease", lease);
    }
    Ok(builder.body(body.into())?)
}

async fn send(router: &Router, request: Request) -> Result<Reply> {
    let response = router.clone().oneshot(request).await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let text = String::from_utf8(bytes.to_vec())?;
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok(Reply {
        status,
        headers,
        body,
        text,
    })
}

async fn rpc(router: &Router, uri: &str, lease: Option<&str>, message: Value) -> Result<Reply> {
    send(router, request(uri, lease, message.to_string())?).await
}

fn list() -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} })
}

fn call(name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    })
}

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": LEGACY,
            "capabilities": {},
            "clientInfo": { "name": "acp-client", "version": "1.0" }
        }
    })
}

fn tool_names(reply: &Reply) -> Vec<String> {
    reply.body["result"]["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
        .collect()
}

fn text_of(reply: &Reply) -> String {
    reply.body["result"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn rpc_error(code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "error": { "code": code, "message": message }, "id": null })
}

/// The real app with no browser behind it (`cdp` 9 refuses, so a developer's running
/// BrowserOS on its default CDP port is never touched).
async fn app(dir: &Path) -> Result<(AppState, Router)> {
    let path = dir.join("config.json");
    std::fs::write(
        &path,
        r#"{"ports":{"server":9200,"cdp":9},"directories":{"resources":".","execution":"."},"instance":{"browseros_version":"0.51.1.3"}}"#,
    )?;
    let config = Config::load(
        &path,
        &EnvConfig {
            browseros_dir: Some(dir.join("data")),
            ..Default::default()
        },
    )?;
    let state = AppState::start(config).await?;
    Ok((state.clone(), server_rust::build_router(state)))
}

// ─── The routes as mounted ───

#[tokio::test]
async fn get_is_status_json_delete_is_plain_404_and_browser_fetches_are_refused() -> Result {
    let dir = tempfile::tempdir()?;
    let (state, router) = app(dir.path()).await?;

    let reply = send(&router, Request::get("/mcp").body(Body::empty())?).await?;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.body,
        json!({"status":"ok","message":"MCP server is running. Use POST to interact."})
    );

    let reply = send(&router, Request::delete("/mcp").body(Body::empty())?).await?;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.text, "404 Not Found");
    assert_eq!(
        reply.headers.get(header::CONTENT_TYPE),
        Some(&http::HeaderValue::from_static("text/plain; charset=UTF-8"))
    );

    // A browser fetch is refused before the lease is even looked at.
    let mut forged = request("/mcp", Some("forged"), list().to_string())?;
    forged.headers_mut().insert(
        "sec-fetch-site",
        http::HeaderValue::from_static("cross-site"),
    );
    let reply = send(&router, forged).await?;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.body["error"]["code"], "FORBIDDEN_BROWSER_REQUEST");

    // No browser on the CDP port: health stays 200 and says so.
    let reply = send(&router, Request::get("/system/health").body(Body::empty())?).await?;
    assert_eq!(reply.body["cdpConnected"], false);
    state.stop().await;
    Ok(())
}

#[tokio::test]
async fn an_unknown_lease_is_401_with_jsonrpc_32001() -> Result {
    let dir = tempfile::tempdir()?;
    let (state, router) = app(dir.path()).await?;
    for body in [list().to_string(), String::new(), "not json".to_owned()] {
        let reply = send(&router, request("/mcp", Some("forged"), body)?).await?;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            reply.body,
            rpc_error(-32001, "Invalid or expired BrowserOS tool lease")
        );
    }
    // A revoked lease is as unknown as a forged one.
    let lease = state.leases.mint(spec(Uuid::new_v4(), false)).await?;
    state.leases.revoke(&lease);
    let reply = rpc(&router, "/mcp", Some(lease.token()), list()).await?;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    // A blank header is no lease at all (TS trims it away).
    let reply = rpc(&router, "/mcp", Some("  "), list()).await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    state.stop().await;
    Ok(())
}

#[tokio::test]
async fn any_host_gets_single_json_replies_with_browseros_identity() -> Result {
    let dir = tempfile::tempdir()?;
    let (state, router) = app(dir.path()).await?;
    let mut lan = request("/mcp", None, initialize().to_string())?;
    lan.headers_mut().insert(
        header::HOST,
        http::HeaderValue::from_static("192.168.1.20:9200"),
    );
    let reply = send(&router, lan).await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(
        reply.headers.get(header::CONTENT_TYPE),
        Some(&http::HeaderValue::from_static("application/json"))
    );
    assert!(reply.headers.get("mcp-session-id").is_none());
    let result = &reply.body["result"];
    assert_eq!(result["serverInfo"]["name"], "browseros_mcp");
    assert_eq!(result["serverInfo"]["title"], "BrowserOS MCP server");
    let instructions = result["instructions"].as_str().unwrap_or_default();
    assert!(instructions.starts_with("BrowserOS MCP Server — compact browser automation"));
    assert!(instructions.contains("## External Integrations (Klavis Strata)"));
    assert!(
        instructions.contains("browser_profiles"),
        "profile guidance for 0.51.1.3"
    );
    state.stop().await;
    Ok(())
}

#[tokio::test]
async fn sdk_transport_rejections_keep_their_status_and_jsonrpc_bodies() -> Result {
    let dir = tempfile::tempdir()?;
    let (state, router) = app(dir.path()).await?;
    let not_acceptable = rpc_error(
        -32000,
        "Not Acceptable: Client must accept both application/json and text/event-stream",
    );
    let unsupported = rpc_error(
        -32000,
        "Unsupported Media Type: Content-Type must be application/json",
    );
    let too_large = rpc_error(
        -32000,
        &format!("Payload Too Large: Request body must not exceed {MAX_BODY_BYTES} bytes"),
    );

    let mut json_only = request("/mcp", None, list().to_string())?;
    json_only.headers_mut().insert(
        header::ACCEPT,
        http::HeaderValue::from_static("application/json"),
    );
    let reply = send(&router, json_only).await?;
    assert_eq!(
        (reply.status, reply.body),
        (StatusCode::NOT_ACCEPTABLE, not_acceptable.clone())
    );

    let mut text = request("/mcp", None, list().to_string())?;
    text.headers_mut().insert(
        header::CONTENT_TYPE,
        http::HeaderValue::from_static("text/plain"),
    );
    let reply = send(&router, text).await?;
    assert_eq!(
        (reply.status, reply.body),
        (StatusCode::UNSUPPORTED_MEDIA_TYPE, unsupported.clone())
    );
    // The legacy transport checks Accept before Content-Type.
    let mut both = request("/mcp", None, list().to_string())?;
    both.headers_mut().insert(
        header::CONTENT_TYPE,
        http::HeaderValue::from_static("text/plain"),
    );
    both.headers_mut()
        .insert(header::ACCEPT, http::HeaderValue::from_static("*/*"));
    assert_eq!(send(&router, both).await?.body, not_acceptable);

    // Media-type parameters and case do not matter, as in the SDK's isJsonContentType.
    let mut charset = request("/mcp", None, list().to_string())?;
    charset.headers_mut().insert(
        header::CONTENT_TYPE,
        http::HeaderValue::from_static("Application/JSON; charset=utf-8"),
    );
    assert_eq!(send(&router, charset).await?.status, StatusCode::OK);

    // Oversized: declared or streamed. The size check precedes Accept (the SDK routes an
    // oversized body to its modern leg), and that leg's 415 precedes its 413.
    let oversized = "x".repeat(MAX_BODY_BYTES + 1);
    let reply = send(&router, request("/mcp", None, oversized.clone())?).await?;
    assert_eq!(
        (reply.status, reply.body),
        (StatusCode::PAYLOAD_TOO_LARGE, too_large.clone())
    );
    let mut declared = request("/mcp", None, list().to_string())?;
    declared.headers_mut().insert(
        header::CONTENT_LENGTH,
        http::HeaderValue::from_str(&(MAX_BODY_BYTES + 1).to_string())?,
    );
    declared
        .headers_mut()
        .insert(header::ACCEPT, http::HeaderValue::from_static("*/*"));
    assert_eq!(send(&router, declared).await?.body, too_large);
    let mut oversized_text = request("/mcp", None, oversized)?;
    oversized_text.headers_mut().insert(
        header::CONTENT_TYPE,
        http::HeaderValue::from_static("text/plain"),
    );
    assert_eq!(send(&router, oversized_text).await?.body, unsupported);

    for (body, status, expected) in [
        (
            String::new(),
            StatusCode::BAD_REQUEST,
            rpc_error(-32700, "Parse error: Invalid JSON"),
        ),
        (
            "{".to_owned(),
            StatusCode::BAD_REQUEST,
            rpc_error(-32700, "Parse error: Invalid JSON"),
        ),
        (
            json!({ "hello": "world" }).to_string(),
            StatusCode::BAD_REQUEST,
            rpc_error(-32700, "Parse error: Invalid JSON-RPC message"),
        ),
        (
            Value::Array(vec![list(); 101]).to_string(),
            StatusCode::BAD_REQUEST,
            rpc_error(
                -32600,
                "Invalid Request: Batch must not exceed 100 messages",
            ),
        ),
    ] {
        let reply = send(&router, request("/mcp", None, body.clone())?).await?;
        assert_eq!((reply.status, reply.body), (status, expected), "{body:.40}");
    }

    // A notification is accepted without a body.
    let reply = rpc(
        &router,
        "/mcp",
        None,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await?;
    assert_eq!(reply.status, StatusCode::ACCEPTED);
    state.stop().await;
    Ok(())
}

#[tokio::test]
async fn the_app_admits_its_own_leases_on_a_bound_session() -> Result {
    let dir = tempfile::tempdir()?;
    let (state, router) = app(dir.path()).await?;
    let conversation = Uuid::new_v4();
    let lease = state.leases.mint(spec(conversation, false)).await?;
    let reply = rpc(
        &router,
        "/mcp",
        Some(lease.token()),
        call("tabs", json!({ "action": "list" })),
    )
    .await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.body["result"]["isError"], true);
    // No browser behind the app: the host's browser guard answers before the run guard.
    assert!(text_of(&reply).contains("no link to the browser yet"));
    assert!(
        reply.body["result"].get("_meta").is_none(),
        "bound callers get no handle"
    );
    assert!(
        state
            .host
            .sessions()
            .lookup(&browseros_mcp_host::ids::SessionId::new(format!(
                "bound-{conversation}"
            )))
            .await
            .is_some()
    );
    state.stop().await;
    Ok(())
}

// ─── Policy over the shared host ───

#[tokio::test]
async fn read_only_comes_from_the_query_or_the_lease_and_never_weakens() -> Result {
    let fixture = fixture(None).await?;
    let conversation = Uuid::new_v4();
    fixture.runs.start(conversation);
    let reader = fixture.leases.mint(spec(conversation, true)).await?;
    let writer = fixture.leases.mint(spec(conversation, false)).await?;
    let mut read_only = READ_ONLY_CATALOG.map(str::to_owned).to_vec();
    read_only.push("connector_mcp_servers".to_owned());

    for (uri, lease) in [
        ("/mcp?read_only=1", None),
        ("/mcp?read_only=1&read_only=0", None),
        ("/mcp", Some(reader.token())),
        ("/mcp?read_only=0", Some(reader.token())),
        ("/mcp?read_only=1", Some(writer.token())),
    ] {
        let reply = rpc(&fixture.router, uri, lease, list()).await?;
        assert_eq!(tool_names(&reply), read_only, "{uri} {lease:?}");
        let tabs = &reply.body["result"]["tools"][0];
        assert_eq!(
            tabs["inputSchema"]["properties"]["action"]["enum"],
            json!(["list", "active"])
        );
    }
    let full = rpc(&fixture.router, "/mcp", Some(writer.token()), list()).await?;
    let names = tool_names(&full);
    assert!(names.contains(&"navigate".to_owned()) && names.contains(&"run".to_owned()));
    // Lease callers are bound: no session handle argument is offered.
    assert_eq!(
        full.body["result"]["tools"][0]["inputSchema"]["properties"]["session"],
        Value::Null
    );

    let reply = rpc(
        &fixture.router,
        "/mcp",
        Some(reader.token()),
        call("navigate", json!({ "url": "https://example.com" })),
    )
    .await?;
    assert_eq!(text_of(&reply), "navigate: unavailable in read-only mode.");
    Ok(())
}

#[tokio::test]
async fn the_run_guard_follows_the_conversation_not_the_lease() -> Result {
    let fixture = fixture(Some(ProfileConnection::new().with_default_profile())).await?;
    let conversation = Uuid::new_v4();
    let lease = fixture.leases.mint(spec(conversation, false)).await?;
    let tabs = call("tabs", json!({ "action": "list" }));

    let reply = rpc(&fixture.router, "/mcp", Some(lease.token()), tabs.clone()).await?;
    assert_eq!(text_of(&reply), NO_ACTIVE_RUN);
    let reply = rpc(
        &fixture.router,
        "/mcp",
        Some(lease.token()),
        call("connector_mcp_servers", json!({})),
    )
    .await?;
    assert_eq!(text_of(&reply), NO_ACTIVE_RUN);

    // Once the conversation runs, the same lease works.
    fixture.runs.start(conversation);
    let reply = rpc(&fixture.router, "/mcp", Some(lease.token()), tabs.clone()).await?;
    assert_eq!(reply.body["result"]["isError"], false, "{}", reply.text);
    let reply = rpc(
        &fixture.router,
        "/mcp",
        Some(lease.token()),
        call("connector_mcp_servers", json!({})),
    )
    .await?;
    assert_eq!(
        text_of(&reply),
        format!("connector_mcp_servers served bound-{conversation} for []")
    );

    // When it stops, the lease is refused again; external clients never need a run.
    fixture.runs.lock_active().clear();
    let reply = rpc(&fixture.router, "/mcp", Some(lease.token()), tabs.clone()).await?;
    assert_eq!(text_of(&reply), NO_ACTIVE_RUN);
    let reply = rpc(&fixture.router, "/mcp", None, tabs).await?;
    assert_eq!(reply.body["result"]["isError"], false, "{}", reply.text);
    Ok(())
}

#[tokio::test]
async fn connector_scope_carries_the_run_refusal_and_selected_connectors() -> Result {
    let fixture = fixture(None).await?;
    let scope = connector_scope(fixture.leases.runs().clone());
    let conversation = Uuid::new_v4();
    let lease = fixture.leases.mint(spec(conversation, false)).await?;
    let context: server_rust::chat::request::BrowserContext =
        serde_json::from_value(json!({ "enabledMcpServers": ["Gmail", "Slack"] }))?;
    lease.update_browser_context(Some(&context));
    let caller = fixture
        .leases
        .resolve(Some(lease.token()), McpQuery::default())?;

    let stopped = scope(caller.as_ref());
    assert_eq!(stopped.rejection.as_deref(), Some(NO_ACTIVE_RUN));
    assert_eq!(stopped.selected_server_names, ["Gmail", "Slack"]);
    fixture.runs.start(conversation);
    assert_eq!(scope(caller.as_ref()).rejection, None);

    // External clients select nothing and need no run.
    let anonymous = scope(None);
    assert!(anonymous.rejection.is_none() && anonymous.selected_server_names.is_empty());

    // Through /mcp, the provider sees the same lease policy.
    let reply = rpc(
        &fixture.router,
        "/mcp",
        Some(lease.token()),
        call("connector_mcp_servers", json!({})),
    )
    .await?;
    assert_eq!(
        text_of(&reply),
        format!(r#"connector_mcp_servers served bound-{conversation} for ["Gmail", "Slack"]"#)
    );
    Ok(())
}

#[tokio::test]
async fn structured_content_is_an_opt_in() -> Result {
    let fixture = fixture(Some(ProfileConnection::new().with_default_profile())).await?;
    let tabs = call("tabs", json!({ "action": "list" }));
    let plain = rpc(&fixture.router, "/mcp", None, tabs.clone()).await?;
    assert_eq!(plain.body["result"]["isError"], false, "{}", plain.text);
    assert!(plain.body["result"].get("structuredContent").is_none());
    let structured = rpc(&fixture.router, "/mcp?structured=1", None, tabs).await?;
    assert!(
        structured.body["result"]["structuredContent"]["pages"].is_array(),
        "{}",
        structured.text
    );
    Ok(())
}

#[tokio::test]
async fn a_tab_a_run_opens_joins_its_side_panel() -> Result {
    let native = ProfileConnection::new().with_default_profile();
    let fixture = fixture(Some(native.clone())).await?;
    let conversation = Uuid::new_v4();
    let run = fixture.runs.start(conversation);
    let lease = fixture.leases.mint(spec(conversation, false)).await?;

    let reply = rpc(
        &fixture.router,
        "/mcp",
        Some(lease.token()),
        call(
            "tabs",
            json!({ "action": "new", "url": "https://example.com" }),
        ),
    )
    .await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    let created = native
        .tabs
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .find(|tab| tab["url"] == "https://example.com")
        .and_then(|tab| tab["tabId"].as_u64())
        .and_then(|tab| u32::try_from(tab).ok())
        .ok_or("no tab was created")?;
    assert_eq!(fixture.runs.associated(), vec![(run.run_id, vec![created])]);

    // Reading the user's tab associates nothing; an external client's new tab neither.
    rpc(
        &fixture.router,
        "/mcp",
        Some(lease.token()),
        call("tabs", json!({ "action": "list" })),
    )
    .await?;
    rpc(
        &fixture.router,
        "/mcp",
        None,
        call(
            "tabs",
            json!({ "action": "new", "url": "https://example.org" }),
        ),
    )
    .await?;
    assert_eq!(fixture.runs.associated().len(), 1);
    Ok(())
}

// ─── Leases and admission ───

#[tokio::test]
async fn a_lease_admits_its_conversation_context() -> Result {
    let fixture = fixture(None).await?;
    let conversation = Uuid::new_v4();
    let lease = fixture.leases.mint(spec(conversation, false)).await?;
    let context: server_rust::chat::request::BrowserContext = serde_json::from_value(json!({
        "windowId": 7,
        "activeTab": { "id": 501, "pageId": 3 },
        "selectedTabs": [{ "id": 502, "pageId": 4 }, { "id": 503 }],
        "enabledMcpServers": ["Gmail"],
    }))?;
    lease.update_browser_context(Some(&context));

    let caller = fixture
        .leases
        .resolve(
            Some(lease.token()),
            McpQuery {
                read_only: false,
                structured: true,
            },
        )?
        .ok_or("lease admitted as anonymous")?;
    assert!(caller.structured_content && !caller.read_only && caller.auto_group);
    assert_eq!(caller.default_window_id.map(|window| window.0), Some(7));
    assert_eq!(caller.lent_pages, BTreeSet::from([3, 4]));
    let session = caller.session.clone().ok_or("lease is not bound")?;
    assert_eq!(
        session.session_id().as_str(),
        format!("bound-{conversation}")
    );
    assert_eq!(session.client.name, "browseros");
    assert_eq!(session.label.as_deref(), Some("Compare flight prices"));
    let scope = caller.host_data::<LeaseScope>().ok_or("no lease scope")?;
    assert_eq!(scope.conversation_id(), conversation);
    assert_eq!(scope.enabled_mcp_servers(), ["Gmail".to_owned()]);

    // Anonymous callers carry only the query flags.
    assert!(fixture.leases.resolve(None, McpQuery::default())?.is_none());
    let anonymous = fixture
        .leases
        .resolve(
            None,
            McpQuery {
                read_only: true,
                structured: false,
            },
        )?
        .ok_or("query flags dropped")?;
    assert!(anonymous.read_only && anonymous.session.is_none());
    Ok(())
}

#[tokio::test]
async fn output_grants_survive_a_lease_re_mint() -> Result {
    let fixture = fixture(None).await?;
    let conversation = Uuid::new_v4();
    let first = fixture.leases.mint(spec(conversation, false)).await?;
    let caller = fixture
        .leases
        .resolve(Some(first.token()), McpQuery::default())?
        .ok_or("not admitted")?;
    let written = PathBuf::from("/tmp/browseros-tool-output/pdf-1.pdf");
    browseros_mcp::output_file::record_browser_output_file(
        caller.output_files.as_ref().ok_or("no grants")?,
        written.clone(),
    )
    .await;
    // A session rebuild revokes and re-mints; the conversation keeps what it wrote.
    fixture.leases.revoke(&first);
    let second = fixture.leases.mint(spec(conversation, true)).await?;
    assert!(second.output_grants().allows(&written));
    assert!(!second.output_grants().allows(Path::new("/etc/passwd")));
    let other = fixture.leases.mint(spec(Uuid::new_v4(), false)).await?;
    assert!(!other.output_grants().allows(&written));
    Ok(())
}

#[test]
fn bound_session_ids_map_back_to_their_conversation() {
    let conversation = Uuid::new_v4();
    let spec = SessionSpec {
        key: conversation.to_string(),
        client: ClientInfo {
            name: "claude".to_owned(),
            version: "1".to_owned(),
            title: None,
        },
        label: None,
    };
    assert_eq!(
        conversation_of_session(spec.session_id().as_str()),
        Some(conversation)
    );
    assert_eq!(conversation_of_session("s1"), None);
    assert_eq!(conversation_of_session("bound-not-a-uuid"), None);
}

// ─── Scheduled runs' background page ───

#[tokio::test]
async fn host_pages_open_find_and_close_a_background_page() -> Result {
    let native = ProfileConnection::new().with_default_profile();
    let online = fixture(Some(native.clone())).await?;
    let pages = HostPages(online.server.host().clone());
    let page = pages.open_background("about:blank").await?;
    let create = native
        .calls
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .find(|(method, _)| method == "Browser.createTab")
        .map(|(_, params)| params.clone())
        .ok_or("no tab created")?;
    assert_eq!(create["url"], "about:blank");
    assert_eq!(create["background"], true);
    assert!(pages.window_of(page).await?.is_some());
    // The synthetic peer does not model closeTab; the request itself is the contract.
    let _ = pages.close(page).await;
    let closed = native
        .calls
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .find(|(method, _)| method == "Browser.closeTab")
        .map(|(_, params)| params["tabId"].clone());
    assert_eq!(
        closed,
        Some(json!(105)),
        "the created tab (fixture tabs are 101-104)"
    );

    // No browser: every operation fails, so the engine falls back to the request context.
    let offline = fixture(None).await?;
    let offline_pages = HostPages(offline.server.host().clone());
    assert!(offline_pages.open_background("about:blank").await.is_err());
    assert!(
        HostTabs(offline.server.host().clone())
            .resolve_tab_ids(&[101])
            .await
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn host_tabs_resolve_chrome_tab_ids_to_page_ids() -> Result {
    let fixture = fixture(Some(ProfileConnection::new().with_default_profile())).await?;
    let browser = fixture
        .server
        .host()
        .browser()
        .session()
        .await
        .ok_or("no browser")?;
    let expected: HashMap<i64, u32> = browser
        .pages
        .list()
        .await?
        .into_iter()
        .filter(|page| [101, 104].contains(&page.tab_id.0))
        .map(|page| (page.tab_id.0, page.page_id.0))
        .collect();
    assert_eq!(expected.len(), 2);
    let resolved = HostTabs(fixture.server.host().clone())
        .resolve_tab_ids(&[101, 104, 999])
        .await;
    assert_eq!(resolved, expected, "unknown tabs are simply absent");
    Ok(())
}
