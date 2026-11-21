//! The display transcript: AI SDK `UIMessage`s, as stored in `conversations.messages`, served to
//! the panel and passed by the extension straight to `setMessages`.
//!
//! The JSON form is the `ai@7.0.114` reducer's, so it is defined here, beside the stream that
//! produces it. Each known part keeps the keys it does not model in `extra`, and a part this
//! server does not model at all (files, sources, data parts, approval states) is `UiPart::Other`,
//! so a row the old server wrote is rewritten by this one without loss.

use serde::de::Error as _;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use super::{MessageId, ToolCallId, ToolOwner};

/// An AI SDK `UIMessage`, the stored and served display message. `UiPart::Other` keeps
/// any part this server never writes (files, sources, rows from the old server) intact.
#[derive(Debug, Clone, PartialEq)]
pub struct UiMessage {
    pub id: MessageId,
    pub role: Role,
    pub parts: Vec<UiPart>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

/// One part of a `UIMessage`. Serializes exactly as the `ai@7.0.114` reducer stores it (the
/// golden test of chunk `runs` holds this); an absent optional field is omitted, never `null`.
/// Every known variant keeps the keys it does not model in `extra`, so a row the old server
/// wrote (`providerMetadata`, approval fields) is rewritten without loss.
#[derive(Debug, Clone, PartialEq)]
pub enum UiPart {
    Text {
        text: String,
        state: Option<PartState>,
        extra: Map<String, Value>,
    },
    Reasoning {
        /// The block id the reducer used on the wire (`r1`, `r2`, …).
        id: Option<String>,
        text: String,
        state: Option<PartState>,
        extra: Map<String, Value>,
    },
    StepStart,
    Tool(ToolPart),
    Other(Value),
}

/// A text or reasoning block's `state`: `"streaming"` while open, `"done"` once closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartState {
    Streaming,
    Done,
}

/// A tool call and, once it returned, its result. `owner: Server` serializes as
/// `type: "tool-<name>"`; `owner: Agent` as `type: "dynamic-tool"` with `toolName` and
/// `providerExecuted: true`.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolPart {
    pub id: ToolCallId,
    pub name: String,
    pub owner: ToolOwner,
    pub title: Option<String>,
    /// `None` only while the input is still arriving, or for an error on a call that never had one.
    pub input: Option<Value>,
    pub outcome: ToolOutcome,
    pub extra: Map<String, Value>,
}

/// The part's `state`: `"input-streaming"`, `"input-available"`, `"output-available"` (with
/// `output`) or `"output-error"` (with `errorText`).
#[derive(Debug, Clone, PartialEq)]
pub enum ToolOutcome {
    InputStreaming,
    InputAvailable,
    Output(Value),
    Error(String),
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }

    fn parse(role: &str) -> Option<Self> {
        match role {
            "system" => Some(Role::System),
            "user" => Some(Role::User),
            "assistant" => Some(Role::Assistant),
            _ => None,
        }
    }
}

impl PartState {
    fn as_str(self) -> &'static str {
        match self {
            PartState::Streaming => "streaming",
            PartState::Done => "done",
        }
    }

    fn parse(state: &str) -> Option<Self> {
        match state {
            "streaming" => Some(PartState::Streaming),
            "done" => Some(PartState::Done),
            _ => None,
        }
    }
}

impl Serialize for UiMessage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("id", &self.id.0)?;
        map.serialize_entry("role", self.role.as_str())?;
        if let Some(metadata) = &self.metadata {
            map.serialize_entry("metadata", metadata)?;
        }
        map.serialize_entry("parts", &self.parts)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for UiMessage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            id: String,
            role: String,
            #[serde(default)]
            parts: Vec<Value>,
            #[serde(default)]
            metadata: Option<Value>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let role = Role::parse(&wire.role)
            .ok_or_else(|| D::Error::custom(format!("unknown message role `{}`", wire.role)))?;
        Ok(UiMessage {
            id: MessageId(wire.id),
            role,
            parts: wire.parts.into_iter().map(UiPart::from_value).collect(),
            metadata: wire.metadata,
        })
    }
}

impl Serialize for UiPart {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_value().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for UiPart {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Value::deserialize(deserializer).map(UiPart::from_value)
    }
}

