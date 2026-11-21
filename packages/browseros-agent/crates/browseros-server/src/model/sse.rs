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
        .map_err(|_| errors::disconnected())?;
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
    use futures_util::stream;

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
}
