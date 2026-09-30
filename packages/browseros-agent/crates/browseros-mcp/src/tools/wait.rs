use crate::framework::{
    ToolCtx, ToolExecResult, ToolResult, abortable_delay, clamp_timeout, error_result, parse_args,
    pending_dialog_result, text_result,
};
use browseros_core::PageId;
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub const DEFAULT_PAUSE_MS: u64 = 2_000;
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 2_000;
const MAX_WAIT_TIMEOUT_MS: u64 = 30_000;
// A for="time" pause does no page work (it is an abortable sleep), so it is not
// bound by the 30s page-work polling cap that text/selector waits use. An explicit
// timeout still bounds it from above; a value past this ceiling is rejected (naming
// the cap) rather than silently clamped, so a caller pacing a long in-page loop
// never under-waits without knowing (#2701). Kept below the tool-call budget so the
// documented maximum completes rather than racing the outer timeout.
const MAX_TIME_WAIT_MS: u64 = 90_000;
const DESCRIPTION: &str = "\
Wait on a signal: for=\"text\" (substring appears) or for=\"selector\" (CSS selector matches) \
beat a blind pause. for=\"time\" (default) pauses value ms (default 2000, honored up to 90000; \
an explicit timeout caps it lower, and a larger value is rejected, not silently shortened) - last resort. \
Best of all: act and read the diff instead of waiting.";

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum WaitFor {
    Text,
    Selector,
    #[default]
    Time,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
enum WaitValue {
    String(String),
    Number(f64),
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WaitArgs {
    page: u32,
    /// What to wait for. Defaults to "time" (a fixed pause).
    #[serde(default)]
    #[serde(rename = "for")]
    wait_for: WaitFor,
    /// Optional. For for="time", ms to pause (default 2000, honored up to 90000). For "text"/"selector", the substring or CSS selector to wait for.
    value: Option<WaitValue>,
    /// Max wait in ms. For "text"/"selector" it caps polling before giving up (default 2000, capped at 30000). For "time" it optionally caps the pause from above (default: pause for value).
    timeout: Option<f64>,
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
    super::def::<WaitArgs>(
        "wait",
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
        let args: WaitArgs = parse_args(raw)?;
        let value = args.value.as_ref().map(wait_value_to_string);
        if matches!(args.wait_for, WaitFor::Time) {
            let wait_ms = match resolve_time_wait_ms(value.as_deref(), args.timeout) {
                Ok(wait_ms) => wait_ms,
                Err(message) => return Ok(Some(error_result(message))),
            };
            abortable_delay(ctx, Duration::from_millis(wait_ms)).await?;
            return Ok(Some(text_result(
                format!("waited {wait_ms}ms"),
                Some(json!({ "matched": true, "waitedMs": wait_ms })),
            )));
        }
        let timeout = clamp_timeout(args.timeout, DEFAULT_WAIT_TIMEOUT_MS, MAX_WAIT_TIMEOUT_MS);
        let Some(value) = value.filter(|value| !value.is_empty()) else {
            return Ok(Some(error_result(format!(
                "wait: \"value\" is required for for=\"{}\" (the text or CSS selector to wait for). To just pause, use for=\"time\".",
                wait_for_name(&args.wait_for)
            ))));
        };
        if let Some(result) = pending_dialog_result(ctx, PageId(args.page)) {
            return Ok(Some(result));
        }
        let page = ctx.session.pages.get_session(PageId(args.page)).await?;
        let expression = match args.wait_for {
            WaitFor::Text => format!(
                "(document.body?.innerText ?? '').includes({})",
                serde_json::to_string(&value)?
            ),
            WaitFor::Selector => format!(
                "!!document.querySelector({})",
                serde_json::to_string(&value)?
            ),
            WaitFor::Time => String::new(),
        };
        let deadline = Instant::now() + Duration::from_millis(timeout);
        while Instant::now() < deadline {
            ctx.throw_if_cancelled()?;
            let result: EvaluateResult = page
                .session
                .send(
                    "Runtime.evaluate",
                    json!({ "expression": expression, "returnByValue": true }),
                )
                .await?;
            if result.result.value.as_ref().and_then(Value::as_bool) == Some(true) {
                return Ok(Some(text_result(
                    format!("matched ({})", wait_for_name(&args.wait_for)),
                    Some(json!({ "matched": true })),
                )));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            abortable_delay(ctx, remaining.min(Duration::from_millis(300))).await?;
        }
        Ok(Some(text_result(
            format!(
                "timed out after {timeout}ms waiting for {}",
                wait_for_name(&args.wait_for)
            ),
            Some(json!({ "matched": false })),
        )))
    })
}

/// Resolves a for="time" pause: the requested ms (optionally capped by an explicit
/// timeout), or an error message when it exceeds the ceiling. An explicit timeout
/// still bounds the pause from above, but the pause is no longer forced down to the
/// 30s page-work polling cap; a value past the ceiling is rejected (rather than
/// silently clamped) so a caller never under-waits without knowing (#2701).
pub fn resolve_time_wait_ms(value: Option<&str>, timeout: Option<f64>) -> Result<u64, String> {
    let mut wait_ms = parse_wait_ms(value, DEFAULT_PAUSE_MS);
    if let Some(timeout) = timeout
        && timeout.is_finite()
        && timeout >= 0.0
    {
        wait_ms = wait_ms.min(timeout.round() as u64);
    }
    if wait_ms > MAX_TIME_WAIT_MS {
        return Err(format!(
            "wait: {wait_ms}ms exceeds the {MAX_TIME_WAIT_MS}ms cap for for=\"time\". Pass {MAX_TIME_WAIT_MS} or less, cap it with timeout, or split it into shorter pauses."
        ));
    }
    Ok(wait_ms)
}

pub fn parse_wait_ms(value: Option<&str>, fallback: u64) -> u64 {
    let Some(value) = value else {
        return fallback;
    };
    if value.trim().is_empty() {
        return fallback;
    }
    let Ok(ms) = value.parse::<f64>() else {
        return fallback;
    };
    if !ms.is_finite() || ms < 0.0 {
        return fallback;
    }
    ms.round() as u64
}

fn wait_value_to_string(value: &WaitValue) -> String {
    match value {
        WaitValue::String(value) => value.clone(),
        WaitValue::Number(value) => value.to_string(),
    }
}

fn wait_for_name(wait_for: &WaitFor) -> &'static str {
    match wait_for {
        WaitFor::Text => "text",
        WaitFor::Selector => "selector",
        WaitFor::Time => "time",
    }
}
