//! A stand-in for BrowserOS on a loopback WebSocket, so tests can run scripts without a real
//! browser. It answers the CDP calls a run makes for one open tab, [`PAGE`] at [`PAGE_URL`],
//! including a click's: the lazy domains, the utility world and the glue's page calls (answered
//! by glue name, as a page holding one ready button, [`BUTTON`], would answer them), quads, and
//! mouse input. A test can hold back the answers to the calls a run may be stopped while
//! awaiting (the attach, the navigation-history read, mouse input), close the tab while the
//! page's domains are being enabled, and read back every command with its answer to see what a
//! run did and what it left behind.
//!
//! Test crates include this file with `#[path]`. Cargo builds only top-level `tests/*.rs` files
//! as test crates, so it is never one on its own.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use futures_util::{SinkExt, StreamExt};
use neo_runtime::{PageId, RunRequest, Runtime};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

/// The fake's one open tab.
pub const PAGE: &str = "fake-page";
pub const PAGE_URL: &str = "https://example.test/start";
/// The one button on the fake's page, which only the selector `#go` matches. It is always
/// visible, enabled, stable and the hit target, so a click on it succeeds.
pub const BUTTON: &str = "<button id=\"go\">Go</button>";

/// When the fake answers one CDP method: each of the first `answers` calls after `delay`, and
/// later calls never. On the history read, `answers: 1` lets the run's own read at start
/// through and stalls the script's first `page.url()`.
#[derive(Debug, Clone, Copy)]
pub struct Reply {
    pub delay: Duration,
    pub answers: usize,
}

impl Default for Reply {
    /// Every call, at once.
    fn default() -> Self {
        Self {
            delay: Duration::ZERO,
            answers: usize::MAX,
        }
    }
}

impl Reply {
    /// When to answer the `call`-th call of the method, counted from 0; `None` is never.
    fn when(self, call: usize) -> Option<Duration> {
        (call < self.answers).then_some(self.delay)
    }
}

/// How the fake answers the calls a test can hold back. It answers every other call at once.
#[derive(Debug, Clone, Copy, Default)]
pub struct Replies {
    /// `Target.attachToTarget`. The n-th attach the fake receives gets session `session-<n>`.
    pub attach: Reply,
    /// `Page.getNavigationHistory`, which backs `page.url()`.
    pub history: Reply,
    /// `Input.dispatchMouseEvent`, counted per event: `answers: 2` acknowledges a click's move
    /// and press and holds its release.
    pub mouse: Reply,
    /// The tab closes the moment a session's first `Page.enable` arrives, and the fake does
    /// what BrowserOS does then (measured): it never answers the command in flight, reports
    /// `Target.detachedFromTarget` for the session, and answers every later command on it with
    /// `Session with given id not found.`.
    pub closes_at_page_enable: bool,
}

/// One command the fake received, and its answer once the fake has sent it.
#[derive(Debug, Clone)]
pub struct Exchange {
    pub command: Value,
    pub reply: Option<Value>,
}

impl Exchange {
    pub fn method(&self) -> &str {
        self.command["method"].as_str().unwrap_or_default()
    }
}

/// The running fake. Dropping it stops its listeners and every connection they accepted.
pub struct FakeBrowser {
    port: u16,
    exchanges: Arc<Mutex<Vec<Exchange>>>,
    tasks: [JoinHandle<()>; 2],
}

