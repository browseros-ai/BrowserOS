//! The reducer for one assistant message and the frame log replay reads.
//!
//! `conversations` owns one `Transcript` per run, behind the run's lock: every event a backend
//! emits goes through `apply`, every subscriber reads `frames`, and every database write stores
//! `sealed`. The AI SDK chunk types and their order rules are private to this file.
//!
//! Each event is turned into chunks and, in the same call, folded into the message exactly as
//! the `ai@7.0.114` client reducer (`processUIMessageStream`) folds those chunks. That is what
//! makes the stored message and the one the panel built equal by construction; the golden
//! fixtures in `tests/fixtures/ui-stream` hold the two side by side, and their Bun half runs the
//! installed SDK over the same bytes.
//!
//! ```text
//!   TurnEvent ──apply──▶ chunks ──▶ frames (append-only, replayed from 0)
//!                          └──────▶ message parts (what `sealed` stores)
//! ```

use std::sync::Arc;

use serde::Serialize;
use serde_json::{Map, Value};

use super::{
    FinishReason, MessageId, PartState, Role, ToolCallId, ToolOutcome, ToolOwner, ToolPart,
    TurnEnd, TurnEvent, UiMessage, UiPart,
};

/// The error text of a call that was still running when the user stopped the turn.
const STOPPED: &str = "Stopped";
/// The error text of a call that was still running when the turn ended any other way, and of
/// every running call in a checkpoint: a crash right after that write leaves it unfinished.
const INTERRUPTED: &str = "Interrupted";
/// The error text of a call that was queued behind another and never started.
const NOT_RUN: &str = "Not run";

/// The reducer for one assistant message. Owns the frame log that replay reads.
#[derive(Debug)]
pub struct Transcript {
    /// The assistant message as the client's reducer holds it at this point of the stream.
    message: UiMessage,
    frames: Vec<Frame>,
    /// The one text or reasoning block a delta may extend. Any other event closes it first, so
    /// blocks never interleave and the client never sees a delta for a closed block.
    open: Option<OpenBlock>,
    /// How many text and reasoning blocks were opened: the next ids are `t<n+1>` and `r<n+1>`.
    texts: usize,
    reasonings: usize,
    /// Between `start-step` and `finish-step`.
    in_step: bool,
    /// `finish` ran: the terminal chunk is written and nothing more is applied.
    finished: bool,
}

#[derive(Debug, Clone)]
struct OpenBlock {
    kind: BlockKind,
    /// The wire id (`t1`, `r1`, …), which is also the reasoning part's `id`.
    id: String,
    /// The index of its part in `message.parts`.
    part: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    Reasoning,
}

/// What the caller must do after `apply`. Only tool boundaries persist; deltas never do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checkpoint {
    None,
    Persist,
}

/// The chunks this server emits: a subset of the `ai@7.0.114` `UIMessageChunk` union, with the
/// SDK's field names and the key order its own encoder writes. `http::dto::UiMessageChunk`
/// describes the same subset for the contract.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum Chunk<'a> {
    Start {
        #[serde(rename = "messageId")]
        message_id: &'a str,
    },
    StartStep,
    FinishStep,
    TextStart {
        id: &'a str,
    },
    TextDelta {
        id: &'a str,
        delta: &'a str,
    },
    TextEnd {
        id: &'a str,
    },
    ReasoningStart {
        id: &'a str,
    },
    ReasoningDelta {
        id: &'a str,
        delta: &'a str,
    },
    ReasoningEnd {
        id: &'a str,
    },
    #[serde(rename_all = "camelCase")]
    ToolInputStart {
        tool_call_id: &'a str,
        tool_name: &'a str,
        #[serde(flatten)]
        flags: AgentFlags,
    },
    #[serde(rename_all = "camelCase")]
    ToolInputAvailable {
        tool_call_id: &'a str,
        tool_name: &'a str,
        input: &'a Value,
        #[serde(flatten)]
        flags: AgentFlags,
    },
    #[serde(rename_all = "camelCase")]
    ToolOutputAvailable {
        tool_call_id: &'a str,
        output: &'a Value,
        #[serde(flatten)]
        flags: AgentFlags,
    },
    #[serde(rename_all = "camelCase")]
    ToolOutputError {
        tool_call_id: &'a str,
        error_text: &'a str,
        #[serde(flatten)]
        flags: AgentFlags,
    },
    #[serde(rename_all = "camelCase")]
    Finish {
        finish_reason: &'static str,
    },
    #[serde(rename_all = "camelCase")]
    Error {
        error_text: &'a str,
    },
    Abort,
}

