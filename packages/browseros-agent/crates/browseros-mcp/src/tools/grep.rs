use crate::{
    constants::{GREP_MATCH_LINE_MAX_CHARS, GREP_MAX_MATCHES, INLINE_PAGE_CONTENT_MAX_CHARS},
    framework::{
        ToolCtx, ToolExecResult, ToolResult, error_result, parse_args, pending_dialog_result,
        text_result,
    },
    output_file::write_temp_tool_output_file,
    trust_boundary::wrap_untrusted,
};
use browseros_core::PageId;
use futures_util::future::BoxFuture;
use regex::RegexBuilder;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DEFAULT_LIMIT: usize = 50;
const LINE_TRUNCATION_MARKER: &str = "... [truncated]";
const DESCRIPTION: &str = "\
Search the page without dumping it. over=\"ax\" greps the snapshot lines \
(matches keep their [ref=eN]); over=\"content\" greps visible text. Returns matching lines.";

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum GrepOver {
    #[default]
    Ax,
    Content,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct GrepArgs {
    page: u32,
    /// Case-insensitive regular expression.
    pattern: String,
    #[serde(default)]
    over: GrepOver,
    /// Max matching lines (default 50).
    limit: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct EvaluateResult {
    result: RemoteObject,
}

#[derive(Debug, Deserialize)]
struct RemoteObject {
    value: Option<Value>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<GrepArgs>(
        "grep",
        DESCRIPTION,
        Some(super::read_only_annotations()),
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: GrepArgs = parse_args(raw)?;
        let regex = match RegexBuilder::new(&args.pattern)
            .case_insensitive(true)
            .build()
        {
            Ok(regex) => regex,
            Err(err) => return Ok(Some(error_result(format!("grep: invalid regex - {err}")))),
        };
        if let Some(result) = pending_dialog_result(ctx, PageId(args.page)) {
            return Ok(Some(result));
        }
        let haystack = match args.over {
            GrepOver::Ax => {
                ctx.session
                    .observe(PageId(args.page))
                    .await
                    .snapshot()
                    .await?
                    .text
            }
            GrepOver::Content => {
                let page = ctx.session.pages.get_session(PageId(args.page)).await?;
                let result: EvaluateResult = page
                    .session
                    .send(
                        "Runtime.evaluate",
                        json!({
                            "expression": "(document.body?.innerText ?? '')",
                            "returnByValue": true
                        }),
                    )
                    .await?;
                result
                    .result
                    .value
                    .and_then(|value| value.as_str().map(ToString::to_string))
                    .unwrap_or_default()
            }
        };
        let limit = clamp_limit(args.limit);
        let matches = haystack
            .split('\n')
            .filter(|line| regex.is_match(line))
            .take(limit)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let over = over_name(&args.over);
        if matches.is_empty() {
            return Ok(Some(text_result(
                "no matches",
                Some(json!({ "page": args.page, "over": over, "count": 0 })),
            )));
        }
        let origin = ctx
            .session
            .pages
            .get_info(PageId(args.page))
            .await
            .map(|info| info.url)
            .unwrap_or_else(|| "unknown".to_string());
        let rendered_matches = matches
            .iter()
            .map(|line| clamp_text(line, GREP_MATCH_LINE_MAX_CHARS))
            .collect::<Vec<_>>();
        let full_matches_text = matches.join("\n");
        let rendered_text = rendered_matches.join("\n");
        let line_truncated = rendered_matches
            .iter()
            .zip(matches.iter())
            .any(|(left, right)| left != right);
        let total_truncated = rendered_text.len() > INLINE_PAGE_CONTENT_MAX_CHARS;
        let inline_text = clamp_text(&rendered_text, INLINE_PAGE_CONTENT_MAX_CHARS);
        if line_truncated || total_truncated {
            match write_temp_tool_output_file(
                &ctx.output_files,
                "grep",
                "txt",
                &wrap_untrusted(&full_matches_text, &origin),
            )
            .await
            {
                Ok(path) => {
                    return Ok(Some(text_result(
                        [
                            wrap_untrusted(&inline_text, &origin),
                            format!(
                                "Grep output truncated for {} match(es). Full matches ({} chars) saved to: {}",
                                matches.len(),
                                full_matches_text.len(),
                                path.display()
                            ),
                        ]
                        .join("\n\n"),
                        Some(json!({
                            "page": args.page,
                            "over": over,
                            "count": matches.len(),
                            "truncated": true,
                            "path": path.to_string_lossy()
                        })),
                    )));
                }
                Err(err) => {
                    let save_error = err.to_string();
                    return Ok(Some(text_result(
                        [
                            wrap_untrusted(&inline_text, &origin),
                            format!(
                                "Grep output truncated for {} match(es). Full matches ({} chars) could not be saved to a BrowserOS output file: {save_error}",
                                matches.len(),
                                full_matches_text.len()
                            ),
                        ]
                        .join("\n\n"),
                        Some(json!({
                            "page": args.page,
                            "over": over,
                            "count": matches.len(),
                            "truncated": true,
                            "writtenToFile": false,
                            "outputWriteFailed": true,
                            "error": save_error
                        })),
                    )));
                }
            }
        }
        Ok(Some(text_result(
            wrap_untrusted(&rendered_text, &origin),
            Some(json!({ "page": args.page, "over": over, "count": matches.len() })),
        )))
    })
}

pub(crate) fn clamp_limit(limit: Option<f64>) -> usize {
    let Some(limit) = limit else {
        return DEFAULT_LIMIT;
    };
    if !limit.is_finite() {
        return DEFAULT_LIMIT;
    }
    limit.floor().clamp(0.0, GREP_MAX_MATCHES as f64) as usize
}

pub(crate) fn clamp_text(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let prefix_length = max_chars.saturating_sub(LINE_TRUNCATION_MARKER.len());
    let mut end = prefix_length;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}{}", &text[..end], LINE_TRUNCATION_MARKER)
}

fn over_name(over: &GrepOver) -> &'static str {
    match over {
        GrepOver::Ax => "ax",
        GrepOver::Content => "content",
    }
}
