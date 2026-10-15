//! Fake upstream plus real rmcp transport/host: no vendor credentials or browser
//! profile. Assertions pin TS ordering, normalization, DELETE bodies and tool wire data.
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use browseros_mcp_host::{
    AdmittedCaller, HostCore, HostCoreConfig, HostCtx, McpRouteConfig, mcp_router, test_support,
};
use http::{Method, StatusCode};
use rmcp::model::{CallToolRequestParams, ClientInfo};
use rmcp::{ServiceExt, transport::StreamableHttpClientTransport};
use serde_json::{Value, json};
use server_rust::{
    http::klavis as routes,
    klavis::{ConnectorScope, Klavis, ProxyState},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt as _;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Clone)]
struct Fake {
    base: String,
    requests: Arc<Mutex<Vec<(String, String, Value)>>>,
    failures: Arc<AtomicUsize>,
    mcp_failures: Arc<AtomicUsize>,
}
struct Upstream {
    fake: Fake,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn upstream() -> Result<Upstream> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let fake = Fake {
        base: format!("http://{}", listener.local_addr()?),
        requests: Default::default(),
        failures: Default::default(),
        mcp_failures: Default::default(),
    };
    let router = Router::new()
        .fallback(any(fake_request))
        .with_state(fake.clone());
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok(Upstream { fake, task })
}
async fn fake_request(State(fake): State<Fake>, request: Request) -> Response {
    let method = request.method().to_string();
    let uri = request.uri().to_string();
    let bytes = to_bytes(request.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    fake.requests
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((method.clone(), uri.clone(), body.clone()));
    if uri == "/mcp-server/strata/create" {
        if fake
            .failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| v.checked_sub(1))
            .is_ok()
        {
            return (StatusCode::BAD_GATEWAY, Json(json!({"error":"offline"}))).into_response();
        }
        return Json(json!({"strataServerUrl":format!("{}/remote-mcp",fake.base),"strataId":"strata-id","addedServers":body["servers"],"oauthUrls":{"google_calendar":"https://auth.test/calendar","Gmail":"https://auth.test/gmail","Slack":""},"apiKeyUrls":{"Brave Search":"https://key.test/?instance_id=instance-1"}})).into_response();
    }
    if uri.ends_with("/integrations") {
        return Json(json!({"integrations":["Gmail",{"name":"Slack","isAuthenticated":true,"is_authenticated":false},{"name":"Notion","is_authenticated":true},{"name":"Linear","isAuthenticated":false,"is_authenticated":true},{"name":""},{"name":7},null,9]})).into_response();
    }
    if uri == "/remote-mcp" {
        if method != "POST" {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        if fake
            .mcp_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| v.checked_sub(1))
            .is_ok()
        {
            return StatusCode::BAD_GATEWAY.into_response();
        }
        let result = match body["method"].as_str().unwrap_or("") {
            "initialize" => {
                json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fake-strata","version":"1"}})
            }
            "notifications/initialized" | "notifications/cancelled" => {
                return StatusCode::ACCEPTED.into_response();
            }
            "tools/list" => {
                json!({"tools":[{"name":"execute_action","description":"Execute an action","inputSchema":{"type":"object","properties":{"server_name":{"type":"string"},"nested":{"type":"object","properties":{"count":{"type":"integer"}}}},"required":["server_name"]}}]})
            }
            "tools/call" => {
                json!({"content":[{"type":"text","text":"remote failure"},{"type":"image","data":"AQID","mimeType":"image/png"}],"isError":true,"structuredContent":{"echo":body["params"]["arguments"]}})
            }
            _ => json!({}),
        };
        return Json(json!({"jsonrpc":"2.0","id":body["id"],"result":result})).into_response();
    }
    Json(json!({"ok":true})).into_response()
}
fn service(fake: &Fake) -> Arc<Klavis> {
    Klavis::start_with_upstream(
        "installation-id".into(),
        reqwest::Client::new(),
        fake.base.clone(),
        vec![Duration::from_millis(10)],
    )
}
async fn ready(klavis: &Klavis) -> Result {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !matches!(klavis.proxy_state(), ProxyState::Connected { .. }) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    Ok(())
}
fn router(klavis: Arc<Klavis>) -> Router {
    Router::new()
        .route("/servers", get(routes::servers))
        .route("/oauth-urls", get(routes::oauth_urls))
        .route("/integrations", get(routes::user_integrations))
        .route("/add", post(routes::add_server))
        .route("/key", post(routes::submit_api_key))
        .route("/remove", axum::routing::delete(routes::remove_server))
        .with_state(klavis)
}
async fn request(
    router: Router,
    method: Method,
    path: &str,
    value: Value,
) -> Result<(StatusCode, Value)> {
    let response = router
        .oneshot(
            http::Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(value.to_string()))?,
        )
        .await?;
    Ok((
        response.status(),
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?,
    ))
}

