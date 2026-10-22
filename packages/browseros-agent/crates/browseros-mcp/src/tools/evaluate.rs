use crate::{
    constants::INLINE_PAGE_CONTENT_MAX_CHARS,
    framework::{
        ToolCtx, ToolExecResult, ToolResult, clamp_timeout, error_result, parse_args,
        pending_dialog_result, text_result,
    },
    output_file::write_temp_tool_output_file,
    trust_boundary::wrap_untrusted,
};
use browseros_core::{PageId, settle, timeouts};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_TIMEOUT_MS: u64 = 30_000;

/// Hard ceiling for the opt-in `maxChars`: a caller may pull up to this much of a
/// large result inline instead of having it spilled to a local file. The default
/// inline size stays small (`INLINE_PAGE_CONTENT_MAX_CHARS`) so ordinary results
/// do not flood the model's context.
const MAX_INLINE_OVERRIDE_CHARS: usize = 200_000;

const DESCRIPTION: &str = "\
Evaluate JavaScript in a page context through CDP Runtime.evaluate. \
Prefer `run` for multi-step work; reach for evaluate only as a fallback for a one-off page-context read or script. \
Use this for page-state reads or small DOM scripts that are awkward with read/grep. \
Provide `code` (an async body; use `return` to read a value) or `func` (a function \
expression like `() => {...}` that gets invoked). Return a value to read it back. \
A script that mutated the page is settled before the result comes back, and the result \
says so, so never pause afterwards to let the page catch up. For an effect \
that is entirely asynchronous, such as a click whose result arrives over the network, pass \
`settle: true` to wait for the page to go quiet regardless. \
`timeout` is capped at 30000 ms; for page work that needs longer, start it on the page and poll with short follow-up calls rather than one long evaluate. \
A result larger than the inline limit is truncated and its full text is written to a local file whose path a remote MCP client cannot read; return only what you need, or raise `maxChars` to receive more of the value inline.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
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
    /// Max evaluation time in ms. Hard cap: 30000 (larger values are clamped to
    /// it). For work longer than 30s, start it on the page and poll the result
    /// with short follow-up calls instead of one long evaluate.
    timeout: Option<f64>,
    /// Wait for the page to go quiet before returning, even when the script
    /// changed nothing the observer could see. An evaluation that mutates the
    /// DOM is detected and settled without this; set it for an effect that is
    /// entirely asynchronous, such as a click whose result arrives over the
    /// network. Never a reason to pause afterwards either way.
    #[serde(default)]
    settle: Option<bool>,
    /// Max size of the result kept inline, measured in UTF-8 bytes to match the
    /// server's inline limit (default 5000, max 200000); a multibyte character
    /// counts as more than one byte. A result larger than this is truncated
    /// inline and its full text is written to a local file, whose path a remote
    /// MCP client cannot open; raise this to receive more of the value inline.
    #[serde(default)]
    max_chars: Option<u64>,
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
        let page = ctx.browser.page_session(PageId(args.page)).await?;
        let timeout = clamp_timeout(args.timeout, DEFAULT_TIMEOUT_MS, MAX_TIMEOUT_MS);
        // Read the page's mutation counter either side of the evaluation, so a
        // script that changed the page is settled before its result is handed
        // back. Without this an evaluate-driven click returns a bare value with
        // no readiness signal of any kind, which is what left callers pausing
        // afterwards and guessing how long. Two cheap CDP reads on a script that
        // changes nothing.
        let before = settle::read_readiness(&page.session).await.ok();
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
        let moved = page_moved(
            before.as_ref(),
            settle::read_readiness(&page.session).await.ok().as_ref(),
        );
        let settled = if moved || args.settle.unwrap_or(false) {
            matches!(
                settle::wait_for_representation_stable(
                    &page.session,
                    timeouts::ACTION_SETTLE_DEFAULT_TIMEOUT,
                )
                .await,
                settle::Readiness::Stable
            )
        } else {
            false
        };

        let value = result.result.value;
        let text = match &value {
            Some(value) => safe_stringify(value),
            None => result
                .result
                .description
                .unwrap_or_else(|| "undefined".to_string()),
        };
        let origin = ctx
            .browser
            .refresh_page(PageId(args.page))
            .await?
            .map(|info| info.url)
            .unwrap_or_else(|| "unknown".to_string());
        let inline_limit = resolve_inline_limit(args.max_chars);
        if text.len() > inline_limit {
            let excerpt = safe_prefix(&text, inline_limit);
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
                                "Evaluate result truncated at {inline_limit} bytes. Full result ({} bytes) saved to: {}",
                                text.len(),
                                path.display()
                            ),
                        ]
                        .into_iter()
                        .chain(readiness_note(moved, settled).map(ToString::to_string))
                        .collect::<Vec<_>>()
                        .join("\n\n"),
                        Some(json!({
                            "page": args.page,
                            "changed": moved,
                            "settled": settled,
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
                                "Evaluate result truncated at {inline_limit} bytes. Full result ({} bytes) could not be saved to a BrowserOS output file: {save_error}",
                                text.len()
                            ),
                        ]
                        .into_iter()
                        .chain(readiness_note(moved, settled).map(ToString::to_string))
                        .collect::<Vec<_>>()
                        .join("\n\n"),
                        Some(json!({
                            "page": args.page,
                            "changed": moved,
                            "settled": settled,
                            "contentLength": content_length,
                            "writtenToFile": false,
                            "outputWriteFailed": true,
                            "error": save_error
                        })),
                    )));
                }
            }
        }
        let mut structured = json!({ "page": args.page, "changed": moved, "settled": settled });
        if let (Value::Object(object), Some(value)) = (&mut structured, value) {
            object.insert("value".to_string(), value);
        }
        let mut body = wrap_untrusted(&text, &origin);
        if let Some(note) = readiness_note(moved, settled) {
            body.push_str("\n\n");
            body.push_str(note);
        }
        Ok(Some(text_result(body, Some(structured))))
    })
}

