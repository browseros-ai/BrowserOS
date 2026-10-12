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
use crate::run::{Asking, Pending, Run};
use crate::space::ActionSpace;
use crate::view::PageView;
use std::time::Duration;

/// How many consecutive actions may change nothing before the run stops.
pub const STALL_LIMIT: u32 = 3;

/// How many times a stale decision may be retried before giving up. A page that
/// changes under every decision is not one this can finish.
pub const STALE_RETRIES: u32 = 3;

/// How many times a run may ask the caller for a value before it stops asking.
///
/// An ask takes no action, so neither the step budget nor the stall counters
/// bound it. A field that changes under every answer would otherwise trade
/// round trips with the caller indefinitely.
pub const ASK_LIMIT: u32 = 3;

/// Why a run gave up making progress.
///
/// One counter used to serve four unrelated situations and the caller was told
/// about one of them, so a run that applied a filter in a single successful
/// action was told that three actions in a row had changed nothing. Naming the
/// cause is the whole point: the reasons call for different responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stuck {
    /// Actions ran and left the page as it was.
    NothingChanged,
    /// The gate declined to act, usually because confidence was short.
    NotConfident,
    /// The chosen control kept moving between the decision and the action.
    ControlKeptMoving,
    /// The browser kept reporting something a retry might clear, and it did
    /// not clear.
    BrowserKeptFailing,
    /// The run kept choosing to wait, which makes no observable progress.
    OnlyWaiting,
}

impl Stuck {
    /// What to tell the caller, which has to be true of what happened.
    #[must_use]
    pub fn describe(self, actions: usize) -> String {
        match self {
            Self::NothingChanged => {
                format!("{actions} action(s) ran and the page was the same afterwards each time")
            }
            // A run that acted and then ran out is not a run that found nothing.
            // Measured on a live facet panel, five times out of five: the run
            // expanded a collapsed section at 0.98, the page changed, and it was
            // then described as having found no control it was confident enough
            // to use. The goal had asked for "checkbox options" and the rows
            // render as labels, so the terminal check could never confirm and
            // the run kept looking. Reasonable of it; unreasonable to then
            // describe as having found nothing.
            Self::NotConfident if actions > 0 => format!(
                "it ran {actions} action(s), then could not find a further control it was \
                 confident enough to use. Check whether what it already did was what you \
                 wanted, because the goal may already be met"
            ),
            Self::NotConfident => {
                "it could not find a control it was confident enough to use".to_string()
            }
            Self::ControlKeptMoving => {
                "the control it chose kept changing before it could be used".to_string()
            }
            Self::BrowserKeptFailing => {
                "the browser kept refusing the action, and looking again did not clear it"
                    .to_string()
            }
            Self::OnlyWaiting => {
                "it kept choosing to wait, which makes no progress to observe".to_string()
            }
        }
    }
}

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

    /// Runs one operation.
    ///
    /// Whether the page moved is not asked for here. The loop observes before
    /// and after and compares page identity itself, which is authoritative and
    /// needs no agreement with another component about how it words a result.
    fn act(&self, decision: &Decision) -> impl Future<Output = Result<(), ActError>>;

    /// Settles the page after an input, in code rather than by spending a
    /// decision on a wait.
    fn settle(&self) -> impl Future<Output = ()>;
}

/// How a run ended.
///
/// No `Eq`: an ask carries the confidence behind the field it chose, and that
/// number is worth more to the caller than the trait is to this enum.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// The model said every requirement was met, on a page that had not moved
    /// since it looked.
    Satisfied,
    /// Nothing offered could advance the goal.
    Blocked(String),
    /// The run stopped making progress, with the reason it stopped.
    Stalled(Stuck),
    /// The caller has to supply something before this can continue, and the
    /// run cannot be continued: it is a judgement rather than a missing value.
    NeedsInput(String),
    /// The run needs one value from the caller and can carry on once it has it.
    NeedsValue(Pending),
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
    /// The run itself, when this ending can be continued.
    ///
    /// Set on the endings where the caller can answer something and carry on,
    /// and on nothing else: a stall, a block and a failure are judgements
    /// rather than missing values, so handing the run back for those would
    /// invite a caller to retry something that already decided it was done.
    pub resume: Option<Run>,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    advance(oracle, driver, Run::new(goal, budget)).await
}

