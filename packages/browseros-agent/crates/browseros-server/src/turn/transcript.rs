//! The reducer for one assistant message and the frame log replay reads.
//!
//! `conversations` owns one `Transcript` per run, behind the run's lock: every event a backend
//! emits goes through `apply`, every subscriber reads `frames`, and every database write stores
//! `sealed`. The AI SDK chunk types and their order rules are private to this file.

use std::sync::Arc;

use super::{MessageId, TurnEnd, TurnEvent, UiMessage};

/// The reducer for one assistant message. Owns the frame log that replay reads.
#[derive(Debug)]
pub struct Transcript {
    #[expect(dead_code, reason = "read once chunk `runs` writes the reducer")]
    assistant: MessageId,
    frames: Vec<Frame>,
}

/// What the caller must do after `apply`. Only tool boundaries persist; deltas never do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checkpoint {
    None,
    Persist,
}

impl Transcript {
    /// Emits `start { messageId }`, so the client, the store and every replay share one id.
    pub fn new(assistant: MessageId) -> Self {
        Self {
            assistant,
            frames: Vec::new(),
        }
    }

    pub fn apply(&mut self, _event: TurnEvent) -> Checkpoint {
        Checkpoint::None
    }

    /// Closes open blocks, then writes the one terminal chunk: `finish`, `abort`, or `error`
    /// with the envelope as `errorText`. Nothing can be applied afterwards.
    pub fn finish(&mut self, _end: &TurnEnd) {}

    /// The frames from index `from` on; empty when `from` is past the end.
    pub fn frames(&self, from: usize) -> &[Frame] {
        self.frames.get(from..).unwrap_or_default()
    }

    /// The message as it must be stored if the turn ended now:
    ///   open text or reasoning          -> state Done
    ///   a call with input and no result -> Error("Interrupted")   (Error("Stopped") after a stop)
    ///   a call that only `ToolStarted`  -> Error("Not run"), input None
    /// `None` while the message has no display content. A `step-start` part alone is not
    /// content (the SDK adds one before anything else), nor is empty text: a turn that fails
    /// before it says anything must store nothing. Every database write uses this, so a crash
    /// never leaves a call without a result on disk.
    pub fn sealed(&self) -> Option<UiMessage> {
        None
    }
}

/// One serialized chunk. Opaque outside this module: the run log stores and replays frames
/// without knowing the chunk types, which stay private here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame(Arc<str>);

impl Frame {
    /// The JSON of one chunk: what follows `data: ` in the event stream.
    pub fn data(&self) -> &str {
        &self.0
    }
}

/// r4's five response headers for the UI message stream, in the order the TypeScript SDK sends
/// them. The extension's transport checks `x-vercel-ai-ui-message-stream`.
pub const STREAM_HEADERS: [(&str, &str); 5] = [
    ("content-type", "text/event-stream"),
    ("cache-control", "no-cache"),
    ("connection", "keep-alive"),
    ("x-vercel-ai-ui-message-stream", "v1"),
    ("x-accel-buffering", "no"),
];

/// The last event of every UI message stream.
pub const DONE: &str = "[DONE]";
