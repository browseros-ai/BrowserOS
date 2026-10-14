//! Exercises public dispatch against real loopback discovery and WebSocket peers.
//! The fixture controls lock admission inside the transport only to force an
//! otherwise narrow reconnect race; mutation evidence comes from the peer.

use super::*;
use futures_util::poll;
use std::{future::poll_fn, task::Poll};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinSet,
};

/// Each accepted socket represents a fresh browser with the same window ID.
/// Servers belong to the test runtime and are aborted together on fixture drop.
struct LoopbackBrowser {
    discovery_port: u16,
    discovery_task: JoinHandle<()>,
    websocket_task: JoinHandle<()>,
}

impl LoopbackBrowser {
    async fn start() -> Self {
        let websocket_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|err| panic!("bind websocket: {err}"));
        let websocket_port = websocket_listener
            .local_addr()
            .unwrap_or_else(|err| panic!("websocket address: {err}"))
            .port();
        let websocket_task = tokio::spawn(async move {
            // Dropping this set also cancels accepted peers when the listener
            // task is aborted, including peers left open by a failed assertion.
            let mut peers = JoinSet::new();
            loop {
                let (stream, _) = websocket_listener
                    .accept()
                    .await
                    .unwrap_or_else(|err| panic!("accept websocket: {err}"));
                peers.spawn(serve_browser(stream));
            }
        });

        let discovery_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|err| panic!("bind discovery: {err}"));
        let discovery_port = discovery_listener
            .local_addr()
            .unwrap_or_else(|err| panic!("discovery address: {err}"))
            .port();
        let discovery_task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = discovery_listener
                    .accept()
                    .await
                    .unwrap_or_else(|err| panic!("accept discovery: {err}"));
                let mut request = Vec::new();
                let mut scratch = [0_u8; 1024];
                while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                    let read = stream
                        .read(&mut scratch)
                        .await
                        .unwrap_or_else(|err| panic!("read discovery: {err}"));
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&scratch[..read]);
                }
                let body = format!(
                    "{{\"webSocketDebuggerUrl\":\"ws://127.0.0.1:{websocket_port}/devtools/browser/test\"}}"
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .unwrap_or_else(|err| panic!("write discovery: {err}"));
            }
        });
        Self {
            discovery_port,
            discovery_task,
            websocket_task,
        }
    }

    async fn connect(&self) -> CdpClient {
        CdpClient::connect(ConnectOptions {
            host: Some("127.0.0.1".to_string()),
            connect_timeout: Duration::from_secs(2),
            request_timeout: Duration::from_secs(2),
            // The test drives the production reopen path explicitly, so neither
            // keepalive traffic nor a supervisor retry can alter its schedule.
            keepalive_interval: Duration::from_secs(3600),
            reconnect_delay: Duration::from_secs(3600),
            link_check_interval: Duration::from_secs(3600),
            reconnect_policy: ReconnectPolicy::KeepTrying,
            ..ConnectOptions::new(self.discovery_port)
        })
        .await
        .unwrap_or_else(|err| panic!("connect loopback browser: {err}"))
    }
}

impl Drop for LoopbackBrowser {
    fn drop(&mut self) {
        self.discovery_task.abort();
        self.websocket_task.abort();
    }
}

async fn serve_browser(stream: TcpStream) {
    let mut socket = tokio_tungstenite::accept_async(stream)
        .await
        .unwrap_or_else(|err| panic!("accept upgrade: {err}"));
    let mut mutations = 0_u64;
    let mut window_open = true;
    while let Some(Ok(message)) = socket.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let request: Value = serde_json::from_str(text.as_str())
            .unwrap_or_else(|err| panic!("parse request: {err}"));
        let result = match request["method"].as_str() {
            Some("Browser.closeWindow") => {
                assert_eq!(request["params"]["windowId"], 7);
                mutations += 1;
                window_open = false;
                json!({})
            }
            Some("Browser.getWindows") => json!({
                "windows": if window_open { json!([{ "windowId": 7 }]) } else { json!([]) },
                "mutations": mutations,
            }),
            Some("Fixture.echo") => json!({
                "params": request["params"],
                "sessionId": request["sessionId"],
                "frame": text.as_str(),
            }),
            _ => panic!("unexpected method: {}", request["method"]),
        };
        let response = json!({ "id": request["id"], "result": result });
        if socket
            .send(Message::Text(response.to_string().into()))
            .await
            .is_err()
        {
            return;
        }
    }
}

