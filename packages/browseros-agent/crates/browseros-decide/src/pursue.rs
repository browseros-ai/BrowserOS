//! The loop, and what ends it.
//!
//! Code owns the control flow here. The model answers one narrow question set
//! per step and never decides what happens next: whether to act, whether to
//! look further, whether to stop and whether to hand back are all decided from
//! the answer plus what the page actually did.
//!
//! Termination is observed rather than judged. The previous implementation
//! asked the model how complete the goal was and thresholded the answer, and
//! that number is documented as not being interpolatable between its levels; it
//! reported a goal met when one of two filters had been applied, and a goal
//! unmet when it had been finished. So nothing here rests on it.

use std::collections::BTreeMap;

use crate::client::{Jev, JevError, Question};
use crate::gate::{self, Decision, Verdict};
use crate::operations::Operation;
use crate::questions;
use crate::space::ActionSpace;
use crate::view::PageView;

/// How many consecutive actions may change nothing before the run stops.
pub const STALL_LIMIT: u32 = 3;

/// How many times a stale decision may be retried before giving up. A page that
/// changes under every decision is not one this can finish.
pub const STALE_RETRIES: u32 = 3;

/// What went wrong when an operation ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActError {
    /// The page moved under the decision. Observe again and decide again.
    Stale(String),
    /// Nothing further is worth trying.
    ///
    /// Includes the case where a mutation may or may not have happened: a
    /// select whose change event may already have fired must not be retried, or
    /// the value gets set twice.
    Fatal(String),
}

/// Where answers come from.
///
/// A trait rather than the client directly, so the loop can be driven by a
/// script in a test. The loop's job is deciding what to do with an answer, and
/// that is the part worth testing without a network.
pub trait Oracle {
    fn ask(
        &self,
        state: &serde_json::Value,
        questions: &BTreeMap<String, Question>,
    ) -> impl Future<Output = Result<crate::client::Response, JevError>>;
}

impl Oracle for Jev {
    async fn ask(
        &self,
        state: &serde_json::Value,
        questions: &BTreeMap<String, Question>,
    ) -> Result<crate::client::Response, JevError> {
        Jev::ask(self, state, questions).await
    }
}

/// What the browser has to be able to do for the loop to run.
pub trait Driver {
    /// Whether the run has been cancelled.
    fn stopped(&self) -> bool;

    /// The page as it is now.
    fn observe(&self) -> impl Future<Output = Result<PageView, String>>;

    /// Runs one operation. Returns whether the page changed.
    fn act(&self, decision: &Decision) -> impl Future<Output = Result<bool, ActError>>;

    /// Settles the page after an input, in code rather than by spending a
    /// decision on a wait.
    fn settle(&self) -> impl Future<Output = ()>;
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The model said every requirement was met, on a page that had not moved
    /// since it looked.
    Satisfied,
    /// Nothing offered could advance the goal.
    Blocked(String),
    /// Three consecutive actions changed nothing.
    Stalled,
    /// The caller has to supply something before this can continue.
    NeedsInput(String),
    /// The step or time budget ran out.
    OutOfBudget,
    /// Cancelled.
    Stopped,
    /// Something failed outright.
    Failed(String),
}

/// One executed action, and what it did.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub operation: Operation,
    pub target: Option<String>,
    pub value: Option<String>,
    pub confidence: f64,
    pub page_changed: bool,
    pub url_after: String,
    pub input_tokens: u64,
}

/// What a run did, stated so a caller can check it rather than trust it.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub status: Status,
    /// Every decision asked for, including those that ended before acting.
    pub decisions: u32,
    pub trail: Vec<Step>,
    pub url_before: String,
    pub url_after: String,
    pub title: String,
    pub input_tokens: u64,
}

impl Outcome {
    /// How many actions changed the page. The honest summary: a caller can
    /// compare this with what the goal asked for, which it cannot do with a
    /// status label.
    #[must_use]
    pub fn changed(&self) -> usize {
        self.trail.iter().filter(|step| step.page_changed).count()
    }

    #[must_use]
    pub fn actions(&self) -> usize {
        self.trail.len()
    }
}

/// Limits on one run, which come from the user's settings rather than from
/// constants here.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub max_steps: u32,
    pub max_seconds: u64,
}

