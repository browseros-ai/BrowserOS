//! The provider's own view of a streamed reply's tool calls, read off the raw SSE bytes beside
//! rig's decoder. rig names a call's provider id only when the call ends and drops empty argument
//! fragments, but the `@ai-sdk/*` providers stream `tool-input-start` under the provider's id and
//! forward every later fragment (empty ones included), and the app persists those ids.
//!
//! ```text
//!  Translator ──scope(WIRE_TOOL_CALLS)──► rig stream poll ──► PolicyClient::send_streaming
//!      ▲  id(k) / fragments(k)                                   │ captures the task-local
//!      └──────────────── WireToolCalls ◄── observe(bytes) ◄──────┘ wraps the body stream
//! ```
//!
//! Calls are numbered in first-appearance order, which is rig's tool `Start` order: a call cannot
//! start before the frame that opens it has been read, and the observer sees every byte before rig
//! parses it. Shapes: Chat Completions `choices[].delta.tool_calls[]` (`index`, `id`, arguments),
//! Responses `response.output_item.added` function calls (`call_id`), Anthropic
//! `content_block_start` `tool_use` blocks (`id`). Other wires (Gemini has no call ids, Bedrock is
//! not SSE) record nothing, and the translator keeps its own ids.

use bytes::Bytes;
use futures_util::StreamExt as _;
use rig_core::http_client::{self, StreamingResponse};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

tokio::task_local! {
    /// The calls log of the model stream being polled. The translator sets it around each poll of
    /// rig's stream; `send_streaming` runs inside the first poll, on the same task, and keeps the
    /// handle for the response body it wraps.
    pub(crate) static WIRE_TOOL_CALLS: Arc<WireToolCalls>;
}

/// One reply's tool calls in first-appearance order. Shared by the body observer (writer) and the
/// translator (reader) of a single model stream.
#[derive(Default)]
pub(crate) struct WireToolCalls {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    calls: Vec<WireCall>,
    /// Wire key (`chat:<index>`, `resp:<output_index>`, `anth:<index>`) → position in `calls`.
    by_key: HashMap<String, usize>,
    /// Bytes after the last newline, waiting for the rest of their line.
    partial_line: Vec<u8>,
}

#[derive(Default, Clone)]
struct WireCall {
    id: String,
    /// The argument fragments the AI SDK would emit as `tool-input-delta`, by emptiness (Chat
    /// Completions only): `true` is an empty fragment rig drops.
    empty_fragments: Vec<bool>,
}

impl WireToolCalls {
    /// The provider id of the `k`-th tool call of the reply, once its opening frame was read.
    pub(crate) fn id(&self, k: usize) -> Option<String> {
        self.lock()
            .calls
            .get(k)
            .map(|call| call.id.clone())
            .filter(|id| !id.is_empty())
    }

    /// How many empty fragments the `k`-th call streamed after its `nonempty`-th non-empty one
    /// (`0` = before the first), as far as the wire has been read.
    pub(crate) fn empties_after(&self, k: usize, nonempty: usize) -> usize {
        let state = self.lock();
        let Some(call) = state.calls.get(k) else {
            return 0;
        };
        let mut seen = 0;
        let mut empties = 0;
        for &empty in &call.empty_fragments {
            if !empty {
                if seen == nonempty {
                    break;
                }
                seen += 1;
            } else if seen == nonempty {
                empties += 1;
            }
        }
        empties
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn observe(&self, bytes: &[u8]) {
        let mut state = self.lock();
        state.partial_line.extend_from_slice(bytes);
        while let Some(end) = state.partial_line.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = state.partial_line.drain(..=end).collect();
            state.line(&line);
        }
    }
}

impl State {
    fn line(&mut self, line: &[u8]) {
        let Some(payload) = line.strip_prefix(b"data:") else {
            return;
        };
        let payload = payload.trim_ascii();
        // Cheap filter: most frames are text deltas, which never name a tool call.
        let mentions = |needle: &[u8]| payload.windows(needle.len()).any(|window| window == needle);
        if !(mentions(b"tool_calls") || mentions(b"function_call") || mentions(b"tool_use")) {
            return;
        }
        let Ok(frame) = serde_json::from_slice::<Value>(payload) else {
            return;
        };
        if let Some(choices) = frame.get("choices").and_then(Value::as_array) {
            for call in choices
                .iter()
                .filter_map(|choice| choice.pointer("/delta/tool_calls")?.as_array())
                .flatten()
            {
                self.chat_delta(call);
            }
            return;
        }
        match frame.get("type").and_then(Value::as_str) {
            Some("response.output_item.added")
                if frame.pointer("/item/type").and_then(Value::as_str) == Some("function_call") =>
            {
                let key = format!("resp:{}", frame["output_index"]);
                let id = frame.pointer("/item/call_id").and_then(Value::as_str);
                self.open(key, id.unwrap_or_default());
            }
            Some("content_block_start")
                if frame.pointer("/content_block/type").and_then(Value::as_str)
                    == Some("tool_use") =>
            {
                let key = format!("anth:{}", frame["index"]);
                let id = frame.pointer("/content_block/id").and_then(Value::as_str);
                self.open(key, id.unwrap_or_default());
            }
            _ => {}
        }
    }

