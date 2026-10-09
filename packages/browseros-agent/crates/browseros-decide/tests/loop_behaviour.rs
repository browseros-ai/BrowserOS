//! The loop's termination and recovery paths, driven by a scripted model and a
//! scripted browser.
//!
//! A fake rather than the real endpoint, because each path has to be reachable
//! on demand: a page that goes stale exactly once, a mutation whose outcome is
//! unknown, a run that changes nothing three times in a row. The live gate
//! covers the opposite case, that the real model drives the real shapes.

use std::collections::BTreeMap;
use std::sync::Mutex;

use browseros_decide::client::{Answer, JevError, Question, Response, Usage};
use browseros_decide::gate::Decision;
use browseros_decide::pursue::{ActError, Budget, Driver, Oracle, Status, pursue};
use browseros_decide::questions::{CLICK_TARGET, OPERATION, SELECT_VALUE};
use browseros_decide::view::{Control, ControlState, PageView};

fn control(reference: &str, role: &str, name: &str) -> Control {
    Control {
        reference: reference.to_string(),
        role: role.to_string(),
        name: name.to_string(),
        value: None,
        state: ControlState::default(),
        options: Vec::new(),
        guard: 0,
    }
}

fn answer(choice: &str, confidence: f64) -> Answer {
    Answer {
        answer_type: "choice".to_string(),
        noul: None,
        choice: Some(choice.to_string()),
        score: None,
        probabilities: None,
        legend: None,
        confidence: Some(confidence),
    }
}

fn says(pairs: &[(&str, &str, f64)]) -> Response {
    Response {
        model: "jev-1.13.0".to_string(),
        answers: pairs
            .iter()
            .map(|(id, choice, confidence)| ((*id).to_string(), answer(choice, *confidence)))
            .collect::<BTreeMap<_, _>>(),
        usage: Usage {
            input_tokens: 2400,
            output_tokens: 20,
        },
    }
}

/// Answers from a script, repeating the last one once the script runs out so a
/// test only has to say what is interesting.
struct Scripted(Mutex<Vec<Response>>);

impl Scripted {
    fn new(mut responses: Vec<Response>) -> Self {
        responses.reverse();
        Self(Mutex::new(responses))
    }
}

impl Oracle for Scripted {
    async fn ask_within(
        &self,
        _state: &serde_json::Value,
        _questions: &BTreeMap<String, Question>,
        _budget: std::time::Duration,
    ) -> Result<Response, JevError> {
        let mut script = self.0.lock().expect("the script");
        if script.len() > 1 {
            Ok(script.pop().expect("checked"))
        } else {
            Ok(script
                .last()
                .cloned()
                .unwrap_or_else(|| says(&[(OPERATION, "BLOCKED", 1.0)])))
        }
    }
}

/// A browser whose page and act results are scripted.
struct Fake {
    acts: Mutex<Vec<Result<bool, ActError>>>,
    pages: Mutex<Vec<PageView>>,
    settles: Mutex<u32>,
    /// Observations succeed this many times, then fail the way a closed tab
    /// does. Counted rather than flagged, so the first view can be built and
    /// only the read-back fails, which is the case worth testing.
    blind_after: std::sync::atomic::AtomicU32,
    observed: std::sync::atomic::AtomicU32,
}

impl Fake {
    fn new(acts: Vec<Result<bool, ActError>>, pages: Vec<PageView>) -> Self {
        Self {
            acts: Mutex::new({
                let mut acts = acts;
                acts.reverse();
                acts
            }),
            pages: Mutex::new({
                let mut pages = pages;
                pages.reverse();
                pages
            }),
            settles: Mutex::new(0),
            blind_after: std::sync::atomic::AtomicU32::new(u32::MAX),
            observed: std::sync::atomic::AtomicU32::new(0),
        }
    }

    fn blind_after(self, observations: u32) -> Self {
        self.blind_after
            .store(observations, std::sync::atomic::Ordering::SeqCst);
        self
    }

    fn with_one_page(acts: Vec<Result<bool, ActError>>) -> Self {
        Self::new(acts, vec![page(1)])
    }

    fn settles(&self) -> u32 {
        *self.settles.lock().expect("the counter")
    }
}

