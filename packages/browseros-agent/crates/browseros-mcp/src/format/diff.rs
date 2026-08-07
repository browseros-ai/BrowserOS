use crate::{
    framework::ToolCtx,
    output_file::write_temp_tool_output_file,
    token_estimate::{estimate_text_tokens, slice_text_by_estimated_tokens},
    trust_boundary::wrap_untrusted,
};
use browseros_core::snapshot::SnapshotDiff;
use serde_json::{Value, json};

const MAX_INLINE_DIFF_TOKENS: usize = 10_000;
const MAX_INLINE_EXCERPT_TOKENS: usize = 5_000;

#[derive(Debug, Clone)]
pub struct FormattedDiff {
    pub text: String,
    pub structured: Value,
}

pub async fn format_diff_result(diff: &SnapshotDiff, origin: &str, ctx: &ToolCtx) -> FormattedDiff {
    if !diff.changed {
        return FormattedDiff {
            text: "no change since last snapshot".to_string(),
            structured: json!({ "changed": false }),
        };
    }

    let diff_text = if diff.text.is_empty() {
        "(empty page)"
    } else {
        &diff.text
    };
    let wrapped_diff = wrap_untrusted(diff_text, origin);
    let token_estimate = estimate_text_tokens(&wrapped_diff);
    let mut structured = json!({
        "changed": true,
        "added": diff.added,
        "removed": diff.removed
    });
    if diff.url_changed
        && let Value::Object(object) = &mut structured
    {
        object.insert("urlChanged".to_string(), Value::Bool(true));
        object.insert(
            "beforeUrl".to_string(),
            diff.before_url
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
        object.insert(
            "afterUrl".to_string(),
            diff.after_url
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
    }

    if token_estimate > MAX_INLINE_DIFF_TOKENS {
        let excerpt = slice_text_by_estimated_tokens(diff_text, MAX_INLINE_EXCERPT_TOKENS);
        let content_length = wrapped_diff.len();
        match write_temp_tool_output_file(&ctx.output_files, "diff", "md", &wrapped_diff).await {
            Ok(path) => {
                let summary = if diff.url_changed {
                    format!(
                        "URL changed; full current snapshot is {token_estimate} estimated tokens, over the {MAX_INLINE_DIFF_TOKENS}-token inline limit, saved to: {}\nRead the file for the full current snapshot.",
                        path.display()
                    )
                } else {
                    format!(
                        "Diff is {token_estimate} estimated tokens, over the {MAX_INLINE_DIFF_TOKENS}-token inline limit, saved to: {}\nRead the file for the full diff.",
                        path.display()
                    )
                };
                add_fields(
                    &mut structured,
                    json!({
                        "truncated": true,
                        "tokenEstimate": token_estimate,
                        "path": path.to_string_lossy(),
                        "contentLength": content_length,
                        "writtenToFile": true
                    }),
                );
                return FormattedDiff {
                    text: [
                        summary,
                        format!(
                            "Showing the first {MAX_INLINE_EXCERPT_TOKENS} estimated tokens inline:"
                        ),
                        wrap_untrusted(&excerpt, origin),
                    ]
                    .join("\n"),
                    structured,
                };
            }
            Err(err) => {
                let save_error = err.to_string();
                let text = if diff.url_changed {
                    format!(
                        "URL changed; full current snapshot is {token_estimate} estimated tokens, over the {MAX_INLINE_DIFF_TOKENS}-token inline limit, but saving it to a BrowserOS output file failed: {save_error}"
                    )
                } else {
                    format!(
                        "Diff is {token_estimate} estimated tokens, over the {MAX_INLINE_DIFF_TOKENS}-token inline limit, but saving it to a BrowserOS output file failed: {save_error}"
                    )
                };
                add_fields(
                    &mut structured,
                    json!({
                        "truncated": true,
                        "tokenEstimate": token_estimate,
                        "contentLength": content_length,
                        "writtenToFile": false,
                        "outputWriteFailed": true,
                        "error": save_error
                    }),
                );
                return FormattedDiff {
                    text: [
                        text,
                        format!(
                            "Showing the first {MAX_INLINE_EXCERPT_TOKENS} estimated tokens instead:"
                        ),
                        wrap_untrusted(&excerpt, origin),
                    ]
                    .join("\n"),
                    structured,
                };
            }
        }
    }

    if diff.url_changed {
        return FormattedDiff {
            text: format!(
                "URL changed; returning full current snapshot instead of a diff:\n{wrapped_diff}"
            ),
            structured,
        };
    }

    FormattedDiff {
        text: wrapped_diff,
        structured,
    }
}

fn add_fields(target: &mut Value, fields: Value) {
    let (Value::Object(target), Value::Object(fields)) = (target, fields) else {
        return;
    };
    target.extend(fields);
}
