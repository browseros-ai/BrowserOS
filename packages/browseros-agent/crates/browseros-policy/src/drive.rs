//! The step loop: observe, decide, act, and stop for a reason the caller can
//! act on.
//!
//! Every exit hands back rather than failing. The run happens inside the
//! caller's own session, so the pages it opened are already the caller's and it
//! can carry on with the ordinary tools from wherever this stopped. That is
//! what makes a partial result useful instead of a dead end.
//!
//! The browser arrives as a trait so the loop, the budgets and the stop rules
//! are testable without one.

use crate::action::Operation;
use crate::answer::Decision;
use crate::decide::{DecideError, Decider, Step};
use crate::questions::{Observation, PastAction};
use std::future::Future;
use std::time::{Duration, Instant};

/// Stop once the model is this sure the goal is visibly satisfied.
pub const SATISFIED_THRESHOLD: f64 = 0.9;

/// How many consecutive steps may fail to improve progress before the run is
/// treated as stuck.
pub const STALL_LIMIT: u32 = 3;

/// Where decisions come from.
///
/// A seam, so the loop's budgets and stop rules are testable with a scripted
/// sequence rather than a live provider. [`Decider`] is the real one.
pub trait Oracle {
    fn decide(
        &self,
        goal: &str,
        observation: &Observation,
        history: &[PastAction],
    ) -> impl Future<Output = Result<Step, DecideError>>;
}

impl Oracle for Decider {
    fn decide(
        &self,
        goal: &str,
        observation: &Observation,
        history: &[PastAction],
    ) -> impl Future<Output = Result<Step, DecideError>> {
        Decider::decide(self, goal, observation, history)
    }
}

/// What the loop needs from a browser.
pub trait Driver {
    /// Whether the run has been cancelled, by the operator or by the caller
    /// going away.
    ///
    /// Asked between every stage rather than once per step, because a run that
    /// keeps clicking after Stop is worse than a single call that does.
    fn stopped(&self) -> bool;

    /// The page as it is right now, as an action space plus the rendered tree.
    fn observe(&self) -> impl Future<Output = Result<Observation, String>>;

    /// Runs one operation. Returns whether the page changed, which is the
    /// signal the next decision is shown.
    fn act(
        &self,
        operation: Operation,
        target: Option<&str>,
    ) -> impl Future<Output = Result<bool, String>>;
}

/// Why a run stopped.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// The model reported visible evidence that every requirement is met.
    /// Still not proof: the caller verifies.
    Satisfied,
    /// No supported operation could make progress.
    Blocked,
    /// A field needs a value only the caller can supply. The caller types it
    /// and calls again.
    NeedsText { reference: String, field: String },
    /// Ran out of steps or time.
    OutOfBudget { steps: u32, elapsed_ms: u64 },
    /// Progress stopped improving.
    Stalled,
    /// The operator stopped the session, or the caller went away.
    Stopped,
    /// An answer did not fit the observation, so nothing was executed.
    Refused(String),
    /// The provider or the browser failed.
    Failed(String),
}

impl Status {
    /// Whether the caller should treat this as the goal being met.
    #[must_use]
    pub fn is_satisfied(&self) -> bool {
        matches!(self, Self::Satisfied)
    }
}

/// One executed step, for the trail the caller and the audit log both read.
#[derive(Debug, Clone)]
pub struct TrailEntry {
    pub operation: Operation,
    pub target: Option<String>,
    pub operation_confidence: f64,
    pub target_confidence: Option<f64>,
    pub satisfied: f64,
    pub progress: f64,
    pub page_changed: bool,
    pub latency_ms: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub dropped_targets: usize,
}

/// What a run did, whatever its ending.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub status: Status,
    pub trail: Vec<TrailEntry>,
    pub url: String,
    pub title: String,
    pub elapsed_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Outcome {
    #[must_use]
    pub fn decisions(&self) -> usize {
        self.trail.len()
    }
}

/// Limits on one run. Both end it by handing back.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub max_steps: u32,
    pub max_duration: Duration,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_steps: 24,
            max_duration: Duration::from_secs(90),
        }
    }
}

