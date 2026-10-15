//! SSE framing for the UI message stream, exactly as the AI SDK's
//! `json-to-sse-transform-stream.ts`: one `data: <compact JSON>\n\n` frame per chunk, then
//! `data: [DONE]\n\n`; no `event:`/`id:` lines. JSON uses JavaScript's number formatting and
//! integer-key ordering as well as the SDK's chunk field order, including inside opaque output.

use crate::{ChunkStream, chunk::UiChunk};
use bytes::Bytes;
use futures_util::StreamExt as _;
use serde::Serialize as _;
use serde_json::Value;
use std::{convert::Infallible, io};

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
    // Value preserves enum field order and nested JS object insertion order. Array-index keys
    // are the exception: JSON.stringify enumerates those numerically before all string keys.
    let json = serde_json::to_value(chunk).map(js_key_order);
    let mut bytes = b"data: ".to_vec();
    let result = json.and_then(|value| {
        value.serialize(&mut serde_json::Serializer::with_formatter(
            &mut bytes,
            JsFormatter,
        ))
    });
    // Every UiChunk is JSON-representable, and writing to Vec cannot fail. If that invariant
    // changes later, emit a valid error frame rather than panic inside an HTTP body stream.
    if result.is_err() {
        return Bytes::from_static(
            b"data: {\"type\":\"error\",\"errorText\":\"Unable to serialize UI chunk\"}\n\n",
        );
    }
    bytes.extend_from_slice(b"\n\n");
    Bytes::from(bytes)
}

fn js_key_order(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            // Stable sort: non-index keys keep insertion order. 2^32-1 and keys with leading
            // zeroes are ordinary strings in ECMAScript, not array indices.
            entries.sort_by_key(|(key, _)| {
                key.parse::<u32>()
                    .ok()
                    .filter(|index| *index != u32::MAX && index.to_string() == *key)
                    .unwrap_or(u32::MAX)
            });
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, js_key_order(value)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.into_iter().map(js_key_order).collect()),
        value => value,
    }
}

/// serde_json uses Rust float spelling (`1.0`, `1e-6`), which differs from JSON.stringify
/// (`1`, `0.000001`). Ryu's ECMAScript implementation also handles exponent signs and -0.
struct JsFormatter;
impl serde_json::ser::Formatter for JsFormatter {
    fn write_f64<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f64) -> io::Result<()> {
        writer.write_all(ryu_js::Buffer::new().format_finite(value).as_bytes())
    }
    fn write_f32<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f32) -> io::Result<()> {
        self.write_f64(writer, f64::from(value))
    }
    fn write_i64<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: i64) -> io::Result<()> {
        self.write_f64(writer, value as f64)
    }
    fn write_u64<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: u64) -> io::Result<()> {
        self.write_f64(writer, value as f64)
    }
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
