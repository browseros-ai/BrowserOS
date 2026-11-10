//! Neo: Drives the reusable router over real loopback HTTP with the runtime's browser double.
//! Separate stateless requests exercise shared ownership, admission and cancellation without
//! the claw-server process, so another embedding host receives the same lifecycle behavior.

#![expect(
    clippy::unwrap_used,
    reason = "fixture startup and malformed HTTP replies fail the test"
)]

#[path = "../../neo-runtime/tests/support/fake_browser.rs"]
#[expect(
    dead_code,
    reason = "the shared browser double serves the wider runtime suite"
)]
mod fake_browser;

use std::time::Duration;

use fake_browser::{FakeBrowser, Replies};
use neo_sdk_server::{Config, NeoSdkServer};
use reqwest::Client;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

struct Server {
    neo: NeoSdkServer,
    url: String,
    task: JoinHandle<()>,
    _state: TempDir,
}

impl Server {
    async fn start(port: u16) -> Self {
        let state = TempDir::new().unwrap();
        let neo = NeoSdkServer::new(
            Config::new(port, state.path().to_path_buf()),
            CancellationToken::new(),
        );
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let router = neo.router();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            neo,
            url,
            task,
            _state: state,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // This task owns the listener. Test failure must not leave a loopback server alive.
        self.task.abort();
    }
}

async fn rpc(client: &Client, url: &str, id: u64, method: &str, mut params: Value) -> Value {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientCapabilities":{}
    });
    let mut request = client
        .post(url)
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", method)
        .header("Accept", "application/json, text/event-stream");
    if method == "tools/call" {
        request = request.header("Mcp-Name", "run");
    }
    let response = request
        .json(&json!({
            "jsonrpc":"2.0", "id":id, "method":method, "params":params
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "HTTP {}", response.status());
    assert!(
        response.headers().get("Mcp-Session-Id").is_none(),
        "modern requests remain stateless"
    );
    let body = response.text().await.unwrap();
    let reply: Value = if body.trim_start().starts_with('{') {
        serde_json::from_str(&body).unwrap()
    } else {
        body.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
            .find(|reply| reply["id"] == id)
            .unwrap_or_else(|| panic!("no reply: {body}"))
    };
    assert!(reply.get("error").is_none(), "{reply}");
    reply["result"].clone()
}

async fn run(client: &Client, url: &str, id: u64, args: Value) -> Value {
    rpc(
        client,
        url,
        id,
        "tools/call",
        json!({"name":"run", "arguments":args}),
    )
    .await
}

// Correlation belongs to metadata, outside output budgeting and retained report fields.
fn run_id(reply: &Value) -> &str {
    let id = reply["_meta"]["com.browseros.neo/run"].as_str().unwrap();
    assert_eq!(
        id.len(),
        26,
        "the traced run uses the shared ULID representation"
    );
    assert!(
        id.bytes()
            .all(|byte| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&byte)),
        "the correlation id is a serialized RunId: {id}"
    );
    id
}