#[tokio::test]
async fn catalog_and_auth_routes_match_ts() -> Result {
    let upstream = upstream().await?;
    let klavis = service(&upstream.fake);
    ready(&klavis).await?;
    let router = router(klavis.clone());
    let (_, body) = request(router.clone(), Method::GET, "/servers", json!({})).await?;
    assert_eq!(body["count"], 45);
    assert_eq!(
        body["servers"][0],
        json!({"name":"Gmail","description":"Send, read, and search emails"})
    );
    assert_eq!(body["servers"][44]["name"], "Intercom");
    let (_, body) = request(router.clone(), Method::GET, "/oauth-urls", json!({})).await?;
    assert_eq!(body["servers"].as_array().ok_or("servers")?.len(), 45);
    assert_eq!(
        body["oauthUrls"]["Google Calendar"],
        "https://auth.test/calendar"
    );
    assert!(body["oauthUrls"].get("Slack").is_none());
    let (_, body) = request(router.clone(), Method::GET, "/integrations", json!({})).await?;
    assert_eq!(
        body,
        json!({"count":4,"integrations":[{"name":"Gmail","is_authenticated":false},{"name":"Slack","is_authenticated":true},{"name":"Notion","is_authenticated":true},{"name":"Linear","is_authenticated":false}]})
    );
    let (status, body) = request(
        router.clone(),
        Method::POST,
        "/add",
        json!({"serverName":"Google Calendar","ignored":1}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"success":true,"serverName":"Google Calendar","strataId":"strata-id","addedServers":["Google Calendar"],"oauthUrl":"https://auth.test/calendar"})
    );
    let (status,body) = request(router.clone(),Method::POST,"/key",json!({"serverName":"Brave Search","apiKey":"fixture-key","apiKeyUrl":"https://untrusted.invalid/?instance_id=instance-1"})).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"success":true,"serverName":"Brave Search"}));
    let (status, body) = request(
        router,
        Method::DELETE,
        "/remove",
        json!({"serverName":"Google Calendar"}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"success":true,"serverName":"Google Calendar"}));
    let requests = upstream
        .fake
        .requests
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    assert!(requests.iter().any(|(m, p, b)| m == "POST"
        && p == "/mcp-server/instance/set-auth"
        && b == &json!({"instanceId":"instance-1","authData":{"api_key":"fixture-key"}})));
    let deletion = requests
        .iter()
        .position(|(m, p, _)| {
            m == "DELETE" && p.contains("/strata-id/servers?servers=Google%20Calendar")
        })
        .ok_or("encoded upstream DELETE")?;
    assert_eq!(
        requests[deletion - 1].2["servers"],
        json!(["Google Calendar"])
    );
    klavis.stop().await;
    Ok(())
}

