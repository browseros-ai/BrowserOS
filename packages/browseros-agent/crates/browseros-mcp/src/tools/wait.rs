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
const DESCRIPTION: &str = "\
Wait on a signal: for=\"text\" (substring appears) or for=\"selector\" (CSS selector matches) \
beat a blind pause. for=\"time\" (default) pauses value ms (default 2000) - last resort. \
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
struct WaitArgs {
    page: u32,
    /// What to wait for. Defaults to "time" (a fixed pause).
    #[serde(default)]
    #[serde(rename = "for")]
    wait_for: WaitFor,
    /// Optional. For for="time", ms to pause (default 2000). For "text"/"selector", the substring or CSS selector to wait for.
    value: Option<WaitValue>,
    /// Max wait in ms before giving up (default 2000).
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
        let timeout = clamp_timeout(args.timeout, DEFAULT_WAIT_TIMEOUT_MS, MAX_WAIT_TIMEOUT_MS);
        let value = args.value.as_ref().map(wait_value_to_string);
        if matches!(args.wait_for, WaitFor::Time) {
            let wait_ms = parse_wait_ms(value.as_deref(), DEFAULT_PAUSE_MS).min(timeout);
            abortable_delay(ctx, Duration::from_millis(wait_ms)).await?;
            return Ok(Some(text_result(
                format!("waited {wait_ms}ms"),
                Some(json!({ "matched": true, "waitedMs": wait_ms })),
            )));
        }
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