impl UiPart {
    /// The part as the SDK stores it: the modeled keys first, then `extra`.
    fn to_value(&self) -> Value {
        let mut object = Map::new();
        match self {
            UiPart::Text { text, state, extra } => {
                object.insert("type".into(), "text".into());
                object.insert("text".into(), text.as_str().into());
                if let Some(state) = state {
                    object.insert("state".into(), state.as_str().into());
                }
                append(&mut object, extra);
            }
            UiPart::Reasoning {
                id,
                text,
                state,
                extra,
            } => {
                object.insert("type".into(), "reasoning".into());
                if let Some(id) = id {
                    object.insert("id".into(), id.as_str().into());
                }
                object.insert("text".into(), text.as_str().into());
                if let Some(state) = state {
                    object.insert("state".into(), state.as_str().into());
                }
                append(&mut object, extra);
            }
            UiPart::StepStart => {
                object.insert("type".into(), "step-start".into());
            }
            UiPart::Tool(tool) => tool.write(&mut object),
            UiPart::Other(value) => return value.clone(),
        }
        Value::Object(object)
    }

    /// Reads one stored part. A part whose modeled keys do not have the expected types stays
    /// `Other`, untouched.
    fn from_value(value: Value) -> UiPart {
        let Value::Object(object) = &value else {
            return UiPart::Other(value);
        };
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let typed = match kind {
            "text" => text_part(object),
            "reasoning" => reasoning_part(object),
            "step-start" if object.len() == 1 => Some(UiPart::StepStart),
            "dynamic-tool" => tool_part(object, ToolOwner::Agent),
            _ if kind.starts_with("tool-") => tool_part(object, ToolOwner::Server),
            _ => None,
        };
        typed.unwrap_or(UiPart::Other(value))
    }
}

impl ToolPart {
    fn write(&self, object: &mut Map<String, Value>) {
        match self.owner {
            ToolOwner::Server => {
                object.insert("type".into(), format!("tool-{}", self.name).into());
            }
            ToolOwner::Agent => {
                object.insert("type".into(), "dynamic-tool".into());
                object.insert("toolName".into(), self.name.as_str().into());
            }
        }
        object.insert("toolCallId".into(), self.id.0.as_str().into());
        if let Some(title) = &self.title {
            object.insert("title".into(), title.as_str().into());
        }
        let state = match &self.outcome {
            ToolOutcome::InputStreaming => "input-streaming",
            ToolOutcome::InputAvailable => "input-available",
            ToolOutcome::Output(_) => "output-available",
            ToolOutcome::Error(_) => "output-error",
        };
        object.insert("state".into(), state.into());
        if let Some(input) = &self.input {
            object.insert("input".into(), input.clone());
        }
        match &self.outcome {
            ToolOutcome::Output(output) => {
                object.insert("output".into(), output.clone());
            }
            ToolOutcome::Error(error) => {
                object.insert("errorText".into(), error.as_str().into());
            }
            ToolOutcome::InputStreaming | ToolOutcome::InputAvailable => {}
        }
        // `extra` may carry an explicit `providerExecuted` read from an old row; it wins.
        if self.owner == ToolOwner::Agent && !self.extra.contains_key("providerExecuted") {
            object.insert("providerExecuted".into(), true.into());
        }
        append(object, &self.extra);
    }
}

fn append(object: &mut Map<String, Value>, extra: &Map<String, Value>) {
    for (key, value) in extra {
        object.insert(key.clone(), value.clone());
    }
}

