//! A stand-in for BrowserOS on a loopback WebSocket, for the host side's own CDP socket
//! (`src/host/link.rs`) and what runs over it: presences and live capture. It attaches flat
//! sessions (`session-1`, `session-2`, ..., never reused, so a stale id stays recognizable after
//! a reconnect), answers the commands a presence and a capture send, pushes
//! `Page.screencastFrame` events with their metadata, serves a recording's `IO.read` chunks,
//! emits `Target.detachedFromTarget`, and can drop its socket so the client reconnects under a
//! new epoch. It records every command with the socket it arrived on and when, so a test reads
//! back exactly what the host side sent.
//!
//! Paused time: tokio auto-advances a paused clock to the next timer whenever the runtime has
//! nothing to run, and a task waiting on this fake's socket counts as nothing to run. The fake
//! keeps a [`CLOCK_STEP`] timer running, so each such advance is at most a millisecond: a
//! loopback round trip costs a few virtual milliseconds, and a test's grace, pacing and stall
//! timers keep their meaning.
//!
//! Test crates include this file with `#[path]`. Cargo builds only top-level `tests/*.rs` files
//! as test crates, so it is never one on its own.

use std::collections::{HashMap, HashSet, VecDeque};
use std::pin::pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, mpsc};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{self, Instant};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

/// The longest step the paused clock takes while a test waits on the fake's socket.
pub const CLOCK_STEP: Duration = Duration::from_millis(1);

/// How long [`FakeHostCdp::wait_for`] waits, on tokio's clock, before it fails the test.
const WAIT: Duration = Duration::from_mins(1);

/// One command the host side sent.
#[derive(Clone, Debug)]
pub struct Command {
    /// The socket it arrived on, counting from 1: every reconnect opens the next one.
    pub connection: usize,
    pub method: String,
    pub params: Value,
    pub session: Option<String>,
    /// When it arrived, on tokio's clock.
    pub at: Instant,
}

/// One session the fake handed out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub session: String,
    pub target: String,
    pub connection: usize,
}

pub struct FakeHostCdp {
    port: u16,
    shared: Arc<Shared>,
    tasks: [JoinHandle<()>; 3],
}

struct Shared {
    state: Mutex<State>,
    /// Woken whenever a command is recorded.
    recorded: Notify,
}

#[derive(Default)]
struct State {
    commands: Vec<Command>,
    connections: usize,
    live: Option<Live>,
    attachments: Vec<Attachment>,
    /// Sessions the browser detached (the tab closed, or a detach command): commands on them
    /// fail as Chromium fails them.
    detached: HashSet<String>,
    recordings: HashMap<String, VecDeque<Vec<u8>>>,
    failing_reads: HashMap<String, usize>,
    attach_delay: Duration,
    frames: i64,
}

/// The socket the client is connected on now.
struct Live {
    connection: usize,
    outgoing: mpsc::UnboundedSender<String>,
    closed: CancellationToken,
}

