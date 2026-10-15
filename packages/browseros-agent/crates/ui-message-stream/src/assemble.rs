//! Reduces either engine's ordered chunks into the assistant message persisted by the run owner.
//! This ports `ai@7.0.114/src/ui/process-ui-message-stream.ts`, including step-local tool lookup,
//! partial JSON, and the distinction between an explicit part end and an interrupted turn.

use crate::{
    chunk::UiChunk,
    message::{
        PartState, ReasoningPart, Role, TextPart, ToolPart, ToolPartKind, ToolState, UiMessage,
        UiPart,
    },
};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

/// A protocol ordering violation. Like the SDK reducer throwing, it stops subsequent reduction
/// while retaining the last valid message; ordinary `error` chunks do not cause this failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{chunk_type}: missing part {id:?}")]
pub struct AssembleError {
    pub chunk_type: &'static str,
    pub id: String,
}

/// Owns one turn's message and correlation maps. Part indices stay valid until `reset-step`,
/// which discards the current step and clears every active map, as the SDK does.
#[derive(Debug, Default)]
pub struct UiMessageAssembler {
    message: Option<UiMessage>,
    text: HashMap<String, usize>,
    reasoning: HashMap<String, usize>,
    partial_tools: HashMap<String, PartialTool>,
    error: Option<AssembleError>,
}

#[derive(Debug)]
struct PartialTool {
    text: String,
    name: String,
    dynamic: bool,
    title: Option<String>,
    metadata: Option<Value>,
}

/// Absent output/error/preliminary clear old values, whereas title, tool metadata and provider
/// execution are sticky across updates. Both SDK tool update paths implement these rules.
struct ToolUpdate<'a> {
    id: &'a str,
    name: &'a str,
    dynamic: bool,
    state: ToolState,
    input: Option<Value>,
    output: Option<Value>,
    raw_input: Option<Value>,
    error: Option<String>,
    preliminary: Option<bool>,
    title: Option<String>,
    metadata: Option<Value>,
    provider_executed: Option<bool>,
    provider_metadata: Option<Value>,
}