/// Drives one goal to a stopping point.
pub async fn drive<O: Oracle, D: Driver>(
    oracle: &O,
    driver: &D,
    goal: &str,
    budget: Budget,
) -> Outcome {
    let started = Instant::now();
    let mut history: Vec<PastAction> = Vec::new();
    let mut trail: Vec<TrailEntry> = Vec::new();
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut best_progress = f64::NEG_INFINITY;
    let mut stalled = 0u32;
    let mut url = String::new();
    let mut title = String::new();

    let finish = |status: Status,
                  trail: Vec<TrailEntry>,
                  url: String,
                  title: String,
                  input_tokens: u64,
                  output_tokens: u64| Outcome {
        status,
        trail,
        url,
        title,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        input_tokens,
        output_tokens,
    };

    for step_index in 0..budget.max_steps {
        let out_of_time = |trail: Vec<TrailEntry>,
                           url: String,
                           title: String,
                           input_tokens: u64,
                           output_tokens: u64| {
            finish(
                Status::OutOfBudget {
                    steps: step_index,
                    elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                },
                trail,
                url,
                title,
                input_tokens,
                output_tokens,
            )
        };
        if driver.stopped() {
            return finish(
                Status::Stopped,
                trail,
                url,
                title,
                input_tokens,
                output_tokens,
            );
        }
        if started.elapsed() >= budget.max_duration {
            return out_of_time(trail, url, title, input_tokens, output_tokens);
        }

        let observation = match driver.observe().await {
            Ok(observation) => observation,
            Err(error) => {
                return finish(
                    Status::Failed(error),
                    trail,
                    url,
                    title,
                    input_tokens,
                    output_tokens,
                );
            }
        };
        url = observation.url.clone();
        title = observation.title.clone();

        let step = match oracle.decide(goal, &observation, &history).await {
            Ok(step) => step,
            Err(DecideError::Answer(error)) => {
                // The answer did not fit this observation, so nothing ran.
                return finish(
                    Status::Refused(error.to_string()),
                    trail,
                    url,
                    title,
                    input_tokens,
                    output_tokens,
                );
            }
            Err(error) => {
                return finish(
                    Status::Failed(error.to_string()),
                    trail,
                    url,
                    title,
                    input_tokens,
                    output_tokens,
                );
            }
        };
        input_tokens += step.input_tokens.unwrap_or(0);
        output_tokens += step.output_tokens.unwrap_or(0);

        // The done gate is checked before the operation, so an already
        // satisfied goal stops without one more action against the page.
        if step.decision.satisfied >= SATISFIED_THRESHOLD {
            return finish(
                Status::Satisfied,
                trail,
                url,
                title,
                input_tokens,
                output_tokens,
            );
        }

        match step.decision.operation {
            Operation::Done => {
                // Chosen without the gate agreeing. Trust the number over the
                // label, and treat weak evidence as being stuck rather than
                // finished.
                return finish(
                    Status::Stalled,
                    trail,
                    url,
                    title,
                    input_tokens,
                    output_tokens,
                );
            }
            Operation::Blocked => {
                return finish(
                    Status::Blocked,
                    trail,
                    url,
                    title,
                    input_tokens,
                    output_tokens,
                );
            }
            Operation::TypeText => {
                let reference = step.decision.target.clone().unwrap_or_default();
                let field = observation
                    .space
                    .elements
                    .iter()
                    .find(|element| element.reference == reference)
                    .map_or_else(|| reference.clone(), |element| element.label());
                return finish(
                    Status::NeedsText { reference, field },
                    trail,
                    url,
                    title,
                    input_tokens,
                    output_tokens,
                );
            }
            _ => {}
        }

        // Asked again here because observing and deciding can take seconds, and
        // this is the last moment before the page is changed.
        if driver.stopped() {
            return finish(
                Status::Stopped,
                trail,
                url,
                title,
                input_tokens,
                output_tokens,
            );
        }
        if started.elapsed() >= budget.max_duration {
            return out_of_time(trail, url, title, input_tokens, output_tokens);
        }

        let page_changed = match driver
            .act(step.decision.operation, step.decision.target.as_deref())
            .await
        {
            Ok(changed) => changed,
            Err(error) => {
                return finish(
                    Status::Failed(error),
                    trail,
                    url,
                    title,
                    input_tokens,
                    output_tokens,
                );
            }
        };

        trail.push(entry(&step, page_changed));
        history.push(PastAction {
            operation: step.decision.operation.as_str().to_string(),
            target: step.decision.target.clone(),
            text: None,
            page_changed,
        });

        // Progress that never improves means the run is going nowhere, whatever
        // the operations claim.
        if step.decision.progress > best_progress {
            best_progress = step.decision.progress;
            stalled = 0;
        } else {
            stalled += 1;
            if stalled >= STALL_LIMIT {
                let (url, title) = settled_location(driver, url, title).await;
                return finish(
                    Status::Stalled,
                    trail,
                    url,
                    title,
                    input_tokens,
                    output_tokens,
                );
            }
        }
    }

    let (url, title) = settled_location(driver, url, title).await;
    finish(
        Status::OutOfBudget {
            steps: budget.max_steps,
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        },
        trail,
        url,
        title,
        input_tokens,
        output_tokens,
    )
}

