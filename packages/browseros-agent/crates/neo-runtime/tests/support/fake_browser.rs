//! A stand-in for BrowserOS on a loopback WebSocket, so tests can run scripts without a real
//! browser. It answers the CDP calls a run makes for one open tab, [`PAGE`] at [`PAGE_URL`],
//! including a click's: the lazy domains, the utility world and the glue's page calls (answered
//! by glue name, as a page holding one ready button, [`BUTTON`], would answer them), quads, and
//! mouse input. A test can hold back the answers to the calls a run may be stopped while
//! awaiting (the attach, the navigation-history read, the hit-target setup, mouse input), close
//! the tab while the page's domains are being enabled, and read back every command with its
//! answer to see what a run did and what it left behind.
//!
//! The page also keeps the one piece of state a run can leave in it: the injected script's
//! hit-target interceptor slot ([`FakeBrowser::armed_interceptor`]), and the clicks that reached
//! the page's handlers past it ([`FakeBrowser::page_clicks`]).
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
    /// Review probe: fail the acknowledgement that enables drag interception.
    pub fail_drag_enable: bool,
    /// Review probe: delay only a mouse press acknowledgement.
    pub mouse_press_delay: Option<Duration>,
    /// `Target.attachToTarget`. The n-th attach the fake receives gets session `session-<n>`.
    pub attach: Reply,
    /// An optional schedule for attaches to tabs other than the initial run tab.
    pub new_page_attach: Option<Reply>,
    /// Empty committed history URLs before a newly created page reaches about:blank.
    pub new_page_empty_history: usize,
    /// The new target may already have navigated before its first history read.
    pub new_page_url: Option<&'static str>,
    /// The tab exists immediately, but its creation acknowledgement arrives later.
    pub create_tab_delay: Option<Duration>,
    /// Closing removes the tab immediately, before acknowledging the request.
    pub close_tab_delay: Option<Duration>,
    /// Beforeunload completes after the Page.close acknowledgement, provided its watcher
    /// is still attached to answer the delayed dialog.
    pub before_unload_delay: Option<Duration>,
    /// `Page.getNavigationHistory`, which backs `page.url()`.
    pub history: Reply,
    /// `Input.dispatchMouseEvent`, counted per event: `answers: 2` acknowledges a click's move
    /// and press and holds its release.
    pub mouse: Reply,
    /// The `setupHitTargetInterceptor` glue call. The page arms the interceptor when the call
    /// arrives, as a real page does before it answers, so holding the answer back leaves it
    /// armed with the run not knowing yet.
    pub arm: Reply,
    /// Every press lands on another element after the hit check passed: the armed interceptor
    /// blocks it and every later event of its click, as a real one does, and its stop names
    /// that element.
    pub press_lands_elsewhere: bool,
    pub title: Reply,
    pub world: Reply,
    /// A commit after the named operation starts, before its delayed answer is delivered.
    pub commit_during: Option<CommitDuring>,
    /// The tab closes the moment a session's first `Page.enable` arrives, and the fake does
    /// what BrowserOS does then (measured): it never answers the command in flight, reports
    /// `Target.detachedFromTarget` for the session, and answers every later command on it with
    /// `Session with given id not found.`.
    pub closes_at_page_enable: bool,
    /// Optional navigation script for commit/reply ordering tests.
    pub navigation: Option<NavigationScript>,
}

#[derive(Debug, Clone, Copy)]
#[expect(
    dead_code,
    reason = "Each test binary exercises different commit boundaries"
)]
pub enum CommitDuring {
    Arm,
    Title,
    Attach,
}

#[derive(Debug, Clone, Copy)]
pub struct NavigationScript {
    pub reply_after: Duration,
    pub commit_after: Duration,
    pub same_document: bool,
    pub stop_loading: bool,
    pub restored_from_bfcache: bool,
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
    pub page: Arc<Mutex<Page>>,
    tasks: [JoinHandle<()>; 2],
}