impl UiMessageAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a chunk in emission order. A `start` opens the message with an empty id unless
    /// the engine supplies one (built-in turns intentionally persist `id: ""`, recon F8).
    /// Malformed ordering freezes reduction; callers can inspect [`Self::error`].
    pub fn push(&mut self, chunk: &UiChunk) {
        if self.error.is_some() {
            return;
        }
        if let Err(error) = self.apply(chunk) {
            self.error = Some(error);
        }
    }

    /// `None` before the first `start` chunk.
    pub fn message(&self) -> Option<&UiMessage> {
        self.message.as_ref()
    }
    pub fn into_message(self) -> Option<UiMessage> {
        self.message
    }
    pub fn error(&self) -> Option<&AssembleError> {
        self.error.as_ref()
    }

    fn apply(&mut self, chunk: &UiChunk) -> Result<(), AssembleError> {
        if matches!(chunk, UiChunk::Start { .. }) && self.message.is_none() {
            self.message = Some(UiMessage {
                id: String::new(),
                role: Role::Assistant,
                metadata: None,
                parts: Vec::new(),
            });
        }
        let Some(message) = self.message.as_mut() else {
            return Ok(());
        };
        match chunk {
            UiChunk::Start {
                message_id,
                message_metadata,
            } => {
                if let Some(id) = message_id {
                    message.id.clone_from(id);
                }
                update_metadata(message, message_metadata.as_ref());
            }
            UiChunk::Finish {
                message_metadata, ..
            } => update_metadata(message, message_metadata.as_ref()),
            UiChunk::MessageMetadata { message_metadata } => {
                update_metadata(message, Some(message_metadata))
            }
            UiChunk::StartStep => message.parts.push(UiPart::StepStart),
            // Finish/error/abort do not close parts: an interrupted part must still render as
            // streaming after reload. Merged streams may also finish a step with active parts.
            UiChunk::FinishStep | UiChunk::Error { .. } | UiChunk::Abort { .. } => {}
            UiChunk::ResetStep => {
                message.parts.truncate(step_start(message));
                self.text.clear();
                self.reasoning.clear();
                self.partial_tools.clear();
            }
            UiChunk::TextStart {
                id,
                provider_metadata,
            } => {
                self.text.insert(id.clone(), message.parts.len());
                message.parts.push(UiPart::Text(TextPart {
                    text: String::new(),
                    state: Some(PartState::Streaming),
                    provider_metadata: provider_metadata.clone(),
                }));
            }
            UiChunk::TextDelta {
                id,
                delta,
                provider_metadata,
            } => {
                let index = active_index(&self.text, "text-delta", id)?;
                if let UiPart::Text(part) = &mut message.parts[index] {
                    part.text.push_str(delta);
                    replace_if_some(&mut part.provider_metadata, provider_metadata);
                }
            }
            UiChunk::TextEnd {
                id,
                provider_metadata,
            } => {
                let index = active_index(&self.text, "text-end", id)?;
                if let UiPart::Text(part) = &mut message.parts[index] {
                    part.state = Some(PartState::Done);
                    replace_if_some(&mut part.provider_metadata, provider_metadata);
                }
                self.text.remove(id);
            }
            UiChunk::ReasoningStart {
                id,
                provider_metadata,
            } => {
                self.reasoning.insert(id.clone(), message.parts.len());
                message.parts.push(UiPart::Reasoning(ReasoningPart {
                    id: Some(id.clone()),
                    text: String::new(),
                    state: Some(PartState::Streaming),
                    provider_metadata: provider_metadata.clone(),
                }));
            }
            UiChunk::ReasoningDelta {
                id,
                delta,
                provider_metadata,
            } => {
                let index = active_index(&self.reasoning, "reasoning-delta", id)?;
                if let UiPart::Reasoning(part) = &mut message.parts[index] {
                    part.text.push_str(delta);
                    replace_if_some(&mut part.provider_metadata, provider_metadata);
                }
            }
            UiChunk::ReasoningEnd {
                id,
                provider_metadata,
            } => {
                let index = active_index(&self.reasoning, "reasoning-end", id)?;
                if let UiPart::Reasoning(part) = &mut message.parts[index] {
                    part.state = Some(PartState::Done);
                    replace_if_some(&mut part.provider_metadata, provider_metadata);
                }
                self.reasoning.remove(id);
            }
            UiChunk::ToolInputStart {
                tool_call_id,
                tool_name,
                provider_executed,
                provider_metadata,
                tool_metadata,
                dynamic,
                title,
            } => {
                self.partial_tools.insert(
                    tool_call_id.clone(),
                    PartialTool {
                        text: String::new(),
                        name: tool_name.clone(),
                        dynamic: dynamic.unwrap_or(false),
                        title: title.clone(),
                        metadata: tool_metadata.clone(),
                    },
                );
                update_tool(
                    message,
                    ToolUpdate {
                        id: tool_call_id,
                        name: tool_name,
                        dynamic: dynamic.unwrap_or(false),
                        state: ToolState::InputStreaming,
                        input: None,
                        output: None,
                        raw_input: None,
                        error: None,
                        preliminary: None,
                        title: title.clone(),
                        metadata: tool_metadata.clone(),
                        provider_executed: *provider_executed,
                        provider_metadata: provider_metadata.clone(),
                    },
                    None,
                );
            }
            UiChunk::ToolInputDelta {
                tool_call_id,
                input_text_delta,
            } => {
                let partial = self
                    .partial_tools
                    .get_mut(tool_call_id)
                    .ok_or_else(|| missing("tool-input-delta", tool_call_id))?;
                partial.text.push_str(input_text_delta);
                update_tool(
                    message,
                    ToolUpdate {
                        id: tool_call_id,
                        name: &partial.name,
                        dynamic: partial.dynamic,
                        state: ToolState::InputStreaming,
                        input: partial_json::parse(&partial.text),
                        output: None,
                        raw_input: None,
                        error: None,
                        preliminary: None,
                        title: partial.title.clone(),
                        metadata: partial.metadata.clone(),
                        provider_executed: None,
                        provider_metadata: None,
                    },
                    None,
                );
            }
            UiChunk::ToolInputAvailable {
                tool_call_id,
                tool_name,
                input,
                provider_executed,
                provider_metadata,
                tool_metadata,
                dynamic,
                title,
            } => {
                update_tool(
                    message,
                    ToolUpdate {
                        id: tool_call_id,
                        name: tool_name,
                        dynamic: dynamic.unwrap_or(false),
                        state: ToolState::InputAvailable,
                        input: Some(input.clone()),
                        output: None,
                        raw_input: None,
                        error: None,
                        preliminary: None,
                        title: title.clone(),
                        metadata: tool_metadata.clone(),
                        provider_executed: *provider_executed,
                        provider_metadata: provider_metadata.clone(),
                    },
                    None,
                );
            }
            UiChunk::ToolInputError {
                tool_call_id,
                tool_name,
                input,
                provider_executed,
                provider_metadata,
                tool_metadata,
                dynamic,
                error_text,
                ..
            } => {
                // ACP's legacy invalid-tool chunk says dynamic:true after a static start. The
                // existing part wins; otherwise one invocation would persist twice (recon F6).
                let existing = find_tool(message, tool_call_id, None, false);
                let dynamic = existing.map(|index| matches!(&message.parts[index], UiPart::Tool(part) if matches!(part.kind, ToolPartKind::Dynamic { .. }))).unwrap_or(dynamic.unwrap_or(false));
                update_tool(
                    message,
                    ToolUpdate {
                        id: tool_call_id,
                        name: tool_name,
                        dynamic,
                        state: ToolState::OutputError,
                        input: dynamic.then(|| input.clone()),
                        output: None,
                        raw_input: (!dynamic).then(|| input.clone()),
                        error: Some(error_text.clone()),
                        preliminary: None,
                        title: None,
                        metadata: tool_metadata.clone(),
                        provider_executed: *provider_executed,
                        provider_metadata: provider_metadata.clone(),
                    },
                    None,
                );
            }
            UiChunk::ToolOutputAvailable {
                tool_call_id,
                provider_executed,
                provider_metadata,
                ..
            }
            | UiChunk::ToolOutputError {
                tool_call_id,
                provider_executed,
                provider_metadata,
                ..
            } => {
                let index = tool_index(message, tool_call_id)?;
                if let UiPart::Tool(part) = &message.parts[index] {
                    let (dynamic, name) = match &part.kind {
                        ToolPartKind::Static { name } => (false, name.clone()),
                        ToolPartKind::Dynamic { tool_name } => (true, tool_name.clone()),
                    };
                    let (state, output, error, preliminary) = match chunk {
                        UiChunk::ToolOutputAvailable {
                            output,
                            preliminary,
                            ..
                        } => (
                            ToolState::OutputAvailable,
                            Some(output.clone()),
                            None,
                            *preliminary,
                        ),
                        UiChunk::ToolOutputError { error_text, .. } => {
                            (ToolState::OutputError, None, Some(error_text.clone()), None)
                        }
                        _ => unreachable!(),
                    };
                    let update = ToolUpdate {
                        id: tool_call_id,
                        name: &name,
                        dynamic,
                        state,
                        input: part.input.clone(),
                        output,
                        raw_input: part
                            .extra
                            .get("rawInput")
                            .cloned()
                            .filter(|_| state == ToolState::OutputError),
                        error,
                        preliminary,
                        title: part.title.clone(),
                        metadata: part.tool_metadata.clone(),
                        provider_executed: *provider_executed,
                        provider_metadata: provider_metadata.clone(),
                    };
                    update_tool(message, update, Some(index));
                }
            }
            UiChunk::ToolApprovalRequest {
                approval_id,
                tool_call_id,
                approval_descriptor,
                input_schema_input,
                reason,
                is_automatic,
                signature,
            } => {
                let index = tool_index(message, tool_call_id)?;
                if let UiPart::Tool(part) = &mut message.parts[index] {
                    let mut approval = json!({"id": approval_id});
                    for (key, value) in [
                        ("descriptor", approval_descriptor.clone()),
                        ("inputSchemaInput", input_schema_input.clone()),
                        ("requestReason", reason.clone().map(Value::String)),
                        (
                            "isAutomatic",
                            (*is_automatic == Some(true)).then_some(Value::Bool(true)),
                        ),
                        ("signature", signature.clone().map(Value::String)),
                    ] {
                        if let Some(value) = value {
                            approval[key] = value;
                        }
                    }
                    part.state = ToolState::ApprovalRequested;
                    part.approval = Some(approval);
                }
            }
            UiChunk::ToolApprovalResponse {
                approval_id,
                approved,
                reason,
                provider_executed,
                provider_metadata,
            } => {
                let part = message
                    .parts
                    .iter_mut()
                    .find_map(|part| match part {
                        UiPart::Tool(tool)
                            if tool
                                .approval
                                .as_ref()
                                .and_then(|approval| approval.get("id"))
                                .and_then(Value::as_str)
                                == Some(approval_id) =>
                        {
                            Some(tool)
                        }
                        _ => None,
                    })
                    .ok_or_else(|| missing("tool-approval-response", approval_id))?;
                if let Some(approval) = &mut part.approval {
                    approval["approved"] = Value::Bool(*approved);
                    if let Some(reason) = reason {
                        approval["reason"] = Value::String(reason.clone());
                    }
                }
                part.state = ToolState::ApprovalResponded;
                replace_if_some(&mut part.provider_executed, provider_executed);
                replace_if_some(&mut part.call_provider_metadata, provider_metadata);
            }
            UiChunk::ToolOutputDenied { tool_call_id } => {
                let index = tool_index(message, tool_call_id)?;
                if let UiPart::Tool(part) = &mut message.parts[index] {
                    part.state = ToolState::OutputDenied;
                }
            }
            UiChunk::Data(data) => {
                if data.transient != Some(true) {
                    let existing = data.id.as_ref().and_then(|id| {
                        message.parts.iter_mut().find_map(|part| match part {
                            UiPart::Other(map)
                                if map.get("type").and_then(Value::as_str) == Some(&data.kind)
                                    && map.get("id").and_then(Value::as_str) == Some(id) =>
                            {
                                Some(map)
                            }
                            _ => None,
                        })
                    });
                    if let Some(existing) = existing {
                        existing.insert("data".into(), data.data.clone());
                    } else {
                        push_other(message, chunk);
                    }
                }
            }
            UiChunk::File { .. }
            | UiChunk::ReasoningFile { .. }
            | UiChunk::SourceUrl { .. }
            | UiChunk::SourceDocument { .. }
            | UiChunk::Custom { .. } => push_other(message, chunk),
        }
        Ok(())
    }
}