/// Advances a run, whether it is new or resumed from an earlier call.
///
/// Code owns the control flow here, as it did when this was `pursue` with every
/// counter in a local. The only change is where the state lives: in the run, so
/// an ending can be continued instead of only reported.
pub async fn advance<O: Oracle, D: Driver>(oracle: &O, driver: &D, mut run: Run) -> Outcome {
    let started = std::time::Instant::now();
    // Borrowed from the run once, so the loop can hold `&mut run` without
    // fighting a live borrow of a field it never writes.
    let goal = run.goal.clone();
    // What the last decision could see. Every ending reports it, because a run
    // that stalled after choosing from a capped list is exactly as worth
    // explaining as one that gave up.
    //
    // Plain counters now: these were atomics only because the closure that
    // built an outcome read them while the loop wrote them, and with the state
    // in a struct there is no such closure.
    let mut offered = 0usize;
    let mut omitted = 0usize;

    let first = match driver.observe().await {
        Ok(view) => view,
        Err(error) => {
            run.spent += started.elapsed();
            return Outcome {
                status: Status::Failed(error),
                offered: 0,
                omitted: 0,
                offered_controls: Vec::new(),
                terminal_confidence: None,
                decisions: run.decisions,
                trail: run.trail.clone(),
                url_before: run.started_at(),
                url_after: String::new(),
                title: String::new(),
                input_tokens: run.input_tokens,
                resume: None,
            };
        }
    };
    // Only on the first call. A resumed run reports the url it originally
    // started on rather than the one it was resumed from, because the caller is
    // judging the whole pursuit against its goal.
    if run.url_before.is_none() {
        run.url_before = Some(first.url.clone());
    }
    let mut view = first;

    loop {
        if driver.stopped() {
            return end(&mut run, &started, Status::Stopped, &view, offered, omitted);
        }
        let out_of_steps = run.trail.len() >= run.budget.max_steps as usize;
        if out_of_steps || (run.spent + started.elapsed()).as_secs() >= run.budget.max_seconds {
            let outcome = end(
                &mut run,
                &started,
                Status::OutOfBudget,
                &view,
                offered,
                omitted,
            );
            // A run out of steps can be continued, because the caller can raise
            // the step limit on the next call. A run out of time cannot: the
            // time it has spent travels with it and there is no argument that
            // gives it more, so handing it back would invite a call that ends
            // the moment it starts.
            return if out_of_steps {
                park(run, outcome)
            } else {
                outcome
            };
        }

        let space = ActionSpace::build(&view, &goal, run.last_acted.as_deref());
        offered = space.offered.len();
        omitted = space.omitted;
        let operations = questions::available(&space, &view, run.fallback_needed);
        let built = questions::build(&goal, &space, &operations);
        let state = questions::state(&view, &space, &run.recent);

        run.decisions += 1;
        // The smaller of the published default and what the run has left, so a
        // slow provider cannot carry a run past the user's own limit. What the
        // run has left counts every call that has advanced it, not just this
        // one.
        let run_left = Duration::from_secs(run.budget.max_seconds)
            .saturating_sub(run.spent + started.elapsed());
        let ask_budget = run_left.min(crate::client::DEFAULT_TIMEOUT);
        if ask_budget.is_zero() {
            // Out of time, not out of steps, so not resumable: see above.
            return end(
                &mut run,
                &started,
                Status::OutOfBudget,
                &view,
                offered,
                omitted,
            );
        }
        let response = match ask_with_retries(oracle, &state, &built, ask_budget).await {
            Ok(response) => response,
            Err(error) => {
                return end(
                    &mut run,
                    &started,
                    Status::Failed(error.to_string()),
                    &view,
                    offered,
                    omitted,
                );
            }
        };
        run.input_tokens += response.usage.input_tokens;

        // Checked again after the answer: the request may have taken the rest of
        // the run's time, and acting past the limit is what the limit forbids.
        if driver.stopped() {
            return end(&mut run, &started, Status::Stopped, &view, offered, omitted);
        }
        if (run.spent + started.elapsed()).as_secs() >= run.budget.max_seconds {
            return end(
                &mut run,
                &started,
                Status::OutOfBudget,
                &view,
                offered,
                omitted,
            );
        }

        let mut decision = match gate::read(&response) {
            Ok(decision) => decision,
            Err(error) => {
                return end(
                    &mut run,
                    &started,
                    Status::Failed(error.to_string()),
                    &view,
                    offered,
                    omitted,
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
                    return end(
                        &mut run,
                        &started,
                        Status::NeedsInput(format!(
                            "the run answered {} but the page could not be read back to check it: \
                             {error}. Nothing is confirmed; look at the page yourself",
                            decision.operation.as_str()
                        )),
                        &view,
                        offered,
                        omitted,
                    );
                }
            };
            let fresh = settled.is_fresh_for(view.fingerprint);
            view = settled;
            if decision.operation == Operation::Done && fresh {
                let mut outcome = end(
                    &mut run,
                    &started,
                    Status::Satisfied,
                    &view,
                    offered,
                    omitted,
                );
                outcome.terminal_confidence = Some(decision.operation_confidence);
                return outcome;
            }
            if decision.operation == Operation::Blocked {
                let mut outcome = end(
                    &mut run,
                    &started,
                    Status::Blocked("no offered operation could advance the goal".to_string()),
                    &view,
                    offered,
                    omitted,
                );
                outcome.terminal_confidence = Some(decision.operation_confidence);
                // Measured against the real model: when nothing on the page can
                // advance the goal it answers BLOCKED at the operation, which is
                // a more coherent answer than naming an operation and then
                // refusing every target. So this is the common route to "the
                // caller should decide", and it carries the same context the
                // escape path does.
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
            run.waits += 1;
            if run.waits > WAIT_LIMIT {
                return end(
                    &mut run,
                    &started,
                    Status::Stalled(Stuck::OnlyWaiting),
                    &view,
                    offered,
                    omitted,
                );
            }
            driver.settle().await;
            view = refresh(driver, &view).await;
            continue;
        }

        // A reference is a handle into one snapshot, so the guard is what ties
        // the caller's answer to the control it was asked about. Read from the
        // candidate set the decision was made from, which is the same place the
        // freshness check below reads it.
        let chosen_guard = decision
            .target
            .as_deref()
            .and_then(|reference| space.candidate(reference).map(|candidate| candidate.guard));
        let supplied = decision
            .target
            .as_deref()
            .and_then(|reference| run.text_for(reference, chosen_guard))
            .map(str::to_string);

        match gate::verdict(&decision, target_name.as_deref(), supplied.as_deref()) {
            Verdict::Execute => {
                if decision.operation == Operation::Fill {
                    // The only operation whose argument comes from outside the
                    // model. `args_for` already forwards this to the act tool
                    // as `value`, so nothing downstream has to know where it
                    // came from.
                    decision.value = supplied;
                }
            }
            Verdict::HandBack(reason) if decision.operation == Operation::Fill => {
                let Some(reference) = decision.target.clone() else {
                    // A fill with no target is not a question anyone can
                    // answer, so it is reported rather than asked.
                    return end(
                        &mut run,
                        &started,
                        Status::NeedsInput(reason),
                        &view,
                        offered,
                        omitted,
                    );
                };
                let asking = if run.answer_went_stale(&reference, chosen_guard) {
                    // The caller answered and the field moved before the text
                    // could be typed. Forget the answer, because it was bound
                    // to a control that is no longer there, and say so.
                    run.forget(&reference);
                    Asking::FieldChanged
                } else {
                    Asking::FirstTime
                };
                run.asks += 1;
                let pending = Pending::Text {
                    reference,
                    control: control_label(&space, decision.target.as_deref()),
                    confidence: decision.target_confidence.unwrap_or(0.0),
                    guard: chosen_guard,
                    asking,
                };
                if run.asks > ASK_LIMIT {
                    // A field that will not hold still would ask forever, and
                    // every ask costs the caller a round trip while taking no
                    // action, so nothing else in the loop would stop it.
                    return end(
                        &mut run,
                        &started,
                        Status::NeedsInput(
                            "it asked for text three times and the field changed under every \
                             answer, so it has stopped asking. Look at the page yourself"
                                .to_string(),
                        ),
                        &view,
                        offered,
                        omitted,
                    );
                }
                run.pending = Some(pending.clone());
                let mut outcome = end(
                    &mut run,
                    &started,
                    Status::NeedsValue(pending),
                    &view,
                    offered,
                    omitted,
                );
                // What it was choosing between, so a caller told the wrong
                // field was picked can see the alternatives it had.
                outcome.offered_controls = offered_labels(&space);
                return park(run, outcome);
            }
            Verdict::HandBack(reason) => {
                let mut outcome = end(
                    &mut run,
                    &started,
                    Status::NeedsInput(reason),
                    &view,
                    offered,
                    omitted,
                );
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
                    return end(
                        &mut run,
                        &started,
                        Status::Blocked(reason),
                        &view,
                        offered,
                        omitted,
                    );
                }
                run.declined += 1;
                if run.declined >= STALL_LIMIT {
                    let outcome = end(
                        &mut run,
                        &started,
                        Status::Stalled(Stuck::NotConfident),
                        &view,
                        offered,
                        omitted,
                    );
                    return stuck(&space, outcome);
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
                    return end(
                        &mut run,
                        &started,
                        Status::Failed(error),
                        &view,
                        offered,
                        omitted,
                    );
                }
            };
            let still_there = guard.is_some_and(|guard| settled.control_is_fresh(reference, guard));
            view = settled;
            if !still_there {
                run.moved += 1;
                if run.moved >= STALL_LIMIT {
                    let outcome = end(
                        &mut run,
                        &started,
                        Status::Stalled(Stuck::ControlKeptMoving),
                        &view,
                        offered,
                        omitted,
                    );
                    return stuck(&space, outcome);
                }
                continue;
            }
        }

        let before = view.fingerprint;
        match driver.act(&decision).await {
            Ok(()) => {}
            Err(ActError::Stale(reason)) => {
                if reason.contains("covered") {
                    run.fallback_needed = true;
                }
                run.refused += 1;
                if run.refused > STALE_RETRIES {
                    let outcome = end(
                        &mut run,
                        &started,
                        Status::Stalled(Stuck::BrowserKeptFailing),
                        &view,
                        offered,
                        omitted,
                    );
                    return stuck(&space, outcome);
                }
                view = refresh(driver, &view).await;
                continue;
            }
            Err(ActError::Fatal(reason)) => {
                return end(
                    &mut run,
                    &started,
                    Status::Failed(reason),
                    &view,
                    offered,
                    omitted,
                );
            }
        };

        driver.settle().await;
        view = refresh(driver, &view).await;
        // Authoritative, and free: the page either has the identity it had
        // before the action or it does not. Reading a tool's wording for this
        // was wrong twice, once by searching prose that never carried it and
        // once by reading structured content the dispatch strips from any tool
        // without an output schema.
        let changed = !view.is_fresh_for(before);

        run.recent.push(describe(&decision, &target_name, changed));
        if run.recent.len() > 5 {
            run.recent.remove(0);
        }
        run.last_acted = decision.target.clone();
        run.trail.push(Step {
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
            run.unchanged = 0;
        } else {
            run.unchanged += 1;
            if run.unchanged >= STALL_LIMIT {
                let outcome = end(
                    &mut run,
                    &started,
                    Status::Stalled(Stuck::NothingChanged),
                    &view,
                    offered,
                    omitted,
                );
                return stuck(&space, outcome);
            }
        }
    }
}

/// Builds the outcome for an ending, and records the time the run has now
/// spent in total.
///
/// Every exit goes through here, so no path can forget to add its own elapsed
/// time: a run that parked without recording it would hand the caller a fresh
/// time allowance on every answer.
fn end(
    run: &mut Run,
    started: &std::time::Instant,
    status: Status,
    view: &PageView,
    offered: usize,
    omitted: usize,
) -> Outcome {
    run.spent += started.elapsed();
    Outcome {
        status,
        offered,
        omitted,
        offered_controls: Vec::new(),
        terminal_confidence: None,
        decisions: run.decisions,
        // Cloned rather than moved, because the run outlives this outcome when
        // the ending is resumable and both sides need the trail: the caller to
        // judge what happened, the run to continue it.
        trail: run.trail.clone(),
        url_before: run.started_at(),
        url_after: view.url.clone(),
        title: view.title.clone(),
        input_tokens: run.input_tokens,
        resume: None,
    }
}

/// Attaches the run itself, so the caller can continue it.
fn park(run: Run, mut outcome: Outcome) -> Outcome {
    outcome.resume = Some(run);
    outcome
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

/// Attaches what the decision could see to a stall.
///
/// A stall is the ending that fires most often on a hard page, and it was the
/// one that told the caller least: its neighbours already carry this and it did
/// not. Knowing what it was choosing between is often the whole answer.
fn stuck(space: &ActionSpace, mut outcome: Outcome) -> Outcome {
    outcome.offered_controls = offered_labels(space);
    outcome
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

/// One offered control as the caller sees it, in the same words the
/// offered-controls list uses, so a question and a list never disagree about
/// what something is called.
fn control_label(space: &ActionSpace, reference: Option<&str>) -> String {
    let Some(reference) = reference else {
        return "an unnamed control".to_string();
    };
    space
        .candidate(reference)
        .and_then(|candidate| candidate.descriptor.get("control"))
        .and_then(|value| value.as_str())
        .unwrap_or(reference)
        .to_string()
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
