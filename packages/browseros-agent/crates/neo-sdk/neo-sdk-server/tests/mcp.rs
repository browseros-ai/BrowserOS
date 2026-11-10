//! Neo: Drives the reusable router over real loopback HTTP with the runtime's browser double.
//! Separate stateless requests exercise shared ownership, admission and cancellation without
//! the claw-server process, so another embedding host receives the same lifecycle behavior.
//!
//! T1 discriminates W1 deterministically. T1, T4, T6 and T7 are deterministic for outcome and
//! non-discriminating for W2. Client-observed order across the POST and GET connections is not
//! asserted, because it is not a server property. T3, T3c and T3c-L are the deterministic W2 and
//! ordering proofs. T3b stays probabilistic.

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

use std::fs::File;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use fake_browser::{FakeBrowser, Replies};
use futures_util::future::BoxFuture;
use neo_runtime::{CaptchaHost, PageId, SolveReply};
use neo_sdk_server::{Config, NeoSdkServer};
use reqwest::Client;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::fs;
use tokio::net::TcpListener;
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

struct Server {
    neo: NeoSdkServer,
    url: String,
    task: JoinHandle<()>,
    state: TempDir,
}

impl Server {
    async fn start(port: u16) -> Self {
        Self::configured(port, |server| server).await
    }

