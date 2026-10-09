use crate::{CdpError, CdpEvent, EventStream, SessionId, discovery::discover_websocket_url};
use futures_util::{
    SinkExt, StreamExt,
    future::BoxFuture,
    stream::{SplitSink, SplitStream},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use socket2::SockRef;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    net::TcpStream,
    sync::{Mutex, Notify, broadcast, mpsc, oneshot},
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_tungstenite::{
    WebSocketStream, client_async_with_config,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        protocol::{CloseFrame, WebSocketConfig},
    },
};
use tracing::{debug, info, warn};
use url::Url;

type WsStream = WebSocketStream<TcpStream>;
type WsSink = SplitSink<WsStream, Message>;
type WsReader = SplitStream<WsStream>;
type PendingSender = oneshot::Sender<Result<Value, CdpError>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectPolicy {
    Exit(i32),
    KeepTrying,
}

#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub port: u16,
    pub host: Option<String>,
    pub connect_timeout: Duration,
    pub connect_retry_delay: Duration,
    pub reconnect_delay: Duration,
    pub keepalive_interval: Duration,
    pub keepalive_timeout: Duration,
    /// How often the link supervisor re-reads whether the socket should be up.
    /// Bounds how long a lost wake-up can delay recovery.
    pub link_check_interval: Duration,
    pub request_timeout: Duration,
    pub connect_max_retries: usize,
    pub reconnect_max_retries: usize,
    pub reconnect_policy: ReconnectPolicy,
}

impl ConnectOptions {
    #[must_use]
    pub fn new(port: u16) -> Self {
        Self {
            port,
            ..Self::default()
        }
    }
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self {
            port: 9000,
            host: None,
            connect_timeout: Duration::from_secs(10),
            connect_retry_delay: Duration::from_secs(1),
            reconnect_delay: Duration::from_secs(5),
            keepalive_interval: Duration::from_secs(30),
            keepalive_timeout: Duration::from_secs(10),
            link_check_interval: Duration::from_secs(1),
            request_timeout: Duration::from_secs(60),
            connect_max_retries: 3,
            reconnect_max_retries: 3,
            reconnect_policy: ReconnectPolicy::Exit(1),
        }
    }
}

#[derive(Clone)]
pub struct CdpClient {
    inner: Arc<Inner>,
}

struct Inner {
    opts: ConnectOptions,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, PendingSender>>,
    sink: Mutex<Option<WsSink>>,
    events_tx: broadcast::Sender<CdpEvent>,
    targeted: std::sync::Mutex<HashMap<String, mpsc::UnboundedSender<CdpEvent>>>,
    connected: AtomicBool,
    disconnecting: AtomicBool,
    epoch: AtomicU64,
    /// When the link went down, so an outage can be reported and logged. `None`
    /// while the link is up. Never held across an await.
    down_since: std::sync::Mutex<Option<Instant>>,
    /// Prompts the link supervisor to look again. Losing a wake-up only costs
    /// `link_check_interval`, because the supervisor also re-reads on a timer.
    link_wake: Notify,
    last_host: Mutex<Option<String>>,
    reader_task: Mutex<Option<JoinHandle<()>>>,
    keepalive_task: Mutex<Option<JoinHandle<()>>>,
    link_task: Mutex<Option<JoinHandle<()>>>,
}

impl CdpClient {
    pub async fn connect(opts: ConnectOptions) -> Result<Self, CdpError> {
        let (events_tx, _) = broadcast::channel(4096);
        let inner = Arc::new(Inner {
            opts,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            sink: Mutex::new(None),
            events_tx,
            targeted: std::sync::Mutex::new(HashMap::new()),
            connected: AtomicBool::new(false),
            disconnecting: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            down_since: std::sync::Mutex::new(None),
            link_wake: Notify::new(),
            last_host: Mutex::new(None),
            reader_task: Mutex::new(None),
            keepalive_task: Mutex::new(None),
            link_task: Mutex::new(None),
        });

        open_socket(inner.clone()).await?;
        start_keepalive(inner.clone()).await;
        start_link_supervisor(inner.clone()).await;
        Ok(Self { inner })
    }

