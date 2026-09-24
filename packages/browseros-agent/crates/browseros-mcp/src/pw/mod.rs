//! Playwright bridge routing; leaf implementations share the legacy script host hooks.

mod actions;
mod expect;
mod waits;

#[cfg(test)]
mod actions_tests;
#[cfg(test)]
mod expect_tests;
#[cfg(test)]
mod facade_tests;
#[cfg(test)]
mod waits_tests;

use crate::tools::run::{BrowserBridge, BrowserCallValue};
use browseros_core::{CoreError, PageId, locator::Deadline};
use serde_json::{Value, json};
use std::{future::Future, time::Duration};
use tokio::time::Instant;

pub const FACADE_JS: &str = include_str!("facade.js");

pub(crate) fn is_pw_method(method: &str) -> bool {
    [
        "page.",
        "locator.",
        "expect.",
        "context.",
        "keyboard.",
        "mouse.",
        "frame.",
        "neo.",
    ]
    .iter()
    .any(|prefix| method.starts_with(prefix))
}

/// Leaf results carry host side effects without duplicating audit or page-claim ordering.
pub(crate) struct PwCallOutcome {
    pub value: BrowserCallValue,
    pub audit_args: Option<Value>,
    pub created_page: Option<u32>,
    pub secrets: Vec<String>,
}

impl PwCallOutcome {
    pub(crate) fn json(value: Value) -> Self {
        Self {
            value: BrowserCallValue::Json(value),
            audit_args: None,
            created_page: None,
            secrets: Vec::new(),
        }
    }
}

pub(crate) async fn dispatch(
    bridge: &BrowserBridge,
    method: &str,
    args: Vec<Value>,
) -> Result<PwCallOutcome, String> {
    match route(method) {
        Route::Expect => expect::dispatch(bridge, method, &args).await,
        Route::Waits => waits::dispatch(bridge, method, &args).await,
        Route::Actions => actions::dispatch(bridge, method, &args).await,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Route {
    Expect,
    Waits,
    Actions,
}

fn route(method: &str) -> Route {
    match method {
        m if m.starts_with("expect.") => Route::Expect,
        "page.goto"
        | "page.reload"
        | "page.goBack"
        | "page.goForward"
        | "page.waitForLoadState"
        | "page.waitForURL"
        | "page.waitForFunction"
        | "page.waitForEvent"
        | "page.frames"
        | "page.dialog"
        | "context.waitForEvent"
        | "locator.waitFor" => Route::Waits,
        _ => Route::Actions,
    }
}

#[allow(
    dead_code,
    reason = "P4-P6 consume the shared options when replacing leaf stubs"
)]
pub(crate) struct Opts {
    pub timeout_ms: Option<u64>,
    pub strict: bool,
    pub force: bool,
    pub wait_until: Option<String>,
    pub exact: Option<bool>,
}

#[allow(
    dead_code,
    reason = "P4-P6 consume the shared options when replacing leaf stubs"
)]
pub(crate) fn opts(args: &[Value], index: usize) -> Opts {
    let value = args.get(index);
    Opts {
        timeout_ms: value.and_then(|v| v.get("timeout")).and_then(Value::as_u64),
        strict: value
            .and_then(|v| v.get("strict"))
            .and_then(Value::as_bool)
            .unwrap_or(true),
        force: value
            .and_then(|v| v.get("force"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        wait_until: value
            .and_then(|v| v.get("waitUntil"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        exact: value.and_then(|v| v.get("exact")).and_then(Value::as_bool),
    }
}

#[allow(
    dead_code,
    reason = "P4-P6 consume deadlines when replacing leaf stubs"
)]
pub(crate) fn deadline(bridge: &BrowserBridge, opts: &Opts, default_ms: u64) -> Deadline {
    Deadline {
        at: clamp_deadline(
            Instant::now(),
            bridge.control.deadline,
            opts.timeout_ms.unwrap_or(default_ms),
        ),
        cancel: bridge.control.cancel.clone(),
    }
}

fn clamp_deadline(now: Instant, run_deadline: Instant, timeout_ms: u64) -> Instant {
    // Playwright's timeout:0 removes the per-operation cap, never the run cap.
    if timeout_ms == 0 {
        run_deadline
    } else {
        now.checked_add(Duration::from_millis(timeout_ms))
            .unwrap_or(run_deadline)
            .min(run_deadline)
    }
}

pub(crate) fn page_arg(args: &[Value]) -> Result<PageId, String> {
    args.first()
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .filter(|id| *id > 0)
        .map(PageId)
        .ok_or_else(|| "pageId must be a positive 32-bit integer".to_string())
}

pub(crate) fn string_arg<'a>(
    args: &'a [Value],
    index: usize,
    name: &str,
) -> Result<&'a str, String> {
    args.get(index)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} argument is required"))
}

pub(crate) fn selector_arg(args: &[Value]) -> Result<&str, String> {
    string_arg(args, 1, "selector")
}

/// All protocol work in a leaf shares this budget, including probes and cleanup.
/// The injected engine observes the same cancellation token while polling states.
pub(crate) async fn bounded<T>(
    dl: &Deadline,
    future: impl Future<Output = Result<T, CoreError>>,
) -> Result<T, CoreError> {
    tokio::select! {
        biased;
        () = dl.cancel.cancelled() => Err(CoreError::Message("cancelled".into())),
        () = tokio::time::sleep_until(dl.at) => Err(CoreError::Message("operation timed out".into())),
        result = future => result,
    }
}

/// Mutating leaves return this metadata so the facade can refresh its synchronous
/// URL cache. Value-returning leaves keep their original JSON value contract.
pub(crate) async fn page_info(
    bridge: &BrowserBridge,
    page: PageId,
    dl: &Deadline,
) -> Result<Value, CoreError> {
    let info = bounded(dl, bridge.ctx.session.pages.refresh(page.clone()))
        .await?
        .ok_or(CoreError::UnknownPage(page))?;
    Ok(json!({"pageId": info.page_id.0, "url": info.url, "title": info.title}))
}

pub(crate) fn action_error(
    method: &str,
    selector: Option<&str>,
    timeout_ms: u64,
    error: CoreError,
) -> String {
    let detail = match error {
        CoreError::ElementCovered { blocker, .. } => {
            format!("<{blocker}> intercepts pointer events")
        }
        error => error.to_string(),
    };
    let lower = detail.to_ascii_lowercase();
    let headline = if lower.contains("timed out") || lower.contains("timeout") {
        format!("TimeoutError: {method}: Timeout {timeout_ms}ms exceeded.")
    } else {
        format!("Error: {method}: {detail}")
    };
    let waiting = selector
        .map(|s| format!("\n  - waiting for {s}"))
        .unwrap_or_default();
    format!("{headline}\nCall log:{waiting}\n  - {detail}")
}