/// `providerExecuted` and `dynamic`, both present for a tool the coding agent ran and both
/// absent for a server tool, in that order (the SDK's). `dynamic` makes the client store a
/// `dynamic-tool` part; `providerExecuted` tells it the tool already ran.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentFlags {
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_executed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dynamic: Option<bool>,
}

impl AgentFlags {
    fn of(owner: ToolOwner) -> Self {
        match owner {
            ToolOwner::Server => Self {
                provider_executed: None,
                dynamic: None,
            },
            ToolOwner::Agent => Self {
                provider_executed: Some(true),
                dynamic: Some(true),
            },
        }
    }
}

impl Transcript {
    /// Emits `start { messageId }`, so the client, the store and every replay share one id.
    pub fn new(assistant: MessageId) -> Self {
        let mut transcript = Self {
            message: UiMessage {
                id: assistant,
                role: Role::Assistant,
                parts: Vec::new(),
                metadata: None,
            },
            frames: Vec::new(),
            open: None,
            texts: 0,
            reasonings: 0,
            in_step: false,
            finished: false,
        };
        let id = transcript.message.id.0.clone();
        transcript.push(&Chunk::Start { message_id: &id });
        transcript
    }

    /// Reduces one event: appends its chunks to the log and folds them into the message. Says
    /// whether the message just crossed a tool boundary the caller must write to disk. An event
    /// the client's reducer would refuse (an output for an unknown call, a second result) is
    /// dropped rather than emitted, because one bad chunk fails the user's whole turn there.
    pub fn apply(&mut self, event: TurnEvent) -> Checkpoint {
        if self.finished {
            return Checkpoint::None;
        }
        match event {
            TurnEvent::Text(delta) => self.delta(BlockKind::Text, &delta),
            TurnEvent::Reasoning(delta) => self.delta(BlockKind::Reasoning, &delta),
            TurnEvent::StepStart => {
                self.close_block();
                if self.in_step {
                    self.push(&Chunk::FinishStep);
                }
                self.push(&Chunk::StartStep);
                self.message.parts.push(UiPart::StepStart);
                self.in_step = true;
                Checkpoint::None
            }
            TurnEvent::StepEnd => {
                self.close_block();
                if self.in_step {
                    self.push(&Chunk::FinishStep);
                    self.in_step = false;
                }
                Checkpoint::None
            }
            TurnEvent::ToolStarted { id, name, owner } => {
                self.close_block();
                self.tool_started(id, name, owner)
            }
            TurnEvent::ToolInput {
                id,
                name,
                input,
                owner,
            } => {
                self.close_block();
                self.tool_input(id, name, input, owner)
            }
            TurnEvent::ToolDone { id, result } => {
                self.close_block();
                self.tool_done(&id, result)
            }
        }
    }