async fn getter_started(browser: &FakeBrowser) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if browser
            .exchanges()
            .iter()
            .filter(|e| e.method() == "Page.getNavigationHistory" && e.reply.is_some())
            .count()
            >= 2
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the script never reached its getter"
        );
        time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tools_list_needs_no_browser_and_the_router_keeps_request_hygiene() {
    let server = Server::start(1).await;
    let client = Client::new();
    let listed = rpc(&client, &server.url, 1, "tools/list", json!({})).await;
    assert_eq!(listed["tools"].as_array().unwrap().len(), 1);
    assert_eq!(listed["tools"][0]["name"], "run");
    assert!(
        listed["tools"][0]["description"]
            .as_str()
            .unwrap()
            .contains(neo_runtime::api_description())
    );
    let denied = client
        .post(&server.url)
        .header("Origin", "https://example.test")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(
        denied.json::<Value>().await.unwrap(),
        json!({"error":"unsupported request"})
    );
    let non_json = client
        .post(&server.url)
        .header("Content-Type", "text/plain")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(
        non_json.status(),
        reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        non_json.json::<Value>().await.unwrap(),
        json!({"error":"unsupported content type"})
    );
    assert!(
        !server.neo.connected(),
        "listing tools opens no browser connection"
    );
    server.neo.shutdown().await;
    assert!(
        !server.neo.ready().await,
        "shutdown cannot reconnect through readiness"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn separate_stateless_requests_continue_a_page_and_retrieve_the_shared_original() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let server = Server::start(browser.port()).await;
    let client = Client::new();
    let first = run(
        &client,
        &server.url,
        1,
        json!({"code":"async (page) => 'x'.repeat(12000)","maxResponseTokens":1000}),
    )
    .await;
    let first_id = run_id(&first);
    let report = &first["structuredContent"];
    assert_eq!(report["status"], "ok", "{first}");
    assert_eq!(report["pages"][0]["created"], true);
    let page = report["pages"][0]["id"].as_str().unwrap();
    let second = run(
        &client,
        &server.url,
        2,
        json!({"page":page,"code":"page.url()"}),
    )
    .await;
    assert_ne!(
        run_id(&second),
        first_id,
        "continuing a page starts a distinct run"
    );
    assert_eq!(second["structuredContent"]["pages"][0]["id"], page);
    assert_eq!(second["structuredContent"]["pages"][0]["created"], false);
    assert_eq!(second["structuredContent"]["value"], "about:blank");
    let result = report["truncated"]["result"].as_str().unwrap();
    let original = run(
        &client,
        &server.url,
        3,
        json!({"result":result,"field":"value","offset":0,"maxResponseTokens":20000}),
    )
    .await;
    assert_eq!(original["structuredContent"]["value"], "x".repeat(12000));
    assert!(
        original["_meta"].get("com.browseros.neo/run").is_none(),
        "retrieving a retained result does not start another runtime run"
    );
    let failed = run(
        &client,
        &server.url,
        4,
        json!({"page":page,"code":"throw new Error('trace failure')"}),
    )
    .await;
    assert_eq!(failed["structuredContent"]["status"], "error", "{failed}");
    assert_ne!(
        run_id(&failed),
        first_id,
        "failed runs also retain correlation"
    );
    server.neo.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admission_uses_the_run_deadline_and_leaves_control_requests_a_worker() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let server = Server::start(browser.port()).await;
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let first_client = client.clone();
    let first_url = server.url.clone();
    let first = tokio::spawn(async move {
        run(
            &first_client,
            &first_url,
            1,
            json!({"page":fake_browser::PAGE,
            "code":"page.url(); while (true) {}", "timeout":3000}),
        )
        .await
    });
    getter_started(&browser).await;
    let queued = run(
        &client,
        &server.url,
        2,
        json!({"page":fake_browser::PAGE,
        "code":"await page.click('#go')", "timeout":100}),
    )
    .await;
    assert_eq!(queued["structuredContent"]["status"], "timeout", "{queued}");
    assert!(
        queued["_meta"].get("com.browseros.neo/run").is_none(),
        "a timeout in admission never starts a traced runtime run"
    );
    assert_eq!(
        browser.page_clicks(),
        0,
        "a queued timeout delivers no input"
    );
    let listed = rpc(&client, &server.url, 3, "tools/list", json!({})).await;
    assert_eq!(listed["tools"][0]["name"], "run");
    first.abort();
    let _cancelled_client = first.await;
    server.neo.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_a_modern_request_cancels_before_its_delayed_input() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let server = Server::start(browser.port()).await;
    let client = Client::new();
    let first_client = client.clone();
    let first_url = server.url.clone();
    let first = tokio::spawn(async move {
        run(&first_client, &first_url, 1, json!({"page":fake_browser::PAGE,
            "code":"page.url(); await new Promise(r => setTimeout(r, 500)); await page.click('#go')"})).await
    });
    getter_started(&browser).await;
    first.abort();
    let _cancelled_client = first.await;
    let after = run(
        &client,
        &server.url,
        2,
        json!({"page":fake_browser::PAGE,
        "code":"await new Promise(r => setTimeout(r, 700)); return page.url()"}),
    )
    .await;
    assert_eq!(after["structuredContent"]["status"], "ok", "{after}");
    assert_eq!(
        browser.page_clicks(),
        0,
        "the disconnected request delivered no input"
    );
    server.neo.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_waits_for_an_inflight_readiness_connection_and_keeps_it_closed() {
    let browser = FakeBrowser::start(Replies::default()).await;
    browser.page.lock().unwrap().tab_info_delay = Some(Duration::from_millis(500));
    let server = Server::start(browser.port()).await;
    let neo = server.neo.clone();
    let readiness = tokio::spawn(async move { neo.ready().await });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if browser
            .exchanges()
            .iter()
            .any(|e| e.method() == "Browser.getTabInfo" && e.reply.is_none())
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "readiness never reached the browser capability check"
        );
        time::sleep(Duration::from_millis(5)).await;
    }
    server.neo.shutdown().await;
    assert!(
        !readiness.await.unwrap(),
        "an initializer cancelled by shutdown never becomes ready"
    );
    assert!(
        !server.neo.connected(),
        "shutdown leaves no late browser connection cached"
    );
}
