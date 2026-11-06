//! The realm's limits and conversions, through `Runtime::run`, against a loopback peer that
//! answers the few CDP calls a run makes for one fake tab. No browser needed.

use std::time::{Duration, Instant};

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use futures_util::{SinkExt, StreamExt};
use neo_runtime::{PageId, RunOutcome, RunRequest, RunStatus, Runtime};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio::time;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

const PAGE: &str = "fake-page";
const PAGE_URL: &str = "https://example.test/start";

/// A browser with one open tab, `PAGE`, at `PAGE_URL`. Its tasks end with the fixture.
struct FakeBrowser {
    port: u16,
    tasks: [JoinHandle<()>; 2],
}

impl FakeBrowser {
    async fn start() -> Self {
        let (websocket, websocket_port) = bind().await;
        let websocket_task = tokio::spawn(async move {
            while let Ok((stream, _)) = websocket.accept().await {
                tokio::spawn(serve_cdp(stream));
            }
        });
        let (discovery, port) = bind().await;
        let discovery_task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = discovery.accept().await {
                let mut request = Vec::new();
                let mut buffer = [0_u8; 1024];
                while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => break,
                        Ok(read) => request.extend_from_slice(&buffer[..read]),
                    }
                }
                let body = json!({
                    "webSocketDebuggerUrl": format!("ws://127.0.0.1:{websocket_port}/devtools/browser/fake"),
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                // A failed write shows up as the client's failed discovery.
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        Self {
            port,
            tasks: [websocket_task, discovery_task],
        }
    }

    async fn runtime(&self) -> Runtime {
        let cdp = CdpClient::connect(ConnectOptions {
            host: Some("127.0.0.1".to_string()),
            reconnect_policy: ReconnectPolicy::KeepTrying,
            ..ConnectOptions::new(self.port)
        })
        .await
        .unwrap_or_else(|error| panic!("connect to the fake browser: {error}"));
        Runtime::connect(cdp).await.unwrap_or_else(|error| {
            panic!("the fake browser reports closed tabs as BrowserOS does: {error}")
        })
    }
}

async fn bind() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("bind a loopback port: {error}"));
    let port = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("read the bound port: {error}"))
        .port();
    (listener, port)
}

impl Drop for FakeBrowser {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn serve_cdp(stream: TcpStream) {
    let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    while let Some(Ok(message)) = socket.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(request) = serde_json::from_str::<Value>(text.as_str()) else {
            continue;
        };
        let reply = match (
            request["method"].as_str(),
            request["params"]["targetId"].as_str(),
        ) {
            (Some("Browser.getTabInfo"), Some(PAGE)) => json!({ "result": { "tab": {} } }),
            (Some("Browser.getTabInfo"), _) => {
                json!({ "error": { "code": -32000, "message": "No target with given id" } })
            }
            (Some("Target.attachToTarget"), _) => {
                json!({ "result": { "sessionId": "fake-session" } })
            }
            (Some("Page.getNavigationHistory"), _) => json!({
                "result": { "currentIndex": 0, "entries": [{ "id": 1, "url": PAGE_URL }] },
            }),
            (Some("Target.detachFromTarget" | "Browser.getVersion"), _) => {
                json!({ "result": {} })
            }
            (method, _) => json!({
                "error": { "code": -32601, "message": format!("'{method:?}' wasn't found") },
            }),
        };
        let mut reply = reply;
        reply["id"] = request["id"].clone();
        if !request["sessionId"].is_null() {
            reply["sessionId"] = request["sessionId"].clone();
        }
        if socket
            .send(Message::Text(reply.to_string().into()))
            .await
            .is_err()
        {
            return;
        }
    }
}

async fn run(code: &str, timeout: Duration) -> RunOutcome {
    let browser = FakeBrowser::start().await;
    browser
        .runtime()
        .await
        .run(RunRequest {
            code: code.to_string(),
            page: PageId::new(PAGE.to_string()),
            timeout,
            cancel: CancellationToken::new(),
        })
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn an_infinite_loop_ends_as_a_timeout() {
    let started = Instant::now();
    let outcome = run("while (true) {}", Duration::from_millis(300)).await;

    assert_eq!(outcome.status, RunStatus::Timeout, "{outcome:?}");
    let error = outcome
        .error
        .expect("a timeout carries Playwright's TimeoutError");
    assert_eq!(error.name, "TimeoutError", "{error:?}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the interrupt handler stops running JS at the deadline, took {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_timer_past_the_deadline_ends_as_a_timeout() {
    let started = Instant::now();
    let code = "await new Promise((resolve) => setTimeout(resolve, 60_000)); return 'woke'";
    let outcome = run(code, Duration::from_millis(300)).await;

    assert_eq!(outcome.status, RunStatus::Timeout, "{outcome:?}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a script parked on a timer still ends at the deadline, took {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_ends_the_run_as_cancelled() {
    let browser = FakeBrowser::start().await;
    let runtime = browser.runtime().await;
    let cancel = CancellationToken::new();
    let request = RunRequest {
        code: "await new Promise((resolve) => setTimeout(resolve, 60_000))".to_string(),
        page: PageId::new(PAGE.to_string()),
        timeout: Duration::from_secs(30),
        cancel: cancel.clone(),
    };
    let canceller = tokio::spawn(async move {
        time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
    });
    let outcome = runtime.run(request).await;
    canceller.await.unwrap();

    assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_syntax_error_is_an_error_named_syntax_error() {
    let outcome = run("return {", Duration::from_secs(5)).await;

    assert_eq!(outcome.status, RunStatus::Error, "{outcome:?}");
    let error = outcome.error.expect("a failed run says why");
    assert_eq!(error.name, "SyntaxError", "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thrown_error_keeps_its_name_and_the_logs_before_it() {
    let code = "console.log('before', { step: 1 }); null.boom";
    let outcome = run(code, Duration::from_secs(5)).await;

    assert_eq!(outcome.status, RunStatus::Error, "{outcome:?}");
    assert_eq!(outcome.logs, ["before {\"step\":1}"], "{outcome:?}");
    let error = outcome.error.expect("a failed run says why");
    assert_eq!(error.name, "TypeError", "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn return_values_convert_to_json() {
    let cases = [
        (
            "return { a: 1, b: [true, null] }",
            Some(json!({ "a": 1, "b": [true, null] })),
        ),
        (
            "return [1, 'two', { three: 3 }]",
            Some(json!([1, "two", { "three": 3 }])),
        ),
        ("return 'text'", Some(json!("text"))),
        ("return 4.5", Some(json!(4.5))),
        ("return null", Some(Value::Null)),
        ("return undefined", None),
        ("await Promise.resolve()", None),
    ];
    for (code, expected) in cases {
        let outcome = run(code, Duration::from_secs(5)).await;
        assert_eq!(outcome.status, RunStatus::Ok, "{code}: {outcome:?}");
        assert_eq!(outcome.value, expected, "{code}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_page_getters_read_the_browser() {
    let code = "return { url: page.url(), closed: page.isClosed() }";
    let outcome = run(code, Duration::from_secs(5)).await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value,
        Some(json!({ "url": PAGE_URL, "closed": false })),
        "{outcome:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_script_cannot_read_a_page_outside_its_run() {
    let code = "__neo_runtime.get(JSON.stringify({ guid: 'page@another-tab', field: 'url' }))";
    let outcome = run(code, Duration::from_secs(5)).await;

    assert_eq!(outcome.status, RunStatus::Error, "{outcome:?}");
    let error = outcome.error.expect("a failed run says why");
    assert!(
        error.message.contains("page@another-tab"),
        "the bridge names the guid it refused: {error:?}"
    );
}