/// Where the browser actually is, for an ending that follows an action.
///
/// Both the step cap and the stall rule can fire straight after an action, and
/// the location captured at the top of that step is then out of date. If the
/// last action navigated, the handback would name the previous page and the
/// caller would resume against the wrong one. A failed read keeps what was
/// already known rather than losing it.
async fn settled_location<D: Driver>(driver: &D, url: String, title: String) -> (String, String) {
    match driver.observe().await {
        Ok(observation) => (observation.url, observation.title),
        Err(_) => (url, title),
    }
}

fn entry(step: &Step, page_changed: bool) -> TrailEntry {
    let Decision {
        operation,
        target,
        operation_confidence,
        target_confidence,
        satisfied,
        progress,
        ..
    } = &step.decision;
    TrailEntry {
        operation: *operation,
        target: target.clone(),
        operation_confidence: *operation_confidence,
        target_confidence: *target_confidence,
        satisfied: *satisfied,
        progress: *progress,
        page_changed,
        latency_ms: step.latency_ms,
        input_tokens: step.input_tokens,
        output_tokens: step.output_tokens,
        dropped_targets: step.dropped_targets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{ActionSpace, Element};
    use crate::answer::AnswerError;
    use std::cell::RefCell;

    fn observation() -> Observation {
        Observation {
            url: "https://example.com/form".to_string(),
            title: "Form".to_string(),
            tree: "- button \"Go\" [ref=e1]\n- button \"Stop\" [ref=e2]".to_string(),
            space: ActionSpace::new(
                vec![
                    Element::new("e1", "button", "Go"),
                    Element::new("e2", "button", "Stop"),
                    Element::new("e3", "textbox", "Name"),
                    Element::new("e4", "textbox", "Email"),
                ],
                false,
                false,
            ),
        }
    }

    fn step(operation: Operation, target: Option<&str>, satisfied: f64, progress: f64) -> Step {
        Step {
            decision: Decision {
                operation,
                target: target.map(str::to_string),
                operation_confidence: 0.8,
                target_confidence: target.map(|_| 0.9),
                satisfied,
                progress,
                operation_probabilities: Default::default(),
            },
            latency_ms: 300,
            model: "jev-test".to_string(),
            input_tokens: Some(100),
            output_tokens: Some(20),
            dropped_targets: 0,
        }
    }

    /// Hands back scripted decisions, then repeats the last one forever so a
    /// budget test does not run out of script.
    struct ScriptedOracle {
        steps: RefCell<Vec<Result<Step, DecideError>>>,
        asked: RefCell<usize>,
    }

    impl ScriptedOracle {
        fn new(steps: Vec<Result<Step, DecideError>>) -> Self {
            Self {
                steps: RefCell::new(steps),
                asked: RefCell::new(0),
            }
        }
    }

    impl Oracle for ScriptedOracle {
        async fn decide(
            &self,
            _goal: &str,
            _observation: &Observation,
            _history: &[PastAction],
        ) -> Result<Step, DecideError> {
            *self.asked.borrow_mut() += 1;
            let mut steps = self.steps.borrow_mut();
            if steps.len() > 1 {
                match steps.remove(0) {
                    Ok(step) => Ok(step),
                    Err(DecideError::Answer(error)) => Err(DecideError::Answer(error)),
                    Err(DecideError::Provider(reason)) => Err(DecideError::Provider(reason)),
                    Err(DecideError::Questions(reason)) => Err(DecideError::Questions(reason)),
                }
            } else {
                match steps.first() {
                    Some(Ok(step)) => Ok(step.clone()),
                    Some(Err(DecideError::Answer(error))) => {
                        Err(DecideError::Answer(error.clone()))
                    }
                    Some(Err(DecideError::Provider(reason))) => {
                        Err(DecideError::Provider(reason.clone()))
                    }
                    Some(Err(DecideError::Questions(reason))) => {
                        Err(DecideError::Questions(reason.clone()))
                    }
                    None => Err(DecideError::Provider("script exhausted".to_string())),
                }
            }
        }
    }

    /// Takes long enough to decide that the run's deadline passes mid-step.
    struct SlowOracle {
        delay: Duration,
        step: Step,
    }

    impl Oracle for SlowOracle {
        async fn decide(
            &self,
            _goal: &str,
            _observation: &Observation,
            _history: &[PastAction],
        ) -> Result<Step, DecideError> {
            tokio::time::sleep(self.delay).await;
            Ok(self.step.clone())
        }
    }

    struct FakeBrowser {
        acted: RefCell<Vec<(Operation, Option<String>)>>,
        page_changes: bool,
        observe_error: Option<String>,
        /// Reported by `observe` once any action has run, so a test can model a
        /// page that moved under the run.
        navigates_to: Option<&'static str>,
        /// Flips after this many `stopped` enquiries, so a test can stop a run
        /// at a chosen stage rather than only before it starts.
        stop_after: Option<usize>,
        asked: RefCell<usize>,
    }

    impl FakeBrowser {
        fn new(page_changes: bool) -> Self {
            Self {
                acted: RefCell::new(Vec::new()),
                page_changes,
                observe_error: None,
                navigates_to: None,
                stop_after: None,
                asked: RefCell::new(0),
            }
        }

        fn navigating_to(url: &'static str) -> Self {
            Self {
                acted: RefCell::new(Vec::new()),
                page_changes: true,
                observe_error: None,
                navigates_to: Some(url),
                stop_after: None,
                asked: RefCell::new(0),
            }
        }

        fn stopping_after(enquiries: usize) -> Self {
            Self {
                acted: RefCell::new(Vec::new()),
                page_changes: true,
                observe_error: None,
                navigates_to: None,
                stop_after: Some(enquiries),
                asked: RefCell::new(0),
            }
        }

        fn broken() -> Self {
            Self {
                acted: RefCell::new(Vec::new()),
                page_changes: false,
                observe_error: Some("browser session not connected".to_string()),
                navigates_to: None,
                stop_after: None,
                asked: RefCell::new(0),
            }
        }

        fn actions(&self) -> Vec<(Operation, Option<String>)> {
            self.acted.borrow().clone()
        }
    }

    impl Driver for FakeBrowser {
        fn stopped(&self) -> bool {
            let mut asked = self.asked.borrow_mut();
            *asked += 1;
            self.stop_after.is_some_and(|limit| *asked > limit)
        }

        async fn observe(&self) -> Result<Observation, String> {
            if let Some(error) = &self.observe_error {
                return Err(error.clone());
            }
            let mut current = observation();
            if let Some(prefix) = self.navigates_to {
                // Each action moves the page on, so the location read before an
                // action never matches the one after it.
                let moves = self.acted.borrow().len();
                current.url = format!("{prefix}/{moves}");
                current.title = format!("Page {moves}");
            }
            Ok(current)
        }

        async fn act(&self, operation: Operation, target: Option<&str>) -> Result<bool, String> {
            self.acted
                .borrow_mut()
                .push((operation, target.map(str::to_string)));
            Ok(self.page_changes)
        }
    }

    fn budget(max_steps: u32) -> Budget {
        Budget {
            max_steps,
            max_duration: Duration::from_secs(60),
        }
    }

    #[tokio::test]
    async fn a_confident_done_gate_stops_without_another_action() {
        let oracle = ScriptedOracle::new(vec![Ok(step(Operation::Click, Some("e1"), 0.95, 2.0))]);
        let browser = FakeBrowser::new(true);
        let outcome = drive(&oracle, &browser, "Submit the form.", budget(10)).await;
        assert_eq!(outcome.status, Status::Satisfied);
        assert!(
            browser.actions().is_empty(),
            "the gate is checked before acting, so nothing should have run"
        );
    }

    /// DONE chosen while the gate disagrees is treated as stuck. The number is
    /// trusted over the label.
    #[tokio::test]
    async fn done_without_the_gate_agreeing_is_not_success() {
        let oracle = ScriptedOracle::new(vec![Ok(step(Operation::Done, None, 0.3, 1.0))]);
        let browser = FakeBrowser::new(true);
        let outcome = drive(&oracle, &browser, "Submit the form.", budget(10)).await;
        assert_eq!(outcome.status, Status::Stalled);
        assert!(!outcome.status.is_satisfied());
    }

    #[tokio::test]
    async fn blocked_is_reported_as_blocked() {
        let oracle = ScriptedOracle::new(vec![Ok(step(Operation::Blocked, None, 0.1, 0.0))]);
        let outcome = drive(
            &oracle,
            &FakeBrowser::new(false),
            "Do the impossible.",
            budget(10),
        )
        .await;
        assert_eq!(outcome.status, Status::Blocked);
    }

    /// Typing needs a value only the caller can supply, so the run hands back
    /// naming the field rather than inventing one.
    #[tokio::test]
    async fn typing_hands_back_naming_the_field() {
        let oracle = ScriptedOracle::new(vec![Ok(step(Operation::TypeText, Some("e4"), 0.2, 1.0))]);
        let browser = FakeBrowser::new(true);
        let outcome = drive(&oracle, &browser, "Fill in the email.", budget(10)).await;
        assert_eq!(
            outcome.status,
            Status::NeedsText {
                reference: "e4".to_string(),
                field: "[e4] textbox Email".to_string()
            }
        );
        assert!(
            browser.actions().is_empty(),
            "nothing is typed without a value"
        );
    }

    #[tokio::test]
    async fn clicking_runs_and_lands_in_the_trail() {
        let oracle = ScriptedOracle::new(vec![
            Ok(step(Operation::Click, Some("e1"), 0.2, 0.5)),
            Ok(step(Operation::Click, Some("e2"), 0.95, 2.0)),
        ]);
        let browser = FakeBrowser::new(true);
        let outcome = drive(&oracle, &browser, "Click both.", budget(10)).await;
        assert_eq!(outcome.status, Status::Satisfied);
        assert_eq!(
            browser.actions(),
            vec![(Operation::Click, Some("e1".to_string()))]
        );
        assert_eq!(outcome.decisions(), 1);
        let entry = &outcome.trail[0];
        assert_eq!(entry.operation, Operation::Click);
        assert_eq!(entry.target.as_deref(), Some("e1"));
        assert!(entry.page_changed);
        assert_eq!(outcome.input_tokens, 200, "both decisions are counted");
        assert_eq!(outcome.url, "https://example.com/form");
    }

    /// Progress that never improves ends the run, whatever the operations claim.
    #[tokio::test]
    async fn a_run_whose_progress_never_improves_is_stopped() {
        let oracle = ScriptedOracle::new(vec![Ok(step(Operation::Click, Some("e1"), 0.1, 0.5))]);
        let browser = FakeBrowser::new(false);
        let outcome = drive(&oracle, &browser, "Go nowhere.", budget(50)).await;
        assert_eq!(outcome.status, Status::Stalled);
        assert_eq!(
            browser.actions().len(),
            1 + STALL_LIMIT as usize,
            "the first step sets the progress baseline, then the limit counts \
             the steps that fail to beat it"
        );
    }

    #[tokio::test]
    async fn the_step_cap_hands_back_with_what_it_did() {
        // Progress improves every step, so only the cap can stop it.
        let steps: Vec<Result<Step, DecideError>> = (1..=10)
            .map(|index| {
                Ok(step(
                    Operation::Click,
                    Some("e1"),
                    0.1,
                    f64::from(index) * 0.1,
                ))
            })
            .collect();
        let oracle = ScriptedOracle::new(steps);
        let browser = FakeBrowser::new(true);
        let outcome = drive(&oracle, &browser, "Keep going.", budget(4)).await;
        assert!(
            matches!(outcome.status, Status::OutOfBudget { steps: 4, .. }),
            "got {:?}",
            outcome.status
        );
        assert_eq!(outcome.decisions(), 4);
    }

    /// The bug this closes: if the last permitted action navigates, the handback
    /// used to name the page from before it, so a caller resuming from the
    /// result was pointed at the wrong one.
    #[tokio::test]
    async fn the_step_cap_reports_the_page_the_run_ended_on() {
        let steps: Vec<Result<Step, DecideError>> = (1..=4)
            .map(|index| {
                Ok(step(
                    Operation::Click,
                    Some("e1"),
                    0.1,
                    f64::from(index) * 0.1,
                ))
            })
            .collect();
        let oracle = ScriptedOracle::new(steps);
        let browser = FakeBrowser::navigating_to("https://example.com/step");
        let outcome = drive(&oracle, &browser, "Keep going.", budget(2)).await;
        assert!(matches!(outcome.status, Status::OutOfBudget { .. }));
        assert_eq!(
            browser.actions().len(),
            2,
            "two actions ran, so the page moved twice"
        );
        assert_eq!(
            outcome.url, "https://example.com/step/2",
            "the handback must name the page after the last action, not before it"
        );
        assert_eq!(outcome.title, "Page 2");
    }

    #[tokio::test]
    async fn a_stalled_run_also_reports_where_it_ended() {
        let oracle = ScriptedOracle::new(vec![Ok(step(Operation::Click, Some("e1"), 0.1, 0.5))]);
        let browser = FakeBrowser::navigating_to("https://example.com/step");
        let outcome = drive(&oracle, &browser, "Go nowhere.", budget(50)).await;
        assert_eq!(outcome.status, Status::Stalled);
        assert_eq!(
            outcome.url,
            format!("https://example.com/step/{}", browser.actions().len()),
            "the handback must name the page after the last action"
        );
    }

    /// An answer that does not fit the observation stops the run and executes
    /// nothing.
    #[tokio::test]
    async fn a_refused_answer_executes_nothing() {
        let oracle = ScriptedOracle::new(vec![Err(DecideError::Answer(
            AnswerError::UnofferedTarget {
                operation: "CLICK",
                reference: "e99".to_string(),
            },
        ))]);
        let browser = FakeBrowser::new(true);
        let outcome = drive(&oracle, &browser, "Click something.", budget(10)).await;
        assert!(
            matches!(outcome.status, Status::Refused(_)),
            "got {:?}",
            outcome.status
        );
        assert!(browser.actions().is_empty());
    }

    #[tokio::test]
    async fn a_provider_failure_is_reported_as_a_failure() {
        let oracle = ScriptedOracle::new(vec![Err(DecideError::Provider("503".to_string()))]);
        let outcome = drive(&oracle, &FakeBrowser::new(true), "Anything.", budget(10)).await;
        assert!(
            matches!(&outcome.status, Status::Failed(reason) if reason.contains("503")),
            "got {:?}",
            outcome.status
        );
    }

    #[tokio::test]
    async fn a_browser_that_cannot_be_observed_fails_before_deciding() {
        let oracle = ScriptedOracle::new(vec![Ok(step(Operation::Click, Some("e1"), 0.1, 0.5))]);
        let outcome = drive(&oracle, &FakeBrowser::broken(), "Anything.", budget(10)).await;
        assert_eq!(
            outcome.status,
            Status::Failed("browser session not connected".to_string())
        );
        assert_eq!(outcome.decisions(), 0);
    }

    /// Stop before anything happens: nothing is observed and nothing runs.
    #[tokio::test]
    async fn a_stopped_run_does_nothing_at_all() {
        let oracle = ScriptedOracle::new(vec![Ok(step(Operation::Click, Some("e1"), 0.1, 0.5))]);
        let browser = FakeBrowser::stopping_after(0);
        let outcome = drive(&oracle, &browser, "Click things.", budget(10)).await;
        assert_eq!(outcome.status, Status::Stopped);
        assert!(browser.actions().is_empty());
        assert_eq!(outcome.decisions(), 0);
    }

    /// The gap that mattered: a stop arriving while the provider is deciding
    /// must land before the page is changed, not after.
    #[tokio::test]
    async fn a_stop_between_deciding_and_acting_changes_nothing() {
        let oracle = ScriptedOracle::new(vec![Ok(step(Operation::Click, Some("e1"), 0.1, 0.5))]);
        // One enquiry passes at the top of the step, the next one stops it.
        let browser = FakeBrowser::stopping_after(1);
        let outcome = drive(&oracle, &browser, "Click things.", budget(10)).await;
        assert_eq!(outcome.status, Status::Stopped);
        assert!(
            browser.actions().is_empty(),
            "a stop before acting must leave the page alone"
        );
    }

    /// Already expired when the step begins: the check at the top catches it.
    #[tokio::test]
    async fn an_expired_deadline_stops_before_observing() {
        let oracle = ScriptedOracle::new(vec![Ok(step(Operation::Click, Some("e1"), 0.1, 0.5))]);
        let browser = FakeBrowser::new(true);
        let outcome = drive(
            &oracle,
            &browser,
            "Click things.",
            Budget {
                max_steps: 5,
                max_duration: Duration::ZERO,
            },
        )
        .await;
        assert!(
            matches!(outcome.status, Status::OutOfBudget { .. }),
            "got {:?}",
            outcome.status
        );
        assert!(browser.actions().is_empty());
    }

    /// The deadline expiring *while* the provider is deciding is the case the
    /// top-of-step check cannot see. Only the check after deciding stops the
    /// page from being changed past the budget.
    #[tokio::test]
    async fn a_deadline_passed_while_deciding_stops_before_acting() {
        let oracle = SlowOracle {
            delay: Duration::from_millis(60),
            step: step(Operation::Click, Some("e1"), 0.1, 0.5),
        };
        let browser = FakeBrowser::new(true);
        let outcome = drive(
            &oracle,
            &browser,
            "Click things.",
            Budget {
                max_steps: 5,
                // Alive at the top of the step, expired by the time the
                // decision comes back.
                max_duration: Duration::from_millis(20),
            },
        )
        .await;
        assert!(
            matches!(outcome.status, Status::OutOfBudget { .. }),
            "got {:?}",
            outcome.status
        );
        assert!(
            browser.actions().is_empty(),
            "the budget must not be overrun by a whole action"
        );
    }

    #[tokio::test]
    async fn a_zero_step_budget_does_nothing() {
        let oracle = ScriptedOracle::new(vec![Ok(step(Operation::Click, Some("e1"), 0.1, 0.5))]);
        let browser = FakeBrowser::new(true);
        let outcome = drive(&oracle, &browser, "Anything.", budget(0)).await;
        assert!(matches!(
            outcome.status,
            Status::OutOfBudget { steps: 0, .. }
        ));
        assert!(browser.actions().is_empty());
    }
}