/// The line appended to a result whose script moved the page.
///
/// Structured content does not survive the browser-tool wire envelope for a
/// tool with no output schema, and `evaluate` has none, so `changed` and
/// `settled` reach a script running inside `run` but not a client calling
/// `evaluate` directly. The text does survive, and readiness is only useful
/// to whoever has to decide whether to wait, so it is stated there too.
///
/// Silent when the script changed nothing: the value is the whole answer, and
/// a line on every read would cost context to say so.
fn readiness_note(moved: bool, settled: bool) -> Option<&'static str> {
    match (moved, settled) {
        (false, _) => None,
        (true, true) => Some(
            "The script changed the page, which then went quiet before this result. \
             Nothing further to wait for.",
        ),
        (true, false) => Some(
            "The script changed the page and it was still changing when the readiness \
             budget expired, so this result may predate the effect. Wait on the thing \
             you expect with `wait`, rather than pausing for a fixed time.",
        ),
    }
}

/// Whether the evaluation changed the page, read from the mutation counter
/// either side of it.
///
/// Conservative in the one direction that matters: when the counter could not
/// be read after the evaluation but could be read before, the execution context
/// went away under us, and the usual cause is the script navigating. That is a
/// change, so it reports one rather than letting a navigation pass for a quiet
/// page.
fn page_moved(
    before: Option<&settle::ReadinessReading>,
    after: Option<&settle::ReadinessReading>,
) -> bool {
    match (before, after) {
        // `installed` means the observer was re-armed, which a navigation does
        // by replacing the body, so it counts as the page having moved.
        (Some(before), Some(after)) => after.installed || after.count != before.count,
        (Some(_), None) => true,
        // Never read it at all, so there is nothing to claim.
        (None, _) => false,
    }
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

/// Resolves the inline size budget for a result: the caller's `maxChars` capped
/// at `MAX_INLINE_OVERRIDE_CHARS`, or the small default when unset. A result
/// larger than this is truncated inline and its full text spilled to a file.
fn resolve_inline_limit(max_chars: Option<u64>) -> usize {
    match max_chars {
        Some(n) => (n as usize).min(MAX_INLINE_OVERRIDE_CHARS),
        None => INLINE_PAGE_CONTENT_MAX_CHARS,
    }
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

    #[test]
    fn resolve_inline_limit_defaults_and_clamps() {
        // No override falls back to the small default.
        assert_eq!(resolve_inline_limit(None), INLINE_PAGE_CONTENT_MAX_CHARS);
        // A caller can raise it, up to the hard ceiling.
        assert_eq!(resolve_inline_limit(Some(50_000)), 50_000);
        // Anything above the ceiling is clamped.
        assert_eq!(
            resolve_inline_limit(Some(10_000_000)),
            MAX_INLINE_OVERRIDE_CHARS
        );
    }

    fn reading(count: u64, installed: bool) -> settle::ReadinessReading {
        settle::ReadinessReading {
            count,
            installed,
            showing: true,
            complete: true,
        }
    }

    /// A read is the common case and says nothing, so the note costs context
    /// only where it carries a decision.
    #[test]
    fn a_read_gets_no_readiness_note() {
        assert!(readiness_note(false, false).is_none());
        assert!(readiness_note(false, true).is_none());
    }

    #[test]
    fn a_settled_change_says_there_is_nothing_left_to_wait_for() {
        let note = readiness_note(true, true).unwrap_or_default();
        assert!(note.contains("went quiet"), "unexpected note {note}");
    }

    /// The case the note exists for: the budget expired with the page still
    /// moving, which is also when a caller is most tempted to pause.
    #[test]
    fn an_unsettled_change_names_the_signal_to_wait_on() {
        let note = readiness_note(true, false).unwrap_or_default();
        assert!(
            note.contains("may predate the effect"),
            "unexpected note {note}"
        );
        assert!(note.contains("`wait`"), "unexpected note {note}");
        assert!(
            !note.contains("sleep"),
            "the note must not bless a pause: {note}"
        );
    }

    #[test]
    fn a_script_that_changed_nothing_does_not_report_a_change() {
        assert!(!page_moved(
            Some(&reading(7, false)),
            Some(&reading(7, false))
        ));
    }

    #[test]
    fn a_script_that_mutated_the_page_reports_a_change() {
        assert!(page_moved(
            Some(&reading(7, false)),
            Some(&reading(9, false))
        ));
    }

    /// A navigation replaces the body, which re-arms the observer and restarts
    /// its count, so an unchanged count across a reinstall proves nothing.
    #[test]
    fn a_reinstalled_observer_reports_a_change() {
        assert!(page_moved(
            Some(&reading(7, false)),
            Some(&reading(0, true))
        ));
    }

    /// The context going away is the usual signature of the script navigating.
    #[test]
    fn losing_the_context_reports_a_change() {
        assert!(page_moved(Some(&reading(7, false)), None));
    }

    /// Nothing was ever read, so there is nothing to claim either way.
    #[test]
    fn an_unreadable_page_claims_nothing() {
        assert!(!page_moved(None, Some(&reading(9, false))));
        assert!(!page_moved(None, None));
    }
}