fn page(fingerprint: u64) -> PageView {
    PageView {
        url: format!("https://example.com/?v={fingerprint}"),
        title: "Page".to_string(),
        text: "visible text".to_string(),
        controls: vec![control("e1", "button", "Apply")],
        fingerprint,
    }
}

impl Driver for Fake {
    fn stopped(&self) -> bool {
        false
    }

    async fn observe(&self) -> Result<PageView, String> {
        let seen = self
            .observed
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if seen >= self.blind_after.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("the tab was closed".to_string());
        }
        let mut pages = self.pages.lock().expect("the pages");
        if pages.len() > 1 {
            Ok(pages.pop().expect("checked"))
        } else {
            Ok(pages.last().cloned().unwrap_or_else(|| page(1)))
        }
    }

    async fn act(&self, _decision: &Decision) -> Result<bool, ActError> {
        self.acts
            .lock()
            .expect("the acts")
            .pop()
            .unwrap_or(Ok(false))
    }

    async fn settle(&self) {
        *self.settles.lock().expect("the counter") += 1;
    }
}

fn budget() -> Budget {
    Budget {
        max_steps: 12,
        max_seconds: 30,
    }
}

/// A stale page is not a failure. The previous implementation ended a run on
/// any error from the browser, which cost three runs in one benchmark round for
/// zero actions and about 15k input tokens each.
#[tokio::test]
async fn a_stale_act_is_retried_rather_than_ending_the_run() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])]);
    let fake = Fake::with_one_page(vec![
        Err(ActError::Stale("the control moved".to_string())),
        Ok(true),
    ]);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert!(
        outcome.actions() >= 1,
        "the retry produced an action, status {:?}",
        outcome.status
    );
    assert_ne!(
        outcome.status,
        Status::Failed("the control moved".to_string()),
        "a stale page must not end the run"
    );
}

/// A mutation whose completion is unknown ends the run. A select's change event
/// may already have fired, so running it again would set the value twice.
#[tokio::test]
async fn an_uncertain_mutation_ends_the_run_rather_than_retrying() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "SELECT", 0.98),
        (SELECT_VALUE, "e1::Price", 0.98),
    ])]);
    let fake = Fake::with_one_page(vec![Err(ActError::Fatal(
        "the dropdown's result was not confirmed".to_string(),
    ))]);
    let outcome = pursue(&oracle, &fake, "Sort by price", budget()).await;
    assert_eq!(
        outcome.status,
        Status::Failed("the dropdown's result was not confirmed".to_string())
    );
    assert_eq!(outcome.actions(), 0);
}

/// Three consecutive actions that changed nothing stop the run. Counting only
/// actions, so a model that keeps claiming progress cannot keep it alive.
#[tokio::test]
async fn three_actions_that_change_nothing_stop_the_run() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])]);
    let fake = Fake::with_one_page(vec![Ok(false), Ok(false), Ok(false), Ok(false)]);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert_eq!(outcome.status, Status::Stalled);
    assert_eq!(
        outcome.changed(),
        0,
        "the outcome says plainly that nothing moved"
    );
}

/// A goal needing typed text is handed back, not guessed at. This model is not
/// trained to generate text, so inventing a value would be the wrong fix.
#[tokio::test]
async fn a_goal_needing_text_is_handed_back() {
    let mut field = control("e2", "textbox", "Search");
    field.state.editable = true;
    let mut view = page(1);
    view.controls.push(field);

    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "FILL", 0.99),
        ("fill_target", "e2", 0.99),
    ])]);
    let fake = Fake::new(vec![], vec![view]);
    let outcome = pursue(&oracle, &fake, "Search for 32GB DDR5", budget()).await;
    assert!(
        matches!(outcome.status, Status::NeedsInput(_)),
        "got {:?}",
        outcome.status
    );
    assert_eq!(outcome.actions(), 0, "nothing was typed");
}

