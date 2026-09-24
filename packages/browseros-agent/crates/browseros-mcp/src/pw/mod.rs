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
use std::{future::Future, sync::Mutex, time::Duration};
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

/// Failed leaves retain redaction facts until the host records the child call
/// and masks the parent script. A message alone loses those audit side effects.
pub(crate) struct PwCallError {
    pub message: String,
    pub audit_args: Option<Value>,
    pub secrets: Vec<String>,
}

impl From<String> for PwCallError {
    fn from(message: String) -> Self {
        Self {
            message,
            audit_args: None,
            secrets: Vec::new(),
        }
    }
}

/// Run-local activity is shared by cloned bridge futures. Ownership and liveness
/// are checked afresh when selecting a page, so closed or transferred tabs cannot
/// become the facade's lazy default page.
#[derive(Default)]
pub(crate) struct PageActivity(Mutex<Vec<u32>>);

impl PageActivity {
    pub(crate) fn touch(&self, page: u32) {
        let mut recent = self.0.lock().unwrap_or_else(|e| e.into_inner());
        recent.retain(|id| *id != page);
        recent.push(page);
    }

    pub(crate) fn last_owned(&self, pages: &[Value]) -> Option<u32> {
        let owned: Vec<u32> = pages
            .iter()
            .filter(|page| page["ownership"] == "mine")
            .filter_map(|page| {
                page["pageId"]
                    .as_u64()
                    .and_then(|id| u32::try_from(id).ok())
            })
            .collect();
        let recent = self.0.lock().unwrap_or_else(|e| e.into_inner());
        recent
            .iter()
            .rev()
            .find(|id| owned.contains(id))
            .copied()
            // The host exposes ownership but not prior-run activity timestamps.
            // Before this run touches a tab, prefer the newest still-open own tab.
            .or_else(|| owned.last().copied())
    }
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
) -> Result<PwCallOutcome, PwCallError> {
    if method == "context.waitForEvent" {
        return context_page_wait(bridge, &args).await;
    }
    match route(method) {
        Route::Expect => expect::dispatch(bridge, method, &args)
            .await
            .map_err(Into::into),
        Route::Waits => waits::dispatch(bridge, method, &args)
            .await
            .map_err(Into::into),
        Route::Actions => actions::dispatch(bridge, method, &args).await,
    }
}

/// The context facade has no page argument. P5's `page` event observes all new
/// tabs (unlike `popup`, which filters by opener), so any live page is an anchor.
/// Call the leaf directly to preserve one authorization and one audit record.
async fn context_page_wait(
    bridge: &BrowserBridge,
    args: &[Value],
) -> Result<PwCallOutcome, PwCallError> {
    if args.first().and_then(Value::as_str) != Some("page") {
        return Err("context.waitForEvent supports only the page event"
            .to_owned()
            .into());
    }
    let options = opts(args, 2);
    let dl = deadline(bridge, &options, 10_000);
    let result = bounded(&dl, async {
        let pages = bridge.ctx.session.pages.list().await?;
        if let Some(page) = pages.first() {
            let translated = [
                json!(page.page_id.0),
                json!("page"),
                args.get(1).cloned().unwrap_or(Value::Null),
                args.get(2).cloned().unwrap_or(Value::Null),
            ];
            return waits::dispatch(bridge, "page.waitForEvent", &translated)
                .await
                .map_err(CoreError::from);
        }
        // There is no opener to anchor P5 when the browser has zero live tabs.
        // The first tab to appear is itself the awaited context page event.
        loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if let Some(page) = bridge.ctx.session.pages.list().await?.first() {
                let mut outcome = PwCallOutcome::json(json!(page.page_id.0));
                outcome.created_page = Some(page.page_id.0);
                return Ok(outcome);
            }
        }
    })
    .await;
    result.map_err(|error| {
        action_error(
            "context.waitForEvent",
            None,
            options.timeout_ms.unwrap_or(10_000),
            error,
        )
        .into()
    })
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