    /// Closes open blocks (`text-end`, `reasoning-end`; `finish-step` does not close them), gives
    /// every call without a result a `tool-output-error`, then writes the one terminal chunk:
    /// `finish`, `abort`, or `error` with the envelope as `errorText`. Nothing can be applied
    /// afterwards, and a second call does nothing.
    pub fn finish(&mut self, end: &TurnEnd) {
        if self.finished {
            return;
        }
        self.close_block();
        let running = match end {
            TurnEnd::Stopped => STOPPED,
            TurnEnd::Done(_) | TurnEnd::Failed(_) => INTERRUPTED,
        };
        let unresolved: Vec<(usize, &'static str)> = self
            .message
            .parts
            .iter()
            .enumerate()
            .filter_map(|(index, part)| match part {
                UiPart::Tool(tool) => match tool.outcome {
                    ToolOutcome::InputStreaming => Some((index, NOT_RUN)),
                    ToolOutcome::InputAvailable => Some((index, running)),
                    ToolOutcome::Output(_) | ToolOutcome::Error(_) => None,
                },
                _ => None,
            })
            .collect();
        for (index, error) in unresolved {
            self.resolve(index, Err(error.to_owned()));
        }
        match end {
            TurnEnd::Done(reason) => self.push(&Chunk::Finish {
                finish_reason: finish_reason(*reason),
            }),
            TurnEnd::Stopped => self.push(&Chunk::Abort),
            TurnEnd::Failed(error) => {
                let error_text = error.envelope_text();
                self.push(&Chunk::Error {
                    error_text: &error_text,
                });
            }
        }
        self.finished = true;
    }

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
    ///
    /// After `finish` every call already has its result ("Stopped" included), so this is the
    /// message the client's reducer built from the whole stream.
    pub fn sealed(&self) -> Option<UiMessage> {
        if !self.message.parts.iter().any(has_display_content) {
            return None;
        }
        let mut message = self.message.clone();
        for part in &mut message.parts {
            match part {
                UiPart::Text { state, .. } | UiPart::Reasoning { state, .. } => {
                    if *state == Some(PartState::Streaming) {
                        *state = Some(PartState::Done);
                    }
                }
                UiPart::Tool(tool) => match tool.outcome {
                    ToolOutcome::InputStreaming => {
                        tool.outcome = ToolOutcome::Error(NOT_RUN.to_owned());
                        tool.input = None;
                    }
                    ToolOutcome::InputAvailable => {
                        tool.outcome = ToolOutcome::Error(INTERRUPTED.to_owned());
                    }
                    ToolOutcome::Output(_) | ToolOutcome::Error(_) => {}
                },
                UiPart::StepStart | UiPart::Other(_) => {}
            }
        }
        Some(message)
    }

    fn push(&mut self, chunk: &Chunk<'_>) {
        // Strings, booleans and JSON values only: serialization cannot fail.
        let json = serde_json::to_string(chunk).unwrap_or_default();
        self.frames.push(Frame(Arc::from(json)));
    }

    /// A text or reasoning delta: extends the open block of the same kind, or closes whatever is
    /// open and starts a new block with the next id. An empty delta is nothing: it would open a
    /// block with no text, which the client renders as an empty bubble.
    fn delta(&mut self, kind: BlockKind, delta: &str) -> Checkpoint {
        if delta.is_empty() {
            return Checkpoint::None;
        }
        let block = match &self.open {
            Some(open) if open.kind == kind => open.clone(),
            _ => {
                self.close_block();
                self.open_block(kind)
            }
        };
        match kind {
            BlockKind::Text => self.push(&Chunk::TextDelta {
                id: &block.id,
                delta,
            }),
            BlockKind::Reasoning => self.push(&Chunk::ReasoningDelta {
                id: &block.id,
                delta,
            }),
        }
        if let Some(UiPart::Text { text, .. } | UiPart::Reasoning { text, .. }) =
            self.message.parts.get_mut(block.part)
        {
            text.push_str(delta);
        }
        Checkpoint::None
    }

    fn open_block(&mut self, kind: BlockKind) -> OpenBlock {
        let (id, part) = match kind {
            BlockKind::Text => {
                self.texts += 1;
                let id = format!("t{}", self.texts);
                self.push(&Chunk::TextStart { id: &id });
                let part = UiPart::Text {
                    text: String::new(),
                    state: Some(PartState::Streaming),
                    extra: Map::new(),
                };
                (id, part)
            }
            BlockKind::Reasoning => {
                self.reasonings += 1;
                let id = format!("r{}", self.reasonings);
                self.push(&Chunk::ReasoningStart { id: &id });
                let part = UiPart::Reasoning {
                    id: Some(id.clone()),
                    text: String::new(),
                    state: Some(PartState::Streaming),
                    extra: Map::new(),
                };
                (id, part)
            }
        };
        self.message.parts.push(part);
        let block = OpenBlock {
            kind,
            id,
            part: self.message.parts.len() - 1,
        };
        self.open = Some(block.clone());
        block
    }

    fn close_block(&mut self) {
        let Some(block) = self.open.take() else {
            return;
        };
        match block.kind {
            BlockKind::Text => self.push(&Chunk::TextEnd { id: &block.id }),
            BlockKind::Reasoning => self.push(&Chunk::ReasoningEnd { id: &block.id }),
        }
        if let Some(UiPart::Text { state, .. } | UiPart::Reasoning { state, .. }) =
            self.message.parts.get_mut(block.part)
        {
            *state = Some(PartState::Done);
        }
    }

    /// The latest tool part for `id`. A call id is unique within a turn, so this is the call.
    fn tool(&self, id: &ToolCallId) -> Option<usize> {
        self.message
            .parts
            .iter()
            .rposition(|part| matches!(part, UiPart::Tool(tool) if tool.id == *id))
    }

    fn tool_started(&mut self, id: ToolCallId, name: String, owner: ToolOwner) -> Checkpoint {
        if self.tool(&id).is_some() {
            return Checkpoint::None;
        }
        self.push(&Chunk::ToolInputStart {
            tool_call_id: &id.0,
            tool_name: &name,
            flags: AgentFlags::of(owner),
        });
        self.message.parts.push(UiPart::Tool(ToolPart {
            id,
            name,
            owner,
            title: None,
            input: None,
            outcome: ToolOutcome::InputStreaming,
            extra: Map::new(),
        }));
        Checkpoint::None
    }

    /// A call's complete input: the call is about to run, so the caller persists. A call that
    /// never had a `ToolStarted` is created here (`tool-input-available` may open a call on its
    /// own). For a call that exists, its first name and owner are kept and sent again, because
    /// the client keys the part's type on them and would otherwise add a second part.
    fn tool_input(
        &mut self,
        id: ToolCallId,
        name: String,
        input: Value,
        owner: ToolOwner,
    ) -> Checkpoint {
        let index = match self.tool(&id) {
            Some(index) => index,
            None => {
                self.message.parts.push(UiPart::Tool(ToolPart {
                    id,
                    name,
                    owner,
                    title: None,
                    input: None,
                    outcome: ToolOutcome::InputStreaming,
                    extra: Map::new(),
                }));
                self.message.parts.len() - 1
            }
        };
        let Some(UiPart::Tool(tool)) = self.message.parts.get_mut(index) else {
            return Checkpoint::None;
        };
        if matches!(tool.outcome, ToolOutcome::Output(_) | ToolOutcome::Error(_)) {
            return Checkpoint::None;
        }
        tool.input = Some(input);
        tool.outcome = ToolOutcome::InputAvailable;
        let chunk = serde_json::to_string(&Chunk::ToolInputAvailable {
            tool_call_id: &tool.id.0,
            tool_name: &tool.name,
            input: tool.input.as_ref().unwrap_or(&Value::Null),
            flags: AgentFlags::of(tool.owner),
        })
        .unwrap_or_default();
        self.frames.push(Frame(Arc::from(chunk)));
        Checkpoint::Persist
    }

    /// A call's result. Only a call that exists and has no result yet takes one: the client's
    /// reducer fails the turn on an output for a call it never saw.
    fn tool_done(&mut self, id: &ToolCallId, result: Result<Value, String>) -> Checkpoint {
        let Some(index) = self.tool(id) else {
            return Checkpoint::None;
        };
        if let Some(UiPart::Tool(tool)) = self.message.parts.get(index)
            && matches!(tool.outcome, ToolOutcome::Output(_) | ToolOutcome::Error(_))
        {
            return Checkpoint::None;
        }
        self.resolve(index, result);
        Checkpoint::Persist
    }

    /// Writes the result chunk for the tool part at `index` and records it on the part.
    fn resolve(&mut self, index: usize, result: Result<Value, String>) {
        let Some(UiPart::Tool(tool)) = self.message.parts.get_mut(index) else {
            return;
        };
        let flags = AgentFlags::of(tool.owner);
        let chunk = match &result {
            Ok(output) => serde_json::to_string(&Chunk::ToolOutputAvailable {
                tool_call_id: &tool.id.0,
                output,
                flags,
            }),
            Err(error_text) => serde_json::to_string(&Chunk::ToolOutputError {
                tool_call_id: &tool.id.0,
                error_text,
                flags,
            }),
        }
        .unwrap_or_default();
        tool.outcome = match result {
            Ok(output) => ToolOutcome::Output(output),
            Err(error) => ToolOutcome::Error(error),
        };
        self.frames.push(Frame(Arc::from(chunk)));
    }
}

/// Whether a part is something the panel shows: a step marker and empty text are not.
fn has_display_content(part: &UiPart) -> bool {
    match part {
        UiPart::StepStart => false,
        UiPart::Text { text, .. } => !text.is_empty(),
        UiPart::Reasoning { .. } | UiPart::Tool(_) | UiPart::Other(_) => true,
    }
}

fn finish_reason(reason: FinishReason) -> &'static str {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::Length => "length",
        FinishReason::ContentFilter => "content-filter",
        FinishReason::ToolCalls => "tool-calls",
        FinishReason::Error => "error",
        FinishReason::Other => "other",
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chunks(transcript: &Transcript) -> Vec<Value> {
        transcript
            .frames(0)
            .iter()
            .map(|frame| serde_json::from_str(frame.data()).unwrap_or(Value::Null))
            .collect()
    }

