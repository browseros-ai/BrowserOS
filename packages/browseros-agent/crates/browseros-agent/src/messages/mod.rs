//! Model-facing transcript: our own AI-SDK-v7-shaped `ModelMessage` (rig's `Message` appears only
//! inside the rig adapter), the UI → model conversion (port of `convertToModelMessages`), and
//! history hygiene. Compaction's prune rules are defined over exactly these part kinds, which is
//! why the shape is ours. Stored/display history is never changed by these request-copy helpers.

mod convert;
mod media;
mod stringify;

pub(crate) use media::{estimate_output, file_placeholder, strip_output};
pub(crate) use stringify::js_stringify;

use crate::tools::ToolCatalog;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use ui_message_stream::UiMessage;

/// Field-for-field AI SDK v7 `ModelMessage` (serde matches its JSON for golden tests).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "role",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum ModelMessage {
    System {
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },
    User {
        content: Vec<UserPart>,
    },
    Assistant {
        content: Vec<AssistantPart>,
    },
    Tool {
        content: Vec<ToolMessagePart>,
    },
}

/// SDK v7 accepts legacy strings and tagged data/url/reference objects. Keeping the tagged
/// object intact is essential: provider references must never be treated as base64 bytes.
pub type MediaData = Value;

/// User input after UI conversion; files retain provider references and tagged URL data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum UserPart {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },
    Image {
        image: MediaData,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },
    File {
        data: MediaData,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },
}

/// Assistant text, provider-owned content, and tool calls share one ordered step block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum AssistantPart {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },
    Reasoning {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },
    ReasoningFile {
        data: MediaData,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },
    Custom {
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },
    File {
        data: MediaData,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        input: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
    },
    ToolResult(ToolResultPart),
    ToolApprovalRequest {
        approval_id: String,
        tool_call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_automatic: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(
            default,
            deserialize_with = "present",
            skip_serializing_if = "Option::is_none"
        )]
        input_schema_input: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
}

/// Client-side results and approval responses follow their originating assistant step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ToolMessagePart {
    ToolResult(ToolResultPart),
    ToolApprovalResponse {
        approval_id: String,
        approved: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultPart {
    pub tool_call_id: String,
    pub tool_name: String,
    pub output: ToolResultOutput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<Value>,
}

/// The SDK's `ToolResultOutput` (`toModelOutput` per tool kind decides which variant).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ToolResultOutput {
    Text {
        value: String,
    },
    Json {
        value: Value,
    },
    ErrorText {
        value: String,
    },
    ErrorJson {
        value: Value,
    },
    ExecutionDenied {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Content {
        value: Vec<ToolContentItem>,
    },
}

/// SDK v7 content items, including deprecated shapes persisted history can carry. Extra
/// provider fields survive a no-op normalization pass even when this port does not inspect them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ToolContentItem {
    Text {
        text: String,
        #[serde(flatten)]
        extra: serde_json::Map<String, Value>,
    },
    File {
        data: MediaData,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(flatten)]
        extra: serde_json::Map<String, Value>,
    },
    ImageData {
        data: MediaData,
        media_type: String,
        #[serde(flatten)]
        extra: serde_json::Map<String, Value>,
    },
    FileData {
        data: MediaData,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(flatten)]
        extra: serde_json::Map<String, Value>,
    },
    FileUrl {
        url: String,
        #[serde(flatten)]
        extra: serde_json::Map<String, Value>,
    },
    ImageUrl {
        url: String,
        #[serde(flatten)]
        extra: serde_json::Map<String, Value>,
    },
    FileId {
        file_id: Value,
        #[serde(flatten)]
        extra: serde_json::Map<String, Value>,
    },
    ImageFileId {
        file_id: Value,
        #[serde(flatten)]
        extra: serde_json::Map<String, Value>,
    },
    Custom {
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<Value>,
        #[serde(flatten)]
        extra: serde_json::Map<String, Value>,
    },
}

// Missing and explicit null differ for an approval's original schema input.
fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ConvertError(pub String);

/// Port of `convertToModelMessages` (step-start splitting, `toModelOutput` per tool kind,
/// `ignoreIncompleteToolCalls`).
pub fn ui_to_model(
    history: &[UiMessage],
    catalog: &ToolCatalog,
) -> Result<Vec<ModelMessage>, ConvertError> {
    convert::convert(history, catalog)
}

/// `filterValidMessages`: drops empty messages and all-blank text using JS whitespace rules.
pub fn filter_valid(messages: Vec<UiMessage>) -> Vec<UiMessage> {
    messages.into_iter().filter(has_content).collect()
}

/// Removes only `reasoning`, matching TS (reasoning-file parts survive this hygiene pass).
pub fn strip_reasoning(messages: Vec<UiMessage>) -> Vec<UiMessage> {
    filter_valid(
        messages
            .into_iter()
            .map(|mut message| {
                message
                    .parts
                    .retain(|part| !matches!(part, ui_message_stream::UiPart::Reasoning(_)));
                message
            })
            .collect(),
    )
}

/// `sanitizeMessagesForToolset`, applied to the per-turn prompt copy only (never to stored
/// history). Dynamic MCP parts survive even if their source disconnected, just as in TS.
pub fn sanitize_for_toolset(
    messages: Vec<UiMessage>,
    tool_names: &BTreeSet<String>,
) -> Vec<UiMessage> {
    filter_valid(
        messages
            .into_iter()
            .map(|mut message| {
                message.parts.retain(|part| match part {
                    ui_message_stream::UiPart::Tool(part) => match &part.kind {
                        ui_message_stream::ToolPartKind::Static { name } => {
                            tool_names.contains(name)
                        }
                        ui_message_stream::ToolPartKind::Dynamic { .. } => true,
                    },
                    _ => true,
                });
                message
            })
            .collect(),
    )
}

/// Hoists tool-result media into a following user message when the provider cannot take media
/// in tool results. The capability flag only controls extracted media, not existing user files.
pub fn normalize_for_model(
    messages: Vec<ModelMessage>,
    media_in_tool_results: bool,
    supports_images: bool,
) -> Vec<ModelMessage> {
    media::normalize(messages, media_in_tool_results, supports_images)
}

fn has_content(message: &UiMessage) -> bool {
    message.parts.iter().any(|part| match part {
        ui_message_stream::UiPart::Text(part) => !js_trim(&part.text).is_empty(),
        _ => true,
    })
}

/// ECMAScript trim differs from Rust at BOM and NEL; history validity and prompt cleanup
/// must agree with the TS server even for pasted Unicode whitespace.
pub(crate) fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'))
}
