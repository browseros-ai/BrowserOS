//! Presents the SDK's returned image bytes as MCP images. The text value and
//! structured value share image markers, while run metadata and console output
//! retain the SDK's form. The host API supplies fetched blobs; this adapter owns
//! MIME detection, image blocks and the matching text/structured markers.

use base64::{Engine, engine::general_purpose::STANDARD};
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;
use std::collections::BTreeMap;

use super::AgentReply;

const BLOB_NOTE: &str = "Returned output was too large to inline. Blob bytes are not available in this reply; for a picture, use page.screenshot({ path }) or a smaller clip.";

pub(super) fn shape(reply: CallToolResult) -> AgentReply {
    shape_with_blobs(reply, &BTreeMap::new())
}

/// Reuses the inline-image presentation after the host fetched returned blobs.
/// The wire already shaped inline bytes, so their content indices stay fixed.
pub(super) fn shape_with_blobs(
    mut reply: CallToolResult,
    blobs: &BTreeMap<String, Vec<u8>>,
) -> AgentReply {
    let Some(value) = reply
        .structured_content
        .as_ref()
        .and_then(|structured| structured.get("value"))
    else {
        return reply;
    };
    let Some(ContentBlock::Text(text)) = reply.content.first() else {
        return reply;
    };
    let original = value.to_string();
    // The SDK puts the JSON value first, then console lines and operation notes.
    // Match that boundary before replacing anything, so neither arbitrary text
    // nor a console message can be mistaken for the returned value.
    let Some(suffix) = text
        .text
        .strip_prefix(&original)
        .filter(|suffix| suffix.is_empty() || suffix.starts_with('\n'))
    else {
        return reply;
    };
    let value = value.clone();
    let suffix = suffix.to_owned();
    if !blobs.is_empty() {
        reply
            .content
            .retain(|block| !matches!(block, ContentBlock::Text(text) if text.text == BLOB_NOTE));
    }
    let presentation = Presentation::new(value, reply.content.len(), blobs);
    if presentation.images.is_empty() && !presentation.has_blobs {
        return reply;
    }
    let text = format!("{}{suffix}", presentation.value);
    if let Some(ContentBlock::Text(first)) = reply.content.first_mut() {
        first.text = text;
    }
    if let Some(value) = reply
        .structured_content
        .as_mut()
        .and_then(|structured| structured.get_mut("value"))
    {
        *value = presentation.value;
    }
    reply.content.extend(presentation.images);
    if presentation.has_blobs {
        reply.content.push(ContentBlock::text(BLOB_NOTE));
    }
    reply
}

/// Only canonical returned blob markers may trigger a host fetch. Console text
/// and ordinary objects with extra fields remain script data.
pub(super) fn blob_requests(reply: &AgentReply) -> Option<BTreeMap<String, u64>> {
    let mut blobs = BTreeMap::new();
    let mut pending: Vec<_> = reply
        .structured_content
        .as_ref()
        .and_then(|structured| structured.get("value"))
        .into_iter()
        .collect();
    while let Some(node) = pending.pop() {
        if is_blob(node) {
            if let (Some(id), Some(size)) = (node["id"].as_str(), node["size"].as_u64())
                && blobs
                    .insert(id.to_owned(), size)
                    .is_some_and(|previous| previous != size)
            {
                return None;
            }
        } else {
            match node {
                Value::Array(items) => pending.extend(items),
                Value::Object(fields) => pending.extend(fields.values()),
                _ => {}
            }
        }
    }
    Some(blobs)
}

/// One presentation of a returned value. Image indices refer to the final MCP
/// content array, so text and structured consumers find the same image block.
struct Presentation {
    value: Value,
    images: Vec<ContentBlock>,
    has_blobs: bool,
}

impl Presentation {
    fn new(mut value: Value, first_image_index: usize, blobs: &BTreeMap<String, Vec<u8>>) -> Self {
        let mut images = Vec::new();
        // Repeated references share one content block. The fetch budget bounds
        // unique bytes; duplicating a large blob here would bypass that bound.
        let mut blob_images = BTreeMap::new();
        let mut has_blobs = false;
        let mut pending = vec![&mut value];
        // Like the SDK's result fitter, walk iteratively to avoid adding a
        // recursive host stack requirement to values accepted at the wire.
        while let Some(node) = pending.pop() {
            if is_blob(node)
                && let Some(id) = node["id"].as_str()
                && let Some(bytes) = blobs.get(id)
                && let Some(mime_type) = image_type(bytes)
            {
                let index = *blob_images.entry(id.to_owned()).or_insert_with(|| {
                    let index = first_image_index + images.len();
                    images.push(ContentBlock::image(STANDARD.encode(bytes), mime_type));
                    index
                });
                *node = Value::String(format!("[image at content[{index}] ({mime_type})]"));
                continue;
            }
            if let Some(base64) = bytes_brand(node)
                && let Ok(bytes) = STANDARD.decode(base64)
                && let Some(mime_type) = image_type(&bytes)
            {
                let index = first_image_index + images.len();
                images.push(ContentBlock::image(base64.to_owned(), mime_type));
                *node = Value::String(format!("[image at content[{index}] ({mime_type})]"));
                continue;
            }
            has_blobs |= is_blob(node);
            match node {
                Value::Array(items) => pending.extend(items.iter_mut().rev()),
                Value::Object(fields) => pending.extend(fields.values_mut().rev()),
                _ => {}
            }
        }
        Self {
            value,
            images,
            has_blobs,
        }
    }
}