    pub async fn disconnect(&self) {
        self.inner.disconnecting.store(true, Ordering::SeqCst);
        self.inner.connected.store(false, Ordering::SeqCst);

        if let Some(mut sink) = self.inner.sink.lock().await.take() {
            let _ = sink
                .send(Message::Close(Some(CloseFrame {
                    code:
                        tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal,
                    reason: "disconnect".into(),
                })))
                .await;
        }

        if let Some(handle) = self.inner.reader_task.lock().await.take() {
            handle.abort();
        }
        if let Some(handle) = self.inner.keepalive_task.lock().await.take() {
            handle.abort();
        }
        // The supervisor is asked to stop rather than aborted. Aborting it can land
        // between a reopen installing its sink and registering its reader, which
        // leaves a live socket and reader behind with the client marked connected,
        // and skips the teardown below. It exits cooperatively instead, closing any
        // socket it opened, so teardown finishes a moment after this returns.
        self.inner.link_wake.notify_one();
        reject_all_pending(&self.inner, CdpError::NotConnected).await;
    }

    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.inner.connected.load(Ordering::SeqCst)
    }

    /// Identifies this client's WebSocket incarnation. Each successfully installed socket
    /// advances it so consumers can invalidate and reseed connection-scoped state.
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.inner.epoch.load(Ordering::SeqCst)
    }

    /// How long the link has been down, or `None` while it is up.
    #[must_use]
    pub fn down_for(&self) -> Option<Duration> {
        self.inner
            .down_since
            .lock()
            .ok()
            .and_then(|since| since.map(|at| at.elapsed()))
    }

    pub async fn send(
        &self,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let mut frame = serde_json::Map::new();
        frame.insert("id".to_string(), Value::from(id));
        frame.insert("method".to_string(), Value::from(method.to_string()));
        frame.insert("params".to_string(), params);
        if let Some(session) = session {
            frame.insert(
                "sessionId".to_string(),
                Value::from(session.as_str().to_string()),
            );
        }
        self.send_frame(
            id,
            method,
            Message::Text(Value::Object(frame).to_string().into()),
        )
        .await
    }

    pub async fn send_typed<P, R>(
        &self,
        method: &str,
        params: P,
        session: Option<&SessionId>,
    ) -> Result<R, CdpError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let value = serde_json::to_value(params)?;
        let response = self.send(method, value, session).await?;
        serde_json::from_value(response).map_err(CdpError::from)
    }

    pub async fn send_raw_json(
        &self,
        method: &str,
        params_json: &str,
        session: Option<&SessionId>,
    ) -> Result<String, CdpError> {
        let _validated: Value = serde_json::from_str(params_json)?;
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let method_json = serde_json::to_string(method)?;
        let mut frame = format!("{{\"id\":{id},\"method\":{method_json},\"params\":{params_json}");
        if let Some(session) = session {
            let session_json = serde_json::to_string(session.as_str())?;
            frame.push_str(",\"sessionId\":");
            frame.push_str(&session_json);
        }
        frame.push('}');
        let response = self
            .send_frame(id, method, Message::Text(frame.into()))
            .await?;
        serde_json::to_string(&response).map_err(CdpError::from)
    }

    pub fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.inner.events_tx.subscribe()
    }

    pub fn on_event(&self, method: &str) -> EventStream {
        EventStream::new(method, self.events())
    }

    /// Subscribe to one event method on a dedicated channel. Matching events
    /// are delivered here INSTEAD of the shared broadcast ring and are freed
    /// as soon as they are consumed — use for high-rate, large-payload events
    /// (e.g. `Page.screencastFrame`) whose retention in the 4096-slot
    /// broadcast ring would pin memory. One subscriber per method: a new
    /// call replaces the previous one (whose channel closes); dropping the
    /// receiver restores broadcast delivery for the method.
    #[must_use]
    pub fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        if let Ok(mut targeted) = self.inner.targeted.lock() {
            targeted.insert(method.to_string(), tx);
        }
        rx
    }

    async fn send_frame(&self, id: u64, method: &str, message: Message) -> Result<Value, CdpError> {
        if !self.is_connected() {
            return Err(CdpError::NotConnected);
        }

        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().await.insert(id, tx);

        let send_result = {
            let mut guard = self.inner.sink.lock().await;
            match guard.as_mut() {
                Some(sink) => sink
                    .send(message)
                    .await
                    .map_err(|err| CdpError::Transport(err.to_string())),
                None => Err(CdpError::NotConnected),
            }
        };

        if let Err(err) = send_result {
            self.inner.pending.lock().await.remove(&id);
            return Err(err);
        }

        match timeout(self.inner.opts.request_timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_closed)) => Err(CdpError::ConnectionLost),
            Err(_elapsed) => {
                self.inner.pending.lock().await.remove(&id);
                Err(CdpError::Timeout {
                    method: method.to_string(),
                })
            }
        }
    }
}