fn missing(chunk_type: &'static str, id: &str) -> AssembleError {
    AssembleError {
        chunk_type,
        id: id.to_owned(),
    }
}
fn active_index(
    active: &HashMap<String, usize>,
    kind: &'static str,
    id: &str,
) -> Result<usize, AssembleError> {
    active.get(id).copied().ok_or_else(|| missing(kind, id))
}
fn replace_if_some<T: Clone>(target: &mut Option<T>, value: &Option<T>) {
    if value.is_some() {
        target.clone_from(value);
    }
}
fn step_start(message: &UiMessage) -> usize {
    message
        .parts
        .iter()
        .rposition(|part| matches!(part, UiPart::StepStart))
        .map_or(0, |index| index + 1)
}

/// Input chunks update only the current step and same tool kind. Outputs first search that
/// step, then fall back to the newest matching earlier invocation (approval continuations).
fn find_tool(
    message: &UiMessage,
    id: &str,
    dynamic: Option<bool>,
    fallback: bool,
) -> Option<usize> {
    let matches = |part: &UiPart| matches!(part, UiPart::Tool(tool) if tool.tool_call_id == id && dynamic.is_none_or(|dynamic| dynamic == matches!(tool.kind, ToolPartKind::Dynamic { .. })));
    let start = step_start(message);
    message.parts[start..]
        .iter()
        .position(matches)
        .map(|index| start + index)
        .or_else(|| {
            fallback
                .then(|| message.parts.iter().rposition(matches))
                .flatten()
        })
}
fn tool_index(message: &UiMessage, id: &str) -> Result<usize, AssembleError> {
    find_tool(message, id, None, true).ok_or_else(|| missing("tool-invocation", id))
}

