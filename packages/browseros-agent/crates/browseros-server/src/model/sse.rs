//! Frames a provider's HTTP body into SSE data payloads. The wire adapters own JSON and
//! the terminal event for each protocol, so EOF here never implies a successful model step.

use bytes::Bytes;
use eventsource_stream::{EventStreamError, Eventsource};
use futures_util::{Stream, StreamExt};
use http::HeaderMap;
use serde_json::Value;
use url::Url;

use super::errors;
use crate::turn::ChatError;

pub(super) async fn post(
    http: &reqwest::Client,
    url: &Url,
    headers: &HeaderMap,
    body: &Value,
) -> Result<reqwest::Response, ChatError> {
    let mut response = http
        .post(url.clone())
        .headers(headers.clone())
        .json(body)
        .send()
        .await
        .map_err(|error| {
            if error.is_connect() {
                errors::connect(url)
            } else {
                errors::disconnected()
            }
        })?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let retry_after = errors::retry_after(response.headers());
    // Keep enough of an upstream failure for the error card without buffering an unlimited body.
    const LIMIT: usize = 64 * 1024;
    let mut bytes = Vec::new();
    while bytes.len() < LIMIT {
        let Ok(Some(chunk)) = response.chunk().await else {
            break;
        };
        bytes.extend_from_slice(&chunk[..chunk.len().min(LIMIT - bytes.len())]);
    }
    Err(errors::upstream(
        Some(status.as_u16()),
        retry_after,
        &String::from_utf8_lossy(&bytes),
        headers,
    ))
}

pub(super) fn events(response: reqwest::Response) -> impl Stream<Item = Result<String, ChatError>> {
    decode(response.bytes_stream()).map(|event| {
        event.map_err(|error| match error {
            EventStreamError::Transport(_) => errors::disconnected(),
            _ => errors::protocol("The provider sent invalid server-sent events."),
        })
    })
}

pub(super) fn json(data: &str) -> Result<Value, ChatError> {
    serde_json::from_str(data)
        .map_err(|_| errors::protocol("The provider sent invalid JSON in its event stream."))
}

fn decode<E>(
    bytes: impl Stream<Item = Result<Bytes, E>>,
) -> impl Stream<Item = Result<String, EventStreamError<E>>> {
    bytes
        .eventsource()
        .map(|event| event.map(|event| event.data))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::stream;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    use super::*;

    #[tokio::test]
    async fn frames_comments_multiline_data_and_split_utf8() {
        let chunks = [
            &b": heartbeat\r\n\r\nevent: update\r\nda"[..],
            &b"ta: first\r\ndata: caf\xc3"[..],
            &b"\xa9\r\n\r\ndata: [DONE]\n\n"[..],
        ];
        let bytes = stream::iter(chunks.map(|chunk| Ok::<_, ()>(Bytes::from_static(chunk))));
        let received: Vec<_> = decode(bytes).collect().await;
        assert_eq!(received.len(), 2);
        assert_eq!(received[0].as_deref().ok(), Some("first\ncafé"));
        assert_eq!(received[1].as_deref().ok(), Some("[DONE]"));
    }

    #[tokio::test]
    async fn reports_a_disconnect_after_the_stream_starts() -> anyhow::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = Url::parse(&format!("http://{}", listener.local_addr()?))?;
        let (read_first_event, first_event_read) = oneshot::channel();
        let provider = async {
            let (socket, _) = listener.accept().await?;
            let mut socket = BufReader::new(socket);
            let mut line = String::new();
            loop {
                line.clear();
                if socket.read_line(&mut line).await? == 0 {
                    anyhow::bail!("the request ended before the headers");
                }
                if line == "\r\n" {
                    break;
                }
            }
            // Drain the empty JSON object so closing the socket truncates only the response.
            socket.read_exact(&mut [0; 2]).await?;
            let mut socket = socket.into_inner();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 64\r\n\r\ndata: first\n\n")
                .await?;
            first_event_read.await?;
            socket.shutdown().await?;
            Ok(())
        };
        let request = async {
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()?;
            let response = post(&client, &url, &HeaderMap::new(), &serde_json::json!({})).await?;
            let mut stream = std::pin::pin!(events(response));
            assert_eq!(stream.next().await.transpose()?.as_deref(), Some("first"));
            read_first_event
                .send(())
                .map_err(|_| anyhow::anyhow!("the provider closed before the first event"))?;
            let Some(Err(error)) = stream.next().await else {
                anyhow::bail!("the interrupted stream did not return an error");
            };
            assert_eq!(error.code, crate::turn::ChatErrorCode::ConnectionFailed);
            assert_eq!(
                error.message,
                "The connection to the provider closed before the response finished."
            );
            Ok(())
        };
        tokio::try_join!(provider, request)?;
        Ok(())
    }
}
