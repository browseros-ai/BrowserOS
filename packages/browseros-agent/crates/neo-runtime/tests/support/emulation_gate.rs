//! A real CDP relay that holds emulation cleanup before forwarding it to the browser.
//! Only this test's client crosses the relay; its independent observer remains live.

use std::sync::Arc;

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy, SessionId};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

pub struct Gate {
    pub cdp: CdpClient,
    pub held: mpsc::UnboundedReceiver<String>,
    pub release: CancellationToken,
    tasks: [JoinHandle<()>; 2],
}

#[derive(Clone, Copy)]
pub enum Hold {
    Cleanup,
    ViewportReply,
}

impl Gate {
    pub async fn start(browser: CdpClient, mode: Hold) -> Self {
        let websocket = listen().await;
        let address = websocket
            .local_addr()
            .unwrap_or_else(|e| panic!("address: {e}"));
        let (held, received) = mpsc::unbounded_channel();
        let release = CancellationToken::new();
        let released = release.clone();
        let websocket_task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            while let Ok((stream, _)) = websocket.accept().await {
                connections.spawn(relay(
                    stream,
                    browser.clone(),
                    held.clone(),
                    released.clone(),
                    mode,
                ));
            }
        });
        let discovery = listen().await;
        let port = discovery
            .local_addr()
            .unwrap_or_else(|e| panic!("address: {e}"))
            .port();
        let discovery_task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = discovery.accept().await {
                let mut request = [0; 4096];
                let _read = stream.read(&mut request).await;
                let body =
                    json!({"webSocketDebuggerUrl": format!("ws://{address}/browser")}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _sent = stream.write_all(response.as_bytes()).await;
            }
        });
        let cdp = CdpClient::connect(ConnectOptions {
            host: Some("127.0.0.1".to_string()),
            reconnect_policy: ReconnectPolicy::KeepTrying,
            ..ConnectOptions::new(port)
        })
        .await
        .unwrap_or_else(|e| panic!("relay connection: {e}"));
        Self {
            cdp,
            held: received,
            release,
            tasks: [websocket_task, discovery_task],
        }
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self.release.cancel();
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn listen() -> TcpListener {
    TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|e| panic!("listen: {e}"))
}

async fn relay(
    stream: TcpStream,
    browser: CdpClient,
    held: mpsc::UnboundedSender<String>,
    release: CancellationToken,
    mode: Hold,
) {
    let socket = tokio_tungstenite::accept_async(stream)
        .await
        .unwrap_or_else(|e| panic!("websocket: {e}"));
    let (mut output, mut input) = socket.split();
    let (replies, mut completed) = mpsc::unbounded_channel::<Value>();
    let mut events = browser.events();
    let mut calls = JoinSet::new();
    let browser = Arc::new(browser);
    loop {
        let next = tokio::select! {
            message = input.next() => {
                let Some(Ok(Message::Text(text))) = message else { break };
                let command: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("command: {e}"));
                calls.spawn(forward(Arc::clone(&browser), command, replies.clone(), held.clone(), release.clone(), mode));
                continue;
            }
            Some(reply) = completed.recv() => reply,
            event = events.recv() => {
                let Ok(event) = event else { continue };
                let mut value = json!({"method": event.method, "params": event.params});
                if let Some(session) = event.session_id { value["sessionId"] = json!(session); }
                value
            }
        };
        if output
            .send(Message::Text(next.to_string().into()))
            .await
            .is_err()
        {
            break;
        }
    }
}

async fn forward(
    browser: Arc<CdpClient>,
    command: Value,
    replies: mpsc::UnboundedSender<Value>,
    held: mpsc::UnboundedSender<String>,
    release: CancellationToken,
    mode: Hold,
) {
    let method = command["method"].as_str().unwrap_or_default();
    let params = command.get("params").cloned().unwrap_or_else(|| json!({}));
    // Hold the old detach-based cleanup as well as an explicit reset. This forces the same
    // browser-visible gap on both implementations, without delaying any unrelated command.
    if method == "Target.detachFromTarget"
        || (matches!(mode, Hold::Cleanup)
            && (method == "Emulation.clearDeviceMetricsOverride"
                || (method == "Emulation.setEmulatedMedia" && params["media"] == "")))
    {
        let _notified = held.send(method.to_string());
        release.cancelled().await;
    }
    let session = command["sessionId"].as_str().map(SessionId::from);
    let result = browser.send(method, params, session.as_ref()).await;
    if matches!(mode, Hold::ViewportReply) && method == "Emulation.setDeviceMetricsOverride" {
        let _notified = held.send(method.to_string());
        release.cancelled().await;
    }
    let mut response = match result {
        Ok(result) => json!({"id": command["id"], "result": result}),
        Err(error) => {
            json!({"id": command["id"], "error": {"code": -32000, "message": error.to_string()}})
        }
    };
    if let Some(session) = session {
        response["sessionId"] = json!(session);
    }
    let _sent = replies.send(response);
}