async fn assert_replacement_untouched(
    client: &CdpClient,
    result: Result<Value, CdpError>,
    expected_epoch: u64,
) {
    // This response is an ordered fence after the paused request. Checking the
    // replacement's state after it replies proves absence of mutation without
    // relying on a sleep or on whether a request happened to be logged yet.
    let state = client
        .send("Browser.getWindows", json!({}), None)
        .await
        .unwrap_or_else(|err| panic!("query replacement state: {err}"));
    assert_eq!(state["mutations"], 0, "stale dispatch mutated replacement");
    assert_eq!(state["windows"], json!([{ "windowId": 7 }]));
    assert_eq!(
        result,
        Err(CdpError::ConnectionEpochChanged {
            expected: expected_epoch,
            actual: expected_epoch + 1,
        })
    );
}

#[derive(Clone, Copy)]
enum Encoding {
    Value,
    Raw,
}

impl Encoding {
    async fn close_window(
        self,
        client: &CdpClient,
        epoch: u64,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        match self {
            Self::Value => {
                client
                    .send_at_epoch(
                        epoch,
                        "Browser.closeWindow",
                        json!({ "windowId": 7 }),
                        session,
                    )
                    .await
            }
            Self::Raw => {
                let response = client
                    .send_raw_json_at_epoch(
                        epoch,
                        "Browser.closeWindow",
                        r#"{ "windowId": 7 }"#,
                        session,
                    )
                    .await?;
                serde_json::from_str(&response).map_err(CdpError::from)
            }
        }
    }
}

#[derive(Clone, Copy)]
enum DispatchWait {
    PendingMap,
    Socket,
}

async fn rejects_paused_dispatch(encoding: Encoding, wait: DispatchWait) {
    // Session IDs may also be reused by a replacement. Cover both browser-root
    // and session-routed frames without giving either encoder a separate guard.
    for session in [None, Some(SessionId::from("reused-session"))] {
        let browser = LoopbackBrowser::start().await;
        let client = browser.connect().await;
        let epoch = client.epoch();
        let mut request = Box::pin(encoding.close_window(&client, epoch, session.as_ref()));

        match wait {
            DispatchWait::PendingMap => {
                let pending_guard = client.inner.pending.lock().await;
                // Polling once enters the public API and suspends at the held
                // pending-map lock. The real reopen can now overtake dispatch.
                assert!(poll!(request.as_mut()).is_pending());
                timeout(Duration::from_secs(2), open_socket(client.inner.clone()))
                    .await
                    .unwrap_or_else(|err| panic!("reopen timed out: {err}"))
                    .unwrap_or_else(|err| panic!("reopen failed: {err}"));
                drop(pending_guard);
            }
            DispatchWait::Socket => {
                let sink_guard = client.inner.sink.lock().await;
                let mut reopen = Box::pin(open_socket(client.inner.clone()));
                // Drive discovery/upgrade until publication queues on the held
                // sink guard. Taking the old reader precedes that await, and no
                // other await lies between them. Manual polling on this single
                // thread makes FIFO mutex admission deterministic, without sleeps
                // or a production test hook.
                timeout(
                    Duration::from_secs(2),
                    poll_fn(|cx| {
                        if let Poll::Ready(result) = reopen.as_mut().poll(cx) {
                            panic!("reopen should wait for held sink: {result:?}");
                        }
                        if client
                            .inner
                            .reader_task
                            .try_lock()
                            .is_ok_and(|reader| reader.is_none())
                        {
                            Poll::Ready(())
                        } else {
                            Poll::Pending
                        }
                    }),
                )
                .await
                .unwrap_or_else(|err| panic!("publication did not queue: {err}"));
                // Dispatch queues after publication, so it selects the new sink
                // despite having entered with IDs resolved on the old socket.
                assert!(poll!(request.as_mut()).is_pending());
                drop(sink_guard);
                reopen
                    .await
                    .unwrap_or_else(|err| panic!("reopen failed: {err}"));
            }
        }

        let result = request.await;
        assert_replacement_untouched(&client, result, epoch).await;
        client.disconnect().await;
    }
}