    async fn configured(port: u16, configure: impl FnOnce(NeoSdkServer) -> NeoSdkServer) -> Self {
        let state = TempDir::new().unwrap();
        let neo = configure(NeoSdkServer::new(
            Config::new(port, state.path().to_path_buf()),
            CancellationToken::new(),
        ));
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
            state,
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

/// A capability supplied by the embedding host; the reusable server only binds and owns it.
struct ValueCaptcha(AtomicUsize);

impl CaptchaHost for ValueCaptcha {
    fn solve(&self, _page: PageId, _stop: CancellationToken) -> BoxFuture<'static, SolveReply> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            SolveReply::Value {
                value: json!({"verdict":"already_clear"}),
                log: Vec::new(),
                input_settled: false,
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_factory_binds_once_and_keeps_truthful_delivery_across_requests() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let factories = Arc::new(AtomicUsize::new(0));
    let host = Arc::new(ValueCaptcha(AtomicUsize::new(0)));
    let server = Server::configured(browser.port(), {
        let factories = Arc::clone(&factories);
        let host = Arc::clone(&host);
        move |server| {
            server.with_captcha(
                move |cdp| -> Arc<dyn CaptchaHost> {
                    assert!(
                        cdp.is_connected(),
                        "factory receives the owned live connection"
                    );
                    factories.fetch_add(1, Ordering::SeqCst);
                    Arc::<ValueCaptcha>::clone(&host)
                },
                "CAPTCHA fixture capability",
            )
        }
    })
    .await;
    let client = Client::new();
    let listed = rpc(&client, &server.url, 1, "tools/list", json!({})).await;
    let description = listed["tools"][0]["description"].as_str().unwrap();
    assert!(description.contains("CAPTCHA fixture capability"));
    assert!(description.contains(neo_runtime::api_description()));
    assert_eq!(
        factories.load(Ordering::SeqCst),
        0,
        "listing stays browser-free"
    );
    for id in [2, 3] {
        let reply = run(
            &client,
            &server.url,
            id,
            json!({"page":fake_browser::PAGE,"code":"return await neo.solveCaptcha(page)"}),
        )
        .await;
        assert_eq!(reply["structuredContent"]["status"], "ok", "{reply}");
        assert_eq!(
            reply["structuredContent"]["value"]["verdict"],
            "already_clear"
        );
        assert_eq!(
            reply["structuredContent"]["calls"][0]["delivery"],
            "unknown"
        );
        let _trace = run_id(&reply);
    }
    assert_eq!(
        factories.load(Ordering::SeqCst),
        1,
        "requests share one bound host"
    );
    assert_eq!(host.0.load(Ordering::SeqCst), 2);
    server.neo.shutdown().await;
}

/// Models input cleanup that must retain the runtime's turn after request cancellation.
struct DrainingCaptcha {
    entered: Arc<Notify>,
    cancelled: Arc<Notify>,
    release: Arc<Semaphore>,
}

impl CaptchaHost for DrainingCaptcha {
    fn solve(&self, _page: PageId, stop: CancellationToken) -> BoxFuture<'static, SolveReply> {
        let entered = Arc::clone(&self.entered);
        let cancelled = Arc::clone(&self.cancelled);
        let release = Arc::clone(&self.release);
        Box::pin(async move {
            entered.notify_one();
            stop.cancelled().await;
            cancelled.notify_one();
            release.acquire().await.unwrap().forget();
            SolveReply::Stopped
        })
    }
}

/// Combined T2 preserves both boundaries: CAPTCHA cleanup blocks CDP disconnect, and
/// transport shutdown delivers a successful cancelled reply to the connected caller.
/// The shared RPC helper keeps HTTP and JSON-RPC failures fatal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captcha_shutdown_waits_for_host_cleanup_before_disconnecting() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let host = Arc::new(DrainingCaptcha {
        entered: Arc::new(Notify::new()),
        cancelled: Arc::new(Notify::new()),
        release: Arc::new(Semaphore::new(0)),
    });
    let server = Server::configured(browser.port(), {
        let host = Arc::clone(&host);
        move |server| {
            server.with_captcha(
                move |_cdp| -> Arc<dyn CaptchaHost> { Arc::<DrainingCaptcha>::clone(&host) },
                "CAPTCHA fixture capability",
            )
        }
    })
    .await;
    let request = tokio::spawn({
        let url = server.url.clone();
        async move {
            let client = Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            run(
                &client,
                &url,
                1,
                json!({"page":fake_browser::PAGE,"code":"return await neo.solveCaptcha(page)"}),
            )
            .await
        }
    });
    time::timeout(Duration::from_secs(5), host.entered.notified())
        .await
        .unwrap();
    let shutdown = tokio::spawn({
        let neo = server.neo.clone();
        async move { neo.shutdown().await }
    });
    time::timeout(Duration::from_secs(5), host.cancelled.notified())
        .await
        .unwrap();
    assert!(
        !shutdown.is_finished(),
        "shutdown cannot abandon host input cleanup"
    );
    assert!(
        server.neo.connected(),
        "CDP remains connected while host input cleanup is held"
    );
    host.release.add_permits(1);
    time::timeout(Duration::from_secs(5), shutdown)
        .await
        .unwrap()
        .unwrap();
    let reply = time::timeout(Duration::from_secs(5), request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply["structuredContent"]["status"], "cancelled", "{reply}");
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
async fn the_first_mcp_run_persists_its_reply_id_and_artifact_pruning_preserves_the_database() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let server = Server::start(browser.port()).await;
    let client = Client::new();
    let code = "return await page.title()";
    let reply = run(
        &client,
        &server.url,
        1,
        json!({"page":fake_browser::PAGE,"code":code}),
    )
    .await;
    assert_eq!(reply["structuredContent"]["status"], "ok", "{reply}");
    let id = run_id(&reply)
        .parse::<neo_trace::RunId>()
        .expect("reply ULID");
    // Shutdown must drain the lazy opener and its first run, not merely the browser work.
    server.neo.shutdown().await;
    let path = server.state.path().join("traces.db");
    assert!(
        path.is_file(),
        "the first MCP request creates the process trace database"
    );
    let artifacts = server.state.path().join("artifacts");
    fs::create_dir_all(&artifacts)
        .await
        .expect("artifact directory");
    let stale = artifacts.join("stale.txt");
    let file = File::create(&stale).expect("old artifact");
    file.set_modified(SystemTime::UNIX_EPOCH)
        .expect("age artifact");
    let pruning = NeoSdkServer::new(
        Config::new(browser.port(), server.state.path().to_path_buf()),
        CancellationToken::new(),
    );
    pruning.shutdown().await;
    assert!(!stale.exists(), "the boot artifact sweep actually ran");
    assert!(
        path.is_file(),
        "pruning artifacts leaves traces.db outside its ownership root"
    );
    let traces = neo_trace::TraceStore::open(&path)
        .await
        .expect("read committed traces");
    let detail = traces
        .run(id)
        .await
        .expect("query trace")
        .expect("the reply identifies a saved run");
    assert_eq!(detail.run.id, run_id(&reply));
    assert_eq!(detail.code, code);
    assert_eq!(detail.run.status, "ok");
    assert_eq!(detail.calls.len(), 1);
    assert_eq!(detail.calls[0].method, "title");
    traces.close().await;
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

#[path = "support/wire.rs"]
mod wire;