impl FakeHostCdp {
    pub async fn start() -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            recorded: Notify::new(),
        });
        let (websocket, websocket_port) = bind().await;
        let served = Arc::clone(&shared);
        let websocket_task = tokio::spawn(async move {
            // Aborting this task drops the set, which ends the connections it accepted too.
            let mut connections = JoinSet::new();
            while let Ok((stream, _)) = websocket.accept().await {
                connections.spawn(serve(stream, Arc::clone(&served)));
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
        let clock_task = tokio::spawn(async {
            loop {
                time::sleep(CLOCK_STEP).await;
            }
        });
        Self {
            port,
            shared,
            tasks: [websocket_task, discovery_task, clock_task],
        }
    }

    /// The CDP port, as `HostLink::connect` takes it.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Every command received so far, in order.
    pub fn commands(&self) -> Vec<Command> {
        lock(&self.shared.state).commands.clone()
    }

    /// Every `method` command received so far, in order.
    pub fn sent(&self, method: &str) -> Vec<Command> {
        lock(&self.shared.state)
            .commands
            .iter()
            .filter(|command| command.method == method)
            .cloned()
            .collect()
    }

    /// Waits until at least `count` `method` commands arrived, and returns all of them. Fails
    /// the test after [`WAIT`].
    pub async fn wait_for(&self, method: &str, count: usize) -> Vec<Command> {
        let deadline = Instant::now() + WAIT;
        loop {
            let mut recorded = pin!(self.shared.recorded.notified());
            recorded.as_mut().enable();
            let sent = self.sent(method);
            if sent.len() >= count {
                return sent;
            }
            assert!(
                Instant::now() < deadline,
                "waited {WAIT:?} for {count} {method} commands; {} arrived",
                sent.len()
            );
            let _ = time::timeout_at(deadline, recorded).await;
        }
    }

    /// Every session handed out so far, in order.
    pub fn attachments(&self) -> Vec<Attachment> {
        lock(&self.shared.state).attachments.clone()
    }

    /// The session `method` was sent on in its `nth` command (from 0), once it arrived.
    pub async fn session_of(&self, method: &str, nth: usize) -> String {
        let sent = self.wait_for(method, nth + 1).await;
        sent[nth]
            .session
            .clone()
            .unwrap_or_else(|| panic!("{method} #{nth} named no session"))
    }

    /// Makes every later `Target.attachToTarget` answer only after `delay`, like a busy browser.
    pub fn delay_attaches(&self, delay: Duration) {
        lock(&self.shared.state).attach_delay = delay;
    }

    /// Delivers a screencast frame on `session`, as Chromium does after a page change, and
    /// returns its frame number (the `sessionId` its acknowledgement must echo). `timestamp`
    /// is `metadata.timestamp`, epoch seconds; `None` leaves it out.
    pub fn frame(&self, session: &str, jpeg: &[u8], timestamp: Option<f64>) -> i64 {
        let number = {
            let mut state = lock(&self.shared.state);
            state.frames += 1;
            state.frames
        };
        let mut metadata = json!({
            "offsetTop": 0,
            "pageScaleFactor": 1,
            "deviceWidth": 1280,
            "deviceHeight": 800,
            "scrollOffsetX": 0,
            "scrollOffsetY": 0,
        });
        if let Some(timestamp) = timestamp {
            metadata["timestamp"] = json!(timestamp);
        }
        self.emit(&json!({
            "method": "Page.screencastFrame",
            "sessionId": session,
            "params": { "data": BASE64.encode(jpeg), "metadata": metadata, "sessionId": number },
        }));
        number
    }

    /// Detaches `session`, as Chromium does when its tab closes.
    pub fn detach(&self, session: &str) {
        let target = {
            let mut state = lock(&self.shared.state);
            state.detached.insert(session.to_string());
            target_of(&state, session)
        };
        self.emit(&detached_event(session, &target));
    }

    /// Bytes the recording on `session` returns from its next `IO.read`.
    pub fn push_recording(&self, session: &str, bytes: &[u8]) {
        lock(&self.shared.state)
            .recordings
            .entry(session.to_string())
            .or_default()
            .push_back(bytes.to_vec());
    }

    /// Fails the next `count` `IO.read`s on `session`, as Chromium does before a recording's
    /// first bytes are written.
    pub fn fail_reads(&self, session: &str, count: usize) {
        lock(&self.shared.state)
            .failing_reads
            .insert(session.to_string(), count);
    }

    /// Drops the socket the client is connected on. The client reconnects on its own, and the
    /// new socket is the next `connection`.
    pub fn drop_connection(&self) {
        if let Some(live) = lock(&self.shared.state).live.take() {
            live.closed.cancel();
        }
    }

    fn emit(&self, event: &Value) {
        if let Some(live) = &lock(&self.shared.state).live {
            // Fails only once the connection is gone, when nobody is left to read it.
            let _ = live.outgoing.send(event.to_string());
        }
    }
}

impl Drop for FakeHostCdp {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
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
        .unwrap_or_else(|error| panic!("a fake host CDP task panicked: {error}"))
}

fn target_of(state: &State, session: &str) -> String {
    state
        .attachments
        .iter()
        .find(|attachment| attachment.session == session)
        .map(|attachment| attachment.target.clone())
        .unwrap_or_default()
}

fn detached_event(session: &str, target: &str) -> Value {
    json!({
        "method": "Target.detachedFromTarget",
        "params": { "sessionId": session, "targetId": target },
    })
}

/// Serves one CDP connection. Answers go out through one writer task, so an answer held back
/// for a while never delays the ones after it.
async fn serve(stream: TcpStream, shared: Arc<Shared>) {
    let Ok(socket) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    let (mut sink, mut source) = socket.split();
    let (outgoing, mut queued) = mpsc::unbounded_channel::<String>();
    let closed = CancellationToken::new();
    let connection = {
        let mut state = lock(&shared.state);
        state.connections += 1;
        state.live = Some(Live {
            connection: state.connections,
            outgoing: outgoing.clone(),
            closed: closed.clone(),
        });
        state.connections
    };
    // Dropped, and so aborted, when this connection ends.
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        while let Some(text) = queued.recv().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                return;
            }
        }
    });
    loop {
        let message = tokio::select! {
            () = closed.cancelled() => break,
            message = source.next() => message,
        };
        let Some(Ok(message)) = message else {
            break;
        };
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(command) = serde_json::from_str::<Value>(text.as_str()) else {
            continue;
        };
        let answer = answer(&shared, connection, &command);
        let outgoing = outgoing.clone();
        let send = move || {
            // Fails only once the connection is gone, when nobody is left to read it.
            let _ = outgoing.send(answer.reply.to_string());
            for event in answer.events {
                let _ = outgoing.send(event.to_string());
            }
        };
        if answer.delay.is_zero() {
            send();
        } else {
            tasks.spawn(async move {
                time::sleep(answer.delay).await;
                send();
            });
        }
    }
    let mut state = lock(&shared.state);
    if state
        .live
        .as_ref()
        .is_some_and(|live| live.connection == connection)
    {
        state.live = None;
    }
}

