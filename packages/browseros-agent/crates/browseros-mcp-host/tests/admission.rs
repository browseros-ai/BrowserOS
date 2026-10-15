//! Caller admission end to end: a host maps its own header to an `AdmittedCaller` and the
//! shared layer honours it on every transport — refused leases, the read-only catalog,
//! one bound session across stateful SSE and stateless JSON, runtime provider tools, the
//! structured-content opt-in, auto-grouping off, and lent pages.

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use browseros_core::PageId;
use browseros_mcp::ToolResult;
use browseros_mcp_host::{
    Admission, AdmittedCaller, HostCore, HostCoreConfig, HostCtx, LocalToolProvider,
    McpRouteConfig, SessionSpec,
    identity::ClientInfo,
    ids::SessionId,
    mcp::{
        dispatch::{NamedToolEffect, NamedToolObserver, ToolGuard, dispatch_tool_call_with},
        effects::{page_ownership_notice, tab_groups},
    },
    mcp_router,
    seams::LocalToolCall,
    test_support::{self, ProfileConnection},
};
use futures_util::future::BoxFuture;
use rmcp::model::{CallToolResult, ContentBlock, Tool};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
    time::Duration,
};
use tower::ServiceExt;

const LEASE_HEADER: &str = "x-test-lease";
const LEGACY: &str = "2025-06-18";

/// The BrowserOS server's shape in miniature: an opaque header names a lease, an unknown
/// one is refused with JSON-RPC -32001, no header is an anonymous client.
struct LeaseAdmission(HashMap<String, AdmittedCaller>);

impl Admission for LeaseAdmission {
    fn admit(&self, request: &Parts) -> Result<Option<AdmittedCaller>, Box<Response>> {
        let Some(token) = request
            .headers
            .get(LEASE_HEADER)
            .and_then(|value| value.to_str().ok())
        else {
            return Ok(None);
        };
        self.0.get(token).cloned().map(Some).ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                axum::Json(json!({
                    "jsonrpc": "2.0",
                    "error": { "code": -32001, "message": "Invalid or expired BrowserOS tool lease" },
                    "id": null
                })),
            )
                .into_response()
                .into()
        })
    }
}

/// A runtime tool source (Klavis-like): one connector tool, refused to read-only callers.
struct ConnectorProvider;

impl LocalToolProvider for ConnectorProvider {
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
            if call.caller.is_some_and(|caller| caller.read_only) {
                return CallToolResult::error(vec![ContentBlock::text(
                    "Managed connector tools are unavailable in read-only mode.",
                )]);
            }
            CallToolResult::success(vec![ContentBlock::text(format!(
                "{name} served session {}",
                call.session.id()
            ))])
        })
    }
}

async fn host() -> anyhow::Result<(Arc<HostCtx>, tempfile::TempDir)> {
    let dir = tempfile::tempdir()?;
    let core = HostCore::open(HostCoreConfig {
        cdp_port: 9,
        data_root: dir.path().join("host"),
        native: test_support::current_native(),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(600),
        session_sweep_interval: Duration::from_secs(60),
    })
    .await?;
    let host = HostCtx::builder(core)
        .tool_provider(Arc::new(ConnectorProvider))
        .build();
    Ok((host, dir))
}

fn conversation(key: &str) -> Arc<SessionSpec> {
    Arc::new(SessionSpec {
        key: key.to_string(),
        client: ClientInfo {
            name: "BrowserOS".to_string(),
            version: "1.0".to_string(),
            title: None,
        },
        label: Some("Compare flight prices".to_string()),
    })
}

fn routers(host: &Arc<HostCtx>, leases: &[(&str, AdmittedCaller)]) -> (Router, Router) {
    let admission: Arc<dyn Admission> = Arc::new(LeaseAdmission(
        leases
            .iter()
            .map(|(token, caller)| ((*token).to_string(), caller.clone()))
            .collect(),
    ));
    (
        mcp_router(
            host.clone(),
            McpRouteConfig::default().with_admission(admission.clone()),
        ),
        mcp_router(
            host.clone(),
            McpRouteConfig::stateless_json().with_admission(admission),
        ),
    )
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

async fn post(
    router: &Router,
    lease: Option<&str>,
    session: Option<&str>,
    host_header: &str,
    body: Value,
) -> anyhow::Result<Reply> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::HOST, host_header)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("mcp-protocol-version", LEGACY);
    if let Some(lease) = lease {
        request = request.header(LEASE_HEADER, lease);
    }
    if let Some(session) = session {
        request = request.header("mcp-session-id", session);
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(body.to_string()))?)
        .await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let text = std::str::from_utf8(&bytes)?;
    let body = match text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .find(|data| !data.is_empty())
    {
        Some(data) => serde_json::from_str(data)?,
        None if text.trim().is_empty() => Value::Null,
        // rmcp answers transport-level refusals (Host, session) in plain text.
        None => serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string())),
    };
    Ok(Reply {
        status,
        headers,
        body,
    })
}

