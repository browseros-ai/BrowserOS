//! A run's API request contexts on a real BrowserOS neo (`src/request_contexts.rs`): their
//! connections end with the run. A script reaches a keep-alive server through `page.request` and
//! a `playwright.request.newContext()`, and leaves a request in flight; once the run is over,
//! every connection the server saw is closed, and the next run's requests go on afresh.

#[expect(
    dead_code,
    reason = "the shared fixture also provides navigation helpers"
)]
#[path = "support/browser.rs"]
mod browser;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{self, Instant};

const PAGE: &str = "<!doctype html><title>api</title><p>api</p>";

/// What the keep-alive server saw: connections accepted, and those still open.
#[derive(Default)]
struct Connections {
    accepted: AtomicUsize,
    open: AtomicUsize,
}

/// A keep-alive HTTP/1.1 server on 127.0.0.1: `/hold` never answers, any other path answers
/// `ok`; a connection stays open until its client closes it.
async fn serve() -> (String, Arc<Connections>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("bind: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("address: {error}"));
    let connections = Arc::new(Connections::default());
    let seen = Arc::clone(&connections);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            seen.accepted.fetch_add(1, Ordering::SeqCst);
            seen.open.fetch_add(1, Ordering::SeqCst);
            let open = Arc::clone(&seen);
            tokio::spawn(async move {
                answer(socket).await;
                open.open.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    (format!("http://{address}"), connections)
}

/// Answers each request of one connection until its client closes it.
async fn answer(mut socket: TcpStream) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match socket.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
        while let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
            buffer.drain(..end + 4);
            if head.starts_with("GET /hold") {
                continue;
            }
            let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: keep-alive\r\n\r\nok";
            if socket.write_all(reply).await.is_err() {
                return;
            }
        }
    }
}

async fn run(tab: &Tab, code: &str) -> RunOutcome {
    tab.start_run_within(code.to_string(), Duration::from_mins(1))
        .await
        .await
        .unwrap_or_else(|error| panic!("run task: {error}"))
}

fn value(outcome: &RunOutcome) -> &Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    outcome.value.as_ref().unwrap_or(&Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_runs_api_requests_leave_no_connection_open() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![("/api.html".into(), PAGE.into())],
            ..Fixture::default()
        },
        "/api.html",
    )
    .await;
    let (base, connections) = serve().await;
    let first = run(
        &tab,
        &format!(
            r"const own = await playwright.request.newContext()
const answers = [
  await (await page.request.get('{base}/a')).text(),
  await (await page.request.get('{base}/b')).text(),
  await (await own.get('{base}/c')).text(),
]
// Left in flight: the run's end aborts it.
own.get('{base}/hold').catch(() => {{}})
page.request.get('{base}/hold').catch(() => {{}})
await new Promise((resolve) => setTimeout(resolve, 300))
return answers"
        ),
    )
    .await;
    assert_eq!(value(&first), &json!(["ok", "ok", "ok"]));
    assert!(
        connections.accepted.load(Ordering::SeqCst) >= 2,
        "the two contexts each opened their own connection"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while connections.open.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
        time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        connections.open.load(Ordering::SeqCst),
        0,
        "every connection, pooled or in flight, closed with the run"
    );
    let second = run(
        &tab,
        &format!("return await (await page.request.get('{base}/again')).text()"),
    )
    .await;
    tab.close().await;
    assert_eq!(
        value(&second),
        &json!("ok"),
        "the next run's request context starts afresh: {second:#?}"
    );
}