// T1 deterministically discriminates W1. Hyper is not controlled here, so this test alone
// cannot discriminate W2; T3 and T3c own that deterministic proof.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_delivers_the_active_modern_runs_cancelled_reply() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let server = Server::start(browser.port()).await;
    let client = Client::new();
    let url = server.url.clone();
    let pending = tokio::spawn(async move {
        let response = wire::modern(&client, &url, 501, "tools/call", json!({"name":"run","arguments":{
            "page":fake_browser::PAGE,
            "code":"await page.title(); page.url(); await new Promise(r => setTimeout(r, 60000))",
            "timeout":120_000
        }})).send().await.unwrap();
        wire::result(response, 501).await
    });
    getter_started(&browser).await;
    server.neo.shutdown().await;
    let result = time::timeout(Duration::from_secs(10), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        result["structuredContent"]["status"], "cancelled",
        "{result}"
    );
    assert!(
        !result["structuredContent"]["calls"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

// T3b supplements T3/T3c. This is a probabilistic real-rmcp handoff stress test, not the
// deterministic proof that the reply token is attached at the correct production layer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn probabilistic_rmcp_handoff_stress_never_loses_a_terminal_reply() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let client = Client::new();
    for iteration in 0..200 {
        let server = Server::start(browser.port()).await;
        assert!(server.neo.ready().await);
        let before = browser.exchanges().len();
        let request = wire::modern(
            &client,
            &server.url,
            iteration,
            "tools/call",
            json!({"name":"run","arguments":{
                "page":fake_browser::PAGE,"code":"return 1"
            }}),
        );
        let pending =
            tokio::spawn(
                async move { wire::result(request.send().await.unwrap(), iteration).await },
            );
        // HTTP arrival alone is not enough: post-shutdown arrivals are legitimately refused.
        // A run's first live getter proves this run was admitted before cancellation.
        wire::until(
            || {
                browser.exchanges()[before..]
                    .iter()
                    .any(|e| e.method() == "Page.getNavigationHistory")
            },
            "the stress run never entered the runtime",
        )
        .await;
        let closing = server.neo.clone();
        let shutdown = tokio::spawn(async move { closing.shutdown().await });
        let result = pending.await.unwrap();
        assert!(
            matches!(
                result["structuredContent"]["status"].as_str(),
                Some("ok" | "cancelled")
            ),
            "iteration {iteration}: {result}"
        );
        shutdown.await.unwrap();
    }
}

// T4 requires HTTP200/cancelled, a complete POST and clean GET EOF, driven concurrently.
// T3c-L proves that GET cannot end while the legacy POST body is unread before the consumer
// grace expires, and that POST ends on its own before transport cancellation. This real-HTTP
// test is non-discriminating for W2 and asserts no client-observed connection order.
#[expect(
    clippy::print_stderr,
    reason = "C5 requires per-run observation order, elapsed time and outcome evidence"
)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_cancelled_post_completes_and_the_open_get_stream_ends() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let server = Server::start(browser.port()).await;
    let client = Client::new();
    let session = wire::legacy_session(&client, &server.url).await;
    let get = wire::legacy_get(&client, &server.url, &session).await;
    let request = wire::legacy_run(
        &client,
        &server.url,
        &session,
        601,
        &json!({
            "page":fake_browser::PAGE,"code":"page.url(); await new Promise(r => setTimeout(r, 60000))",
            "timeout":120_000
        }),
    );
    let pending =
        tokio::spawn(async move { wire::legacy_result(request.send().await.unwrap(), 601).await });
    getter_started(&browser).await;
    let closing = server.neo.clone();
    let started = Instant::now();
    let shutdown = tokio::spawn(async move { closing.shutdown().await });
    let (result, observed_order) = wire::reply_and_get_eof(pending, get).await;
    let elapsed = started.elapsed();
    eprintln!(
        "NEO_LEGACY_SHUTDOWN {}",
        json!({"observedOrder":observed_order,"elapsedMs":elapsed.as_secs_f64() * 1000.0,
            "status":result["structuredContent"]["status"],"postHttpStatus":200,
            "postComplete":true,"getCleanEof":true})
    );
    // Mirrors the crate-private REPLY_DELIVERY_GRACE. A reply/GET ending only at that
    // grace is a failure, even with a terminal result: this is no connection-order claim
    // and creates no new production run or cleanup deadline.
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "legacy shutdown reached the five-second consumer grace: {elapsed:?}"
    );
    assert_eq!(result["structuredContent"]["status"], "cancelled");
    time::timeout(Duration::from_secs(20), shutdown)
        .await
        .unwrap()
        .unwrap();
}

// T5: a listen acknowledgment is an explicit barrier; closure must not need the 5 s
// stalled-consumer grace. The 2 s bound is a harness discriminator, not a cleanup deadline.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_completes_listen_with_the_protocol_final_result_without_the_grace() {
    let server = Server::start(1).await;
    let client = Client::new();
    let listen = wire::Listen::open(&client, &server.url, 701).await;
    time::timeout(Duration::from_secs(2), async {
        tokio::join!(server.neo.shutdown(), listen.complete());
    })
    .await
    .unwrap();
}