/// The keys of `object` other than `modeled`.
fn rest(object: &Map<String, Value>, modeled: &[&str]) -> Map<String, Value> {
    object
        .iter()
        .filter(|(key, _)| !modeled.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// `Some(None)` when the key is absent, `Some(Some(state))` for a known state, `None` (not
/// readable) for anything else.
fn optional_state(object: &Map<String, Value>) -> Option<Option<PartState>> {
    match object.get("state") {
        None => Some(None),
        Some(state) => state.as_str().and_then(PartState::parse).map(Some),
    }
}

fn text_part(object: &Map<String, Value>) -> Option<UiPart> {
    Some(UiPart::Text {
        text: object.get("text")?.as_str()?.to_owned(),
        state: optional_state(object)?,
        extra: rest(object, &["type", "text", "state"]),
    })
}

fn reasoning_part(object: &Map<String, Value>) -> Option<UiPart> {
    let id = match object.get("id") {
        None => None,
        Some(id) => Some(id.as_str()?.to_owned()),
    };
    Some(UiPart::Reasoning {
        id,
        text: object.get("text")?.as_str()?.to_owned(),
        state: optional_state(object)?,
        extra: rest(object, &["type", "id", "text", "state"]),
    })
}

fn tool_part(object: &Map<String, Value>, owner: ToolOwner) -> Option<UiPart> {
    let name = match owner {
        ToolOwner::Server => object.get("type")?.as_str()?.strip_prefix("tool-")?,
        ToolOwner::Agent => object.get("toolName")?.as_str()?,
    };
    let title = match object.get("title") {
        None => None,
        Some(title) => Some(title.as_str()?.to_owned()),
    };
    let (outcome, outcome_key) = match object.get("state")?.as_str()? {
        "input-streaming" => (ToolOutcome::InputStreaming, None),
        "input-available" => (ToolOutcome::InputAvailable, None),
        "output-available" => (
            ToolOutcome::Output(object.get("output")?.clone()),
            Some("output"),
        ),
        "output-error" => (
            ToolOutcome::Error(object.get("errorText")?.as_str()?.to_owned()),
            Some("errorText"),
        ),
        _ => return None,
    };
    let mut modeled = vec!["type", "toolCallId", "title", "state", "input"];
    modeled.extend(outcome_key);
    if owner == ToolOwner::Agent {
        modeled.push("toolName");
        // Modeled by the owner only when it says what the owner implies.
        if object.get("providerExecuted") == Some(&Value::Bool(true)) {
            modeled.push("providerExecuted");
        }
    }
    Some(UiPart::Tool(ToolPart {
        id: ToolCallId(object.get("toolCallId")?.as_str()?.to_owned()),
        name: name.to_owned(),
        owner,
        title,
        input: object.get("input").cloned(),
        outcome,
        extra: rest(object, &modeled),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_sdk_message_round_trips_without_loss() -> Result<(), serde_json::Error> {
        let stored = json!({
            "id": "m1",
            "role": "assistant",
            "parts": [
                { "type": "step-start" },
                { "type": "reasoning", "id": "r1", "text": "Look it up", "state": "done",
                  "providerMetadata": { "anthropic": { "signature": "s" } } },
                { "type": "text", "text": "Checking", "state": "done" },
                { "type": "tool-run", "toolCallId": "call-1", "state": "output-available",
                  "input": { "code": "1 + 1" }, "output": "2" },
                { "type": "dynamic-tool", "toolName": "Read", "toolCallId": "t1",
                  "state": "output-error", "errorText": "Not run", "providerExecuted": true },
                { "type": "tool-run", "toolCallId": "c2", "state": "input-streaming" },
                { "type": "file", "mediaType": "image/png", "url": "data:," }
            ]
        });
        let message: UiMessage = serde_json::from_value(stored.clone())?;
        assert!(
            matches!(message.parts[1], UiPart::Reasoning { ref id, .. } if id.as_deref() == Some("r1"))
        );
        assert!(matches!(&message.parts[4], UiPart::Tool(tool) if tool.input.is_none()));
        assert!(matches!(message.parts[6], UiPart::Other(_)));
        assert_eq!(serde_json::to_value(&message)?, stored);
        Ok(())
    }

    #[test]
    fn a_state_this_server_does_not_model_stays_as_it_was() -> Result<(), serde_json::Error> {
        let part = json!({ "type": "tool-run", "toolCallId": "c", "state": "approval-requested" });
        let parsed: UiPart = serde_json::from_value(part.clone())?;
        assert_eq!(parsed, UiPart::Other(part));
        Ok(())
    }

    #[test]
    fn an_agent_tool_written_by_this_server_says_the_agent_ran_it() -> Result<(), serde_json::Error>
    {
        let part = UiPart::Tool(ToolPart {
            id: ToolCallId("t".into()),
            name: "Bash".into(),
            owner: ToolOwner::Agent,
            title: None,
            input: Some(json!({})),
            outcome: ToolOutcome::InputAvailable,
            extra: Map::new(),
        });
        assert_eq!(
            serde_json::to_value(&part)?,
            json!({ "type": "dynamic-tool", "toolName": "Bash", "toolCallId": "t",
                    "state": "input-available", "input": {}, "providerExecuted": true })
        );
        Ok(())
    }
}