fn initialize(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": LEGACY,
            "capabilities": {},
            "clientInfo": { "name": "acp-client", "version": "1.0" }
        }
    })
}

fn list(id: u64) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/list", "params": {} })
}

fn call(id: u64, name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    })
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

fn tool_names(list: &Value) -> Vec<String> {
    list["result"]["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["name"].as_str().map(str::to_string))
        .collect()
}

/// Opens a stateful (legacy SSE) transport session and returns its id.
async fn open_session(router: &Router, lease: Option<&str>) -> anyhow::Result<String> {
    let init = post(router, lease, None, "localhost", initialize(1)).await?;
    assert_eq!(init.status, StatusCode::OK, "{}", init.body);
    let session = init
        .headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("missing mcp-session-id"))?
        .to_string();
    let initialized = post(
        router,
        lease,
        Some(&session),
        "localhost",
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await?;
    assert_eq!(initialized.status, StatusCode::ACCEPTED);
    Ok(session)
}

#[tokio::test]
async fn an_unknown_lease_is_refused_before_mcp() -> anyhow::Result<()> {
    let (host, _dir) = host().await?;
    let (stateful, stateless) = routers(&host, &[]);
    for router in [&stateful, &stateless] {
        let reply = post(router, Some("forged"), None, "localhost", list(1)).await?;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert_eq!(reply.body["error"]["code"], -32001);
        assert_eq!(reply.body["id"], Value::Null);
    }
    assert_eq!(host.sessions().count().await, 0);
    Ok(())
}

#[tokio::test]
async fn a_read_only_caller_gets_the_read_only_catalog_and_nothing_else() -> anyhow::Result<()> {
    let (host, _dir) = host().await?;
    let reader = AdmittedCaller {
        read_only: true,
        ..AdmittedCaller::default()
    };
    let (_, router) = routers(&host, &[("reader", reader)]);

    let listed = post(&router, Some("reader"), None, "localhost", list(1)).await?;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.body);
    let names = tool_names(&listed.body);
    assert_eq!(
        names,
        [
            "tabs",
            "history",
            "snapshot",
            "diff",
            "read",
            "grep",
            "screenshot",
            "pdf",
            "wait",
            "connector_mcp_servers",
        ]
    );
    let tabs = &listed.body["result"]["tools"][0];
    assert_eq!(
        tabs["inputSchema"]["properties"]["action"]["enum"],
        json!(["list", "active"])
    );
    assert_eq!(tabs["annotations"]["readOnlyHint"], true);

    // Even a caller that ignores its list is refused, before any browser guard.
    for (name, arguments, refusal) in [
        (
            "navigate",
            json!({ "url": "https://example.com" }),
            "navigate: unavailable in read-only mode.",
        ),
        (
            "tabs",
            json!({ "action": "new" }),
            "tabs: read-only mode only supports action=\"list\" or \"active\".",
        ),
        (
            "name_session",
            json!({ "name": "x" }),
            "name_session: unavailable in read-only mode.",
        ),
        (
            "connector_mcp_servers",
            json!({}),
            "Managed connector tools are unavailable in read-only mode.",
        ),
    ] {
        let reply = post(
            &router,
            Some("reader"),
            None,
            "localhost",
            call(2, name, arguments),
        )
        .await?;
        assert_eq!(reply.body["result"]["isError"], true, "{name}");
        assert_eq!(text_of(&reply.body["result"]).lines().next(), Some(refusal));
    }

    // The same request without the lease is an ordinary client with the full catalog.
    let open = post(&router, None, None, "localhost", list(3)).await?;
    let names = tool_names(&open.body);
    assert!(names.contains(&"navigate".to_string()) && names.contains(&"run".to_string()));
    Ok(())
}