/// The fake's reply to one command, and the events that follow it.
struct Answer {
    reply: Value,
    events: Vec<Value>,
    delay: Duration,
}

/// Records `command` and works out its answer.
fn answer(shared: &Shared, connection: usize, command: &Value) -> Answer {
    let method = command["method"].as_str().unwrap_or_default().to_string();
    let params = command["params"].clone();
    let session = command["sessionId"].as_str().map(str::to_string);
    let mut state = lock(&shared.state);
    state.commands.push(Command {
        connection,
        method: method.clone(),
        params: params.clone(),
        session: session.clone(),
        at: Instant::now(),
    });
    shared.recorded.notify_waiters();

    let mut events = Vec::new();
    let mut delay = Duration::ZERO;
    let on_live_session = session.as_deref().is_none_or(|session| {
        !state.detached.contains(session)
            && state.attachments.iter().any(|attachment| {
                attachment.session == session && attachment.connection == connection
            })
    });
    let result = if on_live_session {
        match method.as_str() {
            "Target.attachToTarget" => {
                let id = format!("session-{}", state.attachments.len() + 1);
                state.attachments.push(Attachment {
                    session: id.clone(),
                    target: params["targetId"].as_str().unwrap_or_default().to_string(),
                    connection,
                });
                delay = state.attach_delay;
                Ok(json!({ "sessionId": id }))
            }
            "Target.detachFromTarget" => {
                let id = params["sessionId"].as_str().unwrap_or_default().to_string();
                if state.detached.insert(id.clone()) {
                    events.push(detached_event(&id, &target_of(&state, &id)));
                }
                Ok(json!({}))
            }
            "Page.startScreenRecording" => Ok(json!({
                "stream": format!("stream-{}", session.as_deref().unwrap_or_default()),
            })),
            "IO.read" => read_recording(&mut state, session.as_deref().unwrap_or_default()),
            "Browser.getVersion" => Ok(json!({
                "protocolVersion": "1.3",
                "product": "FakeHostCdp/1.0",
                "revision": "0",
                "userAgent": "FakeHostCdp",
                "jsVersion": "0",
            })),
            _ => Ok(json!({})),
        }
    } else {
        Err((-32001, "Session with given id not found.".to_string()))
    };
    let mut reply = match result {
        Ok(result) => json!({ "result": result }),
        Err((code, message)) => json!({ "error": { "code": code, "message": message } }),
    };
    reply["id"] = command["id"].clone();
    if let Some(session) = session {
        reply["sessionId"] = json!(session);
    }
    Answer {
        reply,
        events,
        delay,
    }
}

/// An `IO.read` on a recording: a failure while one is scripted, else the next queued chunk,
/// else an empty read (`eof`, which while recording only means "nothing more yet").
fn read_recording(state: &mut State, session: &str) -> Result<Value, (i64, String)> {
    if let Some(left) = state.failing_reads.get_mut(session)
        && *left > 0
    {
        *left -= 1;
        return Err((-32000, "Read failed".to_string()));
    }
    Ok(
        match state
            .recordings
            .get_mut(session)
            .and_then(VecDeque::pop_front)
        {
            Some(bytes) => {
                json!({ "data": BASE64.encode(bytes), "base64Encoded": true, "eof": false })
            }
            None => json!({ "data": "", "base64Encoded": true, "eof": true }),
        },
    )
}