impl FakeBrowser {
    pub async fn start(replies: Replies) -> Self {
        let exchanges = Arc::new(Mutex::new(Vec::new()));
        let (websocket, websocket_port) = bind().await;
        let recorded = Arc::clone(&exchanges);
        let websocket_task = tokio::spawn(async move {
            // Aborting this task drops the set, which ends the connections it accepted too.
            let mut connections = JoinSet::new();
            while let Ok((stream, _)) = websocket.accept().await {
                connections.spawn(serve_cdp(stream, replies, Arc::clone(&recorded)));
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
            exchanges,
            tasks: [websocket_task, discovery_task],
        }
    }

    pub async fn runtime(&self) -> Runtime {
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

    /// Every command received so far, in order, each with its answer if the fake sent one.
    pub fn exchanges(&self) -> Vec<Exchange> {
        lock(&self.exchanges).clone()
    }
}

impl Drop for FakeBrowser {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// A run of `code` against the fake's tab.
pub fn run_request(code: &str, timeout: Duration, cancel: CancellationToken) -> RunRequest {
    RunRequest {
        code: code.to_string(),
        page: PageId::new(PAGE.to_string()),
        timeout,
        cancel,
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

fn lock(exchanges: &Mutex<Vec<Exchange>>) -> MutexGuard<'_, Vec<Exchange>> {
    exchanges
        .lock()
        .unwrap_or_else(|error| panic!("a fake browser task panicked: {error}"))
}

/// The glue object's id in the fake's page.
pub const GLUE: &str = "glue";

/// One connection's state: how many times each held-back method has been called, and the page
/// objects the fake handed out.
#[derive(Default)]
struct Calls {
    attach: usize,
    history: usize,
    mouse: usize,
    /// Whether `install.js` ran, so a later `attach.js` finds the glue.
    installed: bool,
    /// Sessions whose tab closed under them (`Replies::closes_at_page_enable`).
    closed: HashSet<String>,
    /// Returned page objects by id, with the own properties `Runtime.getProperties` lists.
    objects: HashMap<String, Value>,
    next_object: usize,
}

impl Calls {
    /// A fresh page object id.
    fn object(&mut self, kind: &str) -> String {
        self.next_object += 1;
        format!("{kind}-{}", self.next_object)
    }
}

/// Serves one CDP connection. Answers go out through one writer task, so an answer held back
/// for a while never delays the ones after it.
async fn serve_cdp(stream: TcpStream, replies: Replies, exchanges: Arc<Mutex<Vec<Exchange>>>) {
    let Ok(socket) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    let (mut sink, mut source) = socket.split();
    let (outgoing, mut queued) = mpsc::unbounded_channel::<String>();
    // Dropped, and so aborted, when this connection ends.
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        while let Some(text) = queued.recv().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                return;
            }
        }
    });
    let mut calls = Calls::default();
    while let Some(Ok(message)) = source.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(command) = serde_json::from_str::<Value>(text.as_str()) else {
            continue;
        };
        let index = {
            let mut exchanges = lock(&exchanges);
            exchanges.push(Exchange {
                command: command.clone(),
                reply: None,
            });
            exchanges.len() - 1
        };
        let session = command["sessionId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let (mut reply, when) = if calls.closed.contains(&session) {
            (
                json!({ "error": { "code": -32001, "message": "Session with given id not found." } }),
                Some(Duration::ZERO),
            )
        } else if replies.closes_at_page_enable && command["method"] == "Page.enable" {
            calls.closed.insert(session.clone());
            let detached = json!({
                "method": "Target.detachedFromTarget",
                "params": { "sessionId": session, "targetId": PAGE },
            });
            // Fails only once the connection is gone, when nobody is left to read it.
            let _ = outgoing.send(detached.to_string());
            (Value::Null, None)
        } else {
            answer(&command, replies, &mut calls)
        };
        let Some(delay) = when else {
            continue;
        };
        reply["id"] = command["id"].clone();
        if !command["sessionId"].is_null() {
            reply["sessionId"] = command["sessionId"].clone();
        }
        if delay.is_zero() {
            deliver(&exchanges, index, reply, &outgoing);
        } else {
            let exchanges = Arc::clone(&exchanges);
            let outgoing = outgoing.clone();
            tasks.spawn(async move {
                time::sleep(delay).await;
                deliver(&exchanges, index, reply, &outgoing);
            });
        }
    }
}

/// The fake's answer to `command`, and when to send it.
fn answer(command: &Value, replies: Replies, calls: &mut Calls) -> (Value, Option<Duration>) {
    let now = Some(Duration::ZERO);
    match command["method"].as_str().unwrap_or_default() {
        "Browser.getTabInfo" if command["params"]["targetId"] == PAGE => {
            (json!({ "result": { "tab": {} } }), now)
        }
        // BrowserOS's answer for a tab that does not exist, which `Runtime::connect` checks.
        "Browser.getTabInfo" => (
            json!({ "error": { "code": -32000, "message": "No target with given id" } }),
            now,
        ),
        "Target.attachToTarget" => {
            let call = calls.attach;
            calls.attach += 1;
            let session = format!("session-{}", call + 1);
            (
                json!({ "result": { "sessionId": session } }),
                replies.attach.when(call),
            )
        }
        "Page.getNavigationHistory" => {
            let call = calls.history;
            calls.history += 1;
            (
                json!({
                    "result": { "currentIndex": 0, "entries": [{ "id": 1, "url": PAGE_URL }] },
                }),
                replies.history.when(call),
            )
        }
        "Target.detachFromTarget"
        | "Browser.getVersion"
        | "Page.enable"
        | "Emulation.setFocusEmulationEnabled"
        | "Runtime.releaseObjectGroup"
        | "DOM.scrollIntoViewIfNeeded" => (json!({ "result": {} }), now),
        "Page.createIsolatedWorld" => (json!({ "result": { "executionContextId": 7 } }), now),
        "Runtime.callFunctionOn" => (call_function_on(&command["params"], calls), now),
        "Runtime.getProperties" => {
            let object = command["params"]["objectId"].as_str().unwrap_or_default();
            let properties = calls
                .objects
                .get(object)
                .cloned()
                .unwrap_or_else(|| json!([]));
            (json!({ "result": { "result": properties } }), now)
        }
        "DOM.getContentQuads" => (
            json!({ "result": { "quads": [[8, 8, 65.5, 8, 65.5, 29, 8, 29]] } }),
            now,
        ),
        "DOM.getBoxModel" => {
            let quad = json!([8, 8, 65.5, 8, 65.5, 29, 8, 29]);
            (
                json!({ "result": { "model": {
                    "content": quad, "padding": quad, "border": quad, "margin": quad,
                    "width": 58, "height": 21,
                } } }),
                now,
            )
        }
        "Input.dispatchMouseEvent" => {
            let call = calls.mouse;
            calls.mouse += 1;
            (json!({ "result": {} }), replies.mouse.when(call))
        }
        method => (
            json!({ "error": { "code": -32601, "message": format!("'{method}' wasn't found") } }),
            now,
        ),
    }
}