#[tokio::test]
async fn a_bound_caller_keeps_one_session_across_transports() -> anyhow::Result<()> {
    let (host, _dir) = host().await?;
    let agent = AdmittedCaller {
        session: Some(conversation("conv-1")),
        ..AdmittedCaller::default()
    };
    let (stateful, stateless) = routers(&host, &[("agent", agent)]);

    // Stateful SSE: the transport session id is ignored in favour of the bound session.
    let transport = open_session(&stateful, Some("agent")).await?;
    let listed = post(
        &stateful,
        Some("agent"),
        Some(&transport),
        "localhost",
        list(2),
    )
    .await?;
    for tool in listed.body["result"]["tools"]
        .as_array()
        .into_iter()
        .flatten()
    {
        assert_eq!(
            tool["inputSchema"]["properties"]["session"],
            Value::Null,
            "{} offers a session handle to a bound caller",
            tool["name"]
        );
    }
    let first = post(
        &stateful,
        Some("agent"),
        Some(&transport),
        "localhost",
        call(3, "connector_mcp_servers", json!({})),
    )
    .await?;
    assert_eq!(
        text_of(&first.body["result"]),
        "connector_mcp_servers served session bound-conv-1"
    );

    // Stateless JSON, a different transport entirely: same session, no handle echoed.
    let second = post(
        &stateless,
        Some("agent"),
        None,
        "localhost",
        call(4, "connector_mcp_servers", json!({})),
    )
    .await?;
    assert_eq!(
        second
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(
        text_of(&second.body["result"]),
        "connector_mcp_servers served session bound-conv-1"
    );
    assert!(second.body["result"].get("_meta").is_none());

    let session = host
        .sessions()
        .lookup(&SessionId::new("bound-conv-1"))
        .await
        .ok_or_else(|| anyhow::anyhow!("bound session missing"))?;
    assert_eq!(session.label().await, "compare-flight-prices");
    assert_eq!(session.convo_id().as_str(), "browseros-conv-1");
    // The legacy transport session itself never became a host session.
    assert!(
        host.sessions()
            .lookup(&SessionId::new(&transport))
            .await
            .is_none()
    );
    assert_eq!(host.sessions().count().await, 1);

    // Ended (idle sweep, host teardown): the next request re-mints the same session.
    assert!(host.end_session(&conversation("conv-1"), "test").await?);
    post(
        &stateless,
        Some("agent"),
        None,
        "localhost",
        call(5, "connector_mcp_servers", json!({})),
    )
    .await?;
    let rebound = host
        .sessions()
        .lookup(&SessionId::new("bound-conv-1"))
        .await
        .ok_or_else(|| anyhow::anyhow!("bound session was not re-minted"))?;
    assert_eq!(rebound.convo_id().as_str(), "browseros-conv-1");
    Ok(())
}

#[tokio::test]
async fn the_stateless_preset_accepts_any_host_and_keeps_anonymous_clients_in_one_session()
-> anyhow::Result<()> {
    let (host, _dir) = host().await?;
    let (stateful, stateless) = routers(&host, &[]);
    // claw's default keeps rmcp's loopback-only Host allowlist.
    let refused = post(&stateful, None, None, "192.168.1.20:9100", initialize(1)).await?;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);

    let first = post(
        &stateless,
        None,
        None,
        "192.168.1.20:9100",
        call(1, "name_session", json!({ "name": "lan task" })),
    )
    .await?;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    let handle = first.body["result"]["_meta"]["com.browseros/session"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("stateless legacy call got no handle: {}", first.body))?
        .to_string();
    // The handle, not a per-request throwaway session, carries continuity.
    post(
        &stateless,
        None,
        None,
        "192.168.1.20:9100",
        call(
            2,
            "name_session",
            json!({ "name": "lan task two", "session": handle }),
        ),
    )
    .await?;
    let session = host
        .sessions()
        .lookup(&SessionId::new(&handle))
        .await
        .ok_or_else(|| anyhow::anyhow!("handle session missing"))?;
    assert_eq!(session.label().await, "lan-task-two");
    assert_eq!(host.sessions().count().await, 1);
    Ok(())
}