/// The page's own state, shared by every connection, as one tab is.
pub struct Page {
    pub tabs: Vec<Value>,
    /// Review probe: the page's drag listeners survive object-group release.
    pub drag_listeners: bool,
    pub frames: Option<FrameReplies>,
    pub tab_info_delay: Option<Duration>,
    next_tab: usize,
    active_window: i64,
    next_window: i64,
    detached: HashSet<String>,
    /// The injected script's single hit-target interceptor slot, as Playwright keeps it: a setup
    /// replaces it, and a stop clears it only while it still holds the stopped interceptor.
    /// Releasing an object group leaves it alone.
    interceptor: Option<Interceptor>,
    next_interceptor: usize,
    /// Releases the page handled with no blocking interceptor armed: each fired the button's
    /// `click`. A release counts when the fake acknowledges it, as Chromium acknowledges a
    /// mouse event only after the page has dispatched it.
    clicks: usize,
}

/// Recorded CDP shapes used to exercise the full local/remote frame-tree join.
#[derive(Default)]
pub struct FrameReplies {
    pub targets: Vec<Value>,
    /// Page.getFrameTree answers, keyed by the session's target id.
    pub trees: HashMap<String, Value>,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            drag_listeners: false,
            tabs: vec![json!({
                "targetId": PAGE, "url": PAGE_URL, "windowId": 1,
                "liveBrowserContextId": "profile", "isActive": true,
            })],
            frames: None,
            tab_info_delay: None,
            next_tab: 0,
            active_window: 1,
            next_window: 2,
            detached: HashSet::new(),
            interceptor: None,
            next_interceptor: 0,
            clicks: 0,
        }
    }
}

impl Page {
    fn add_tab(&mut self, window: i64, url: &str) -> Value {
        self.next_tab += 1;
        let tab = json!({
            "targetId":format!("new-page-{}",self.next_tab), "url":url, "windowId":window,
            "liveBrowserContextId":"profile", "isActive":false,
        });
        self.tabs.push(tab.clone());
        tab
    }
}

struct Interceptor {
    id: String,
    /// Stops every event: armed with `blockAllEvents`, or by a press that landed elsewhere
    /// ([`Replies::press_lands_elsewhere`]).
    blocks_all: bool,
}

