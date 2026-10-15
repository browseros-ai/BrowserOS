//! The shared tool-content codec feeds normalization and compaction. Only inline binary
//! payloads can be hoisted; remote URLs and provider references remain text placeholders.
use super::{
    ModelMessage, ToolContentItem as Item, ToolMessagePart, ToolResultOutput as Output, UserPart,
};
use serde_json::Value;

pub(crate) fn file_placeholder(media_type: &str, filename: Option<&str>) -> String {
    if media_type.starts_with("image/") {
        "[Image]".into()
    } else if let Some(name) = filename.filter(|name| !name.is_empty()) {
        format!("[File: {name}]")
    } else {
        "[File]".into()
    }
}
fn file_id(id: &Value) -> String {
    id.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| super::js_stringify(id))
}
fn content_text(item: &Item) -> String {
    match item {
        Item::Text { text, .. } => text.clone(),
        Item::File {
            media_type,
            filename,
            ..
        } => file_placeholder(media_type, filename.as_deref()),
        Item::ImageData { .. } => "[Image]".into(),
        Item::FileData { filename, .. } => file_placeholder("", filename.as_deref()),
        Item::FileUrl { url, .. } | Item::ImageUrl { url, .. } => url.clone(),
        Item::FileId { file_id: id, .. } | Item::ImageFileId { file_id: id, .. } => file_id(id),
        Item::Custom { .. } => "[Custom content]".into(),
    }
}
pub(crate) fn strip_output(output: &mut Output) {
    if let Output::Content { value } = output {
        let text = value
            .iter()
            .map(content_text)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        *output = Output::Text {
            value: if text.is_empty() {
                "[Tool content omitted]".into()
            } else {
                text
            },
        };
    }
}
fn utf16(text: &str) -> u64 {
    text.encode_utf16().count() as u64
}
pub(crate) fn estimate_output(output: &Output) -> (u64, u64) {
    match output {
        Output::Text { value } | Output::ErrorText { value } => (utf16(value), 0),
        Output::Json { value } | Output::ErrorJson { value } => {
            (utf16(&super::js_stringify(value)), 0)
        }
        Output::ExecutionDenied { reason } => (
            utf16(
                &reason
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("[Execution denied: {s}]"))
                    .unwrap_or_else(|| "[Execution denied]".into()),
            ),
            0,
        ),
        Output::Content { value } => value.iter().fold((0, 0), |(chars, images), item| {
            let (c, i) = match item {
                Item::Text { text, .. } => (utf16(text), 0),
                Item::File { .. } | Item::ImageData { .. } | Item::FileData { .. } => (0, 1),
                Item::FileUrl { url, .. } | Item::ImageUrl { url, .. } => (utf16(url), 0),
                Item::FileId { file_id: id, .. } | Item::ImageFileId { file_id: id, .. } => {
                    (utf16(&file_id(id)), 0)
                }
                Item::Custom { .. } => (0, 0),
            };
            (chars + c, images + i)
        }),
    }
}
fn media(item: &Item) -> Option<UserPart> {
    let (data, media_type, filename) = match item {
        Item::File {
            data,
            media_type,
            filename,
            ..
        } if data["type"] == "data" => (&data["data"], media_type, filename.clone()),
        Item::ImageData {
            data, media_type, ..
        } => (data, media_type, None),
        Item::FileData {
            data,
            media_type,
            filename,
            ..
        } => (data, media_type, filename.clone()),
        _ => return None,
    };
    if media_type.starts_with("image/") {
        Some(UserPart::Image {
            image: data.clone(),
            media_type: Some(media_type.clone()),
            provider_options: None,
        })
    } else {
        Some(UserPart::File {
            data: data.clone(),
            media_type: media_type.clone(),
            filename,
            provider_options: None,
        })
    }
}
pub(super) fn normalize(
    messages: Vec<ModelMessage>,
    media_in_results: bool,
    supports_images: bool,
) -> Vec<ModelMessage> {
    if media_in_results {
        return messages;
    }
    let mut normalized = Vec::new();
    for mut message in messages {
        let mut extracted = Vec::new();
        if let ModelMessage::Tool { content } = &mut message {
            for part in content {
                if let ToolMessagePart::ToolResult(result) = part {
                    if supports_images && let Output::Content { value } = &result.output {
                        extracted.extend(value.iter().filter_map(media));
                    }
                    strip_output(&mut result.output);
                }
            }
        }
        normalized.push(message);
        if !extracted.is_empty() {
            let images = extracted.iter().filter(|part| matches!(part, UserPart::Image { .. }) || matches!(part, UserPart::File { media_type, .. } if media_type.starts_with("image/"))).count();
            let label = if images == extracted.len() {
                "Attached image(s) from tool result:"
            } else if images == 0 {
                "Attached file(s) from tool result:"
            } else {
                "Attached files from tool result:"
            };
            let mut content = vec![UserPart::Text {
                text: label.into(),
                provider_options: None,
            }];
            content.extend(extracted);
            normalized.push(ModelMessage::User { content });
        }
    }
    normalized
}
