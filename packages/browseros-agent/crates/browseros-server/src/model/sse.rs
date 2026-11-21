//! Frames a provider's HTTP body into SSE data payloads. The wire adapters own JSON and
//! the terminal event for each protocol, so EOF here never implies a successful model step.

use bytes::Bytes;
use eventsource_stream::{EventStreamError, Eventsource};
use futures_util::{Stream, StreamExt};

#[expect(dead_code, reason = "used by the provider wire adapters")]
pub(super) fn events(
    response: reqwest::Response,
) -> impl Stream<Item = Result<String, EventStreamError<reqwest::Error>>> {
    decode(response.bytes_stream())
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