fn update_tool(message: &mut UiMessage, update: ToolUpdate<'_>, existing: Option<usize>) {
    let index = existing.or_else(|| find_tool(message, update.id, Some(update.dynamic), false));
    let part = if let Some(index) = index {
        let UiPart::Tool(part) = &mut message.parts[index] else {
            return;
        };
        part
    } else {
        message.parts.push(UiPart::Tool(Box::new(ToolPart {
            kind: if update.dynamic {
                ToolPartKind::Dynamic {
                    tool_name: update.name.to_owned(),
                }
            } else {
                ToolPartKind::Static {
                    name: update.name.to_owned(),
                }
            },
            tool_call_id: update.id.to_owned(),
            state: update.state,
            input: None,
            output: None,
            error_text: None,
            title: None,
            tool_metadata: None,
            provider_executed: None,
            call_provider_metadata: None,
            result_provider_metadata: None,
            preliminary: None,
            approval: None,
            extra: Map::new(),
        })));
        let Some(UiPart::Tool(part)) = message.parts.last_mut() else {
            return;
        };
        part
    };
    part.state = update.state;
    if let ToolPartKind::Dynamic { tool_name } = &mut part.kind {
        *tool_name = update.name.to_owned();
    }
    part.input = update.input;
    part.output = update.output;
    part.error_text = update.error;
    part.preliminary = update.preliminary;
    if let Some(raw) = update.raw_input {
        part.extra.insert("rawInput".into(), raw);
    } else if !update.dynamic {
        part.extra.shift_remove("rawInput");
    }
    replace_if_some(&mut part.title, &update.title);
    replace_if_some(&mut part.tool_metadata, &update.metadata);
    replace_if_some(&mut part.provider_executed, &update.provider_executed);
    let metadata = if matches!(
        update.state,
        ToolState::OutputAvailable | ToolState::OutputError
    ) {
        &mut part.result_provider_metadata
    } else {
        &mut part.call_provider_metadata
    };
    replace_if_some(metadata, &update.provider_metadata);
}