    fn started(id: &str) -> TurnEvent {
        TurnEvent::ToolStarted {
            id: ToolCallId(id.into()),
            name: "run".into(),
            owner: ToolOwner::Server,
        }
    }

    fn input(id: &str) -> TurnEvent {
        TurnEvent::ToolInput {
            id: ToolCallId(id.into()),
            name: "run".into(),
            input: json!({ "code": "1" }),
            owner: ToolOwner::Server,
        }
    }

    #[test]
    fn only_tool_boundaries_are_checkpoints() {
        let mut transcript = Transcript::new(MessageId("m".into()));
        assert_eq!(transcript.apply(TurnEvent::StepStart), Checkpoint::None);
        assert_eq!(
            transcript.apply(TurnEvent::Text("a".into())),
            Checkpoint::None
        );
        assert_eq!(transcript.apply(started("c")), Checkpoint::None);
        assert_eq!(transcript.apply(input("c")), Checkpoint::Persist);
        let done = TurnEvent::ToolDone {
            id: ToolCallId("c".into()),
            result: Ok(json!(2)),
        };
        assert_eq!(transcript.apply(done.clone()), Checkpoint::Persist);
        assert_eq!(
            transcript.apply(done),
            Checkpoint::None,
            "a second result is dropped"
        );
    }

