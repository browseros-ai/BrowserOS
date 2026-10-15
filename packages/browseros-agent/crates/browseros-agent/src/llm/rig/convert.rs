//! Our AI-SDK-shaped transcript (`messages::ModelMessage`) → rig `Message`s, plus the round trip
//! of provider-native items through it.
//!
//! **Native replay.** A provider item that must go back verbatim on the next step (Anthropic
//! thinking signatures, OpenAI encrypted reasoning, Gemini thought signatures) leaves the adapter
//! as `provider_metadata = {"rig": {"origin": …, "block": …}}` on the `*End`/`ToolCall` event.
//! The loop stores it as that part's `provider_options` (the AI SDK providerMetadata →
//! providerOptions convention), and this module restores the rig block when the part still says
//! what the block says. rig itself then replays the native item only when the block's origin is
//! the model being called, so a provider switch mid-conversation degrades to canonical replay.
//! Reasoning that carries no native item is dropped, as the AI SDK providers drop unsigned
//! reasoning; across turns the loop strips reasoning anyway.

use crate::llm::{ModelError, ToolDefinition};
use crate::messages::{
    AssistantPart, ModelMessage, ToolContentItem, ToolMessagePart, ToolResultOutput,
    ToolResultPart, UserPart,
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rig_core::completion::ToolDefinition as RigToolDefinition;
use rig_core::message::{
    AssistantContent, AssistantMessage, Audio, AudioMediaType, CallId, Document, DocumentMediaType,
    DocumentSourceKind, Image, ImageMediaType, Message, MimeType, Origin, ToolCall, ToolFunction,
    ToolName, ToolResult, ToolResultContent, UserContent, Video, VideoMediaType,
};
use serde::Deserialize;
use serde_json::{Value, json};

const META_KEY: &str = "rig";

/// The `provider_metadata` for a finished block, or `None` when it carries no provider item (the
/// canonical fields alone replay it).
pub(crate) fn block_metadata(origin: Option<&Value>, block: &AssistantContent) -> Option<Value> {
    block.native_item()?;
    let block = serde_json::to_value(block).ok()?;
    Some(json!({ META_KEY: { "origin": origin?, "block": block } }))
}

fn stored_block(options: Option<&Value>) -> Option<(Origin, AssistantContent)> {
    let meta = options?.get(META_KEY)?;
    let origin = Origin::deserialize(meta.get("origin")?).ok()?;
    let block = AssistantContent::deserialize(meta.get("block")?).ok()?;
    Some((origin, block))
}

fn tool_name(name: &str) -> Result<ToolName, ModelError> {
    ToolName::new(name)
        .map_err(|_| ModelError::InvalidPrompt("tool name must not be empty".to_owned()))
}

pub(crate) fn tool_definitions(
    tools: &[ToolDefinition],
) -> Result<Vec<RigToolDefinition>, ModelError> {
    tools
        .iter()
        .map(|tool| {
            Ok(RigToolDefinition::new(
                tool_name(&tool.name)?,
                tool.description.clone(),
                tool.input_schema.clone(),
            ))
        })
        .collect()
}

pub(crate) fn to_rig_messages(messages: &[ModelMessage]) -> Result<Vec<Message>, ModelError> {
    let mut out = Vec::with_capacity(messages.len());
    for message in messages {
        let converted = match message {
            ModelMessage::System { content } => Some(Message::system(content.clone())),
            ModelMessage::User { content } => {
                let content: Vec<UserContent> = content.iter().map(user_part).collect();
                (!content.is_empty()).then_some(Message::User { content })
            }
            ModelMessage::Assistant { content } => assistant(content)?,
            ModelMessage::Tool { content } => {
                let content = content
                    .iter()
                    .filter_map(|part| match part {
                        ToolMessagePart::ToolResult(result) => Some(tool_result(result)),
                        // Approvals are BrowserOS-internal; providers never see them.
                        ToolMessagePart::ToolApprovalResponse { .. } => None,
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                (!content.is_empty()).then_some(Message::User { content })
            }
        };
        out.extend(converted);
    }
    Ok(out)
}

fn assistant(parts: &[AssistantPart]) -> Result<Option<Message>, ModelError> {
    let mut origin: Option<Origin> = None;
    let mut content = Vec::new();
    for part in parts {
        let (canonical, stored) = match part {
            AssistantPart::Text {
                text,
                provider_options,
            } => (
                Some(AssistantContent::text(text.clone())),
                stored_block(provider_options.as_ref()).filter(
                    |(_, block)| matches!(block, AssistantContent::Text(stored) if stored.text == *text),
                ),
            ),
            AssistantPart::Reasoning {
                provider_options, ..
            } => (
                None,
                stored_block(provider_options.as_ref())
                    .filter(|(_, block)| matches!(block, AssistantContent::Reasoning(_))),
            ),
            AssistantPart::ToolCall {
                tool_call_id,
                tool_name: name,
                input,
                provider_executed,
                provider_options,
            } => {
                // Provider-executed (hosted) tools belong to the provider's own transcript.
                if *provider_executed == Some(true) {
                    continue;
                }
                let call = ToolCall::from_wire(tool_call_id.clone(), ToolFunction::new(tool_name(name)?, input.clone()));
                let stored = stored_block(provider_options.as_ref()).filter(|(_, block)| {
                    matches!(block, AssistantContent::ToolCall(stored)
                        if stored.id.wire() == tool_call_id.as_str()
                            && stored.function.name.as_str() == name
                            && stored.function.arguments_value() == call.function.arguments_value())
                });
                (Some(AssistantContent::ToolCall(call)), stored)
            }
            AssistantPart::ReasoningFile { .. }
            | AssistantPart::File { .. }
            | AssistantPart::ToolResult(_)
            | AssistantPart::ToolApprovalRequest { .. } => continue,
        };
        let block = match stored {
            Some((block_origin, block))
                if origin.as_ref().is_none_or(|origin| *origin == block_origin) =>
            {
                origin.get_or_insert(block_origin);
                block
            }
            _ => match canonical {
                Some(canonical) => canonical,
                None => continue,
            },
        };
        content.push(block);
    }
    if content.is_empty() {
        return Ok(None);
    }
    Ok(Some(Message::Assistant(
        AssistantMessage::new(content).with_origin(origin),
    )))
}

/// AI SDK `DataContent | URL` as a rig source: a `data:` URL, an http(s) URL, or bare base64.
/// Returns the media type a data URL names, for parts that left theirs out.
fn media_source(data: &str) -> (DocumentSourceKind, Option<String>) {
    if let Some(rest) = data.strip_prefix("data:")
        && let Some((meta, payload)) = rest.split_once(',')
    {
        let mime = meta
            .split(';')
            .next()
            .filter(|mime| !mime.is_empty())
            .map(str::to_owned);
        let base64 = if meta.ends_with(";base64") {
            payload.to_owned()
        } else {
            STANDARD.encode(payload.as_bytes())
        };
        return (DocumentSourceKind::Base64(base64), mime);
    }
    if data.starts_with("http://") || data.starts_with("https://") {
        return (DocumentSourceKind::Url(data.to_owned()), None);
    }
    (DocumentSourceKind::Base64(data.to_owned()), None)
}

fn media_content(data: &str, media_type: Option<&str>) -> UserContent {
    let (source, url_mime) = media_source(data);
    let mime = media_type
        .map(str::to_owned)
        .or(url_mime)
        .unwrap_or_default();
    if mime.starts_with("image/") {
        UserContent::Image(Image {
            data: source,
            media_type: ImageMediaType::from_mime_type(&mime),
            ..Image::default()
        })
    } else if mime.starts_with("audio/") {
        UserContent::Audio(Audio {
            data: source,
            media_type: AudioMediaType::from_mime_type(&mime),
        })
    } else if mime.starts_with("video/") {
        UserContent::Video(Video {
            data: source,
            media_type: VideoMediaType::from_mime_type(&mime),
            additional_params: None,
        })
    } else {
        UserContent::Document(Document {
            data: source,
            media_type: DocumentMediaType::from_mime_type(&mime),
            additional_params: None,
        })
    }
}

fn user_part(part: &UserPart) -> UserContent {
    match part {
        UserPart::Text { text, .. } => UserContent::text(text.clone()),
        UserPart::Image {
            image, media_type, ..
        } => {
            let (source, url_mime) = media_source(image);
            let mime = media_type.clone().or(url_mime);
            UserContent::Image(Image {
                data: source,
                media_type: mime.as_deref().and_then(ImageMediaType::from_mime_type),
                ..Image::default()
            })
        }
        UserPart::File {
            data, media_type, ..
        } => media_content(data, Some(media_type)),
    }
}

fn json_text(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// `ToolResultOutput` → rig tool-result content, as the AI SDK providers render each output kind
/// (JSON is sent as its JSON text; `error-*` marks the result as an error).
fn tool_result(part: &ToolResultPart) -> Result<UserContent, ModelError> {
    let text = ToolResultContent::text;
    let (mut content, is_error) = match &part.output {
        ToolResultOutput::Text { value } => (vec![text(value.clone())], false),
        ToolResultOutput::Json { value } => (vec![text(json_text(value))], false),
        ToolResultOutput::ErrorText { value } => (vec![text(value.clone())], true),
        ToolResultOutput::ErrorJson { value } => (vec![text(json_text(value))], true),
        ToolResultOutput::ExecutionDenied { reason } => (
            vec![text(
                reason
                    .clone()
                    .unwrap_or_else(|| "Tool execution denied.".to_owned()),
            )],
            false,
        ),
        ToolResultOutput::Content { value } => (
            value
                .iter()
                .map(|item| match item {
                    ToolContentItem::Text { text: value } => text(value.clone()),
                    ToolContentItem::Media { data, media_type }
                        if media_type.starts_with("image/") =>
                    {
                        let (source, _) = media_source(data);
                        ToolResultContent::Image(Image {
                            data: source,
                            media_type: ImageMediaType::from_mime_type(media_type),
                            ..Image::default()
                        })
                    }
                    // rig tool results hold text and images only; other media goes as its JSON,
                    // as the compatible providers send unsupported items.
                    other => text(json_text(&serde_json::to_value(other).unwrap_or_default())),
                })
                .collect(),
            false,
        ),
    };
    // Every wire rejects a tool result with no content blocks.
    if content.is_empty() {
        content.push(text(String::new()));
    }
    Ok(UserContent::ToolResult(ToolResult {
        call: CallId::from_wire(part.tool_call_id.clone()),
        name: tool_name(&part.tool_name)?,
        content,
        is_error,
    }))
}