/// Pursues one goal on one page.
pub async fn pursue<O: Oracle, D: Driver>(
    oracle: &O,
    driver: &D,
    goal: &str,
    budget: Budget,
) -> Outcome {
    let started = std::time::Instant::now();
    let mut trail = Vec::new();
    let mut recent: Vec<String> = Vec::new();
    let mut decisions = 0;
    let mut input_tokens = 0;
    let mut stalled = 0;
    let mut last_acted: Option<String> = None;

    let first = match driver.observe().await {
        Ok(view) => view,
        Err(error) => {
            return Outcome {
                status: Status::Failed(error),
                decisions,
                trail,
                url_before: String::new(),
                url_after: String::new(),
                title: String::new(),
                input_tokens,
            };
        }
    };
    let url_before = first.url.clone();
    let mut view = first;

    let finish =
        |status: Status, trail: Vec<Step>, view: &PageView, decisions: u32, input_tokens: u64| {
            Outcome {
                status,
                decisions,
                trail,
                url_before: url_before.clone(),
                url_after: view.url.clone(),
                title: view.title.clone(),
                input_tokens,
            }
        };

    loop {
        if driver.stopped() {
            return finish(Status::Stopped, trail, &view, decisions, input_tokens);
        }
        if trail.len() >= budget.max_steps as usize
            || started.elapsed().as_secs() >= budget.max_seconds
        {
            return finish(Status::OutOfBudget, trail, &view, decisions, input_tokens);
        }

        let space = ActionSpace::build(&view, goal, last_acted.as_deref());
        let operations = questions::available(&space, &view);
        let built = questions::build(goal, &space, &operations);
        let state = questions::state(&view, &space, &recent);

        decisions += 1;
        let response = match ask_with_retries(oracle, &state, &built).await {
            Ok(response) => response,
            Err(error) => {
                return finish(
                    Status::Failed(error.to_string()),
                    trail,
                    &view,
                    decisions,
                    input_tokens,
                );
            }
        };
        input_tokens += response.usage.input_tokens;

        let decision = match gate::read(&response) {
            Ok(decision) => decision,
            Err(error) => {
                return finish(
                    Status::Failed(error.to_string()),
                    trail,
                    &view,
                    decisions,
                    input_tokens,
                );
            }
        };

        // A terminal answer is checked against the page before it is accepted.
        // The model looked at a page; if that page has moved since, its verdict
        // is about something that is no longer there.
        if decision.operation.is_terminal() {
            let settled = refresh(driver, &view).await;
            let fresh = settled.is_fresh_for(view.fingerprint);
            view = settled;
            if decision.operation == Operation::Done && fresh {
                return finish(Status::Satisfied, trail, &view, decisions, input_tokens);
            }
            if decision.operation == Operation::Blocked {
                return finish(
                    Status::Blocked("no offered operation could advance the goal".to_string()),
                    trail,
                    &view,
                    decisions,
                    input_tokens,
                );
            }
            // Done on a page that moved: look again rather than assert.
            continue;
        }

        let target_name = decision
            .target
            .as_deref()
            .and_then(|reference| view.control(reference))
            .map(|control| control.name.clone());

        match gate::verdict(&decision, target_name.as_deref()) {
            Verdict::Execute => {}
            Verdict::HandBack(reason) => {
                return finish(
                    Status::NeedsInput(reason),
                    trail,
                    &view,
                    decisions,
                    input_tokens,
                );
            }
            Verdict::Explore(reason) => {
                // Exploring is only progress if there is somewhere to look.
                if space.omitted == 0 && view.text.is_empty() {
                    return finish(
                        Status::Blocked(reason),
                        trail,
                        &view,
                        decisions,
                        input_tokens,
                    );
                }
                stalled += 1;
                if stalled >= STALL_LIMIT {
                    return finish(Status::Stalled, trail, &view, decisions, input_tokens);
                }
                view = refresh(driver, &view).await;
                continue;
            }
        }

        // The decision was made about a particular control. Check that control,
        // not the whole page: a live page changes constantly in places that have
        // nothing to do with it.
        if let Some(reference) = decision.target.as_deref() {
            let guard = space.candidate(reference).map(|candidate| candidate.guard);
            if let Some(guard) = guard
                && !view.control_is_fresh(reference, guard)
            {
                view = refresh(driver, &view).await;
                continue;
            }
        }

        let changed = match driver.act(&decision).await {
            Ok(changed) => changed,
            Err(ActError::Stale(_)) => {
                stalled += 1;
                if stalled > STALE_RETRIES {
                    return finish(Status::Stalled, trail, &view, decisions, input_tokens);
                }
                view = refresh(driver, &view).await;
                continue;
            }
            Err(ActError::Fatal(reason)) => {
                return finish(
                    Status::Failed(reason),
                    trail,
                    &view,
                    decisions,
                    input_tokens,
                );
            }
        };

        driver.settle().await;
        view = refresh(driver, &view).await;

        recent.push(describe(&decision, &target_name, changed));
        if recent.len() > 5 {
            recent.remove(0);
        }
        last_acted = decision.target.clone();
        trail.push(Step {
            operation: decision.operation,
            target: decision.target.clone(),
            value: decision.value.clone(),
            confidence: decision.confidence(),
            page_changed: changed,
            url_after: view.url.clone(),
            input_tokens: decision.input_tokens,
        });

        // Only a step that changed nothing counts toward the stall. A wait is
        // not an action here, so it cannot be one of the three.
        if changed {
            stalled = 0;
        } else {
            stalled += 1;
            if stalled >= STALL_LIMIT {
                return finish(Status::Stalled, trail, &view, decisions, input_tokens);
            }
        }
    }
}

/// Retries only what retrying can fix.
async fn ask_with_retries<O: Oracle>(
    oracle: &O,
    state: &serde_json::Value,
    questions: &BTreeMap<String, Question>,
) -> Result<crate::client::Response, JevError> {
    let mut attempt: u64 = 0;
    loop {
        match oracle.ask(state, questions).await {
            Ok(response) => return Ok(response),
            Err(error) if error.is_retryable() && attempt < 2 => {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis(250 * attempt)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Re-observes, keeping the last view if the browser cannot answer.
async fn refresh<D: Driver>(driver: &D, previous: &PageView) -> PageView {
    driver.observe().await.unwrap_or_else(|_| previous.clone())
}

fn describe(decision: &Decision, target_name: &Option<String>, changed: bool) -> String {
    let what = target_name
        .clone()
        .or_else(|| decision.target.clone())
        .unwrap_or_else(|| "the page".to_string());
    let value = decision
        .value
        .as_deref()
        .map(|value| format!(" to {value:?}"))
        .unwrap_or_default();
    format!(
        "{} {what}{value}: {}",
        decision.operation.as_str(),
        if changed {
            "the page changed"
        } else {
            "nothing changed"
        }
    )
}