    #[test]
    fn a_checkpoint_mid_call_stores_the_call_as_interrupted() -> Result<(), serde_json::Error> {
        let mut transcript = Transcript::new(MessageId("m".into()));
        for event in [
            TurnEvent::StepStart,
            TurnEvent::Text("Checking".into()),
            started("c1"),
            started("c2"),
            input("c1"),
        ] {
            transcript.apply(event);
        }
        let sealed = serde_json::to_value(transcript.sealed())?;
        assert_eq!(
            sealed["parts"],
            json!([
                { "type": "step-start" },
                { "type": "text", "text": "Checking", "state": "done" },
                { "type": "tool-run", "toolCallId": "c1", "state": "output-error",
                  "input": { "code": "1" }, "errorText": "Interrupted" },
                { "type": "tool-run", "toolCallId": "c2", "state": "output-error",
                  "errorText": "Not run" },
            ])
        );
        Ok(())
    }

    #[test]
    fn events_after_the_end_are_not_applied() {
        let mut transcript = Transcript::new(MessageId("m".into()));
        transcript.finish(&TurnEnd::Stopped);
        let frames = transcript.frames(0).len();
        transcript.apply(TurnEvent::Text("late".into()));
        transcript.finish(&TurnEnd::Done(FinishReason::Stop));
        assert_eq!(transcript.frames(0).len(), frames);
        assert_eq!(
            chunks(&transcript),
            vec![
                json!({ "type": "start", "messageId": "m" }),
                json!({ "type": "abort" })
            ]
        );
    }

    #[test]
    fn an_output_for_a_call_never_seen_is_dropped() {
        let mut transcript = Transcript::new(MessageId("m".into()));
        let checkpoint = transcript.apply(TurnEvent::ToolDone {
            id: ToolCallId("ghost".into()),
            result: Ok(json!(1)),
        });
        assert_eq!(checkpoint, Checkpoint::None);
        assert_eq!(transcript.frames(0).len(), 1, "only `start`");
    }

    #[test]
    fn an_empty_delta_opens_no_block() {
        let mut transcript = Transcript::new(MessageId("m".into()));
        transcript.apply(TurnEvent::StepStart);
        transcript.apply(TurnEvent::Text(String::new()));
        assert_eq!(transcript.sealed(), None);
        assert_eq!(transcript.frames(0).len(), 2, "`start`, `start-step`");
    }
}
