//! Provider-neutral estimates for semantic MCP tool traffic.
//!
//! Version 1 counts UTF-8 text with the shared bytes-per-token heuristic and images with bounded
//! 32x32 patches. It deliberately excludes protocol envelopes, metadata, and opaque binary data.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use rmcp::model::{ContentBlock, ResourceContents};
use serde_json::Value;

const APPROX_CHARS_PER_TOKEN: usize = 3;
const IMAGE_PATCH_EDGE_PX: usize = 32;
const MAX_IMAGE_PATCHES: usize = 1536;

pub const TOKEN_ESTIMATOR_VERSION: i64 = 1;

#[must_use]
pub fn estimate_text_tokens(text: &str) -> usize {
    text.len().div_ceil(APPROX_CHARS_PER_TOKEN)
}

#[must_use]
pub fn estimate_tool_input_tokens(tool_name: &str, arguments: &Value) -> i64 {
    let arguments = serde_json::to_string(arguments)
        .map(|json| bounded_tokens(estimate_text_tokens(&json)))
        .unwrap_or_default();
    bounded_tokens(estimate_text_tokens(tool_name)).saturating_add(arguments)
}

#[must_use]
pub fn estimate_tool_output_tokens(content: &[ContentBlock]) -> i64 {
    saturating_token_sum(content.iter().map(estimate_content_block_tokens))
}

#[must_use]
pub fn slice_text_by_estimated_tokens(text: &str, max_tokens: usize) -> String {
    if estimate_text_tokens(text) <= max_tokens {
        return text.to_string();
    }

    let mut low = 0;
    let mut high = text.len();
    while low < high {
        let mid = (low + high).div_ceil(2);
        let candidate = floor_char_boundary(text, mid);
        if estimate_text_tokens(&text[..candidate]) <= max_tokens {
            low = candidate;
        } else {
            high = candidate.saturating_sub(1);
        }
    }
    let end = floor_char_boundary(text, low);
    text[..end].to_string()
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index = index.saturating_sub(1);
    }
    index
}

fn estimate_content_block_tokens(content: &ContentBlock) -> i64 {
    match content {
        ContentBlock::Text(text) => bounded_tokens(estimate_text_tokens(&text.text)),
        ContentBlock::Image(image) => estimate_image_tokens(&image.data),
        ContentBlock::Resource(resource) => match &resource.resource {
            ResourceContents::TextResourceContents { text, .. } => {
                bounded_tokens(estimate_text_tokens(text))
            }
            _ => 0,
        },
        ContentBlock::Audio(_) | ContentBlock::ResourceLink(_) => 0,
        _ => 0,
    }
}

fn estimate_image_tokens(encoded: &str) -> i64 {
    let Ok(bytes) = STANDARD.decode(encoded) else {
        return 0;
    };
    let Ok(size) = imagesize::blob_size(&bytes) else {
        return 0;
    };
    let width = size.width.div_ceil(IMAGE_PATCH_EDGE_PX);
    let height = size.height.div_ceil(IMAGE_PATCH_EDGE_PX);
    bounded_tokens(width.saturating_mul(height).min(MAX_IMAGE_PATCHES))
}

fn bounded_tokens(tokens: usize) -> i64 {
    i64::try_from(tokens).unwrap_or(i64::MAX)
}

fn saturating_token_sum(tokens: impl IntoIterator<Item = i64>) -> i64 {
    tokens
        .into_iter()
        .fold(0, |total, tokens| total.saturating_add(tokens))
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use rmcp::model::{ContentBlock, Resource, ResourceContents};
    use serde_json::json;

    use super::{
        estimate_text_tokens, estimate_tool_input_tokens, estimate_tool_output_tokens,
        saturating_token_sum,
    };

    fn png_header(width: u32, height: u32) -> String {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        STANDARD.encode(bytes)
    }

    #[test]
    fn text_uses_utf8_bytes_rounded_up() {
        assert_eq!(estimate_text_tokens(""), 0);
        assert_eq!(estimate_text_tokens("abc"), 1);
        assert_eq!(estimate_text_tokens("abcd"), 2);
        assert_eq!(estimate_text_tokens("éé"), 2);
    }

    #[test]
    fn tool_input_counts_name_and_compact_arguments() {
        let arguments = json!({"b": [1, true], "a": "é"});
        let compact_arguments = r#"{"a":"é","b":[1,true]}"#;

        assert_eq!(
            estimate_tool_input_tokens("browser", &arguments),
            (estimate_text_tokens("browser") + estimate_text_tokens(compact_arguments)) as i64
        );
    }

    #[test]
    fn output_sums_text_and_embedded_text_blocks() {
        let content = vec![
            ContentBlock::text("abc"),
            ContentBlock::text("abcdef"),
            ContentBlock::embedded_text("memory://note", "abcd"),
        ];

        assert_eq!(estimate_tool_output_tokens(&content), 5);
    }

    #[test]
    fn image_uses_rounded_patches_without_counting_base64() {
        let image = ContentBlock::image(png_header(33, 65), "image/png");

        assert_eq!(estimate_tool_output_tokens(&[image]), 6);
    }

    #[test]
    fn image_patch_count_is_capped() {
        let image = ContentBlock::image(png_header(2048, 2048), "image/png");

        assert_eq!(estimate_tool_output_tokens(&[image]), 1536);
    }

    #[test]
    fn invalid_and_unsupported_images_contribute_zero() {
        let invalid_base64 = ContentBlock::image("not-base64", "image/png");
        let unsupported = ContentBlock::image(STANDARD.encode("not an image"), "image/example");

        assert_eq!(
            estimate_tool_output_tokens(&[invalid_base64, unsupported]),
            0
        );
    }

    #[test]
    fn opaque_and_linked_content_contributes_zero() {
        let content = vec![
            ContentBlock::audio(STANDARD.encode("audio"), "audio/wav"),
            ContentBlock::resource(ResourceContents::blob("opaque", "memory://blob")),
            ContentBlock::resource_link(Resource::new("memory://linked", "linked")),
        ];

        assert_eq!(estimate_tool_output_tokens(&content), 0);
    }

    #[test]
    fn mixed_text_and_image_content_is_bounded_sum() {
        let content = vec![
            ContentBlock::text("abc"),
            ContentBlock::image(png_header(33, 65), "image/png"),
        ];

        assert_eq!(estimate_tool_output_tokens(&content), 7);
    }

    #[test]
    fn token_totals_saturate_instead_of_wrapping() {
        assert_eq!(saturating_token_sum([i64::MAX, 1]), i64::MAX);
    }
}
