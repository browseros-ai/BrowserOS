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
Use this for page-state reads or small DOM scripts that are awkward with read/grep. \
Return a value to read it back.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct EvaluateArgs {
    /// Page id from `tabs`.
    page: u32,
    /// Async-capable JS body evaluated inside the page. Use `return` to read a value.
    code: String,
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
                    "expression": wrap_as_async_iife(&args.code),
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

fn wrap_as_async_iife(code: &str) -> String {
    format!("(async () => {{\n{code}\n}})()")
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