fn open_socket(inner: Arc<Inner>) -> BoxFuture<'static, Result<(), CdpError>> {
    Box::pin(async move {
        let last_host = inner.last_host.lock().await.clone();
        let (url, host) = discover_websocket_url(&inner.opts, last_host).await?;
        let ws = connect_websocket(&url, inner.opts.connect_timeout).await?;
        *inner.last_host.lock().await = Some(host);

        if let Some(handle) = inner.reader_task.lock().await.take() {
            handle.abort();
        }

        let (sink, reader) = ws.split();
        *inner.sink.lock().await = Some(sink);
        inner.connected.store(true, Ordering::SeqCst);
        let epoch = inner.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        // Logged at info, and with the outage, because "did it come back" is the
        // only question a disconnect report actually asks.
        let outage = inner
            .down_since
            .lock()
            .ok()
            .and_then(|mut since| since.take())
            .map(|at| at.elapsed());
        match outage {
            Some(outage) => info!(
                epoch,
                down_ms = outage.as_millis(),
                "CDP link restored after outage"
            ),
            None => info!(epoch, "CDP link established"),
        }

        let reader_inner = inner.clone();
        let handle = tokio::spawn(async move {
            reader_loop(reader_inner, reader).await;
        });
        *inner.reader_task.lock().await = Some(handle);
        Ok(())
    })
}

async fn connect_websocket(url: &Url, connect_timeout: Duration) -> Result<WsStream, CdpError> {
    if url.scheme() != "ws" {
        return Err(CdpError::Transport(format!(
            "unsupported CDP websocket scheme {}",
            url.scheme()
        )));
    }
    let host = url
        .host_str()
        .ok_or_else(|| CdpError::Transport("websocket URL missing host".to_string()))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| CdpError::Transport("websocket URL missing port".to_string()))?;
    let stream = timeout(connect_timeout, TcpStream::connect((host, port)))
        .await
        .map_err(|_| CdpError::Transport(format!("timed out connecting websocket {url}")))?
        .map_err(|err| CdpError::Transport(err.to_string()))?;
    stream
        .set_nodelay(true)
        .map_err(|err| CdpError::Transport(err.to_string()))?;
    let std_stream = stream
        .into_std()
        .map_err(|err| CdpError::Transport(err.to_string()))?;
    SockRef::from(&std_stream)
        .set_keepalive(true)
        .map_err(|err| CdpError::Transport(err.to_string()))?;
    std_stream
        .set_nonblocking(true)
        .map_err(|err| CdpError::Transport(err.to_string()))?;
    let stream =
        TcpStream::from_std(std_stream).map_err(|err| CdpError::Transport(err.to_string()))?;

    let request = url
        .as_str()
        .into_client_request()
        .map_err(|err| CdpError::Transport(err.to_string()))?;
    let mut config = WebSocketConfig::default();
    config.max_message_size = None;
    config.max_frame_size = None;
    let (ws, _response) = client_async_with_config(request, stream, Some(config))
        .await
        .map_err(|err| CdpError::Transport(err.to_string()))?;
    Ok(ws)
}

async fn reader_loop(inner: Arc<Inner>, mut reader: WsReader) {
    while let Some(message) = reader.next().await {
        match message {
            Ok(Message::Text(text)) => handle_text_message(&inner, text.as_str()).await,
            Ok(Message::Binary(bytes)) => match std::str::from_utf8(&bytes) {
                Ok(text) => handle_text_message(&inner, text).await,
                Err(err) => debug!("ignoring non-utf8 CDP binary frame: {err}"),
            },
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
            Err(err) => {
                warn!("CDP reader error: {err}");
                break;
            }
        }
    }

    if inner.disconnecting.load(Ordering::SeqCst) {
        reject_all_pending(&inner, CdpError::NotConnected).await;
        return;
    }
    mark_connection_lost(inner).await;
}

async fn handle_text_message(inner: &Arc<Inner>, text: &str) {
    let parsed = serde_json::from_str::<Value>(text);
    let Ok(value) = parsed else {
        return;
    };

    if let Some(id) = value.get("id").and_then(Value::as_u64) {
        let result = if let Some(error) = value.get("error") {
            let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown CDP protocol error")
                .to_string();
            Err(CdpError::from_protocol(code, message))
        } else {
            Ok(value.get("result").cloned().unwrap_or_else(|| json!({})))
        };
        if let Some(sender) = inner.pending.lock().await.remove(&id) {
            let _ = sender.send(result);
        }
        return;
    }

    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return;
    };
    let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
    let session_id = value
        .get("sessionId")
        .and_then(Value::as_str)
        .map(SessionId::from);
    let event = CdpEvent {
        method: method.to_string(),
        params,
        session_id,
    };
    let Some(event) = send_targeted(inner, event) else {
        return;
    };
    let _ = inner.events_tx.send(event);
}

/// Hands the event to the method's targeted subscriber if one is live;
/// returns the event back when it should fall through to the broadcast ring.
fn send_targeted(inner: &Inner, event: CdpEvent) -> Option<CdpEvent> {
    let Ok(mut targeted) = inner.targeted.lock() else {
        return Some(event);
    };
    let Some(sender) = targeted.get(&event.method) else {
        return Some(event);
    };
    match sender.send(event) {
        Ok(()) => None,
        Err(rejected) => {
            let event = rejected.0;
            targeted.remove(&event.method);
            Some(event)
        }
    }
}