fn bytes_brand(value: &Value) -> Option<&str> {
    let object = value.as_object()?;
    if object.len() != 2 || object.get("$neo")?.as_str()? != "bytes" {
        return None;
    }
    object.get("base64")?.as_str()
}

fn is_blob(value: &Value) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == 3
            && object.get("$neo").and_then(Value::as_str) == Some("blob")
            && object.get("id").and_then(Value::as_str).is_some_and(|id| {
                // SDK blob ids use a canonical u32 spelling. Other strings
                // are ordinary script data, even with the same field names.
                id.strip_prefix('b')
                    .and_then(|index| index.parse::<u32>().ok())
                    .is_some_and(|index| id == format!("b{index}"))
            })
            && object.get("size").and_then(Value::as_u64).is_some()
    })
}

fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::json;

    use super::*;

    #[test]
    fn fetched_images_reuse_inline_indices_and_non_images_keep_the_blob_note()
    -> Result<(), Box<dyn Error>> {
        let png = b"\x89PNG\r\n\x1a\n".to_vec();
        let blob = json!({"$neo":"blob", "id":"b0", "size":png.len()});
        let original = shape(reply(
            json!([bytes(&png), blob.clone(), blob]),
            "\nconsole:\nkept",
        ));
        assert_eq!(
            blob_requests(&original),
            Some(BTreeMap::from([("b0".to_owned(), 8)]))
        );
        let shaped = serde_json::to_value(shape_with_blobs(
            original,
            &BTreeMap::from([("b0".to_owned(), png)]),
        ))?;
        assert_eq!(shaped["content"][1]["type"], "image");
        assert_eq!(shaped["content"][2]["type"], "image");
        assert_eq!(
            shaped["structuredContent"]["value"][1],
            "[image at content[2] (image/png)]"
        );
        assert_eq!(
            shaped["structuredContent"]["value"][2],
            shaped["structuredContent"]["value"][1]
        );
        assert_eq!(shaped["content"].as_array().map(Vec::len), Some(3));
        assert!(
            shaped["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.ends_with("\nconsole:\nkept"))
        );

        let original = shape(reply(json!({"$neo":"blob", "id":"b0", "size":8}), ""));
        let expected = serde_json::to_vec(&original)?;
        let shaped = shape_with_blobs(
            original,
            &BTreeMap::from([("b0".to_owned(), b"%PDF-1.4".to_vec())]),
        );
        assert_eq!(serde_json::to_vec(&shaped)?, expected);
        let inconsistent = reply(
            json!([
                {"$neo":"blob", "id":"b0", "size":8},
                {"$neo":"blob", "id":"b0", "size":9},
            ]),
            "",
        );
        assert!(blob_requests(&inconsistent).is_none());
        Ok(())
    }

    fn fixture(source: &str) -> Result<CallToolResult, Box<dyn Error>> {
        let envelope: Value = serde_json::from_str(source)?;
        Ok(serde_json::from_value(envelope["result"].clone())?)
    }

    fn reply(value: Value, suffix: &str) -> CallToolResult {
        let mut reply = CallToolResult::structured(json!({"value": value}));
        reply.content = vec![ContentBlock::text(format!("{value}{suffix}"))];
        reply
    }

    fn bytes(data: &[u8]) -> Value {
        json!({"$neo":"bytes", "base64":STANDARD.encode(data)})
    }

    #[test]
    fn measured_png_becomes_one_image_with_matching_markers() -> Result<(), Box<dyn Error>> {
        let original = fixture(include_str!("fixtures/c1-screenshot-inline.json"))?;
        let before = serde_json::to_value(&original)?;
        let shaped = serde_json::to_value(shape(original))?;
        assert_eq!(shaped["content"].as_array().map(Vec::len), Some(2));
        assert_eq!(shaped["content"][1]["type"], "image");
        assert_eq!(shaped["content"][1]["mimeType"], "image/png");
        assert_eq!(
            shaped["content"][1]["data"],
            before["structuredContent"]["value"]["base64"]
        );
        assert_eq!(
            shaped["structuredContent"]["value"],
            "[image at content[1] (image/png)]"
        );
        let text = shaped["content"][0]["text"].as_str().ok_or("no text")?;
        assert_eq!(text, "\"[image at content[1] (image/png)]\"");
        assert!(text.len() < 1024);
        assert!(!text.contains("base64"));
        assert_eq!(shaped["_meta"], before["_meta"]);
        assert_eq!(shaped["isError"], before["isError"]);
        Ok(())
    }

    #[test]
    fn measured_blob_stays_intact_and_gets_one_note() -> Result<(), Box<dyn Error>> {
        let original = fixture(include_str!("fixtures/c3-screenshot-blob.json"))?;
        let before = serde_json::to_value(&original)?;
        let shaped = serde_json::to_value(shape(original))?;
        assert_eq!(shaped["content"].as_array().map(Vec::len), Some(2));
        assert_eq!(shaped["content"][0], before["content"][0]);
        assert_eq!(shaped["content"][1]["type"], "text");
        assert_eq!(shaped["content"][1]["text"], BLOB_NOTE);
        assert_eq!(shaped["structuredContent"], before["structuredContent"]);
        assert_eq!(shaped["_meta"], before["_meta"]);
        Ok(())
    }

    #[test]
    fn ordinary_and_non_image_bytes_replies_are_serialized_identically()
    -> Result<(), Box<dyn Error>> {
        for value in [
            json!({"answer":42}),
            bytes(b"%PDF-1.4"),
            bytes(b"plain text"),
        ] {
            let original = reply(value, "\nconsole:\nhello");
            let before = serde_json::to_vec(&original)?;
            assert_eq!(serde_json::to_vec(&shape(original))?, before);
        }
        Ok(())
    }

    #[test]
    fn nested_images_keep_order_suffix_and_unrelated_content() -> Result<(), Box<dyn Error>> {
        let png = bytes(b"\x89PNG\r\n\x1a\n");
        let jpeg = bytes(b"\xff\xd8\xff\xe0");
        let webp = bytes(b"RIFF\x04\0\0\0WEBP");
        let blob = json!({"$neo":"blob", "id":"b0", "size":700000});
        let suffix =
            "\nconsole:\n[warn] hello\n[unknown outcome] sent but not confirmed\nTip: keep this";
        let mut original = reply(
            json!({"images":[png, {"jpeg":jpeg}, webp], "large":[blob.clone(), blob.clone()]}),
            suffix,
        );
        original.content.push(ContentBlock::text("unrelated"));
        let shaped = serde_json::to_value(shape(original))?;
        assert_eq!(shaped["content"].as_array().map(Vec::len), Some(6));
        assert_eq!(shaped["content"][1]["text"], "unrelated");
        for (index, mime_type) in [(2, "image/png"), (3, "image/jpeg"), (4, "image/webp")] {
            assert_eq!(shaped["content"][index]["type"], "image");
            assert_eq!(shaped["content"][index]["mimeType"], mime_type);
        }
        let value = &shaped["structuredContent"]["value"];
        assert_eq!(value["images"][0], "[image at content[2] (image/png)]");
        assert_eq!(
            value["images"][1]["jpeg"],
            "[image at content[3] (image/jpeg)]"
        );
        assert_eq!(value["images"][2], "[image at content[4] (image/webp)]");
        assert_eq!(value["large"], json!([blob.clone(), blob]));
        assert_eq!(shaped["content"][0]["text"], format!("{value}{suffix}"));
        assert_eq!(shaped["content"][5]["text"], BLOB_NOTE);
        Ok(())
    }

    #[test]
    fn malformed_and_extra_field_brands_are_ordinary_values() -> Result<(), Box<dyn Error>> {
        let png = STANDARD.encode(b"\x89PNG\r\n\x1a\n");
        for value in [
            json!({"$neo":"bytes", "base64":"not base64"}),
            json!({"$neo":"bytes", "base64":png, "ordinary":true}),
            json!({"$neo":"bytes", "base64":false}),
            json!({"$neo":"blob", "id":"b0", "size":50, "ordinary":true}),
            json!({"$neo":"blob", "id":false, "size":50}),
            json!({"$neo":"blob", "id":"b0", "size":-1}),
            json!({"$neo":"blob", "id":"", "size":50}),
            json!({"$neo":"blob", "id":"ordinary", "size":50}),
            json!({"$neo":"blob", "id":"b01", "size":50}),
            json!({"$neo":"blob", "id":"b+1", "size":50}),
            json!({"$neo":"blob", "id":"b4294967296", "size":50}),
        ] {
            let original = reply(value, "");
            let before = serde_json::to_vec(&original)?;
            assert_eq!(serde_json::to_vec(&shape(original))?, before);
        }
        Ok(())
    }

    #[test]
    fn console_bytes_and_mismatching_text_are_not_returned_images() -> Result<(), Box<dyn Error>> {
        let png = bytes(b"\x89PNG\r\n\x1a\n");
        let original = reply(json!("ok"), &format!("\nconsole:\n{png}"));
        let before = serde_json::to_vec(&original)?;
        assert_eq!(serde_json::to_vec(&shape(original))?, before);

        let mut original = reply(png, "");
        original.content = vec![ContentBlock::text("unrelated")];
        let before = serde_json::to_vec(&original)?;
        assert_eq!(serde_json::to_vec(&shape(original))?, before);
        Ok(())
    }

    #[test]
    fn error_reply_is_unchanged() -> Result<(), Box<dyn Error>> {
        let original = CallToolResult::structured_error(json!({"error":{"message":"failed"}}));
        let before = serde_json::to_vec(&original)?;
        assert_eq!(serde_json::to_vec(&shape(original))?, before);
        Ok(())
    }
}