impl FakeBrowser {
    #[expect(
        clippy::large_types_passed_by_value,
        reason = "the fake owns its replies and copies them into every connection it serves"
    )]
    pub async fn start(replies: Replies) -> Self {
        let exchanges = Arc::new(Mutex::new(Vec::new()));
        let page = Arc::new(Mutex::new(Page::default()));
        let (websocket, websocket_port) = bind().await;
        let recorded = Arc::clone(&exchanges);
        let shared_page = Arc::clone(&page);
        let websocket_task = tokio::spawn(async move {
            // Aborting this task drops the set, which ends the connections it accepted too.
            let mut connections = JoinSet::new();
            while let Ok((stream, _)) = websocket.accept().await {
                connections.spawn(serve_cdp(
                    stream,
                    replies,
                    Arc::clone(&recorded),
                    Arc::clone(&shared_page),
                ));
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
            page,
            tasks: [websocket_task, discovery_task],
        }
    }

    pub async fn cdp(&self) -> CdpClient {
        CdpClient::connect(ConnectOptions {
            host: Some("127.0.0.1".to_string()),
            reconnect_policy: ReconnectPolicy::KeepTrying,
            ..ConnectOptions::new(self.port)
        })
        .await
        .unwrap_or_else(|error| panic!("connect to the fake browser: {error}"))
    }

    pub async fn runtime(&self) -> Runtime {
        Runtime::connect(self.cdp().await)
            .await
            .unwrap_or_else(|error| {
                panic!("the fake browser reports closed tabs as BrowserOS does: {error}")
            })
    }

    /// Every command received so far, in order, each with its answer if the fake sent one.
    pub fn exchanges(&self) -> Vec<Exchange> {
        lock(&self.exchanges).clone()
    }

    /// The interceptor armed in the page's slot now, by object id.
    pub fn armed_interceptor(&self) -> Option<String> {
        lock(&self.page)
            .interceptor
            .as_ref()
            .map(|interceptor| interceptor.id.clone())
    }

    /// How many clicks reached the page's handlers.
    pub fn page_clicks(&self) -> usize {
        lock(&self.page).clicks
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
        artifacts: None,
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

fn lock<T>(shared: &Mutex<T>) -> MutexGuard<'_, T> {
    shared
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
    sessions: HashMap<String, String>,
    history: usize,
    new_page_history: usize,
    mouse: usize,
    arm: usize,
    title: usize,
    world: usize,
    committed: bool,
    /// Which document the current utility world belongs to, for stale-read behavior tests.
    world_document: bool,
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
#[expect(
    clippy::large_types_passed_by_value,
    reason = "each connection's task owns its copy of the fake's replies"
)]
async fn serve_cdp(
    stream: TcpStream,
    replies: Replies,
    exchanges: Arc<Mutex<Vec<Exchange>>>,
    page: Arc<Mutex<Page>>,
) {
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
        if command["method"] == "Page.close"
            && let Some(delay) = replies.before_unload_delay
        {
            tasks.spawn(finish_before_unload(
                delay,
                Arc::clone(&page),
                outgoing.clone(),
                calls.sessions.get(&session).cloned(),
                session.clone(),
            ));
        }
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
            answer(&command, &replies, &mut calls, &page)
        };
        navigation_events(&command, &replies, &mut calls, &outgoing, &page, &mut tasks);
        let Some(delay) = when else {
            continue;
        };
        reply["id"] = command["id"].clone();
        if !command["sessionId"].is_null() {
            reply["sessionId"] = command["sessionId"].clone();
        }
        let release = command["method"] == "Input.dispatchMouseEvent"
            && command["params"]["type"] == "mouseReleased";
        let delivery = Delivery {
            exchanges: Arc::clone(&exchanges),
            index,
            page: release.then(|| Arc::clone(&page)),
            outgoing: outgoing.clone(),
        };
        if delay.is_zero() {
            delivery.send(reply);
        } else {
            tasks.spawn(async move {
                time::sleep(delay).await;
                delivery.send(reply);
            });
        }
    }
}

/// The navigation fakes' events for `command`: the lifecycle replay `setLifecycleEventsEnabled`
/// triggers, a commit injected during a scripted call, and a scripted navigation's events.
fn navigation_events(
    command: &Value,
    replies: &Replies,
    calls: &mut Calls,
    outgoing: &UnboundedSender<String>,
    page: &Arc<Mutex<Page>>,
    tasks: &mut JoinSet<()>,
) {
    let session = command["sessionId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if command["method"] == "Page.setLifecycleEventsEnabled" {
        for name in ["init", "DOMContentLoaded", "load"] {
            let _ = outgoing.send(
                json!({"sessionId":session,"method":"Page.lifecycleEvent",
                "params":{"frameId":PAGE,"loaderId":"initial","name":name}})
                .to_string(),
            );
        }
    }
    commit_during_call(command, replies, calls, outgoing);
    if command["method"] == "Page.navigate"
        && let Some(script) = replies.navigation
    {
        tasks.spawn(emit_navigation(
            outgoing.clone(),
            session,
            command["params"]["url"].clone(),
            script,
            Arc::clone(page),
        ));
    }
}

fn commit_during_call(
    command: &Value,
    replies: &Replies,
    calls: &mut Calls,
    outgoing: &UnboundedSender<String>,
) {
    let method = command["method"].as_str().unwrap_or_default();
    let glue = command["params"]["arguments"][0]["value"]
        .as_str()
        .unwrap_or_default();
    let commit = match replies.commit_during {
        Some(CommitDuring::Arm) => glue == "setupHitTargetInterceptor" && calls.arm == 1,
        Some(CommitDuring::Title) => glue == "title" && calls.title == 1,
        Some(CommitDuring::Attach) => method == "Page.createIsolatedWorld" && calls.world == 1,
        None => false,
    };
    if commit && !calls.committed {
        calls.committed = true;
        let _ = outgoing.send(json!({
            "sessionId":command["sessionId"], "method":"Page.frameNavigated",
            "params":{"frame":{"id":PAGE,"loaderId":"next","url":"https://example.test/next"},"type":"Navigation"}
        }).to_string());
    }
}

