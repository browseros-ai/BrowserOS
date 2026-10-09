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
    async fn ask(
        &self,
        _state: &serde_json::Value,
        _questions: &BTreeMap<String, Question>,
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
        }
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
