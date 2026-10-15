//! The deprecated MCP HTTP+SSE transport (protocol 2024-11-05) as an rmcp client [`Transport`]:
//! rmcp 3 ships only Streamable HTTP, but custom MCP servers the transport probe classifies as SSE
//! still speak this. Port of `@ai-sdk/mcp`'s `SseMCPTransport`:
//!
//! ```text
//!  GET  <url>  (Accept: text/event-stream) ──► event: endpoint  data: /messages?session=…
//!                                          ──► event: message   data: <JSON-RPC from server>  …
//!  POST <endpoint> (application/json)      ◄── each JSON-RPC message we send (202, no body)
//! ```
//!
//! The endpoint must share the connection's origin (a server cannot redirect our requests and the
//! lease/custom headers they carry elsewhere). Redirects are refused, as TS's `redirect: 'error'`.

use futures_util::StreamExt;
use reqwest_eventsource::{Event, EventSource, retry::Never};
use rmcp::{
    RoleClient,
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::Transport,
};
use tokio::sync::mpsc;

/// `LATEST_LEGACY_PROTOCOL_VERSION` of `@ai-sdk/mcp`, sent as `mcp-protocol-version`.
const PROTOCOL_VERSION_HEADER: (&str, &str) = ("mcp-protocol-version", "2025-11-25");

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(super) struct SseTransportError(String);

pub(super) struct LegacySseTransport {
    client: reqwest::Client,
    endpoint: url::Url,
    headers: Vec<(String, String)>,
    incoming: mpsc::Receiver<RxJsonRpcMessage<RoleClient>>,
    /// Owns the SSE response; aborting it closes the GET stream.
    reader: tokio::task::JoinHandle<()>,
}

impl LegacySseTransport {
    /// Opens the event stream and waits for the `endpoint` event. Messages that arrive before it
    /// are kept and delivered first.
    pub(super) async fn connect(
        url: &url::Url,
        headers: &[(String, String)],
    ) -> Result<Self, SseTransportError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| SseTransportError(error.to_string()))?;
        let mut request = client
            .get(url.clone())
            .header("Accept", "text/event-stream")
            .header(PROTOCOL_VERSION_HEADER.0, PROTOCOL_VERSION_HEADER.1);
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let mut events =
            EventSource::new(request).map_err(|error| SseTransportError(error.to_string()))?;
        events.set_retry_policy(Box::new(Never));

        let (tx, incoming) = mpsc::channel(64);
        let endpoint = loop {
            match events.next().await {
                Some(Ok(Event::Open)) => {}
                Some(Ok(Event::Message(event))) if event.event == "endpoint" => {
                    let endpoint = url
                        .join(&event.data)
                        .map_err(|error| SseTransportError(error.to_string()))?;
                    if endpoint.origin() != url.origin() {
                        return Err(SseTransportError(format!(
                            "MCP SSE Transport Error: Endpoint origin does not match connection origin: {}",
                            endpoint.origin().ascii_serialization()
                        )));
                    }
                    break endpoint;
                }
                Some(Ok(Event::Message(event))) if event.event == "message" => {
                    forward(&tx, &event.data).await;
                }
                Some(Ok(Event::Message(_))) => {}
                Some(Err(error)) => return Err(connect_error(error)),
                None => {
                    return Err(SseTransportError(
                        "MCP SSE Transport Error: Connection closed unexpectedly".to_owned(),
                    ));
                }
            }
        };

        // After the handshake the stream only carries server → client messages. When it ends or
        // fails, dropping `tx` makes `receive` return `None`, which rmcp treats as a closed
        // transport.
        let reader = tokio::spawn(async move {
            while let Some(event) = events.next().await {
                match event {
                    Ok(Event::Message(event)) if event.event == "message" => {
                        forward(&tx, &event.data).await;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::debug!(%error, "MCP SSE stream ended");
                        break;
                    }
                }
            }
            events.close();
        });

        Ok(Self {
            client,
            endpoint,
            headers: headers.to_vec(),
            incoming,
            reader,
        })
    }
}

async fn forward(tx: &mpsc::Sender<RxJsonRpcMessage<RoleClient>>, data: &str) {
    match serde_json::from_str(data) {
        Ok(message) => {
            let _ = tx.send(message).await;
        }
        // TS reports and keeps reading: one bad frame must not kill the session.
        Err(error) => tracing::warn!(%error, "MCP SSE Transport Error: Failed to parse message"),
    }
}

fn connect_error(error: reqwest_eventsource::Error) -> SseTransportError {
    SseTransportError(match error {
        reqwest_eventsource::Error::InvalidStatusCode(status, _) => {
            let mut message = format!(
                "MCP SSE Transport Error: {} {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or_default()
            );
            if status.as_u16() == 405 {
                message.push_str(
                    ". This server does not support SSE transport. Try using `http` transport instead",
                );
            }
            message
        }
        other => other.to_string(),
    })
}

impl Transport<RoleClient> for LegacySseTransport {
    type Error = SseTransportError;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let client = self.client.clone();
        let endpoint = self.endpoint.clone();
        let headers = self.headers.clone();
        async move {
            let body = serde_json::to_string(&item)
                .map_err(|error| SseTransportError(error.to_string()))?;
            let mut request = client
                .post(endpoint.clone())
                .header(PROTOCOL_VERSION_HEADER.0, PROTOCOL_VERSION_HEADER.1);
            for (name, value) in &headers {
                request = request.header(name.as_str(), value.as_str());
            }
            let response = request
                .header("Content-Type", "application/json")
                .body(body)
                .send()
                .await
                .map_err(|error| SseTransportError(error.to_string()))?;
            if response.status().is_success() {
                return Ok(());
            }
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            Err(SseTransportError(format!(
                "MCP SSE Transport Error: POSTing to endpoint (HTTP {status}): {text}"
            )))
        }
    }

    fn receive(&mut self) -> impl Future<Output = Option<RxJsonRpcMessage<RoleClient>>> + Send {
        self.incoming.recv()
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.reader.abort();
        self.incoming.close();
        Ok(())
    }
}

impl Drop for LegacySseTransport {
    fn drop(&mut self) {
        self.reader.abort();
    }
}