/// A terminal answer is checked against the page before it is believed. The
/// model looked at a page; if that page has moved, its verdict is about
/// something that is no longer there.
#[tokio::test]
async fn done_is_accepted_only_on_the_page_it_was_decided_about() {
    let oracle = Scripted::new(vec![says(&[(OPERATION, "DONE", 0.99)])]);
    // Observes page 1 first, then a different page, so the terminal answer is
    // about a page that has moved.
    let fake = Fake::new(vec![], vec![page(1), page(2), page(2)]);
    let outcome = pursue(&oracle, &fake, "Be finished", budget()).await;
    // The first DONE is about a page that has since moved, so it is asked
    // again rather than accepted. Once the page settles the answer stands, so
    // the contract is that it was re-checked, not that it was refused.
    assert!(
        outcome.decisions >= 2,
        "the terminal answer was re-asked on the settled page, decisions {}",
        outcome.decisions
    );
    assert_eq!(outcome.status, Status::Satisfied);
    assert_eq!(outcome.actions(), 0, "nothing was done to reach it");
}

/// On a settled page, DONE is accepted.
#[tokio::test]
async fn done_on_a_settled_page_is_satisfied() {
    let oracle = Scripted::new(vec![says(&[(OPERATION, "DONE", 0.99)])]);
    let fake = Fake::with_one_page(vec![]);
    let outcome = pursue(&oracle, &fake, "Be finished", budget()).await;
    assert_eq!(outcome.status, Status::Satisfied);
}

/// The page is settled in code after an action rather than by spending a
/// decision on a wait. A wait decision costs a whole request.
#[tokio::test]
async fn the_page_is_settled_in_code_after_an_action() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])]);
    let fake = Fake::with_one_page(vec![Ok(true), Ok(false), Ok(false), Ok(false)]);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert!(
        fake.settles() >= 1,
        "settling happened in code, after {} actions",
        outcome.actions()
    );
}

/// The outcome reports what was observed, so a caller can check the claim
/// rather than trust a label.
#[tokio::test]
async fn the_outcome_reports_what_actually_changed() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])]);
    let fake = Fake::with_one_page(vec![Ok(true), Ok(false), Ok(false), Ok(false)]);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert_eq!(outcome.changed(), 1);
    assert!(outcome.actions() >= 1);
    assert!(outcome.input_tokens > 0, "the cost is reported");
    assert!(!outcome.url_before.is_empty());
}

/// A step budget is a real limit.
#[tokio::test]
async fn the_step_budget_ends_the_run() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])]);
    let fake = Fake::with_one_page(vec![Ok(true); 20]);
    let outcome = pursue(
        &oracle,
        &fake,
        "Click apply",
        Budget {
            max_steps: 2,
            max_seconds: 30,
        },
    )
    .await;
    assert_eq!(outcome.status, Status::OutOfBudget);
    assert_eq!(outcome.actions(), 2);
}

/// An unsure decision is not executed. The gate's job, seen from the loop.
#[tokio::test]
async fn an_unsure_decision_is_not_executed() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.30),
    ])]);
    let fake = Fake::with_one_page(vec![Ok(true); 8]);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert_eq!(
        outcome.actions(),
        0,
        "a 0.30 target never reaches the browser, status {:?}",
        outcome.status
    );
}

/// Waiting never reaches the browser: the act tool has no kind for it, so
/// sending it there ended the run. It is offered on every page, so this was
/// reachable on any goal.
#[tokio::test]
async fn waiting_does_not_reach_the_browser() {
    let oracle = Scripted::new(vec![says(&[(OPERATION, "WAIT", 0.99)])]);
    // Any act at all would be wrong, so the script offers none: the fake
    // returns Ok(false) for an unexpected call, and a run that acted would
    // show an action in its trail.
    let fake = Fake::with_one_page(vec![]);
    let outcome = pursue(&oracle, &fake, "Wait for the page", budget()).await;
    assert_eq!(
        outcome.actions(),
        0,
        "a wait is not an action, status {:?}",
        outcome.status
    );
    assert_ne!(
        outcome.status,
        Status::Failed("WAIT does not reach the browser".to_string()),
        "waiting must not be sent to the browser"
    );
    assert!(fake.settles() >= 1, "it settled and looked again instead");
}

