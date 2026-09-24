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

/// Cache maintenance remains authorized but is not an agent-visible action.
pub(crate) fn is_silent_method(method: &str) -> bool {
    matches!(method, "page.info" | "context.lastPage")
}

/// Preserve the Playwright API name at the audit seam while retaining the
/// compact query protocol for dispatch. Unknown queries keep their wire name.
pub(crate) fn audit_method(method: &str, args: &[Value]) -> String {
    if method == "locator.query"
        && let Some(what) = args.get(2).and_then(Value::as_str)
        && matches!(
            what,
            "count"
                | "allInnerTexts"
                | "allTextContents"
                | "textContent"
                | "innerText"
                | "innerHTML"
                | "inputValue"
                | "getAttribute"
                | "isVisible"
                | "isHidden"
                | "isEnabled"
                | "isChecked"
                | "isEditable"
                | "boundingBox"
                | "ariaSnapshot"
        )
    {
        return format!("locator.{what}");
    }
    method.to_owned()
}

pub(crate) fn captures_new_pages(method: &str) -> bool {
    matches!(
        method,
        "locator.click" | "locator.dblclick" | "keyboard.press" | "mouse.click" | "page.goto"
    )
}

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
pub(crate) struct PageActivity {
    recent: Mutex<Vec<u32>>,
    claimed: tokio::sync::Mutex<std::collections::HashSet<u32>>,
}

impl PageActivity {
    pub(crate) fn touch(&self, page: u32) {
        let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        recent.retain(|id| *id != page);
        recent.push(page);
    }

    /// An explicit popup wait and its triggering click can discover the same
    /// page concurrently. The host must claim/group that page only once per run.
    pub(crate) async fn claim(&self, page: u32, hook: &dyn crate::framework::InnerCallHook) {
        let mut claimed = self.claimed.lock().await;
        if !claimed.contains(&page) {
            // Hold the async lock until ownership is established. A concurrent
            // click/wait must not resolve while the first claimant is still pending.
            hook.on_page_created(page).await;
            claimed.insert(page);
        }
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
        let recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
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

    /// Only action envelopes can supply discovered pages. A page's evaluated
    /// JSON or a raw neo.cdp response must never be interpreted as a claim.
    pub(crate) fn created_pages(&self, method: &str) -> Vec<u32> {
        let mut ids: Vec<u32> = self.created_page.into_iter().collect();
        if captures_new_pages(method)
            && let BrowserCallValue::Json(value) = &self.value
            && let Some(pages) = value.get("newPages").and_then(Value::as_array)
        {
            for page in pages {
                if let Some(id) = page["pageId"]
                    .as_u64()
                    .and_then(|id| u32::try_from(id).ok())
                    && !ids.contains(&id)
                {
                    ids.push(id);
                }
            }
        }
        ids
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
    if captures_new_pages(method) {
        let index = if method == "mouse.click" { 3 } else { 2 };
        let options = opts(&args, index);
        let dl = deadline(bridge, &options, 10_000);
        // This budget encloses both discovery and the leaf's own action budget;
        // popup bookkeeping must never extend a caller's requested timeout.
        return tokio::select! {
            biased;
            () = dl.cancel.cancelled() => Err("cancelled".to_owned().into()),
            () = tokio::time::sleep_until(dl.at) => Err(action_error(method, args.get(1).and_then(Value::as_str), options.timeout_ms.unwrap_or(10_000), CoreError::from("operation timed out")).into()),
            result = async {
                let before = bridge.ctx.session.pages.list().await.unwrap_or_default();
                let mut result = dispatch_leaf(bridge, method, &args).await?;
                // A completed action remains successful if its tab closed during
                // the final metadata read. Only verified opener relationships
                // are returned, so unrelated concurrently opened tabs stay unclaimed.
                if let Ok(new_pages) = opened_pages(bridge, &args, &before).await
                    && !new_pages.is_empty()
                {
                    match &mut result.value {
                        BrowserCallValue::Json(value) if value.is_object() => value["newPages"] = json!(new_pages),
                        value => {
                            let original = match std::mem::replace(value, BrowserCallValue::Undefined) {
                                BrowserCallValue::Json(value) => value,
                                BrowserCallValue::Undefined => Value::Null,
                            };
                            *value = BrowserCallValue::Json(json!({"value":original,"newPages":new_pages}));
                        }
                    }
                }
                Ok(result)
            } => result,
        };
    }
    dispatch_leaf(bridge, method, &args).await
}

async fn opened_pages(
    bridge: &BrowserBridge,
    args: &[Value],
    before: &[browseros_core::pages::PageInfo],
) -> Result<Vec<Value>, CoreError> {
    let page = page_arg(args).map_err(CoreError::from)?;
    let Some(opener) = before.iter().find(|info| info.page_id == page) else {
        return Ok(Vec::new());
    };
    let after = bridge.ctx.session.pages.list().await?;
    let mut opened = Vec::new();
    for info in after {
        if before.iter().any(|old| old.page_id == info.page_id) {
            continue;
        }
        // Target metadata retains the browser's opener identity even when page
        // JavaScript cannot access window.opener (for example target=_blank).
        let target = bridge
            .ctx
            .session
            .cdp(
                "Target.getTargetInfo",
                json!({"targetId":info.target_id.as_str()}),
                None,
            )
            .await?;
        if target["targetInfo"]["openerId"].as_str() == Some(opener.target_id.as_str()) {
            opened.push(json!({"pageId":info.page_id.0,"url":info.url,"title":info.title}));
        }
    }
    Ok(opened)
}

async fn dispatch_leaf(
    bridge: &BrowserBridge,
    method: &str,
    args: &[Value],
) -> Result<PwCallOutcome, PwCallError> {
    match route(method) {
        Route::Expect => expect::dispatch(bridge, method, args)
            .await
            .map_err(Into::into),
        Route::Waits => waits::dispatch(bridge, method, args)
            .await
            .map_err(Into::into),
        Route::Actions => actions::dispatch(bridge, method, args).await,
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
