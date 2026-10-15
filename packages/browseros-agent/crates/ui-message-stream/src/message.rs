//! The persisted AI SDK v7 `UIMessage`: what `conversations.messages` stores and what `/state` and
//! `/conversations/{id}` return.
//!
//! Rows written by the TS server must survive a Rust read-modify-write unchanged, so decoding is
//! lossless: tool parts keep unmodelled keys in `extra`, and part types this crate does not model
//! (`source-url`, `data-*`, `custom`, …) round-trip verbatim through [`UiPart::Other`].

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use serde_json::{Map, Value};

/// Matches `MAX_SNIPPET_CHARS` in `apps/server/src/lib/conversations/conversation-store.ts`.
const LAST_USER_MESSAGE_UTF16_LIMIT: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiMessage {
    pub id: String,
    pub role: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    pub parts: Vec<UiPart>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// One element of `UIMessage.parts`. (De)serialized by hand: `"tool-<name>"` and `"dynamic-tool"`
/// both decode to [`UiPart::Tool`], and unknown part types round-trip through [`UiPart::Other`].
#[derive(Debug, Clone, PartialEq)]
pub enum UiPart {
    Text(TextPart),
    Reasoning(ReasoningPart),
    StepStart,
    /// Boxed: a tool part is several times larger than every other variant.
    Tool(Box<ToolPart>),
    File(FilePart),
    Other(Map<String, Value>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PartState {
    Streaming,
    Done,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextPart {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<PartState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<PartState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePart {
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reference: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

/// A static (`tool-<name>`) or dynamic (`dynamic-tool`, every MCP tool) invocation part.
///
/// `input`/`output` distinguish "absent" (`None`) from an explicit JSON `null` (`Some(Null)`):
/// a tool that returned nothing persists `"output": null`, and that must survive a rewrite.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolPart {
    pub kind: ToolPartKind,
    pub tool_call_id: String,
    pub state: ToolState,
    pub input: Option<Value>,
    pub output: Option<Value>,
    pub error_text: Option<String>,
    pub title: Option<String>,
    pub tool_metadata: Option<Value>,
    pub provider_executed: Option<bool>,
    /// Wire key `callProviderMetadata` (the SDK's name on tool parts).
    pub call_provider_metadata: Option<Value>,
    pub result_provider_metadata: Option<Value>,
    pub preliminary: Option<bool>,
    pub approval: Option<Value>,
    /// Keys this type does not model (e.g. the deprecated `rawInput`), preserved verbatim.
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPartKind {
    Static { name: String },
    Dynamic { tool_name: String },
}

/// All seven SDK v7 invocation states; BrowserOS emits the first two and the output states,
/// but stored rows may carry any of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolState {
    InputStreaming,
    InputAvailable,
    ApprovalRequested,
    ApprovalResponded,
    OutputAvailable,
    OutputError,
    OutputDenied,
}

impl UiMessage {
    /// A user message: file parts first, then the text part, the order the AI SDK client sends.
    pub fn user(id: String, text: Option<String>, files: Vec<FilePart>) -> Self {
        let mut parts: Vec<UiPart> = files.into_iter().map(UiPart::File).collect();
        if let Some(text) = text {
            parts.push(UiPart::Text(TextPart {
                text,
                state: None,
                provider_metadata: None,
            }));
        }
        Self {
            id,
            role: Role::User,
            metadata: None,
            parts,
        }
    }

    /// `hasMessageContent`: parts non-empty and (any non-text part or some non-blank text).
    /// Messages failing this are dropped before they reach a provider.
    pub fn has_content(&self) -> bool {
        if self.parts.is_empty() {
            return false;
        }
        self.parts.iter().any(|part| match part {
            UiPart::Text(text) => !text.text.trim().is_empty(),
            _ => true,
        })
    }

    /// `extractLastUserText`: the newest user message with non-blank text, its parts joined by
    /// `' '` (non-text parts contribute `""`), trimmed, cut to 200 UTF-16 units. A surrogate pair
    /// straddling the cut is dropped whole, where JS would keep a lone high surrogate.
    pub fn last_user_message_preview(messages: &[UiMessage]) -> Option<String> {
        messages
            .iter()
            .rev()
            .filter(|message| message.role == Role::User)
            .find_map(|message| {
                let joined = message
                    .parts
                    .iter()
                    .map(|part| match part {
                        UiPart::Text(text) => text.text.as_str(),
                        _ => "",
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                let trimmed = joined.trim();
                (!trimmed.is_empty())
                    .then(|| truncate_utf16(trimmed, LAST_USER_MESSAGE_UTF16_LIMIT))
            })
    }
}

fn truncate_utf16(text: &str, limit: usize) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|ch| {
            units += ch.len_utf16();
            units <= limit
        })
        .collect()
}

/// Prepends `"type"` to a struct's own keys without an intermediate `Map` (whose key order would
/// depend on serde_json's `preserve_order` feature).
#[derive(Serialize)]
struct Tagged<'a, T> {
    #[serde(rename = "type")]
    kind: &'a str,
    #[serde(flatten)]
    part: &'a T,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolPartWire {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_name: Option<String>,
    tool_call_id: String,
    state: ToolState,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    input: Option<Value>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_executed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    call_provider_metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result_provider_metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preliminary: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    approval: Option<Value>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// A present key (even `null`) is `Some`; only a missing key (`#[serde(default)]`) is `None`.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

impl From<&ToolPart> for ToolPartWire {
    fn from(part: &ToolPart) -> Self {
        let (kind, tool_name) = match &part.kind {
            ToolPartKind::Static { name } => (format!("tool-{name}"), None),
            ToolPartKind::Dynamic { tool_name } => {
                ("dynamic-tool".to_owned(), Some(tool_name.clone()))
            }
        };
        Self {
            kind,
            tool_name,
            tool_call_id: part.tool_call_id.clone(),
            state: part.state,
            input: part.input.clone(),
            output: part.output.clone(),
            error_text: part.error_text.clone(),
            title: part.title.clone(),
            tool_metadata: part.tool_metadata.clone(),
            provider_executed: part.provider_executed,
            call_provider_metadata: part.call_provider_metadata.clone(),
            result_provider_metadata: part.result_provider_metadata.clone(),
            preliminary: part.preliminary,
            approval: part.approval.clone(),
            extra: part.extra.clone(),
        }
    }
}

impl ToolPartWire {
    fn into_part(self) -> Result<ToolPart, String> {
        let kind = if self.kind == "dynamic-tool" {
            ToolPartKind::Dynamic {
                tool_name: self
                    .tool_name
                    .ok_or_else(|| "dynamic-tool part without toolName".to_owned())?,
            }
        } else {
            let name = self
                .kind
                .strip_prefix("tool-")
                .ok_or_else(|| format!("not a tool part: {}", self.kind))?;
            ToolPartKind::Static {
                name: name.to_owned(),
            }
        };
        Ok(ToolPart {
            kind,
            tool_call_id: self.tool_call_id,
            state: self.state,
            input: self.input,
            output: self.output,
            error_text: self.error_text,
            title: self.title,
            tool_metadata: self.tool_metadata,
            provider_executed: self.provider_executed,
            call_provider_metadata: self.call_provider_metadata,
            result_provider_metadata: self.result_provider_metadata,
            preliminary: self.preliminary,
            approval: self.approval,
            extra: self.extra,
        })
    }
}

impl Serialize for UiPart {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            UiPart::Text(part) => Tagged { kind: "text", part }.serialize(serializer),
            UiPart::Reasoning(part) => Tagged {
                kind: "reasoning",
                part,
            }
            .serialize(serializer),
            UiPart::StepStart => Tagged {
                kind: "step-start",
                part: &Map::new(),
            }
            .serialize(serializer),
            UiPart::Tool(part) => ToolPartWire::from(part.as_ref()).serialize(serializer),
            UiPart::File(part) => Tagged { kind: "file", part }.serialize(serializer),
            UiPart::Other(map) => map.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for UiPart {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = Map::<String, Value>::deserialize(deserializer)?;
        let kind = map
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        fn decode<T: serde::de::DeserializeOwned, E: serde::de::Error>(
            map: Map<String, Value>,
        ) -> Result<T, E> {
            serde_json::from_value(Value::Object(map)).map_err(E::custom)
        }
        Ok(match kind.as_str() {
            "text" => UiPart::Text(decode(map)?),
            "reasoning" => UiPart::Reasoning(decode(map)?),
            "step-start" => UiPart::StepStart,
            "file" => UiPart::File(decode(map)?),
            tool if tool == "dynamic-tool" || tool.starts_with("tool-") => UiPart::Tool(Box::new(
                decode::<ToolPartWire, D::Error>(map)?
                    .into_part()
                    .map_err(D::Error::custom)?,
            )),
            _ => UiPart::Other(map),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parts_round_trip_losslessly() -> Result<(), serde_json::Error> {
        let stored = json!({
            "id": "m1",
            "role": "assistant",
            "parts": [
                {"type": "step-start"},
                {"type": "reasoning", "text": "hmm", "state": "done"},
                {"type": "text", "text": "hi", "state": "done", "providerMetadata": {"a": {"b": 1}}},
                {"type": "dynamic-tool", "toolName": "snapshot", "toolCallId": "c1",
                 "state": "output-available", "input": {}, "output": null,
                 "title": "Snapshot", "toolMetadata": {"x": 1}, "rawInput": {"legacy": true}},
                {"type": "tool-suggest_schedule", "toolCallId": "c2", "state": "output-error",
                 "input": {"q": 1}, "errorText": "boom"},
                {"type": "file", "mediaType": "image/png", "url": "data:image/png;base64,AA=="},
                {"type": "source-url", "sourceId": "s", "url": "https://example.com"},
                {"type": "data-notice", "data": {"n": 1}}
            ]
        });
        let message: UiMessage = serde_json::from_value(stored.clone())?;
        assert!(matches!(
            &message.parts[3],
            UiPart::Tool(part)
                if matches!(part.kind, ToolPartKind::Dynamic { .. })
                    && part.output == Some(Value::Null)
        ));
        assert!(matches!(&message.parts[6], UiPart::Other(_)));
        assert_eq!(serde_json::to_value(&message)?, stored);
        Ok(())
    }

    #[test]
    fn content_and_preview_follow_ts() {
        let blank = UiMessage::user("u0".into(), Some("   ".into()), Vec::new());
        assert!(!blank.has_content());
        let long = "é".repeat(250);
        let messages = vec![UiMessage::user("u1".into(), Some(long), Vec::new()), blank];
        let preview = UiMessage::last_user_message_preview(&messages);
        assert_eq!(preview.map(|text| text.chars().count()), Some(200));
    }
}