async fn emit_navigation(
    outgoing: UnboundedSender<String>,
    session: String,
    url: Value,
    script: NavigationScript,
    page: Arc<Mutex<Page>>,
) {
    time::sleep(script.commit_after).await;
    if let Some(tab) = lock(&page)
        .tabs
        .iter_mut()
        .find(|tab| tab["targetId"] == PAGE)
    {
        tab["url"] = url.clone();
    }
    let send = |method: &str, params: Value| {
        let _ =
            outgoing.send(json!({"sessionId":session,"method":method,"params":params}).to_string());
    };
    if !script.same_document {
        // The document's request and response precede its commit, as BrowserOS sends them.
        send(
            "Network.requestWillBeSent",
            json!({"requestId":"new","loaderId":"new","type":"Document","frameId":PAGE,
                   "request":{"url":url,"method":"GET","headers":{"Accept":"text/html"}},
                   "timestamp":1.0,"wallTime":1_700_000_000.0}),
        );
        send(
            "Network.responseReceived",
            json!({"requestId":"new","type":"Document","frameId":PAGE,"hasExtraInfo":false,
                   "response":{"url":url,"status":200,"statusText":"OK",
                               "headers":{"Content-Type":"text/html"},"protocol":"http/1.1",
                               "remoteIPAddress":"127.0.0.1","remotePort":443}}),
        );
    }
    let (method, params) = if script.same_document {
        (
            "Page.navigatedWithinDocument",
            json!({"frameId":PAGE,"url":url}),
        )
    } else {
        (
            "Page.frameNavigated",
            json!({"frame":{"id":PAGE,"loaderId":"new","url":url},"type":if script.restored_from_bfcache { "BackForwardCacheRestore" } else { "Navigation" }}),
        )
    };
    send(method, params);
    if script.stop_loading {
        let _ = outgoing.send(json!({"sessionId":session,"method":"Page.frameStoppedLoading","params":{"frameId":PAGE}}).to_string());
    } else {
        for name in ["DOMContentLoaded", "load"] {
            let loader = if script.same_document {
                "initial"
            } else {
                "new"
            };
            let _ = outgoing.send(json!({"sessionId":session,"method":"Page.lifecycleEvent","params":{"frameId":PAGE,"loaderId":loader,"name":name}}).to_string());
        }
    }
}

/// Delayed beforeunload requires the run's watcher to still own its session.
async fn finish_before_unload(
    delay: Duration,
    page: Arc<Mutex<Page>>,
    outgoing: UnboundedSender<String>,
    target: Option<String>,
    session: String,
) {
    time::sleep(delay).await;
    {
        let mut page = lock(&page);
        if page.detached.contains(&session) {
            return;
        }
        page.tabs
            .retain(|tab| tab["targetId"].as_str() != target.as_deref());
    }
    let _sent = outgoing.send(
        json!({
            "method":"Target.detachedFromTarget", "params":{"sessionId":session},
        })
        .to_string(),
    );
}

fn bring_to_front(command: &Value, calls: &Calls, page: &Mutex<Page>) -> Value {
    let target = command["sessionId"]
        .as_str()
        .and_then(|id| calls.sessions.get(id));
    let open = lock(page)
        .tabs
        .iter()
        .any(|tab| tab["targetId"].as_str() == target.map(String::as_str));
    if open {
        json!({"result":{}})
    } else {
        json!({"error":{"code":-32001,"message":"Session with given id not found."}})
    }
}