async fn start_keepalive(inner: Arc<Inner>) {
    let mut guard = inner.keepalive_task.lock().await;
    if guard.is_some() {
        return;
    }
    let task_inner = inner.clone();
    let handle = tokio::spawn(async move {
        loop {
            sleep(task_inner.opts.keepalive_interval).await;
            if task_inner.disconnecting.load(Ordering::SeqCst) {
                break;
            }
            if !task_inner.connected.load(Ordering::SeqCst) {
                continue;
            }
            let client = CdpClient {
                inner: task_inner.clone(),
            };
            let keepalive = async {
                client.send("Browser.getVersion", json!({}), None).await?;
                let mut guard = task_inner.sink.lock().await;
                let Some(sink) = guard.as_mut() else {
                    return Err(CdpError::NotConnected);
                };
                sink.send(Message::Ping(Vec::new().into()))
                    .await
                    .map_err(|err| CdpError::Transport(err.to_string()))
            };
            if timeout(task_inner.opts.keepalive_timeout, keepalive)
                .await
                .is_err()
            {
                mark_connection_lost(task_inner.clone()).await;
            }
        }
    });
    *guard = Some(handle);
}

/// Records that the link is down and asks the supervisor to look.
///
/// It deliberately decides nothing else. The previous version armed the repair
/// here, guarded by a flag that meant both "a repair loop is running" and "a
/// further attempt is guaranteed". Those come apart when a freshly opened socket
/// fails before the arming path clears the flag: both sides then believe the
/// other owns the retry, and the link stays down with nothing retrying it.
async fn mark_connection_lost(inner: Arc<Inner>) {
    let was_connected = inner.connected.swap(false, Ordering::SeqCst);
    *inner.sink.lock().await = None;
    reject_all_pending(&inner, CdpError::ConnectionLost).await;
    if was_connected && let Ok(mut since) = inner.down_since.lock() {
        since.get_or_insert_with(Instant::now);
    }
    inner.link_wake.notify_one();
}

/// Drops a socket this client should no longer be holding.
async fn close_socket(inner: &Arc<Inner>) {
    inner.connected.store(false, Ordering::SeqCst);
    *inner.sink.lock().await = None;
    if let Some(handle) = inner.reader_task.lock().await.take() {
        handle.abort();
    }
    reject_all_pending(inner, CdpError::NotConnected).await;
}

async fn start_link_supervisor(inner: Arc<Inner>) {
    let mut guard = inner.link_task.lock().await;
    if guard.is_some() {
        return;
    }
    let task_inner = inner.clone();
    *guard = Some(tokio::spawn(async move {
        link_supervisor(task_inner).await;
    }));
}

/// Keeps the socket open for the life of the client.
///
/// Level triggered on purpose: every pass re-reads whether the link should be up
/// and whether it is, so no ordering of loss and recovery events can leave it
/// down with nobody retrying. A reopened socket that dies immediately is just the
/// next pass's work.
async fn link_supervisor(inner: Arc<Inner>) {
    let mut consecutive_failures: usize = 0;
    loop {
        if inner.disconnecting.load(Ordering::SeqCst) {
            return;
        }
        if inner.connected.load(Ordering::SeqCst) {
            consecutive_failures = 0;
            tokio::select! {
                () = inner.link_wake.notified() => {}
                () = sleep(inner.opts.link_check_interval) => {}
            }
            continue;
        }

        sleep(inner.opts.reconnect_delay).await;
        if inner.disconnecting.load(Ordering::SeqCst) {
            return;
        }
        if inner.connected.load(Ordering::SeqCst) {
            continue;
        }
        match open_socket(inner.clone()).await {
            Ok(()) => {
                consecutive_failures = 0;
                if inner.disconnecting.load(Ordering::SeqCst) {
                    close_socket(&inner).await;
                    return;
                }
            }
            Err(err) => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                warn!(
                    attempt = consecutive_failures,
                    max = inner.opts.reconnect_max_retries,
                    "CDP reconnect failed: {err}"
                );
                if consecutive_failures >= inner.opts.reconnect_max_retries {
                    match inner.opts.reconnect_policy {
                        ReconnectPolicy::KeepTrying => consecutive_failures = 0,
                        ReconnectPolicy::Exit(code) => std::process::exit(code),
                    }
                }
            }
        }
    }
}

async fn reject_all_pending(inner: &Arc<Inner>, error: CdpError) {
    let pending = std::mem::take(&mut *inner.pending.lock().await);
    for sender in pending.into_values() {
        let _ = sender.send(Err(error.clone()));
    }
}