// ─── Browser-backed hook points, on the crate's synthetic native peer ───

async fn connected_call(
    tool: &str,
    args: Value,
    caller: Option<AdmittedCaller>,
) -> anyhow::Result<(
    browseros_mcp_host::mcp::dispatch::ToolCall,
    Arc<ProfileConnection>,
)> {
    let native = ProfileConnection::new();
    let browser = test_support::current_session(native.clone());
    browser.pages.list().await?;
    let host = test_support::host().await?;
    host.browser()
        .set_session_for_testing(browser.clone())
        .await;
    let mut call = test_support::tool_call_on(host, tool, args).await?;
    call.browser_session = Some(browser.clone());
    call.view = Some(browseros_core::target::BrowserView::unbound(browser, None));
    call.caller = caller;
    Ok((call, native))
}

#[tokio::test]
async fn structured_content_is_kept_only_for_callers_that_opt_in() -> anyhow::Result<()> {
    const NO_GUARDS: &[ToolGuard] = &[];
    const NO_EFFECTS: &[NamedToolEffect] = &[];
    const NO_OBSERVERS: &[NamedToolObserver] = &[];
    for (caller, kept) in [
        (None, false),
        (
            Some(AdmittedCaller {
                structured_content: true,
                ..AdmittedCaller::default()
            }),
            true,
        ),
    ] {
        let (call, _native) = connected_call("tabs", json!({ "action": "list" }), caller).await?;
        let result = dispatch_tool_call_with(call, NO_GUARDS, NO_EFFECTS, NO_OBSERVERS).await?;
        assert_eq!(result.is_error, Some(false), "{result:?}");
        assert_eq!(result.structured_content.is_some(), kept, "{result:?}");
        if kept {
            assert!(
                result
                    .structured_content
                    .as_ref()
                    .is_some_and(|value| value["pages"].is_array())
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn a_caller_without_auto_group_never_groups_the_tabs_it_opens() -> anyhow::Result<()> {
    for (auto_group, grouped) in [(true, true), (false, false)] {
        let caller = AdmittedCaller {
            auto_group,
            ..AdmittedCaller::default()
        };
        let (call, native) =
            connected_call("tabs", json!({ "action": "new" }), Some(caller)).await?;
        let page = call
            .view
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("view missing"))?
            .new_page("https://synthetic.test/new", Default::default())
            .await?;
        let result = ToolResult::text("opened", Some(json!({ "page": page.page_id.0 })));
        tab_groups::apply(browseros_mcp_host::mcp::dispatch::ToolEffectContext {
            call: &call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await?;
        if grouped {
            tokio::time::timeout(Duration::from_secs(30), native.group_created.notified()).await?;
        } else {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let groups = native
            .calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|(method, _)| method == "Browser.createTabGroup")
            .count();
        assert_eq!(groups > 0, grouped, "auto_group={auto_group}");
    }
    Ok(())
}

#[tokio::test]
async fn a_lent_page_is_not_reported_as_someone_elses() -> anyhow::Result<()> {
    let lent = AdmittedCaller {
        lent_pages: BTreeSet::from([7]),
        ..AdmittedCaller::default()
    };
    for (caller, noticed) in [(None, true), (Some(lent), false)] {
        // Page 7 is the user's own (unclaimed) tab, the side panel's active tab.
        let mut call = test_support::tool_call("snapshot", json!({ "page": 7 })).await?;
        call.caller = caller;
        let result = ToolResult::text("snapshot", None);
        let annotated =
            page_ownership_notice::apply(browseros_mcp_host::mcp::dispatch::ToolEffectContext {
                call: &call,
                result: &result,
                cancelled: false,
                duration_ms: 1,
            })
            .await?;
        let notice = annotated
            .map(|result| {
                result
                    .content
                    .iter()
                    .filter_map(|block| block.as_text().map(|text| text.text.clone()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        assert_eq!(
            notice.contains("page 7 is one of the user's own tabs"),
            noticed,
            "{notice}"
        );
    }
    // Lending never changes ownership: the page is still nobody's.
    let call = test_support::tool_call("snapshot", json!({ "page": 7 })).await?;
    assert_eq!(call.host.sessions().owner_of_page(&PageId(7)).await, None);
    Ok(())
}
