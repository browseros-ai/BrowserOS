//! Automatic response settling through the request's captured view.
//!
//! This preserves core's navigation/DOM quiet budgets without giving tools its
//! global PageManager. Every refresh and evaluation retains profile, pin, live
//! context, and target-incarnation admission, including awaited readback work.

use browseros_core::{
    BrowserView, CoreError, PageId, ProtocolSession, settle::SettleOutcome, timeouts,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{future::Future, time::Duration};
use tokio::time::{Instant, sleep, timeout};

#[derive(Debug, Deserialize)]
struct EvaluateResult {
    result: RemoteObject,
}

#[derive(Debug, Deserialize)]
struct RemoteObject {
    value: Option<Value>,
}

const INSTALL_OBSERVER_JS: &str = r#"
(() => {
  const root = document.body || document.documentElement;
  const previous = globalThis.__browseros_settle;
  if (previous && previous.observer) {
    try { previous.observer.disconnect(); } catch (_) {}
  }
  const state = { n: 0, observer: null };
  if (!root || typeof MutationObserver === 'undefined') {
    globalThis.__browseros_settle = state;
    return state.n;
  }
  state.observer = new MutationObserver(() => { state.n += 1; });
  state.observer.observe(root, {
    childList: true,
    subtree: true,
    attributes: true,
    characterData: true
  });
  globalThis.__browseros_settle = state;
  return state.n;
})()
"#;

const READ_COUNTER_JS: &str = r#"
(() => {
  const state = globalThis.__browseros_settle;
  return state && typeof state.n === 'number' ? state.n : 0;
})()
"#;

pub(crate) async fn wait_for_action_settle(
    pages: &BrowserView,
    page_id: PageId,
    budget: Duration,
) -> SettleOutcome {
    if budget.is_zero() {
        return SettleOutcome::BudgetExpired;
    }

    let deadline = Instant::now() + budget;
    let navigation_observed = match detect_navigation(pages, page_id.clone(), deadline).await {
        Ok(observed) => observed,
        Err(outcome) => return outcome,
    };

    if navigation_observed
        && let Err(outcome) = wait_for_tab_load(pages, page_id.clone(), deadline).await
    {
        return outcome;
    }

    match wait_for_dom_quiet(pages, page_id, deadline).await {
        Ok(()) => SettleOutcome::Settled {
            navigation_observed,
        },
        Err(outcome) => outcome,
    }
}

async fn detect_navigation(
    pages: &BrowserView,
    page_id: PageId,
    deadline: Instant,
) -> Result<bool, SettleOutcome> {
    let detect_until = earliest_deadline(
        deadline,
        Instant::now() + timeouts::ACTION_SETTLE_NAVIGATION_DETECT,
    );
    loop {
        let info = refresh_page(pages, page_id.clone(), deadline).await?;
        if tab_is_loading(&info) {
            return Ok(true);
        }
        if Instant::now() >= detect_until {
            return Ok(false);
        }
        sleep_until_budget(
            earliest_deadline(
                detect_until,
                Instant::now() + timeouts::ACTION_SETTLE_NAVIGATION_POLL,
            ),
            deadline,
        )
        .await?;
    }
}

async fn wait_for_tab_load(
    pages: &BrowserView,
    page_id: PageId,
    deadline: Instant,
) -> Result<(), SettleOutcome> {
    loop {
        let info = refresh_page(pages, page_id.clone(), deadline).await?;
        if !tab_is_loading(&info) {
            return Ok(());
        }
        sleep_for_budget(timeouts::ACTION_SETTLE_NAVIGATION_POLL, deadline).await?;
    }
}

async fn wait_for_dom_quiet(
    pages: &BrowserView,
    page_id: PageId,
    deadline: Instant,
) -> Result<(), SettleOutcome> {
    let page = call_with_deadline(pages.page_session(page_id), deadline).await?;
    let mut previous = evaluate_counter(&page.session, INSTALL_OBSERVER_JS, deadline).await?;

    loop {
        sleep_for_budget(timeouts::ACTION_SETTLE_DOM_QUIET, deadline).await?;
        let current = evaluate_counter(&page.session, READ_COUNTER_JS, deadline).await?;
        if current == previous {
            return Ok(());
        }
        previous = current;
    }
}

async fn refresh_page(
    pages: &BrowserView,
    page_id: PageId,
    deadline: Instant,
) -> Result<browseros_core::pages::PageInfo, SettleOutcome> {
    match call_with_deadline(pages.refresh_page(page_id.clone()), deadline).await? {
        Some(info) => Ok(info),
        None => Err(SettleOutcome::Skipped {
            reason: format!("page {page_id} not found"),
        }),
    }
}

async fn evaluate_counter(
    session: &ProtocolSession,
    expression: &str,
    deadline: Instant,
) -> Result<u64, SettleOutcome> {
    let result: EvaluateResult = call_with_deadline(
        session.send(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true }),
        ),
        deadline,
    )
    .await?;
    Ok(result
        .result
        .value
        .as_ref()
        .and_then(Value::as_u64)
        .unwrap_or(0))
}

async fn call_with_deadline<T, Fut>(future: Fut, deadline: Instant) -> Result<T, SettleOutcome>
where
    Fut: Future<Output = Result<T, CoreError>>,
{
    let call_budget = remaining(deadline)?.min(timeouts::ACTION_SETTLE_CDP_CALL_TIMEOUT);
    match timeout(call_budget, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(SettleOutcome::Skipped {
            reason: err.to_string(),
        }),
        Err(_elapsed) => {
            if remaining(deadline).is_err() {
                Err(SettleOutcome::BudgetExpired)
            } else {
                Err(SettleOutcome::Skipped {
                    reason: format!("CDP call timed out after {}ms", call_budget.as_millis()),
                })
            }
        }
    }
}

fn remaining(deadline: Instant) -> Result<Duration, SettleOutcome> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(SettleOutcome::BudgetExpired)
}

async fn sleep_for_budget(duration: Duration, deadline: Instant) -> Result<(), SettleOutcome> {
    let remaining = remaining(deadline)?;
    if remaining < duration {
        sleep(remaining).await;
        return Err(SettleOutcome::BudgetExpired);
    }
    sleep(duration).await;
    Ok(())
}

async fn sleep_until_budget(until: Instant, deadline: Instant) -> Result<(), SettleOutcome> {
    let now = Instant::now();
    if until <= now {
        return Ok(());
    }
    let capped = earliest_deadline(until, deadline);
    sleep_for_budget(capped.duration_since(now), deadline).await
}

fn earliest_deadline(a: Instant, b: Instant) -> Instant {
    if a <= b { a } else { b }
}

fn tab_is_loading(info: &browseros_core::pages::PageInfo) -> bool {
    info.is_loading && info.load_progress < 1.0
}
