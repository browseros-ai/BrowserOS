//! Model-facing transcript: our own AI-SDK-v7-shaped `ModelMessage` (rig's `Message` appears only
//! inside the rig adapter), the UI → model conversion (port of `convertToModelMessages`), and
//! history hygiene. Compaction's prune rules are defined over exactly these part kinds, which is
//! why the shape is ours. Implemented by P6; S0 ships types plus identity/err stubs.

use crate::tools::ToolCatalog;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use ui_message_stream::UiMessage;

/// Field-for-field AI SDK v7 `ModelMessage` (serde matches its JSON for golden tests).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum ModelMessage {
    System { content: String },
    User { content: Vec<UserPart> },
    Assistant { content: Vec<AssistantPart> },
    Tool { content: Vec<ToolMessagePart> },
}

/// Media payload: base64 data or a URL string, as the SDK's `DataContent | URL`.
pub type MediaData = String;

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
    },
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ToolContentItem {
    Text { text: String },
    Media { data: MediaData, media_type: String },
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
    let _ = (history, catalog);
    Err(ConvertError(
        "UI to model message conversion is not implemented yet".to_owned(),
    ))
}

/// `filterValidMessages`: drops messages failing `UiMessage::has_content`.
pub fn filter_valid(messages: Vec<UiMessage>) -> Vec<UiMessage> {
    messages
        .into_iter()
        .filter(UiMessage::has_content)
        .collect()
}

/// Removes reasoning parts from the model-facing copy. S0 stub: identity.
pub fn strip_reasoning(messages: Vec<UiMessage>) -> Vec<UiMessage> {
    messages
}

/// `sanitizeMessagesForToolset`, applied to the per-turn prompt copy only (never to stored
/// history). S0 stub: identity.
pub fn sanitize_for_toolset(
    messages: Vec<UiMessage>,
    tool_names: &BTreeSet<String>,
) -> Vec<UiMessage> {
    let _ = tool_names;
    messages
}

/// Hoists tool-result media into a following user message when the provider cannot take media
/// in tool results, and drops images for text-only models. S0 stub: identity.
pub fn normalize_for_model(
    messages: Vec<ModelMessage>,
    media_in_tool_results: bool,
    supports_images: bool,
) -> Vec<ModelMessage> {
    let _ = (media_in_tool_results, supports_images);
    messages
}
