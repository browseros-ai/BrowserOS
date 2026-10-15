//! SSE framing for the UI message stream, exactly as the AI SDK's
//! `json-to-sse-transform-stream.ts`: one `data: <compact JSON>\n\n` frame per chunk, then
//! `data: [DONE]\n\n`; no `event:`/`id:` lines. P1 owns this file after S0 (byte-fixture tests).

use crate::{ChunkStream, chunk::UiChunk};
use bytes::Bytes;
use futures_util::StreamExt as _;
use std::convert::Infallible;

/// Response headers of `/chat` and `/chat/{id}/stream` (`UI_MESSAGE_STREAM_HEADERS`).
pub const UI_STREAM_HEADERS: [(&str, &str); 5] = [
    ("content-type", "text/event-stream"),
    ("cache-control", "no-cache"),
    ("connection", "keep-alive"),
    ("x-vercel-ai-ui-message-stream", "v1"),
    ("x-accel-buffering", "no"),
];

pub const DONE_FRAME: &[u8] = b"data: [DONE]\n\n";

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("frame {index} is not a `data: ` line: {frame:?}")]
    NotData { index: usize, frame: String },
    #[error("frame {index} is not a UI chunk: {source}")]
    Chunk {
        index: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("body is not UTF-8")]
    Utf8(#[from] std::str::Utf8Error),
}

/// `b"data: " + compact JSON + b"\n\n"`.
pub fn frame(chunk: &UiChunk) -> Bytes {
    // A `UiChunk` holds only string-keyed JSON, so serialization cannot fail; the fallback keeps
    // the stream well-formed rather than panicking on a request path if that ever changes.
    let json = serde_json::to_string(chunk).unwrap_or_else(|error| {
        format!(r#"{{"type":"error","errorText":{:?}}}"#, error.to_string())
    });
    Bytes::from(format!("data: {json}\n\n"))
}

/// Frames every chunk, then the `[DONE]` sentinel once the turn's stream ends. A response body
/// that drops this stream (client disconnect) only drops the subscriber, never the run.
pub fn frames(
    chunks: ChunkStream,
) -> impl futures_core::Stream<Item = Result<Bytes, Infallible>> + Send {
    chunks
        .map(|chunk| Ok(frame(&chunk)))
        .chain(futures_util::stream::once(async {
            Ok(Bytes::from_static(DONE_FRAME))
        }))
}

/// Parses a complete SSE body back into chunks, ignoring `[DONE]`. For tests and the compat
/// harness; tolerant only of LF framing because that is all [`frames`] produces.
pub fn parse(body: &[u8]) -> Result<Vec<UiChunk>, ParseError> {
    let text = std::str::from_utf8(body)?;
    let mut chunks = Vec::new();
    for (index, frame) in text
        .split("\n\n")
        .filter(|frame| !frame.is_empty())
        .enumerate()
    {
        let data = frame
            .strip_prefix("data: ")
            .ok_or_else(|| ParseError::NotData {
                index,
                frame: frame.to_owned(),
            })?;
        if data == "[DONE]" {
            continue;
        }
        chunks.push(
            serde_json::from_str(data).map_err(|source| ParseError::Chunk { index, source })?,
        );
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_end_with_done() -> Result<(), ParseError> {
        let chunks: ChunkStream = Box::pin(futures_util::stream::iter(vec![
            UiChunk::StartStep,
            UiChunk::FinishStep,
        ]));
        let body: Vec<u8> = frames(chunks)
            .filter_map(|frame| async move { frame.ok() })
            .collect::<Vec<_>>()
            .await
            .concat();
        assert_eq!(
            body,
            b"data: {\"type\":\"start-step\"}\n\ndata: {\"type\":\"finish-step\"}\n\ndata: [DONE]\n\n"
        );
        assert_eq!(parse(&body)?, vec![UiChunk::StartStep, UiChunk::FinishStep]);
        Ok(())
    }
}