#[tokio::test]
async fn zod_errors_delete_body_and_fixed_failure_messages() -> Result {
    let upstream = upstream().await?;
    let klavis = service(&upstream.fake);
    ready(&klavis).await?;
    let router = router(klavis.clone());
    for (path, value) in [
        ("/add", json!({})),
        ("/add", json!({"serverName":""})),
        (
            "/key",
            json!({"serverName":"Gmail","apiKey":"x","apiKeyUrl":"bad"}),
        ),
    ] {
        let (status, body) = request(router.clone(), Method::POST, path, value).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["name"], "ZodError");
    }
    let (_, body) = request(
        router.clone(),
        Method::DELETE,
        "/remove",
        json!({"serverName":"gmail"}),
    )
    .await?;
    assert_eq!(body, json!({"error":"Invalid server: gmail"}));
    upstream.fake.failures.store(1, Ordering::SeqCst);
    let (status, body) = request(
        router.clone(),
        Method::POST,
        "/add",
        json!({"serverName":"Gmail"}),
    )
    .await?;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body, json!({"error":"Failed to add server"}));
    let response = router
        .clone()
        .oneshot(
            http::Request::post("/add")
                .header("content-type", "application/json")
                .body(Body::from("{"))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let response = router
        .oneshot(http::Request::post("/add").body(Body::from(r#"{"serverName":"Gmail"}"#))?)
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        klavis
            .submit_api_key("Gmail", "key", &url::Url::parse("https://x.test/")?)
            .await
            .is_err()
    );
    klavis.stop().await;
    Ok(())
}

#[tokio::test]
async fn retry_reuses_metadata_and_shutdown_cancels_backoff() -> Result {
    let upstream = upstream().await?;
    upstream.fake.mcp_failures.store(1, Ordering::SeqCst);
    let klavis = service(&upstream.fake);
    ready(&klavis).await?;
    assert_eq!(
        upstream
            .fake
            .requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(_, p, _)| p == "/mcp-server/strata/create")
            .count(),
        1
    );
    klavis.stop().await;
    assert_eq!(klavis.proxy_status(), json!({"state":"stopped"}));
    upstream.fake.failures.store(100, Ordering::SeqCst);
    let klavis = Klavis::start_with_upstream(
        "id".into(),
        reqwest::Client::new(),
        upstream.fake.base.clone(),
        vec![Duration::from_secs(60)],
    );
    tokio::time::sleep(Duration::from_millis(30)).await;
    tokio::time::timeout(Duration::from_millis(200), klavis.stop()).await?;
    assert_eq!(klavis.proxy_state(), ProxyState::Disconnected);
    Ok(())
}

#[tokio::test]
async fn tools_cross_real_shared_host_with_schemas_results_and_policy() -> Result {
    let upstream = upstream().await?;
    let klavis = service(&upstream.fake);
    ready(&klavis).await?;
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
    let provider = klavis.tool_provider_with_scope(Arc::new(|caller| ConnectorScope {
        selected_server_names: vec!["Gmail".into(), "Gmail".into(), "Slack".into()],
        rejection: caller
            .and_then(|c| c.host_data::<AtomicBool>())
            .filter(|active| !active.load(Ordering::SeqCst))
            .map(|_| "MCP tools require an active conversation run.".into()),
    }));
    let tools = provider.tools(None).await;
    assert_eq!(tools.len(), 2);
    assert_eq!(
        tools[1].input_schema["properties"]["nested"]["properties"]["count"]["type"],
        "integer"
    );
    let host = HostCtx::builder(core).tool_provider(provider).build();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let router = mcp_router(host.clone(), McpRouteConfig::default());
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let mut client = ClientInfo::default()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://{address}/mcp"
        )))
        .await?;
    let inventory = client
        .call_tool(CallToolRequestParams::new("connector_mcp_servers"))
        .await?;
    let wire = serde_json::to_value(inventory)?;
    let inventory: Value =
        serde_json::from_str(wire["content"][0]["text"].as_str().ok_or("text")?)?;
    assert_eq!(inventory["selected"], json!(["Gmail", "Slack"]));
    assert_eq!(
        inventory["connected"][0],
        json!({"name":"Slack","isAuthenticated":true})
    );
    let result = client
        .call_tool(
            CallToolRequestParams::new("execute_action").with_arguments(
                json!({"server_name":"Gmail","nested":{"count":7}})
                    .as_object()
                    .cloned()
                    .ok_or("args")?,
            ),
        )
        .await?;
    let wire = serde_json::to_value(result)?;
    assert_eq!(wire["isError"], true);
    assert_eq!(wire["content"][1]["mimeType"], "image/png");
    assert_eq!(wire["structuredContent"]["echo"]["nested"]["count"], 7);
    let _ = client.close().await?;
    task.abort();
    // A read-only call is also checked inside the provider, independently of
    // host catalog filtering, through an admitted duplex MCP connection.
    let readonly = AdmittedCaller {
        read_only: true,
        ..Default::default()
    };
    let service = browseros_mcp_host::mcp_service(host.clone()).with_caller(readonly);
    let (client_io, server_io) = tokio::io::duplex(65536);
    let server_task = tokio::spawn(async move {
        service
            .serve(server_io)
            .await?
            .waiting()
            .await
            .map_err(anyhow::Error::from)
    });
    let mut client = ().serve(client_io).await?;
    let rejection = client
        .call_tool(CallToolRequestParams::new("connector_mcp_servers"))
        .await?;
    assert!(
        serde_json::to_value(rejection)?["content"][0]["text"]
            .as_str()
            .ok_or("rejection")?
            .contains("read-only")
    );
    let _ = client.close().await?;
    server_task.abort();
    let active = Arc::new(AtomicBool::new(false));
    let service = browseros_mcp_host::mcp_service(host.clone()).with_caller(AdmittedCaller {
        host_data: Some(active.clone()),
        ..Default::default()
    });
    let (client_io, server_io) = tokio::io::duplex(65536);
    let server_task = tokio::spawn(async move {
        service
            .serve(server_io)
            .await?
            .waiting()
            .await
            .map_err(anyhow::Error::from)
    });
    let mut client = ().serve(client_io).await?;
    let rejection = client
        .call_tool(CallToolRequestParams::new("connector_mcp_servers"))
        .await?;
    assert!(
        serde_json::to_value(rejection)?["content"][0]["text"]
            .as_str()
            .ok_or("run rejection")?
            .contains("active conversation run")
    );
    // Recompute authority on each call: the same lease/transport can span turns.
    active.store(true, Ordering::SeqCst);
    let result = client
        .call_tool(CallToolRequestParams::new("connector_mcp_servers"))
        .await?;
    assert_ne!(result.is_error, Some(true));
    let _ = client.close().await?;
    server_task.abort();
    host.shutdown().await?;
    klavis.stop().await;
    Ok(())
}