fn push_other(message: &mut UiMessage, chunk: &UiChunk) {
    // UiChunk contains only JSON values; serializing into a Value cannot fail. Keep file parts
    // typed for downstream model conversion; source/custom/data remain lossless open parts.
    if let Ok(part) = serde_json::to_value(chunk).and_then(serde_json::from_value) {
        message.parts.push(part);
    }
}
fn update_metadata(message: &mut UiMessage, metadata: Option<&Value>) {
    if let Some(metadata) = metadata.filter(|value| !value.is_null()) {
        message.metadata = Some(match message.metadata.take() {
            Some(base) => merge_metadata(base, metadata),
            None => metadata.clone(),
        });
    }
}
fn merge_metadata(base: Value, overrides: &Value) -> Value {
    // SDK mergeObjects uses object spread even for primitive/array root metadata. Nested
    // objects merge recursively; arrays and primitives at a property replace the old value.
    let mut result = object_spread(&base);
    for (key, value) in object_spread(overrides) {
        if matches!(key.as_str(), "__proto__" | "constructor" | "prototype") {
            continue;
        }
        let value = match result.get(&key) {
            Some(old) if old.is_object() && value.is_object() => {
                merge_metadata(old.clone(), &value)
            }
            _ => value,
        };
        result.insert(key, value);
    }
    Value::Object(result)
}
fn object_spread(value: &Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map.clone(),
        Value::Array(values) => values
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v.clone()))
            .collect(),
        Value::String(text) => text
            .chars()
            .enumerate()
            .map(|(i, c)| (i.to_string(), Value::String(c.to_string())))
            .collect(),
        _ => Map::new(),
    }
}