    /// `StreamingToolCallTracker.processDelta`: a new call's first fragment is emitted only when
    /// non-empty; every later fragment, empty or not, is a delta.
    fn chat_delta(&mut self, call: &Value) {
        let key = match call.get("index") {
            Some(index) if !index.is_null() => format!("chat:{index}"),
            _ => format!("chat:id:{}", call["id"]),
        };
        let arguments = call.pointer("/function/arguments").and_then(Value::as_str);
        match self.by_key.get(&key).copied() {
            Some(position) => {
                if let (Some(arguments), Some(open)) = (arguments, self.calls.get_mut(position)) {
                    open.empty_fragments.push(arguments.is_empty());
                }
            }
            None => {
                let position = self.open(key, call["id"].as_str().unwrap_or_default());
                if let (Some(arguments), Some(open)) = (arguments, self.calls.get_mut(position))
                    && !arguments.is_empty()
                {
                    open.empty_fragments.push(false);
                }
            }
        }
    }

    fn open(&mut self, key: String, id: &str) -> usize {
        *self.by_key.entry(key).or_insert_with(|| {
            self.calls.push(WireCall {
                id: id.to_owned(),
                empty_fragments: Vec::new(),
            });
            self.calls.len() - 1
        })
    }
}

/// Wraps a streaming response so every body chunk is observed before rig decodes it, when a
/// translator is polling on this task. Elsewhere (one-shot completions) the body is untouched.
pub(crate) fn observe(
    response: StreamingResponse,
    calls: Option<Arc<WireToolCalls>>,
) -> StreamingResponse {
    let Some(calls) = calls else {
        return response;
    };
    response.map(|body| {
        let observed = body.inspect(move |chunk: &http_client::Result<Bytes>| {
            if let Ok(bytes) = chunk {
                calls.observe(bytes);
            }
        });
        Box::pin(observed) as http_client::BoxedStream
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(calls: &WireToolCalls, lines: &[&str]) {
        let text: String = lines
            .iter()
            .map(|line| format!("data: {line}\n\n"))
            .collect();
        // Split mid-line, as TCP reads do.
        let (head, tail) = text.split_at(text.len() / 2);
        calls.observe(head.as_bytes());
        calls.observe(tail.as_bytes());
    }

    #[test]
    fn chat_calls_keep_first_appearance_order_and_empty_fragments() {
        let calls = WireToolCalls::default();
        feed(
            &calls,
            &[
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"a","arguments":"{\"q\":"}}]}}]}"#,
                r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_2","function":{"name":"b","arguments":""}}]}}]}"#,
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]}}]}"#,
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":""}}]}}]}"#,
                r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"{}"}}]}}]}"#,
                "[DONE]",
            ],
        );
        assert_eq!(calls.id(0).as_deref(), Some("call_1"));
        assert_eq!(calls.id(1).as_deref(), Some("call_2"));
        assert_eq!(calls.id(2), None);
        // call_1: "{"q":" · "1}" · "" → one empty after the second non-empty fragment.
        assert_eq!(calls.empties_after(0, 0), 0);
        assert_eq!(calls.empties_after(0, 1), 0);
        assert_eq!(calls.empties_after(0, 2), 1);
        // call_2's empty opening fragment is not a delta (the SDK skips it).
        assert_eq!(calls.empties_after(1, 0), 0);
    }

    #[test]
    fn responses_and_anthropic_calls_take_the_provider_ids() {
        let calls = WireToolCalls::default();
        feed(
            &calls,
            &[
                r#"{"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_1","call_id":"call_9","name":"t"}}"#,
                r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"t","input":{}}}"#,
            ],
        );
        assert_eq!(calls.id(0).as_deref(), Some("call_9"));
        assert_eq!(calls.id(1).as_deref(), Some("toolu_1"));
    }
}
