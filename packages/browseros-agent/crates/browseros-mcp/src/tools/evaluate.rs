use crate::{
    constants::INLINE_PAGE_CONTENT_MAX_CHARS,
    framework::{
        ToolCtx, ToolExecResult, ToolResult, clamp_timeout, error_result, parse_args,
        pending_dialog_result, text_result,
    },
    output_file::write_temp_tool_output_file,
    trust_boundary::wrap_untrusted,
};
use browseros_core::PageId;
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_TIMEOUT_MS: u64 = 30_000;

const DESCRIPTION: &str = "\
Evaluate JavaScript in a page context through CDP Runtime.evaluate. \
Prefer `run` for multi-step work; reach for evaluate only as a fallback for a one-off page-context read or script. \
Use this for page-state reads or small DOM scripts that are awkward with read/grep. \
Provide `code` (an async body; use `return` to read a value) or `func` (a function \
expression like `() => {...}` that gets invoked). Return a value to read it back.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EvaluateArgs {
    /// Page id from `tabs`.
    page: u32,
    /// Async-capable JS body evaluated inside the page. Use `return` to read a value.
    #[serde(default)]
    code: Option<String>,
    /// A function expression to invoke, e.g. `() => {...}` or `async () => {...}`.
    /// An alternative to `code` for callers that pass a function.
    #[serde(default)]
    func: Option<String>,
    /// Max evaluation time in ms (default 30000).
    timeout: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvaluateResult {
    result: RemoteObject,
    exception_details: Option<ExceptionDetails>,
}

#[derive(Debug, Deserialize)]
struct RemoteObject {
    value: Option<Value>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExceptionDetails {
    text: String,
    exception: Option<RemoteObject>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<EvaluateArgs>(
        "evaluate",
        DESCRIPTION,
        Some(super::open_world_annotations()),
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: EvaluateArgs = parse_args(raw)?;
        let Some(expression) = resolve_expression(args.code.as_deref(), args.func.as_deref())
        else {
            return Ok(Some(error_result(
                "evaluate: provide `code` (an async body) or `func` (a function to invoke)"
                    .to_string(),
            )));
        };
        if let Some(result) = pending_dialog_result(ctx, PageId(args.page)) {
            return Ok(Some(result));
        }
        let page = ctx.session.pages.get_session(PageId(args.page)).await?;
        let timeout = clamp_timeout(args.timeout, DEFAULT_TIMEOUT_MS, MAX_TIMEOUT_MS);
        let result: EvaluateResult = page
            .session
            .send(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                    "timeout": timeout,
                    "userGesture": true
                }),
            )
            .await?;
        if let Some(exception) = result.exception_details {
            return Ok(Some(error_result(format!(
                "evaluate: {}",
                exception_message(exception)
            ))));
        }
        let value = result.result.value;
        let text = match &value {
            Some(value) => safe_stringify(value),
            None => result
                .result
                .description
                .unwrap_or_else(|| "undefined".to_string()),
        };
        let origin = ctx
            .session
            .pages
            .get_info(PageId(args.page))
            .await
            .map(|info| info.url)
            .unwrap_or_else(|| "unknown".to_string());
        if text.len() > INLINE_PAGE_CONTENT_MAX_CHARS {
            let excerpt = safe_prefix(&text, INLINE_PAGE_CONTENT_MAX_CHARS);
            let wrapped_text = wrap_untrusted(&text, &origin);
            let content_length = wrapped_text.len();
            match write_temp_tool_output_file(&ctx.output_files, "evaluate", "txt", &wrapped_text)
                .await
            {
                Ok(path) => {
                    return Ok(Some(text_result(
                        [
                            wrap_untrusted(&excerpt, &origin),
                            format!(
                                "Evaluate result truncated at {INLINE_PAGE_CONTENT_MAX_CHARS} chars. Full result ({} chars) saved to: {}",
                                text.len(),
                                path.display()
                            ),
                        ]
                        .join("\n\n"),
                        Some(json!({
                            "page": args.page,
                            "contentLength": content_length,
                            "writtenToFile": true,
                            "path": path.to_string_lossy()
                        })),
                    )));
                }
                Err(err) => {
                    let save_error = err.to_string();
                    return Ok(Some(text_result(
                        [
                            wrap_untrusted(&excerpt, &origin),
                            format!(
                                "Evaluate result truncated at {INLINE_PAGE_CONTENT_MAX_CHARS} chars. Full result ({} chars) could not be saved to a BrowserOS output file: {save_error}",
                                text.len()
                            ),
                        ]
                        .join("\n\n"),
                        Some(json!({
                            "page": args.page,
                            "contentLength": content_length,
                            "writtenToFile": false,
                            "outputWriteFailed": true,
                            "error": save_error
                        })),
                    )));
                }
            }
        }
        let mut structured = json!({ "page": args.page });
        if let (Value::Object(object), Some(value)) = (&mut structured, value) {
            object.insert("value".to_string(), value);
        }
        Ok(Some(text_result(
            wrap_untrusted(&text, &origin),
            Some(structured),
        )))
    })
}

/// Builds the JS expression to evaluate from either arg form: `code` is an async
/// body, `func` is a function expression to invoke. `code` wins if both are given;
/// `None` when neither is provided.
fn resolve_expression(code: Option<&str>, func: Option<&str>) -> Option<String> {
    match (code, func) {
        (Some(code), _) => Some(wrap_as_async_iife(code)),
        (None, Some(func)) => Some(wrap_as_invoked_fn(func)),
        (None, None) => None,
    }
}

fn wrap_as_async_iife(code: &str) -> String {
    format!("(async () => {{\n{code}\n}})()")
}

fn wrap_as_invoked_fn(func: &str) -> String {
    format!("(async () => {{ return await ({func})(); }})()")
}

fn safe_stringify(value: &Value) -> String {
    if let Some(value) = value.as_str() {
        return value.to_string();
    }
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn exception_message(exception: ExceptionDetails) -> String {
    exception
        .exception
        .and_then(|exception| exception.description)
        .unwrap_or(exception.text)
}

fn safe_prefix(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let mut end = max_chars;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_expression_handles_code_func_and_neither() {
        // A body is wrapped in an async IIFE.
        let code = resolve_expression(Some("return 1;"), None).unwrap_or_default();
        assert!(code.contains("return 1;"));
        assert!(code.starts_with("(async () =>"));
        // A function is invoked, so its return value flows back.
        let func = resolve_expression(None, Some("() => 2")).unwrap_or_default();
        assert!(func.contains("await (() => 2)()"));
        // Code wins if both are provided.
        let both = resolve_expression(Some("return 3;"), Some("() => 4")).unwrap_or_default();
        assert!(both.contains("return 3;"));
        assert!(!both.contains("() => 4"));
        // Neither is an error at the call site.
        assert!(resolve_expression(None, None).is_none());
    }
}
