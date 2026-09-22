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

/// Estimates output tokens for a JSON primitive result, e.g. a code-mode
/// `browser.read` payload, by tokenizing its compact serialization the same way
/// input arguments are counted. A non-serializable value contributes zero.
#[must_use]
pub fn estimate_json_output_tokens(value: &Value) -> i64 {
    serde_json::to_string(value)
        .map(|json| bounded_tokens(estimate_text_tokens(&json)))
        .unwrap_or_default()
}

/// Estimates image tokens from pixel dimensions using the version-1 bounded patch model.
#[must_use]
pub fn estimate_image_tokens_from_dimensions(width_px: usize, height_px: usize) -> i64 {
    let width = width_px.div_ceil(IMAGE_PATCH_EDGE_PX);
    let height = height_px.div_ceil(IMAGE_PATCH_EDGE_PX);
    bounded_tokens(width.saturating_mul(height).min(MAX_IMAGE_PATCHES))
}

#[must_use]
pub fn slice_text_by_estimated_tokens(text: &str, max_tokens: usize) -> String {
    if estimate_text_tokens(text) <= max_tokens {
        return text.to_string();
    }

    // estimate_text_tokens is ceil(bytes / APPROX_CHARS_PER_TOKEN), so the largest
    // prefix within budget is APPROX_CHARS_PER_TOKEN * max_tokens bytes. Computing
    // it directly avoids a binary search whose midpoint could floor back onto an
    // earlier UTF-8 boundary and never make progress, spinning a worker forever
    // (#2707). Keep this in lockstep with estimate_text_tokens; the debug_assert
    // guards the coupling.
    let max_bytes = max_tokens
        .saturating_mul(APPROX_CHARS_PER_TOKEN)
        .min(text.len());
    let end = floor_char_boundary(text, max_bytes);
    let sliced = text[..end].to_string();
    debug_assert!(estimate_text_tokens(&sliced) <= max_tokens);
    sliced
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
    estimate_image_tokens_from_dimensions(size.width, size.height)
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
        estimate_image_tokens_from_dimensions, estimate_json_output_tokens, estimate_text_tokens,
        estimate_tool_input_tokens, estimate_tool_output_tokens, saturating_token_sum,
        slice_text_by_estimated_tokens,
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
    fn standard_screenshot_dimensions_reach_the_patch_cap() {
        assert_eq!(estimate_image_tokens_from_dimensions(1920, 1080), 1536);
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

    #[test]
    fn json_output_counts_the_compact_serialization() {
        assert_eq!(
            estimate_json_output_tokens(&json!("abcdef")),
            estimate_text_tokens(r#""abcdef""#) as i64
        );
        assert_eq!(
            estimate_json_output_tokens(&json!(null)),
            estimate_text_tokens("null") as i64
        );
    }

    #[test]
    fn slicing_multibyte_text_terminates_on_a_utf8_boundary() {
        // Reporter's #2707 case: a midpoint inside a 3-byte character used to
        // floor back onto an earlier boundary and loop forever.
        let text = "汉".repeat(5001);
        let sliced = slice_text_by_estimated_tokens(&text, 5000);

        assert_eq!(estimate_text_tokens(&sliced), 5000);
        assert_eq!(sliced, "汉".repeat(5000));
    }

    #[test]
    fn slicing_two_byte_text_terminates_within_budget() {
        let text = "é".repeat(3001);
        let sliced = slice_text_by_estimated_tokens(&text, 2000);

        assert_eq!(sliced, "é".repeat(3000));
        assert!(estimate_text_tokens(&sliced) <= 2000);
    }

    #[test]
    fn slicing_with_zero_budget_returns_empty() {
        assert_eq!(slice_text_by_estimated_tokens("abc", 0), "");
    }

    #[test]
    fn slicing_within_budget_returns_input_unchanged() {
        assert_eq!(slice_text_by_estimated_tokens("hello", 100), "hello");
    }

    #[test]
    fn slicing_ascii_truncates_at_the_byte_budget() {
        assert_eq!(slice_text_by_estimated_tokens("abcdefghij", 2), "abcdef");
    }

    #[test]
    fn slicing_yields_the_maximal_in_budget_char_boundary_prefix() {
        let cases = [
            ("汉字漢字".repeat(50), 7usize),
            ("café ☕ déjà vu ".repeat(40), 11usize),
            ("emoji 😀😀😀 test ".repeat(30), 9usize),
            ("plain ascii text here".to_string(), 3usize),
        ];

        for (text, max_tokens) in cases {
            let sliced = slice_text_by_estimated_tokens(&text, max_tokens);

            assert!(estimate_text_tokens(&sliced) <= max_tokens);
            assert!(text.starts_with(&sliced));
            assert!(text.is_char_boundary(sliced.len()));

            // Maximal: taking one more character would exceed the budget.
            if let Some(next_char) = text[sliced.len()..].chars().next() {
                let next = sliced.len() + next_char.len_utf8();
                assert!(estimate_text_tokens(&text[..next]) > max_tokens);
            }
        }
    }
}
