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

use crate::client::{Jev, JevError, MAX_RETRIES, Question};
use crate::gate::{self, Decision, Verdict};
use crate::operations::Operation;
use crate::questions;
use crate::space::ActionSpace;
use crate::view::PageView;
use std::time::Duration;

/// How many consecutive actions may change nothing before the run stops.
pub const STALL_LIMIT: u32 = 3;

/// How many times a stale decision may be retried before giving up. A page that
/// changes under every decision is not one this can finish.
pub const STALE_RETRIES: u32 = 3;

/// How many times a run may choose to wait before it is treated as stuck.
///
/// Waiting makes no observable progress, so without a limit a model that keeps
/// choosing it would spend the whole step budget looking busy.
pub const WAIT_LIMIT: u32 = 3;

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
    /// Asks, giving up after `budget`, which covers the whole call.
    fn ask_within(
        &self,
        state: &serde_json::Value,
        questions: &BTreeMap<String, Question>,
        budget: Duration,
    ) -> impl Future<Output = Result<crate::client::Response, JevError>>;
}

impl Oracle for Jev {
    async fn ask_within(
        &self,
        state: &serde_json::Value,
        questions: &BTreeMap<String, Question>,
        budget: Duration,
    ) -> Result<crate::client::Response, JevError> {
        Jev::ask_within(self, state, questions, budget).await
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
    /// How many controls the last decision could choose from, and how many were
    /// held back by the cap. Reported on every ending, because how much of the
    /// page was visible to a decision is part of judging its answer.
    pub offered: usize,
    pub omitted: usize,
    /// The controls that were offered, when the run handed back because none of
    /// them could advance the goal.
    ///
    /// Only populated then. It is what the caller needs to decide what to do
    /// next, and carrying it on every ending would be noise.
    pub offered_controls: Vec<String>,
    /// The confidence behind a terminal answer, when one ended the run.
    ///
    /// Not gated on. A completion claim is a report rather than an action, so
    /// the number travels to the caller to judge instead of being turned into a
    /// refusal here, which would only send the run round again.
    pub terminal_confidence: Option<f64>,
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
    // Set once a control has refused a click, so the keyboard fallback is
    // offered when it is actually needed rather than on every step.
    let mut fallback_needed = false;
    let mut waits = 0;

    let first = match driver.observe().await {
        Ok(view) => view,
        Err(error) => {
            return Outcome {
                status: Status::Failed(error),
                offered: 0,
                omitted: 0,
                offered_controls: Vec::new(),
                terminal_confidence: None,
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
                offered: 0,
                omitted: 0,
                offered_controls: Vec::new(),
                terminal_confidence: None,
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
        let operations = questions::available(&space, &view, fallback_needed);
        let built = questions::build(goal, &space, &operations);
        let state = questions::state(&view, &space, &recent);

        decisions += 1;
        // The smaller of the published default and what the run has left, so a
        // slow provider cannot carry a run past the user's own limit.
        let elapsed = started.elapsed();
        let run_left = Duration::from_secs(budget.max_seconds).saturating_sub(elapsed);
        let ask_budget = run_left.min(crate::client::DEFAULT_TIMEOUT);
        if ask_budget.is_zero() {
            return finish(Status::OutOfBudget, trail, &view, decisions, input_tokens);
        }
        let response = match ask_with_retries(oracle, &state, &built, ask_budget).await {
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

        // Checked again after the answer: the request may have taken the rest of
        // the run's time, and acting past the limit is what the limit forbids.
        if driver.stopped() {
            return finish(Status::Stopped, trail, &view, decisions, input_tokens);
        }
        if started.elapsed().as_secs() >= budget.max_seconds {
            return finish(Status::OutOfBudget, trail, &view, decisions, input_tokens);
        }

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
            // A completion claim needs a real observation to check against.
            // Falling back to the previous page here would make the check pass
            // by construction, so a closed tab or a lost connection would read
            // as a satisfied goal.
            let settled = match driver.observe().await {
                Ok(settled) => settled,
                Err(error) => {
                    return finish(
                        Status::NeedsInput(format!(
                            "the run answered {} but the page could not be read back to check it: \
                             {error}. Nothing is confirmed; look at the page yourself",
                            decision.operation.as_str()
                        )),
                        trail,
                        &view,
                        decisions,
                        input_tokens,
                    );
                }
            };
            let fresh = settled.is_fresh_for(view.fingerprint);
            view = settled;
            if decision.operation == Operation::Done && fresh {
                let mut outcome = finish(Status::Satisfied, trail, &view, decisions, input_tokens);
                outcome.terminal_confidence = Some(decision.operation_confidence);
                return outcome;
            }
            if decision.operation == Operation::Blocked {
                let mut outcome = finish(
                    Status::Blocked("no offered operation could advance the goal".to_string()),
                    trail,
                    &view,
                    decisions,
                    input_tokens,
                );
                outcome.terminal_confidence = Some(decision.operation_confidence);
                // Measured against the real model: when nothing on the page can
                // advance the goal it answers BLOCKED at the operation, which is
                // a more coherent answer than naming an operation and then
                // refusing every target. So this is the common route to "the
                // caller should decide", and it carries the same context the
                // escape path does.
                outcome.offered = space.offered.len();
                outcome.omitted = space.omitted;
                outcome.offered_controls = offered_labels(&space);
                return outcome;
            }
            // Done on a page that moved: look again rather than assert.
            continue;
        }

        let target_name = decision
            .target
            .as_deref()
            .and_then(|reference| view.control(reference))
            .map(|control| control.name.clone());

        // Waiting touches nothing, so it never reaches the browser: the act tool
        // has no kind for it. Settle and look again instead, without counting a
        // wait as an action, so a model that keeps waiting runs out of
        // decisions rather than appearing to make progress.
        if decision.operation == Operation::Wait {
            waits += 1;
            if waits > WAIT_LIMIT {
                return finish(Status::Stalled, trail, &view, decisions, input_tokens);
            }
            driver.settle().await;
            view = refresh(driver, &view).await;
            continue;
        }

        match gate::verdict(&decision, target_name.as_deref()) {
            Verdict::Execute => {}
            Verdict::HandBack(reason) => {
                let mut outcome = finish(
                    Status::NeedsInput(reason),
                    trail,
                    &view,
                    decisions,
                    input_tokens,
                );
                outcome.offered = space.offered.len();
                outcome.omitted = space.omitted;
                // Named so the caller can see what the decision was choosing
                // between, which is the difference between "try scrolling" and
                // "that control is not on this page".
                if decision.none_of_these {
                    outcome.offered_controls = offered_labels(&space);
                }
                return outcome;
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

        // The decision was made about a particular control, on a page observed
        // before the model was asked. Comparing the guard against that same
        // page would always agree with itself, so the page is observed again
        // here and the guard saved with the decision is compared against what
        // is there now.
        //
        // Narrow on purpose: a live page changes constantly in places that have
        // nothing to do with the chosen control, and refusing to act on any of
        // that would never finish anything.
        if let Some(reference) = decision.target.as_deref() {
            let guard = space.candidate(reference).map(|candidate| candidate.guard);
            let settled = match driver.observe().await {
                Ok(settled) => settled,
                Err(error) => {
                    return finish(Status::Failed(error), trail, &view, decisions, input_tokens);
                }
            };
            let still_there = guard.is_some_and(|guard| settled.control_is_fresh(reference, guard));
            view = settled;
            if !still_there {
                stalled += 1;
                if stalled >= STALL_LIMIT {
                    return finish(Status::Stalled, trail, &view, decisions, input_tokens);
                }
                continue;
            }
        }

        let changed = match driver.act(&decision).await {
            Ok(changed) => changed,
            Err(ActError::Stale(reason)) => {
                if reason.contains("covered") {
                    fallback_needed = true;
                }
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

/// Retries only what retrying can fix, inside a budget.
///
/// Follows the published contract rather than a local invention: the budget
/// covers every attempt and every delay between them, a delay that would
/// exhaust what is left is not taken, and a wait the service named itself wins
/// over the computed one.
async fn ask_with_retries<O: Oracle>(
    oracle: &O,
    state: &serde_json::Value,
    questions: &BTreeMap<String, Question>,
    budget: Duration,
) -> Result<crate::client::Response, JevError> {
    let deadline = std::time::Instant::now() + budget;
    let mut attempt: u32 = 0;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err(JevError::TimedOut(budget));
        }
        match oracle.ask_within(state, questions, left).await {
            Ok(response) => return Ok(response),
            Err(error) if error.is_retryable() && attempt < MAX_RETRIES => {
                attempt += 1;
                let delay = crate::client::backoff(attempt, error.retry_after());
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                // A delay that would consume what is left buys nothing: the
                // attempt after it could not run, so the last error stands.
                if delay >= left {
                    return Err(error);
                }
                tokio::time::sleep(delay).await;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Re-observes, keeping the last view if the browser cannot answer.
///
/// Only for the paths where a stale view is harmless: deciding again, where the
/// next answer is about whatever is observed then. It must not be used to check
/// a claim, because the fallback would make that check pass by construction,
/// which is why the terminal path observes directly.
async fn refresh<D: Driver>(driver: &D, previous: &PageView) -> PageView {
    driver.observe().await.unwrap_or_else(|_| previous.clone())
}

/// The offered controls as the caller sees them, so a hand back says what the
/// decision was choosing between rather than only that it could not choose.
fn offered_labels(space: &ActionSpace) -> Vec<String> {
    space
        .offered
        .iter()
        .map(|candidate| {
            candidate
                .descriptor
                .get("control")
                .and_then(|value| value.as_str())
                .unwrap_or(&candidate.reference)
                .to_string()
        })
        .collect()
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