/// The fake's answer to `command`, and when to send it.
fn answer(
    command: &Value,
    replies: &Replies,
    calls: &mut Calls,
    page: &Mutex<Page>,
) -> (Value, Option<Duration>) {
    let now = Some(Duration::ZERO);
    if let Some(answer) = tab_command(command, replies, page)
        .or_else(|| frame_command(command, calls, page).map(|reply| (reply, now)))
        .or_else(|| input_command(command, replies, calls))
    {
        return answer;
    }
    match command["method"].as_str().unwrap_or_default() {
        "Target.getTargets" => (json!({"result":{"targetInfos":[]}}), now),
        "Page.bringToFront" => (bring_to_front(command, calls, page), now),
        "Target.attachToTarget" => {
            let call = calls.attach;
            calls.attach += 1;
            let session = format!("session-{}", call + 1);
            let target = command["params"]["targetId"].as_str().unwrap_or_default();
            calls.sessions.insert(session.clone(), target.to_string());
            (
                json!({ "result": { "sessionId": session } }),
                if target == PAGE {
                    replies.attach.when(call)
                } else {
                    replies
                        .new_page_attach
                        .unwrap_or(replies.attach)
                        .when(call.saturating_sub(1))
                },
            )
        }
        "Page.getNavigationHistory" => navigation_history(command, replies, calls, page),
        "Target.detachFromTarget" => {
            lock(page).detached.insert(
                command["params"]["sessionId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            );
            (json!({"result":{}}), now)
        }
        "Browser.getVersion"
        | "Target.setDiscoverTargets"
        | "Browser.setDownloadBehavior"
        | "Browser.cancelDownload"
        | "Page.setInterceptFileChooserDialog"
        | "Page.close"
        | "Page.enable"
        | "Network.enable"
        | "Page.setLifecycleEventsEnabled"
        | "Emulation.setFocusEmulationEnabled"
        | "Runtime.releaseObjectGroup"
        | "DOM.scrollIntoViewIfNeeded" => (json!({ "result": {} }), now),
        "Page.navigate" => match replies.navigation {
            Some(script) => {
                let result = if script.same_document {
                    json!({"frameId":PAGE})
                } else {
                    json!({"frameId":PAGE,"loaderId":"new"})
                };
                (json!({"result":result}), Some(script.reply_after))
            }
            None => (json!({"result":{"frameId":PAGE,"loaderId":"new"}}), None),
        },
        "Page.createIsolatedWorld" => {
            let call = calls.world;
            calls.world += 1;
            calls.world_document = calls.committed;
            (
                json!({ "result": { "executionContextId": 7 } }),
                replies.world.when(call),
            )
        }
        "Runtime.callFunctionOn" => {
            let params = &command["params"];
            let when = glue_delay(params, calls, replies);
            (call_function_on(params, calls, page, replies), when)
        }
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
        "DOM.getBoxModel" => (box_model(), now),
        method => (
            json!({ "error": { "code": -32601, "message": format!("'{method}' wasn't found") } }),
            now,
        ),
    }
}

/// The input commands: drag interception (the review probe can fail its enable) and mouse
/// events (a press can be held back on its own).
fn input_command(
    command: &Value,
    replies: &Replies,
    calls: &mut Calls,
) -> Option<(Value, Option<Duration>)> {
    let now = Some(Duration::ZERO);
    match command["method"].as_str().unwrap_or_default() {
        "Input.setInterceptDrags" => Some(
            if replies.fail_drag_enable && command["params"]["enabled"] == true {
                (
                    json!({"error":{"code":-32000,"message":"injected enable failure"}}),
                    now,
                )
            } else {
                (json!({"result":{}}), now)
            },
        ),
        "Input.dispatchMouseEvent" => {
            let call = calls.mouse;
            calls.mouse += 1;
            let delay = if command["params"]["type"] == "mousePressed" {
                replies
                    .mouse_press_delay
                    .or_else(|| replies.mouse.when(call))
            } else {
                replies.mouse.when(call)
            };
            Some((json!({ "result": {} }), delay))
        }
        _ => None,
    }
}

fn box_model() -> Value {
    let quad = json!([8, 8, 65.5, 8, 65.5, 29, 8, 29]);
    json!({ "result": { "model": { "content":quad, "padding":quad, "border":quad, "margin":quad, "width":58,"height":21 } } })
}

fn frame_command(command: &Value, calls: &Calls, page: &Mutex<Page>) -> Option<Value> {
    let page = lock(page);
    let frames = page.frames.as_ref()?;
    Some(match command["method"].as_str()? {
        "Page.getFrameTree" => {
            let target = command["sessionId"]
                .as_str()
                .and_then(|session| calls.sessions.get(session))?;
            json!({"result":frames.trees.get(target)?})
        }
        "Target.getTargets" => json!({"result":{"targetInfos":frames.targets}}),
        "Target.getTargetInfo" => match frames
            .targets
            .iter()
            .find(|target| target["targetId"] == command["params"]["targetId"])
        {
            Some(target) => json!({"result":{"targetInfo":target}}),
            None => json!({"error":{"code":-32602,"message":"No target with given id found"}}),
        },
        _ => return None,
    })
}

fn glue_delay(params: &Value, calls: &mut Calls, replies: &Replies) -> Option<Duration> {
    let (counter, reply) = match params["arguments"][0]["value"].as_str() {
        Some("setupHitTargetInterceptor") => (&mut calls.arm, replies.arm),
        Some("title") => (&mut calls.title, replies.title),
        _ => return Some(Duration::ZERO),
    };
    let call = *counter;
    *counter += 1;
    reply.when(call)
}

/// A new target's initial history can lag its tab-strip record.
fn navigation_history(
    command: &Value,
    replies: &Replies,
    calls: &mut Calls,
    page: &Mutex<Page>,
) -> (Value, Option<Duration>) {
    let call = calls.history;
    calls.history += 1;
    let target = command["sessionId"]
        .as_str()
        .and_then(|id| calls.sessions.get(id));
    let mut url = lock(page)
        .tabs
        .iter()
        .find(|tab| tab["targetId"].as_str() == target.map(String::as_str))
        .map_or_else(|| json!(PAGE_URL), |tab| tab["url"].clone());
    if target.is_some_and(|target| target.starts_with("new-page-")) {
        if calls.new_page_history < replies.new_page_empty_history {
            url = json!("");
        }
        calls.new_page_history += 1;
    }
    (
        json!({
            "result": { "currentIndex": 0, "entries": [{ "id": 1, "url": url }] },
        }),
        replies.history.when(call),
    )
}

/// Tab-strip commands share one browser state across all sessions and connections.
fn tab_command(
    command: &Value,
    replies: &Replies,
    page: &Mutex<Page>,
) -> Option<(Value, Option<Duration>)> {
    let now = Some(Duration::ZERO);
    Some(match command["method"].as_str().unwrap_or_default() {
        "Browser.activateTab" => {
            let mut page = lock(page);
            if let Some(window) = page
                .tabs
                .iter()
                .find(|tab| tab["targetId"] == command["params"]["targetId"])
                .and_then(|tab| tab["windowId"].as_i64())
            {
                page.active_window = window;
            }
            for tab in &mut page.tabs {
                tab["isActive"] = json!(tab["targetId"] == command["params"]["targetId"]);
            }
            (json!({"result":{}}), now)
        }
        "Browser.getTabInfo" => {
            let page = lock(page);
            let tab = page
                .tabs
                .iter()
                .find(|tab| tab["targetId"] == command["params"]["targetId"]);
            (
                tab.map_or_else(
                    || json!({ "error": { "code": -32000, "message": "No target with given id" } }),
                    |tab| json!({ "result": { "tab": tab } }),
                ),
                page.tab_info_delay.or(now),
            )
        }
        "Browser.getTabs" => {
            let page = lock(page);
            let mut tabs = page.tabs.clone();
            // BrowserOS enumerates whole windows, most recently activated first.
            tabs.sort_by_key(|tab| {
                let window = tab["windowId"].as_i64().unwrap_or_default();
                (window != page.active_window, window)
            });
            (json!({ "result": { "tabs": tabs } }), now)
        }
        "Browser.createTab" => {
            let mut page = lock(page);
            let window = command["params"]["windowId"]
                .as_i64()
                .or_else(|| page.tabs.first().and_then(|tab| tab["windowId"].as_i64()));
            match window {
                Some(window) => {
                    let tab = page.add_tab(window, replies.new_page_url.unwrap_or("about:blank"));
                    (
                        json!({ "result": { "tab": tab } }),
                        replies.create_tab_delay.or(now),
                    )
                }
                None => (
                    json!({"error":{"code":-32000,"message":"No browser window available"}}),
                    now,
                ),
            }
        }
        "Browser.createWindow" => {
            let mut page = lock(page);
            let window = page.next_window;
            page.next_window += 1;
            page.add_tab(window, "about:blank");
            (
                json!({"result":{"window":{"windowId":window,"isActive":false}}}),
                replies.create_tab_delay.or(now),
            )
        }
        "Target.closeTarget" => {
            lock(page)
                .tabs
                .retain(|tab| tab["targetId"] != command["params"]["targetId"]);
            (
                json!({ "result": { "success": true } }),
                replies.close_tab_delay.or(now),
            )
        }
        _ => return None,
    })
}

/// `Runtime.callFunctionOn`: neo-runtime-injected's bundle entries by execution context, and
/// glue calls on the glue object. A by-value result is the glue's JSON; a result that holds a
/// handle is a page object whose properties `Runtime.getProperties` then lists. The interceptor
/// glue arms and stops the page's slot.
fn call_function_on(
    params: &Value,
    calls: &mut Calls,
    page: &Mutex<Page>,
    replies: &Replies,
) -> Value {
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
        "setupDragListeners" => {
            lock(page).drag_listeners = true;
            json!({"armed":true})
        }
        "cleanupDrag" => {
            lock(page).drag_listeners = false;
            json!({"started":false})
        }
        "title" => json!({"value":if calls.world_document { "After commit" } else { "Fake page" }}),
        "describeSelector" => json!({ "locator": glue_params["selector"] }),
        "checkElementStates" | "scrollIntoView" => json!({}),
        "viewportSize" => json!({ "width": 800, "height": 600 }),
        "getElementBorderWidth" => json!({ "left": 0, "top": 0 }),
        "stopHitTargetInterceptor" => {
            let mut page = lock(page);
            let stopped = &params["arguments"][2]["objectId"];
            if page
                .interceptor
                .as_ref()
                .is_some_and(|armed| *stopped == armed.id)
            {
                page.interceptor = None;
            }
            if replies.press_lands_elsewhere {
                json!({ "outcome": { "hitTargetDescription": "<div id=\"overlay\"></div>" } })
            } else {
                json!({ "outcome": "done" })
            }
        }
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
            let interceptor = {
                let mut page = lock(page);
                page.next_interceptor += 1;
                let id = format!("interceptor-{}", page.next_interceptor);
                page.interceptor = Some(Interceptor {
                    id: id.clone(),
                    blocks_all: replies.press_lands_elsewhere
                        || glue_params["blockAllEvents"] == true,
                });
                id
            };
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

/// Where one answer goes: the exchange it answers, and for a mouse release, the page that
/// handles the release before it is acknowledged.
struct Delivery {
    exchanges: Arc<Mutex<Vec<Exchange>>>,
    index: usize,
    page: Option<Arc<Mutex<Page>>>,
    outgoing: UnboundedSender<String>,
}

impl Delivery {
    /// Lets the page handle a release, records `reply` as the answer to the exchange, then
    /// sends it. Recording first means a test that sees a session's detach also sees the
    /// attach answer that handed it out.
    fn send(self, reply: Value) {
        if let Some(page) = &self.page {
            let mut page = lock(page);
            if !page
                .interceptor
                .as_ref()
                .is_some_and(|armed| armed.blocks_all)
            {
                page.clicks += 1;
            }
        }
        let text = reply.to_string();
        if let Some(exchange) = lock(&self.exchanges).get_mut(self.index) {
            exchange.reply = Some(reply);
        }
        // Fails only once the connection is gone, when nobody is left to read the answer.
        let _ = self.outgoing.send(text);
    }
}
