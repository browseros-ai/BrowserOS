//! Automatic settling between an action and the diff it returns.
//!
//! The diff reads the accessibility tree, so the settle waits for what changes that tree and
//! nothing else: a new document's DOMContentLoaded, then the page's DOM holding still for one
//! quiet interval. Images, iframes and the load event are not waited on.
//!
//! ```text
//!  mark (before the action) ─▶ main-frame navigation since the mark?    PageSignals, pushed by CDP
//!                               ├─ none ───────────────────────────┐
//!                               └─ wait for its DOMContentLoaded ──┴─▶ readiness probe quiet 100 ms
//! ```
//!
//! The budget is the 600 ms action budget, raised to the arrival ceiling once a navigation is
//! seen: a new document takes longer than an in-page change, and a diff of half of it misleads.
//! Every read goes through the request's captured view, so profile, pin, live context and
//! target-incarnation admission hold for the whole wait.

use browseros_core::{
    BrowserView, CoreError, DocumentMark, DocumentSince, PageId, ProtocolSession,
    settle::{ReadinessReading, SettleOutcome, read_readiness},
    timeouts,
};
use std::{future::Future, time::Duration};
use tokio::{
    sync::watch,
    time::{Instant, sleep, timeout, timeout_at},
};

/// Waits until the effect of an action taken after `mark` has landed on the page.
///
/// Advisory, like every settle: `BudgetExpired` and `Skipped` still let the caller read the
/// page, they only say it may not have finished.
pub(crate) async fn wait_for_action_settle(
    pages: &BrowserView,
    page_id: PageId,
    mark: DocumentMark,
) -> SettleOutcome {
    let started = Instant::now();
    let mut deadline = started + timeouts::ACTION_SETTLE_DEFAULT_TIMEOUT;
    let signals = pages.signals().clone();
    let mut changes = signals.document_changes();
    let page = match call_with_deadline(pages.page_session(page_id.clone()), deadline).await {
        Ok(page) => page,
        Err(outcome) => return outcome,
    };
    let main_frame = page.target_id.as_str().to_string();
    let mut navigation_observed = false;
    let mut previous: Option<u64> = None;
    loop {
        // A dialog freezes the renderer: no document or probe answer comes until it closes.
        if let Some(line) = signals.pending_dialog_line(&page_id) {
            return SettleOutcome::Skipped { reason: line };
        }
        let since = signals.document_since(&page_id, &main_frame, mark);
        if since != DocumentSince::Unchanged && !navigation_observed {
            navigation_observed = true;
            deadline = deadline.max(started + timeouts::ARRIVAL_READY_BUDGET);
        }
        if since == DocumentSince::Loading {
            // The old document is still showing; reading it now would settle on the page
            // being left.
            if let Err(outcome) = wait_for_document_event(&mut changes, deadline).await {
                return outcome;
            }
            previous = None;
            continue;
        }
        match read_with_deadline(&page.session, deadline).await {
            Ok(Some(reading)) => {
                if !reading.installed && previous == Some(reading.count) && reading.settled_enough()
                {
                    return SettleOutcome::Settled {
                        navigation_observed,
                    };
                }
                previous = Some(reading.count);
            }
            Ok(None) => previous = None,
            Err(outcome) => return outcome,
        }
        if let Err(outcome) = sleep_for_budget(timeouts::ACTION_SETTLE_DOM_QUIET, deadline).await {
            return outcome;
        }
    }
}

/// Waits for the next document event on any page, for at most one quiet interval, so the
/// caller re-checks dialogs and its own page often even if an event never comes.
async fn wait_for_document_event(
    changes: &mut watch::Receiver<u64>,
    deadline: Instant,
) -> Result<(), SettleOutcome> {
    let until = deadline.min(Instant::now() + timeouts::ACTION_SETTLE_DOM_QUIET);
    match timeout_at(until, changes.changed()).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_closed)) => Err(SettleOutcome::Skipped {
            reason: "page signals stopped".to_string(),
        }),
        Err(_elapsed) if Instant::now() >= deadline => Err(SettleOutcome::BudgetExpired),
        Err(_elapsed) => Ok(()),
    }
}

/// One readiness reading; `None` when the page could not answer this time, as while a
/// committing navigation replaces its context. Only a lost target ends the wait early, the way
/// goto's own readiness wait treats it.
async fn read_with_deadline(
    session: &ProtocolSession,
    deadline: Instant,
) -> Result<Option<ReadinessReading>, SettleOutcome> {
    let call_budget = remaining(deadline)?.min(timeouts::ACTION_SETTLE_CDP_CALL_TIMEOUT);
    match timeout(call_budget, read_readiness(session)).await {
        Ok(Ok(reading)) => Ok(Some(reading)),
        Ok(Err(error @ CoreError::Target(_))) => Err(SettleOutcome::Skipped {
            reason: error.to_string(),
        }),
        Ok(Err(_)) => Ok(None),
        Err(_elapsed) => {
            remaining(deadline)?;
            Ok(None)
        }
    }
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