/// The SDK's fixJson scanner retains the last complete token, closes open containers/strings,
/// and completes literals. It deliberately does not accept arbitrary malformed JSON; the
/// normal parser validates the repaired text before it becomes an input-streaming snapshot.
mod partial_json {
    use serde_json::Value;
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Root,
        Finish,
        String,
        Escape,
        Unicode,
        Literal,
        Number,
        ObjectStart,
        Key,
        AfterKey,
        BeforeValue,
        AfterValue,
        ObjectComma,
        ArrayStart,
        ArrayValue,
        ArrayComma,
    }

    pub(super) fn parse(input: &str) -> Option<Value> {
        parse_safe(input)
            .or_else(|| parse_safe(&fix(input)))
            .map(normalize_numbers)
    }

    fn parse_safe(input: &str) -> Option<Value> {
        let value = serde_json::from_str(input).ok()?;
        // Match provider-utils' secureJsonParse. Rust has no object prototypes, but accepting
        // a value rejected by the SDK would change the persisted partial-input snapshot.
        (!has_prototype_property(&value)).then_some(value)
    }

    fn has_prototype_property(value: &Value) -> bool {
        match value {
            Value::Object(map) => {
                map.contains_key("__proto__")
                    || map
                        .get("constructor")
                        .and_then(Value::as_object)
                        .is_some_and(|map| map.contains_key("prototype"))
                    || map.values().any(has_prototype_property)
            }
            Value::Array(values) => values.iter().any(has_prototype_property),
            _ => false,
        }
    }

    fn normalize_numbers(value: Value) -> Value {
        match value {
            Value::Number(number) => number
                .as_f64()
                .and_then(|number| {
                    serde_json::from_str(ryu_js::Buffer::new().format_finite(number)).ok()
                })
                .unwrap_or(Value::Number(number)),
            Value::Array(values) => {
                Value::Array(values.into_iter().map(normalize_numbers).collect())
            }
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(key, value)| (key, normalize_numbers(value)))
                    .collect(),
            ),
            value => value,
        }
    }

    fn value_start(
        ch: char,
        end: usize,
        swap: State,
        stack: &mut Vec<State>,
        valid: &mut usize,
        literal: &mut usize,
        start: usize,
    ) {
        let state = match ch {
            '"' => State::String,
            't' | 'f' | 'n' => {
                *literal = start;
                State::Literal
            }
            '-' | '0'..='9' => State::Number,
            '{' => State::ObjectStart,
            '[' => State::ArrayStart,
            _ => return,
        };
        if ch != '-' {
            *valid = end;
        }
        stack.pop();
        stack.push(swap);
        stack.push(state);
    }
    fn after_value(ch: char, end: usize, stack: &mut Vec<State>, valid: &mut usize) {
        match (stack.last(), ch) {
            (Some(State::AfterValue), ',') => {
                stack.pop();
                stack.push(State::ObjectComma);
            }
            (Some(State::ArrayValue), ',') => {
                stack.pop();
                stack.push(State::ArrayComma);
            }
            (Some(State::AfterValue), '}') | (Some(State::ArrayValue), ']') => {
                *valid = end;
                stack.pop();
            }
            _ => {}
        }
    }
    fn fix(input: &str) -> String {
        use State::*;
        let mut stack = vec![Root];
        let (mut valid, mut literal, mut digits) = (0, 0, 0);
        for (start, ch) in input.char_indices() {
            let end = start + ch.len_utf8();
            match stack.last().copied().unwrap_or(Finish) {
                Root => value_start(ch, end, Finish, &mut stack, &mut valid, &mut literal, start),
                BeforeValue => value_start(
                    ch,
                    end,
                    AfterValue,
                    &mut stack,
                    &mut valid,
                    &mut literal,
                    start,
                ),
                ArrayComma => value_start(
                    ch,
                    end,
                    ArrayValue,
                    &mut stack,
                    &mut valid,
                    &mut literal,
                    start,
                ),
                ObjectStart | ObjectComma => match ch {
                    '"' => {
                        stack.pop();
                        stack.push(Key);
                    }
                    '}' if stack.last() == Some(&ObjectStart) => {
                        valid = end;
                        stack.pop();
                    }
                    _ => {}
                },
                Key if ch == '"' => {
                    stack.pop();
                    stack.push(AfterKey);
                }
                AfterKey if ch == ':' => {
                    stack.pop();
                    stack.push(BeforeValue);
                }
                AfterValue => after_value(ch, end, &mut stack, &mut valid),
                String => match ch {
                    '"' => {
                        stack.pop();
                        valid = end;
                    }
                    '\\' => stack.push(Escape),
                    _ => valid = end,
                },
                Escape => {
                    stack.pop();
                    if ch == 'u' {
                        digits = 0;
                        stack.push(Unicode);
                    } else {
                        valid = end;
                    }
                }
                Unicode if ch.is_ascii_hexdigit() => {
                    digits += 1;
                    if digits == 4 {
                        stack.pop();
                        valid = end;
                    }
                }
                ArrayStart => {
                    valid = end;
                    if ch == ']' {
                        stack.pop();
                    } else {
                        value_start(
                            ch,
                            end,
                            ArrayValue,
                            &mut stack,
                            &mut valid,
                            &mut literal,
                            start,
                        );
                    }
                }
                ArrayValue => {
                    if ch == ',' || ch == ']' {
                        after_value(ch, end, &mut stack, &mut valid);
                    } else {
                        valid = end;
                    }
                }
                Number => match ch {
                    '0'..='9' => valid = end,
                    'e' | 'E' | '-' | '.' => {}
                    ',' | '}' | ']' => {
                        stack.pop();
                        after_value(ch, end, &mut stack, &mut valid);
                    }
                    _ => {
                        stack.pop();
                    }
                },
                Literal => {
                    if ["true", "false", "null"]
                        .iter()
                        .any(|word| word.starts_with(&input[literal..end]))
                    {
                        valid = end;
                    } else {
                        stack.pop();
                        after_value(ch, end, &mut stack, &mut valid);
                    }
                }
                _ => {}
            }
        }
        let mut result = input[..valid].to_owned();
        for state in stack.into_iter().rev() {
            match state {
                String => result.push('"'),
                ObjectStart | Key | AfterKey | BeforeValue | AfterValue | ObjectComma => {
                    result.push('}')
                }
                ArrayStart | ArrayValue | ArrayComma => result.push(']'),
                Literal => {
                    let partial = &input[literal..];
                    if let Some(word) = ["true", "false", "null"]
                        .into_iter()
                        .find(|word| word.starts_with(partial))
                    {
                        result.push_str(&word[partial.len()..]);
                    }
                }
                _ => {}
            }
        }
        result
    }
}
