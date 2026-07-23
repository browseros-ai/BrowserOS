use crate::{
    framework::ToolCtx,
    output_file::write_temp_tool_output_file,
    token_estimate::{estimate_text_tokens, slice_text_by_estimated_tokens},
    trust_boundary::wrap_untrusted,
};
use serde_json::{Value, json};

const LARGE_SNAPSHOT_TOKEN_THRESHOLD: usize = 15_000;
const MAX_INLINE_EXCERPT_TOKENS: usize = 5_000;

#[derive(Debug, Clone)]
pub struct FormattedSnapshot {
    pub text: String,
    pub structured: Value,
}

pub async fn format_snapshot_result(
    snapshot: &str,
    origin: &str,
    ctx: &ToolCtx,
) -> FormattedSnapshot {
    let snapshot_text = if snapshot.is_empty() {
        "(empty page)"
    } else {
        snapshot
    };
    let wrapped_snapshot = wrap_untrusted(snapshot_text, origin);
    let content_length = wrapped_snapshot.len();
    let token_estimate = estimate_text_tokens(&wrapped_snapshot);

    if token_estimate > LARGE_SNAPSHOT_TOKEN_THRESHOLD {
        let excerpt = slice_text_by_estimated_tokens(snapshot_text, MAX_INLINE_EXCERPT_TOKENS);
        match write_temp_tool_output_file(&ctx.output_files, "snapshot", "md", &wrapped_snapshot)
            .await
        {
            Ok(path) => {
                return FormattedSnapshot {
                    text: [
                        format!(
                            "Large snapshot ({token_estimate} estimated tokens, {content_length} chars) saved to: {}",
                            path.display()
                        ),
                        "Read the file for the full snapshot and refs.".to_string(),
                        format!(
                            "Showing the first {MAX_INLINE_EXCERPT_TOKENS} estimated tokens inline:"
                        ),
                        wrap_untrusted(&excerpt, origin),
                    ]
                    .join("\n"),
                    structured: json!({
                        "path": path.to_string_lossy(),
                        "contentLength": content_length,
                        "tokenEstimate": token_estimate,
                        "writtenToFile": true
                    }),
                };
            }
            Err(err) => {
                let save_error = err.to_string();
                return FormattedSnapshot {
                    text: [
                        format!(
                            "Large snapshot ({token_estimate} estimated tokens, {content_length} chars) could not be saved to a BrowserOS output file: {save_error}"
                        ),
                        format!(
                            "Showing the first {MAX_INLINE_EXCERPT_TOKENS} estimated tokens instead:"
                        ),
                        wrap_untrusted(&excerpt, origin),
                    ]
                    .join("\n"),
                    structured: json!({
                        "contentLength": content_length,
                        "tokenEstimate": token_estimate,
                        "writtenToFile": false,
                        "outputWriteFailed": true,
                        "error": save_error
                    }),
                };
            }
        }
    }

    FormattedSnapshot {
        text: wrapped_snapshot,
        structured: json!({
            "contentLength": content_length,
            "tokenEstimate": token_estimate,
            "writtenToFile": false
        }),
    }
}