/// A run that only ever waits makes no observable progress, so it stops rather
/// than spending its whole budget looking busy.
#[tokio::test]
async fn a_run_that_only_waits_stops() {
    let oracle = Scripted::new(vec![says(&[(OPERATION, "WAIT", 0.99)])]);
    let fake = Fake::with_one_page(vec![]);
    let outcome = pursue(&oracle, &fake, "Wait forever", budget()).await;
    assert_eq!(outcome.status, Status::Stalled);
    assert!(
        outcome.decisions <= 5,
        "it stopped quickly rather than burning the budget, decisions {}",
        outcome.decisions
    );
}

/// The guard is compared against a page observed after the decision was made.
/// Comparing it against the page the decision was made on always agreed with
/// itself, so a control that changed under the decision was still acted on.
///
/// Here the control never settles: every observation gives it a different
/// guard, so no decision is ever about the control that is there now and
/// nothing is clicked.
#[tokio::test]
async fn a_control_that_keeps_changing_is_never_acted_on() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])]);
    let shifting: Vec<PageView> = (1..=12)
        .map(|guard| {
            let mut page = page(guard);
            page.controls = vec![Control {
                reference: "e1".to_string(),
                role: "button".to_string(),
                name: "Apply".to_string(),
                value: None,
                state: ControlState::default(),
                guard,
                options: Vec::new(),
            }];
            page
        })
        .collect();
    let fake = Fake::new(vec![Ok(true); 6], shifting);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert_eq!(
        outcome.actions(),
        0,
        "the control was never the one the decision was about, status {:?}",
        outcome.status
    );
    assert_eq!(outcome.status, Status::Stalled);
    assert!(
        outcome.decisions > 1,
        "decisions were spent and refused, not executed"
    );
}

/// And once it does settle, the re-decided answer is acted on: the check
/// refuses a stale decision rather than refusing to work.
#[tokio::test]
async fn a_control_that_settles_is_acted_on_after_one_refusal() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])]);
    let mut settled = page(2);
    settled.controls = vec![Control {
        reference: "e1".to_string(),
        role: "button".to_string(),
        name: "Apply".to_string(),
        value: None,
        state: ControlState::default(),
        guard: 999,
        options: Vec::new(),
    }];
    let fake = Fake::new(vec![Ok(true); 6], vec![page(1), settled]);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert!(
        outcome.actions() >= 1,
        "it acted once the page settled, status {:?}",
        outcome.status
    );
    assert!(
        outcome.decisions > outcome.actions() as u32,
        "the first decision was refused: {} decisions for {} actions",
        outcome.decisions,
        outcome.actions()
    );
}

/// The escape answer is a question for the caller, not something for the loop
/// to explore around. Previously it re-observed, rebuilt the same shortlist on
/// an unchanged page, got the same answer, and spent three decisions reaching a
/// stall with nothing done.
#[tokio::test]
async fn the_escape_answer_hands_back_with_what_it_was_choosing_between() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.99),
        (CLICK_TARGET, "none_of_these", 1.0),
    ])]);
    let fake = Fake::with_one_page(vec![]);
    let outcome = pursue(&oracle, &fake, "Tick a filter that is not here", budget()).await;

    assert!(
        matches!(outcome.status, Status::NeedsInput(_)),
        "got {:?}",
        outcome.status
    );
    assert_eq!(outcome.actions(), 0, "nothing was done");
    assert_eq!(
        outcome.decisions, 1,
        "and it handed back on the first answer rather than going round three times"
    );
    assert!(
        !outcome.offered_controls.is_empty(),
        "the caller is told what the decision was choosing between"
    );
    assert!(
        outcome
            .offered_controls
            .iter()
            .any(|control| control.contains("Apply")),
        "by name: {:?}",
        outcome.offered_controls
    );
    assert_eq!(outcome.offered, 1);
}

/// A terminal answer carries the confidence behind it, because the published
/// guidance is to use confidence to act, ask for review, or escalate, and
/// handing the number to the caller is the review branch. It is not gated on:
/// a low-confidence claim is still reported, with its number attached.
#[tokio::test]
async fn a_completion_claim_carries_its_confidence_and_is_not_refused() {
    for confidence in [0.21, 0.99] {
        let oracle = Scripted::new(vec![says(&[(OPERATION, "DONE", confidence)])]);
        let fake = Fake::with_one_page(vec![]);
        let outcome = pursue(&oracle, &fake, "Be finished", budget()).await;
        assert_eq!(
            outcome.status,
            Status::Satisfied,
            "a claim at {confidence} is reported, not refused"
        );
        assert_eq!(
            outcome.terminal_confidence,
            Some(confidence),
            "and the caller can see how sure it was"
        );
    }
}