/// `Runtime.callFunctionOn`: neo-runtime-injected's bundle entries by execution context, and
/// glue calls on the glue object. A by-value result is the glue's JSON; a result that holds a
/// handle is a page object whose properties `Runtime.getProperties` then lists.
fn call_function_on(params: &Value, calls: &mut Calls) -> Value {
    if params["executionContextId"].is_i64() {
        // `attach.js` takes the key; `install.js` takes the key, the spec hash and the
        // initializer.
        let installing = params["arguments"]
            .as_array()
            .is_some_and(|args| args.len() == 3);
        calls.installed |= installing;
        return if calls.installed {
            json!({ "result": { "result": { "type": "object", "objectId": GLUE } } })
        } else {
            json!({ "result": { "result": { "type": "undefined" } } })
        };
    }
    let name = params["arguments"][0]["value"].as_str().unwrap_or_default();
    let glue_params = &params["arguments"][1]["value"];
    let value = match name {
        "describeSelector" => json!({ "locator": glue_params["selector"] }),
        "checkElementStates" | "scrollIntoView" => json!({}),
        "viewportSize" => json!({ "width": 800, "height": 600 }),
        "getElementBorderWidth" => json!({ "left": 0, "top": 0 }),
        "stopHitTargetInterceptor" => json!({ "outcome": "done" }),
        "resolve" => {
            let properties = if glue_params["selector"] == "#go" {
                let node = calls.object("node");
                json!([
                    property(
                        "element",
                        &json!({ "type": "object", "subtype": "node", "objectId": node })
                    ),
                    property("count", &json!({ "type": "number", "value": 1 })),
                    property("preview", &json!({ "type": "string", "value": BUTTON })),
                ])
            } else {
                json!([
                    property("element", &json!({ "type": "undefined" })),
                    property("count", &json!({ "type": "number", "value": 0 })),
                    property("preview", &json!({ "type": "undefined" })),
                ])
            };
            return page_object(calls, properties);
        }
        "setupHitTargetInterceptor" => {
            let interceptor = calls.object("interceptor");
            let properties = json!([property(
                "outcome",
                &json!({ "type": "object", "className": "Object", "objectId": interceptor }),
            )]);
            return page_object(calls, properties);
        }
        _ => {
            return json!({ "error": { "code": -32000, "message": format!("no glue function {name}") } });
        }
    };
    json!({ "result": { "result": { "type": "object", "value": value } } })
}

/// A result object holding `properties`, as the glue's handle-holding results come back.
fn page_object(calls: &mut Calls, properties: Value) -> Value {
    let id = calls.object("result");
    calls.objects.insert(id.clone(), properties);
    json!({ "result": { "result": { "type": "object", "className": "Object", "objectId": id } } })
}

/// One own data property, as `Runtime.getProperties` describes it.
fn property(name: &str, value: &Value) -> Value {
    json!({ "name": name, "value": value, "configurable": true, "enumerable": true, "isOwn": true })
}

/// Records `reply` as the answer to exchange `index`, then sends it. Recording first means a
/// test that sees a session's detach also sees the attach answer that handed it out.
fn deliver(
    exchanges: &Mutex<Vec<Exchange>>,
    index: usize,
    reply: Value,
    outgoing: &UnboundedSender<String>,
) {
    let text = reply.to_string();
    if let Some(exchange) = lock(exchanges).get_mut(index) {
        exchange.reply = Some(reply);
    }
    // Fails only once the connection is gone, when nobody is left to read the answer.
    let _ = outgoing.send(text);
}
