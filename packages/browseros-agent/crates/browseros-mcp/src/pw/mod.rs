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
use browseros_core::locator::Deadline;
use serde_json::Value;
use std::time::Duration;
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

pub(crate) async fn dispatch(
    bridge: &BrowserBridge,
    method: &str,
    args: Vec<Value>,
) -> Result<PwCallOutcome, String> {
    match method {
        m if m.starts_with("expect.") => expect::dispatch(bridge, m, &args).await,
        "page.goto"
        | "page.reload"
        | "page.goBack"
        | "page.goForward"
        | "page.waitForLoadState"
        | "page.waitForURL"
        | "page.waitForFunction"
        | "page.waitForEvent"
        | "page.dialog"
        | "locator.waitFor" => waits::dispatch(bridge, method, &args).await,
        _ => actions::dispatch(bridge, method, &args).await,
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
    let remaining = bridge
        .control
        .deadline
        .saturating_duration_since(Instant::now());
    let duration = Duration::from_millis(opts.timeout_ms.unwrap_or(default_ms)).min(remaining);
    Deadline {
        at: (Instant::now() + duration).min(bridge.control.deadline),
        cancel: bridge.control.cancel.clone(),
    }
}