/// An ending that was not a terminal answer has no terminal confidence to
/// report, rather than a zero that would read as certainty of the opposite.
#[tokio::test]
async fn a_non_terminal_ending_reports_no_terminal_confidence() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])]);
    let fake = Fake::with_one_page(vec![Ok(false), Ok(false), Ok(false)]);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert_eq!(outcome.status, Status::Stalled);
    assert_eq!(outcome.terminal_confidence, None);
}

/// A blocked run reports what the decision could see, because that is the
/// answer the model actually gives when nothing on the page advances the goal,
/// and the caller is the one deciding what happens next.
#[tokio::test]
async fn a_blocked_run_tells_the_caller_what_it_was_choosing_between() {
    let oracle = Scripted::new(vec![says(&[(OPERATION, "BLOCKED", 0.96)])]);
    let fake = Fake::with_one_page(vec![]);
    let outcome = pursue(&oracle, &fake, "Tick a filter that is not here", budget()).await;

    assert!(
        matches!(outcome.status, Status::Blocked(_)),
        "got {:?}",
        outcome.status
    );
    assert_eq!(outcome.terminal_confidence, Some(0.96));
    assert_eq!(outcome.offered, 1, "it reports how much it could see");
    assert!(
        outcome
            .offered_controls
            .iter()
            .any(|control| control.contains("Apply")),
        "and names it: {:?}",
        outcome.offered_controls
    );
}

/// A completion claim needs a real observation to check against. Falling back
/// to the previous page made the check pass by construction, so a closed tab
/// read as a satisfied goal.
///
/// The first observation succeeds, so the run gets a view and a decision; only
/// the read-back fails. Blinding the browser from the start would fail at
/// startup instead and never reach the path this covers.
#[tokio::test]
async fn a_completion_claim_is_not_confirmed_from_a_failed_observation() {
    let fake = Fake::with_one_page(vec![]).blind_after(1);
    let oracle = Scripted::new(vec![says(&[(OPERATION, "DONE", 0.99)])]);
    let outcome = pursue(&oracle, &fake, "Be finished", budget()).await;
    assert_ne!(
        outcome.status,
        Status::Satisfied,
        "nothing was read back, so nothing is confirmed"
    );
    assert!(
        matches!(outcome.status, Status::NeedsInput(ref reason) if reason.contains("could not be read back")),
        "and it says why, rather than reporting a failure: {:?}",
        outcome.status
    );
}

/// With the read-back working, the same claim is accepted, so this refuses an
/// unverifiable claim rather than refusing claims.
#[tokio::test]
async fn a_completion_claim_with_a_working_read_back_is_accepted() {
    let fake = Fake::with_one_page(vec![]);
    let oracle = Scripted::new(vec![says(&[(OPERATION, "DONE", 0.99)])]);
    let outcome = pursue(&oracle, &fake, "Be finished", budget()).await;
    assert_eq!(outcome.status, Status::Satisfied);
}

/// Every ending reports what the last decision could see. Only the blocked and
/// hand-back paths set it before, so a stalled or satisfied run reported zero
/// controls offered even after choosing from a capped list, which the output
/// schema declares as always present.
#[tokio::test]
async fn every_ending_reports_what_the_decision_could_see() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])]);
    let fake = Fake::with_one_page(vec![Ok(false), Ok(false), Ok(false)]);
    let stalled = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert_eq!(stalled.status, Status::Stalled);
    assert_eq!(
        stalled.offered, 1,
        "a stalled run still says how much of the page it was choosing from"
    );

    let oracle = Scripted::new(vec![says(&[(OPERATION, "DONE", 0.9)])]);
    let fake = Fake::with_one_page(vec![]);
    let satisfied = pursue(&oracle, &fake, "Be finished", budget()).await;
    assert_eq!(satisfied.status, Status::Satisfied);
    assert_eq!(satisfied.offered, 1, "and so does a satisfied one");
}