#[tokio::test]
async fn bound_dispatch_waiting_for_pending_map_cannot_mutate_replacement() {
    rejects_paused_dispatch(Encoding::Value, DispatchWait::PendingMap).await;
}

#[tokio::test]
async fn bound_raw_dispatch_waiting_for_pending_map_cannot_mutate_replacement() {
    rejects_paused_dispatch(Encoding::Raw, DispatchWait::PendingMap).await;
}

#[tokio::test]
async fn bound_dispatch_waiting_for_socket_cannot_mutate_replacement() {
    rejects_paused_dispatch(Encoding::Value, DispatchWait::Socket).await;
}

#[tokio::test]
async fn bound_raw_dispatch_waiting_for_socket_cannot_mutate_replacement() {
    rejects_paused_dispatch(Encoding::Raw, DispatchWait::Socket).await;
}

#[tokio::test]
async fn bound_and_unbound_sends_preserve_parameters_and_session_routing() {
    let browser = LoopbackBrowser::start().await;
    let client = browser.connect().await;
    let session = SessionId::from("session\"\\");
    let params = json!({ "windowId": 7, "nested": [true, "value"] });

    for bound in [false, true] {
        let response = if bound {
            client
                .send_at_epoch(
                    client.epoch(),
                    "Fixture.echo",
                    params.clone(),
                    Some(&session),
                )
                .await
        } else {
            client
                .send("Fixture.echo", params.clone(), Some(&session))
                .await
        }
        .unwrap_or_else(|err| panic!("send value parameters: {err}"));
        assert_eq!(response["params"], params);
        assert_eq!(response["sessionId"], session.as_str());

        // Duplicate keys and whitespace expose accidental parse/reserialize of
        // raw parameters, independently of the peer's parsed parameter value.
        let raw_params = r#"{ "windowId": 7, "duplicate": 1, "duplicate": 2 }"#;
        let response = if bound {
            client
                .send_raw_json_at_epoch(client.epoch(), "Fixture.echo", raw_params, Some(&session))
                .await
        } else {
            client
                .send_raw_json("Fixture.echo", raw_params, Some(&session))
                .await
        }
        .unwrap_or_else(|err| panic!("send raw parameters: {err}"));
        let response: Value = serde_json::from_str(&response)
            .unwrap_or_else(|err| panic!("parse echo response: {err}"));
        assert_eq!(response["sessionId"], session.as_str());
        assert!(
            response["frame"]
                .as_str()
                .is_some_and(|frame| frame.contains(&format!("\"params\":{raw_params}")))
        );
    }
    client.disconnect().await;
}

#[tokio::test]
async fn bound_sends_on_a_disconnected_current_epoch_are_not_connected() {
    let browser = LoopbackBrowser::start().await;
    let client = browser.connect().await;
    let epoch = client.epoch();
    client.disconnect().await;

    for encoding in [Encoding::Value, Encoding::Raw] {
        assert_eq!(
            encoding.close_window(&client, epoch, None).await,
            Err(CdpError::NotConnected)
        );
    }
}

#[tokio::test]
async fn bound_raw_send_rejects_invalid_parameters_without_mutation() {
    let browser = LoopbackBrowser::start().await;
    let client = browser.connect().await;
    let result = client
        .send_raw_json_at_epoch(client.epoch(), "Browser.closeWindow", "{", None)
        .await;
    assert!(matches!(result, Err(CdpError::Json(_))));
    let state = client
        .send("Browser.getWindows", json!({}), None)
        .await
        .unwrap_or_else(|err| panic!("query browser state: {err}"));
    assert_eq!(state["mutations"], 0);
    client.disconnect().await;
}
