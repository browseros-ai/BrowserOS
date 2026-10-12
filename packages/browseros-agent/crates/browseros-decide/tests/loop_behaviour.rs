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
use browseros_decide::operations::Operation;
use browseros_decide::pursue::{
    ActError, Arrival, Budget, Driver, Journey, Oracle, Status, Stuck, advance, pursue,
};
use browseros_decide::questions::{CLICK_TARGET, OPERATION, SELECT_VALUE};
use browseros_decide::run::{Asking, Destination, NamedValue, Pending, Run};
use browseros_decide::view::{Control, ControlState, PageView};

fn control(reference: &str, role: &str, name: &str) -> Control {
    Control {
        reference: reference.to_string(),
        clickable: false,
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
    acts: Mutex<Vec<Result<(), ActError>>>,
    pages: Mutex<Vec<PageView>>,
    settles: Mutex<u32>,
    /// Every journey asked for, in order, so a test can say where the run went
    /// rather than only that it went somewhere.
    journeys: Mutex<Vec<Journey>>,
    /// Observations succeed this many times, then fail the way a closed tab
    /// does. Counted rather than flagged, so the first view can be built and
    /// only the read-back fails, which is the case worth testing.
    blind_after: std::sync::atomic::AtomicU32,
    observed: std::sync::atomic::AtomicU32,
    /// When set, acting advances the page's identity, the way a real action
    /// that changes something does. Observations alone must not move it, or a
    /// test cannot tell an action's effect from a page settling.
    moves_on_act: std::sync::atomic::AtomicBool,
    version: std::sync::atomic::AtomicU64,
}

impl Fake {
    fn new(acts: Vec<Result<(), ActError>>, pages: Vec<PageView>) -> Self {
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
            journeys: Mutex::new(Vec::new()),
            blind_after: std::sync::atomic::AtomicU32::new(u32::MAX),
            observed: std::sync::atomic::AtomicU32::new(0),
            moves_on_act: std::sync::atomic::AtomicBool::new(false),
            version: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Makes the first action change the page, and later ones leave it alone,
    /// which is the ordinary shape of a run that gets somewhere and then stops.
    fn moving_once(self) -> Self {
        self.moves_on_act
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self
    }

    fn blind_after(self, observations: u32) -> Self {
        self.blind_after
            .store(observations, std::sync::atomic::Ordering::SeqCst);
        self
    }

    fn with_one_page(acts: Vec<Result<(), ActError>>) -> Self {
        Self::new(acts, vec![page(1)])
    }

    fn settles(&self) -> u32 {
        *self.settles.lock().expect("the counter")
    }

    fn journeys(&self) -> Vec<Journey> {
        self.journeys.lock().expect("the journeys").clone()
    }
}

fn page(fingerprint: u64) -> PageView {
    PageView {
        url: format!("https://example.com/?v={fingerprint}"),
        title: "Page".to_string(),
        text: "visible text".to_string(),
        controls: vec![control("e1", "button", "Apply")],
        fingerprint,
        unlabelled_images: 0,
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
            return Ok(pages.pop().expect("checked"));
        }
        let mut current = pages.last().cloned().unwrap_or_else(|| page(1));
        // The version an action moved, so an observation reflects what the
        // action did rather than a page the test queued in advance.
        let version = self.version.load(std::sync::atomic::Ordering::SeqCst);
        if version > 1 {
            current = page(version);
        }
        Ok(current)
    }

    async fn act(&self, _decision: &Decision) -> Result<(), ActError> {
        let result = self.acts.lock().expect("the acts").pop().unwrap_or(Ok(()));
        if result.is_ok()
            && self
                .moves_on_act
                .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.version
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        result
    }

    async fn settle(&self) {
        *self.settles.lock().expect("the counter") += 1;
    }

    async fn travel(&self, journey: &Journey) -> Result<Arrival, ActError> {
        self.journeys
            .lock()
            .expect("the journeys")
            .push(journey.clone());
        let new_tab = matches!(journey, Journey::To(destination) if destination.new_tab);
        if !new_tab {
            // A same-tab move changes the page, the way a real one does.
            self.version
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        let url = match journey {
            Journey::To(destination) => destination.url.clone(),
            Journey::Back => "https://example.com/?v=1".to_string(),
        };
        Ok(Arrival {
            page: 1,
            url,
            opened: new_tab.then_some(7),
        })
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
        Ok(()),
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
    // A page that actually has the dropdown, because the loop only acts on an
    // operation the step offered, and a select is offered only when something
    // on the page has values to set.
    let mut sorter = control("e1", "combobox", "Sort by");
    sorter.options = vec!["Price".to_string(), "Relevance".to_string()];
    let mut view = page(1);
    view.controls = vec![sorter];
    let fake = Fake::new(
        vec![Err(ActError::Fatal(
            "the dropdown's result was not confirmed".to_string(),
        ))],
        vec![view],
    );
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
    let fake = Fake::with_one_page(vec![Ok(()), Ok(()), Ok(()), Ok(())]);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert!(
        matches!(outcome.status, Status::Stalled(_)),
        "got {:?}",
        outcome.status
    );
    assert_eq!(
        outcome.changed(),
        0,
        "the outcome says plainly that nothing moved"
    );
}

/// A page with one search box, which is the smallest shape that reaches a fill.
fn page_with_a_search_box() -> PageView {
    let mut field = control("e2", "textbox", "Search");
    field.state.editable = true;
    let mut view = page(1);
    view.controls.push(field);
    view
}

fn wants_to_type() -> Scripted {
    Scripted::new(vec![says(&[
        (OPERATION, "FILL", 0.99),
        ("fill_target", "e2", 0.99),
    ])])
}

/// A goal needing typed text is asked about, not guessed at. This model is not
/// trained to generate text, so inventing a value would be the wrong fix.
///
/// The ask names the field, because the caller has to know what it is answering
/// about: a sentence saying only that text is needed left it to read prose and
/// then start again.
#[tokio::test]
async fn a_goal_needing_text_asks_for_it_and_names_the_field() {
    let fake = Fake::new(vec![], vec![page_with_a_search_box()]);
    let outcome = pursue(&wants_to_type(), &fake, "Search for 32GB DDR5", budget()).await;

    let Status::NeedsValue(Pending::Text {
        reference,
        control,
        confidence,
        asking,
        ..
    }) = &outcome.status
    else {
        panic!("got {:?}", outcome.status);
    };
    assert_eq!(reference, "e2");
    assert_eq!(control, "textbox \"Search\"");
    assert!((confidence - 0.99).abs() < f64::EPSILON);
    assert_eq!(*asking, Asking::FirstTime);
    assert_eq!(outcome.actions(), 0, "nothing was typed");
    assert!(
        outcome.resume.is_some(),
        "an ask the caller can answer has to carry the run"
    );
    assert!(
        !outcome.offered_controls.is_empty(),
        "and what it was choosing between, in case it chose the wrong field"
    );
}

/// The answer arrives and the run carries on. Before this the caller had to
/// start a new run, which observed, ranked and decided from nothing.
#[tokio::test]
async fn an_answered_run_types_the_text_and_continues() {
    let asked = pursue(
        &wants_to_type(),
        &Fake::new(vec![], vec![page_with_a_search_box()]),
        "Search for 32GB DDR5",
        budget(),
    )
    .await;
    let mut run = asked.resume.expect("an ask is resumable");
    run.supply("e2", "32GB DDR5");

    let fake = Fake::new(vec![Ok(())], vec![page_with_a_search_box()]).moving_once();
    let outcome = advance(&wants_to_type(), &fake, run).await;

    assert_eq!(outcome.actions(), 1, "the field was filled");
    let step = &outcome.trail[0];
    assert_eq!(step.operation, Operation::Fill);
    assert_eq!(step.target.as_deref(), Some("e2"));
    assert_eq!(
        step.value.as_deref(),
        Some("32GB DDR5"),
        "the caller's text is what reaches the browser"
    );
    assert!(step.page_changed);
}

/// A reference is a handle into one snapshot. After the page moves the same
/// `e2` can denote a different control, and typing the caller's text into it is
/// the one harm this whole exchange exists to avoid.
#[tokio::test]
async fn an_answer_is_not_typed_into_a_control_that_changed() {
    let asked = pursue(
        &wants_to_type(),
        &Fake::new(vec![], vec![page_with_a_search_box()]),
        "Search for 32GB DDR5",
        budget(),
    )
    .await;
    let mut run = asked.resume.expect("resumable");
    run.supply("e2", "32GB DDR5");

    // Same reference, different control behind it.
    let mut moved = page_with_a_search_box();
    moved.controls[1].guard = 99;
    let fake = Fake::new(vec![Ok(())], vec![moved]);
    let outcome = advance(&wants_to_type(), &fake, run).await;

    assert_eq!(outcome.actions(), 0, "nothing was typed into it");
    let Status::NeedsValue(Pending::Text { asking, .. }) = &outcome.status else {
        panic!("got {:?}", outcome.status);
    };
    assert_eq!(
        *asking,
        Asking::FieldChanged,
        "and the caller is told its answer was received and then discarded, \
         which is not the same event as the first ask"
    );
}

/// An ask takes no action, so neither the step budget nor the stall counters
/// bound it. Without a limit of its own a field that changes under every answer
/// would trade round trips with the caller forever.
#[tokio::test]
async fn a_field_that_never_holds_still_stops_being_asked_about() {
    let mut run = Run::new("Search for 32GB DDR5", budget());
    run.asks = browseros_decide::pursue::ASK_LIMIT;

    let fake = Fake::new(vec![], vec![page_with_a_search_box()]);
    let outcome = advance(&wants_to_type(), &fake, run).await;

    assert!(
        matches!(outcome.status, Status::NeedsInput(_)),
        "got {:?}",
        outcome.status
    );
    assert!(
        outcome.resume.is_none(),
        "it has stopped asking, so it must not invite another answer"
    );
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
    let fake = Fake::with_one_page(vec![Ok(()), Ok(()), Ok(()), Ok(())]);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert!(
        fake.settles() >= 1,
        "settling happened in code, after {} actions",
        outcome.actions()
    );
}

/// The outcome reports what was observed, so a caller can check the claim
/// rather than trust a label.
///
/// Whether the page moved comes from comparing page identity across the action,
/// not from anything the browser tool said. That was wrong twice: once reading
/// prose that never carried it, once reading structured content the dispatch
/// strips from a tool with no output schema. The page either has the identity
/// it had or it does not.
#[tokio::test]
async fn the_outcome_reports_what_actually_changed() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])]);
    // The page moves once and then settles, so exactly one action changed it.
    let fake = Fake::with_one_page(vec![Ok(()); 6]).moving_once();
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert_eq!(
        outcome.changed(),
        1,
        "one action moved the page, status {:?}",
        outcome.status
    );
    assert!(outcome.actions() >= 1);
    assert!(outcome.input_tokens > 0, "the cost is reported");
    assert!(!outcome.url_before.is_empty());
}

/// And an action that leaves the page as it was is reported as changing
/// nothing, which is what the stall rule counts.
#[tokio::test]
async fn an_action_that_moves_nothing_is_reported_as_such() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])]);
    let fake = Fake::with_one_page(vec![Ok(()); 6]);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert_eq!(outcome.changed(), 0);
    assert!(
        matches!(outcome.status, Status::Stalled(_)),
        "got {:?}",
        outcome.status
    );
}

/// A step budget is a real limit.
#[tokio::test]
async fn the_step_budget_ends_the_run() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])]);
    let fake = Fake::with_one_page(vec![Ok(()); 20]);
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
    let fake = Fake::with_one_page(vec![Ok(()); 8]);
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
    // returns Ok(()) for an unexpected call, and a run that acted would
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
    assert!(
        matches!(outcome.status, Status::Stalled(_)),
        "got {:?}",
        outcome.status
    );
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
                clickable: false,
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
    let fake = Fake::new(vec![Ok(()); 6], shifting);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert_eq!(
        outcome.actions(),
        0,
        "the control was never the one the decision was about, status {:?}",
        outcome.status
    );
    assert!(
        matches!(outcome.status, Status::Stalled(_)),
        "got {:?}",
        outcome.status
    );
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
        clickable: false,
        role: "button".to_string(),
        name: "Apply".to_string(),
        value: None,
        state: ControlState::default(),
        guard: 999,
        options: Vec::new(),
    }];
    let fake = Fake::new(vec![Ok(()); 6], vec![page(1), settled]);
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
    let fake = Fake::with_one_page(vec![Ok(()), Ok(()), Ok(())]);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert!(
        matches!(outcome.status, Status::Stalled(_)),
        "got {:?}",
        outcome.status
    );
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
    let fake = Fake::with_one_page(vec![Ok(()), Ok(()), Ok(())]);
    let stalled = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert!(matches!(stalled.status, Status::Stalled(_)));
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

/// The reason a run stopped has to be true of what happened. One counter used
/// to serve four situations and the caller was told about one of them, so a run
/// that applied a filter in a single successful action was told that three
/// actions in a row had changed nothing.
#[tokio::test]
async fn a_stall_names_the_reason_that_actually_happened() {
    // Nothing changed: actions ran and left the page alone.
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])]);
    let fake = Fake::with_one_page(vec![Ok(()); 6]);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert_eq!(outcome.status, Status::Stalled(Stuck::NothingChanged));
    assert!(
        outcome.actions() >= 1,
        "and it really did run actions, which is what makes that reason the true one"
    );

    // Not confident: the gate declined, so no action ever ran. The old message
    // would have claimed three actions changed nothing.
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.20),
    ])]);
    let fake = Fake::with_one_page(vec![Ok(()); 6]);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert_eq!(outcome.status, Status::Stalled(Stuck::NotConfident));
    assert_eq!(
        outcome.actions(),
        0,
        "no action ran, so no reason mentioning actions could be true"
    );

    // Only waiting: no action either, and a different reason again.
    let oracle = Scripted::new(vec![says(&[(OPERATION, "WAIT", 0.99)])]);
    let fake = Fake::with_one_page(vec![]);
    let outcome = pursue(&oracle, &fake, "Wait forever", budget()).await;
    assert_eq!(outcome.status, Status::Stalled(Stuck::OnlyWaiting));
}

/// A control that keeps moving gets its own reason, rather than being counted
/// against the budget for actions that did nothing.
#[tokio::test]
async fn a_control_that_keeps_moving_says_so() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])]);
    let shifting: Vec<PageView> = (1..=12)
        .map(|guard| {
            let mut page = page(guard);
            page.controls = vec![Control {
                reference: "e1".to_string(),
                clickable: false,
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
    let fake = Fake::new(vec![Ok(()); 6], shifting);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert_eq!(outcome.status, Status::Stalled(Stuck::ControlKeptMoving));
}

/// Each reason has its own budget. Three refusals to act is not three actions
/// that did nothing, and one shared counter meant whichever came first spent
/// the other's allowance.
#[tokio::test]
async fn the_reasons_do_not_share_a_budget() {
    // Two refusals then a working action: the refusals must not have eaten the
    // allowance that lets the run continue.
    let oracle = Scripted::new(vec![
        says(&[(OPERATION, "CLICK", 0.95), (CLICK_TARGET, "e1", 0.20)]),
        says(&[(OPERATION, "CLICK", 0.95), (CLICK_TARGET, "e1", 0.20)]),
        says(&[(OPERATION, "CLICK", 0.95), (CLICK_TARGET, "e1", 0.95)]),
    ]);
    let fake = Fake::with_one_page(vec![Ok(()); 6]).moving_once();
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert!(
        outcome.actions() >= 1,
        "the action after two refusals still ran, status {:?}",
        outcome.status
    );
    assert_eq!(outcome.changed(), 1);
}

/// A stall carries what the decision could see, like its neighbours. It is the
/// ending that fires most on a hard page and it told the caller least.
#[tokio::test]
async fn a_stall_says_what_it_was_choosing_between() {
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.20),
    ])]);
    let fake = Fake::with_one_page(vec![Ok(()); 6]);
    let outcome = pursue(&oracle, &fake, "Click apply", budget()).await;
    assert!(
        !outcome.offered_controls.is_empty(),
        "a stall names the controls it had, status {:?}",
        outcome.status
    );
    assert!(
        outcome
            .offered_controls
            .iter()
            .any(|control| control.contains("Apply"))
    );
}

/// A click that acts once and then runs out of steps, which is the shape every
/// continuation test below starts from.
fn one_click() -> Scripted {
    Scripted::new(vec![says(&[
        (OPERATION, "CLICK", 0.95),
        (CLICK_TARGET, "e1", 0.95),
    ])])
}

fn one_step() -> Budget {
    Budget {
        max_steps: 1,
        max_seconds: 30,
    }
}

/// The caller is told to call pursue again to continue, and until the run
/// became a value that sentence started the whole thing over: a fresh
/// observation, a fresh ranking, and a first decision spent re-deciding what
/// the previous call already knew.
#[tokio::test]
async fn running_out_of_budget_hands_the_run_back() {
    let outcome = pursue(
        &one_click(),
        &Fake::with_one_page(vec![Ok(())]),
        "apply",
        one_step(),
    )
    .await;

    assert_eq!(outcome.status, Status::OutOfBudget);
    assert_eq!(outcome.actions(), 1);
    assert!(
        outcome.resume.is_some(),
        "an ending the caller is invited to continue has to carry the run"
    );
}

/// The endings that are judgements rather than missing values. Handing the run
/// back for these would invite a caller to retry a decision that has already
/// been made three times.
#[tokio::test]
async fn a_stall_is_not_resumable() {
    let outcome = pursue(
        &one_click(),
        &Fake::with_one_page(vec![Ok(()), Ok(()), Ok(())]),
        "apply",
        budget(),
    )
    .await;

    assert_eq!(outcome.status, Status::Stalled(Stuck::NothingChanged));
    assert!(outcome.resume.is_none());
}

#[tokio::test]
async fn a_resumed_run_keeps_the_trail_it_already_had() {
    let first = pursue(
        &one_click(),
        &Fake::with_one_page(vec![Ok(())]),
        "apply",
        one_step(),
    )
    .await;
    let run = first.resume.expect("out of budget is resumable");
    assert_eq!(run.trail.len(), 1);

    let second = advance(&one_click(), &Fake::with_one_page(vec![Ok(())]), run).await;

    assert_eq!(
        second.trail.len(),
        1,
        "the resumed run reports the step the first call took, not a fresh start"
    );
    assert_eq!(second.decisions, 1, "and the decisions it already paid for");
}

/// The step budget is the whole run's, not each call's. Without this a caller
/// could resume forever and never reach a limit.
#[tokio::test]
async fn a_resumed_run_counts_the_steps_the_first_call_took() {
    let first = pursue(
        &one_click(),
        &Fake::with_one_page(vec![Ok(())]),
        "apply",
        one_step(),
    )
    .await;
    let run = first.resume.expect("resumable");

    let second = advance(&one_click(), &Fake::with_one_page(vec![Ok(())]), run).await;

    assert_eq!(second.status, Status::OutOfBudget);
    assert_eq!(
        second.actions(),
        1,
        "the one action is the first call's; the resumed call took none"
    );
    assert!(second.resume.is_some());
}

/// The time budget is the whole run's too. An Instant cannot cross a call, so
/// the run carries the total and each call adds its own elapsed time to it.
#[tokio::test]
async fn a_resumed_run_spends_the_time_the_first_call_used() {
    let mut run = Run::new("apply", budget());
    run.spent = std::time::Duration::from_secs(budget().max_seconds);

    let outcome = advance(&one_click(), &Fake::with_one_page(vec![Ok(())]), run).await;

    assert_eq!(outcome.status, Status::OutOfBudget);
    assert_eq!(
        outcome.actions(),
        0,
        "a run that has already spent its time must not act once more per call"
    );
    assert!(
        outcome.resume.is_none(),
        "and it is not offered back, because no argument gives a run more time: \
         handing it back would invite a call that ends the moment it starts"
    );
}

/// What the caller is judging is the whole pursuit, so the url it started on is
/// the first call's and not the one a later call happened to resume from.
#[tokio::test]
async fn a_resumed_run_reports_the_url_it_originally_started_on() {
    let first = pursue(
        &one_click(),
        &Fake::with_one_page(vec![Ok(())]).moving_once(),
        "apply",
        one_step(),
    )
    .await;
    let run = first.resume.expect("resumable");
    assert_eq!(first.url_before, "https://example.com/?v=1");

    let second = advance(&one_click(), &Fake::new(vec![Ok(())], vec![page(2)]), run).await;

    assert_eq!(second.url_before, "https://example.com/?v=1");
    assert_eq!(second.url_after, "https://example.com/?v=2");
}

fn destination(label: &str, new_tab: bool) -> Destination {
    Destination {
        label: label.to_string(),
        url: format!("https://example.com/{label}"),
        new_tab,
    }
}

fn wants_to_move() -> Scripted {
    Scripted::new(vec![says(&[
        (OPERATION, "GO_TO", 0.95),
        ("destination", "d0", 0.95),
    ])])
}

/// A url is open-ended text and this model does not generate. Choosing between
/// labels the caller wrote is a closed set, which is the one shape it is good
/// at, so navigation is offered only when the caller has authored somewhere to
/// go.
#[tokio::test]
async fn moving_is_not_offered_without_destinations() {
    let fake = Fake::with_one_page(vec![Ok(())]);
    let outcome = pursue(&wants_to_move(), &fake, "show page 2", budget()).await;

    assert!(
        fake.journeys().is_empty(),
        "nothing moved, because nothing was on offer"
    );
    assert_eq!(outcome.actions(), 0);
}

#[tokio::test]
async fn a_destination_the_caller_authored_is_moved_to() {
    let mut run = Run::new("show page 2", budget());
    run.destinations = vec![destination("page 2 of the results", false)];

    let fake = Fake::with_one_page(vec![]);
    let outcome = advance(&wants_to_move(), &fake, run).await;

    assert_eq!(
        fake.journeys(),
        vec![Journey::To(destination("page 2 of the results", false))]
    );
    let step = &outcome.trail[0];
    assert_eq!(step.operation, Operation::GoTo);
    assert_eq!(step.target.as_deref(), Some("page 2 of the results"));
    assert!(step.page_changed, "a move in the same tab changes the page");
}

/// The caller decides whether leaving the page is acceptable, because it is the
/// party that knows the wider intent. A new tab keeps the run where it was, so
/// the one-page story holds and the caller is told what exists now.
#[tokio::test]
async fn a_new_tab_destination_leaves_the_run_where_it_was() {
    let mut run = Run::new("open the deals page", budget());
    run.destinations = vec![destination("the deals page", true)];

    let fake = Fake::with_one_page(vec![]);
    let outcome = advance(&wants_to_move(), &fake, run).await;

    assert_eq!(outcome.opened.len(), 1);
    assert_eq!(outcome.opened[0].page, 7);
    assert_eq!(outcome.url_after, "https://example.com/?v=1");
    assert!(
        !outcome.trail[0].page_changed,
        "this page really did not change, and saying otherwise would be a nicer lie"
    );
}

/// A destination that has been visited cannot advance the goal a second time,
/// and a new-tab destination leaves this page unchanged, so without this the
/// model could spend a whole budget opening one tab.
#[tokio::test]
async fn a_destination_is_not_offered_twice() {
    let mut run = Run::new("open the deals page", budget());
    run.destinations = vec![destination("the deals page", true)];
    run.travelled = vec!["the deals page".to_string()];

    let fake = Fake::with_one_page(vec![Ok(())]);
    let outcome = advance(&wants_to_move(), &fake, run).await;

    assert!(fake.journeys().is_empty());
    assert_eq!(outcome.actions(), 0);
}

/// Going back is offered only once the run has moved, so it can never reverse
/// off the page the caller handed it.
#[tokio::test]
async fn going_back_is_not_offered_before_the_run_has_moved() {
    let oracle = Scripted::new(vec![says(&[(OPERATION, "GO_BACK", 0.95)])]);
    let fake = Fake::with_one_page(vec![Ok(())]);
    let outcome = pursue(&oracle, &fake, "go back", budget()).await;

    assert!(fake.journeys().is_empty());
    assert_eq!(outcome.actions(), 0);
}

#[tokio::test]
async fn going_back_is_offered_once_the_run_has_moved() {
    let mut run = Run::new("go back", budget());
    run.navigations = 1;

    let oracle = Scripted::new(vec![says(&[(OPERATION, "GO_BACK", 0.95)])]);
    let fake = Fake::with_one_page(vec![]);
    let outcome = advance(&oracle, &fake, run).await;

    assert_eq!(fake.journeys(), vec![Journey::Back]);
    assert_eq!(outcome.trail[0].operation, Operation::GoBack);
}

/// Leaving the page is the one thing in this vocabulary that looking again
/// cannot undo, so it is rated high and judged at the high bar.
#[tokio::test]
async fn a_move_below_the_high_bar_does_not_happen() {
    let mut run = Run::new("show page 2", budget());
    run.destinations = vec![destination("page 2 of the results", false)];

    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "GO_TO", 0.80),
        ("destination", "d0", 0.80),
    ])]);
    let fake = Fake::with_one_page(vec![Ok(())]);
    let outcome = advance(&oracle, &fake, run).await;

    assert!(fake.journeys().is_empty(), "0.80 is under the 0.85 bar");
    assert_eq!(outcome.actions(), 0);
}

/// The url costs many tokens and says nothing a Choice can use. The label costs
/// a few and is what the goal is phrased in.
#[test]
fn a_destination_question_never_carries_a_url() {
    let view = page(1);
    let space = browseros_decide::space::ActionSpace::build(&view, "show page 2", None);
    let destinations = [destination("page 2 of the results", false)];
    let offered: Vec<(String, &Destination)> = vec![("d0".to_string(), &destinations[0])];
    let travel = browseros_decide::questions::Travel {
        offered: &offered,
        can_return: false,
    };
    let operations = browseros_decide::questions::available(&space, &view, false, travel);
    let built = browseros_decide::questions::build("show page 2", &space, &operations, travel, &[]);

    let rendered = serde_json::to_string(&built).expect("the questions serialise");
    assert!(rendered.contains("page 2 of the results"));
    assert!(
        !rendered.contains("https://"),
        "the url must not travel to the model: {rendered}"
    );
}

/// Every move is high consequence and cannot be undone by looking again, so a
/// run with no limit on them is a crawler rather than one goal on one page.
#[tokio::test]
async fn a_run_stops_being_offered_moves_once_it_has_made_enough() {
    let mut run = Run::new("show page 2", budget());
    run.destinations = vec![destination("page 2 of the results", false)];
    run.navigations = browseros_decide::pursue::NAVIGATION_LIMIT;

    let fake = Fake::with_one_page(vec![Ok(())]);
    let outcome = advance(&wants_to_move(), &fake, run).await;

    assert!(fake.journeys().is_empty());
    assert_eq!(outcome.actions(), 0);
}

/// Going back twice in a row is an oscillation rather than progress for a run
/// with one goal, and the page keeps changing so the stall counter would never
/// catch it.
#[tokio::test]
async fn going_back_is_not_offered_twice_in_a_row() {
    let mut run = Run::new("go back", budget());
    run.navigations = 1;

    let oracle = Scripted::new(vec![says(&[(OPERATION, "GO_BACK", 0.95)])]);
    let fake = Fake::with_one_page(vec![]);
    let outcome = advance(&oracle, &fake, run).await;

    assert_eq!(
        fake.journeys(),
        vec![Journey::Back],
        "once, and then not again"
    );
    assert_eq!(outcome.trail.len(), 1);
}

/// An operation the step did not offer cannot have been read off the question.
/// Obeying it is how a run with a used-up destination list would reverse off
/// the page the caller handed over.
#[tokio::test]
async fn an_operation_that_was_not_offered_is_not_acted_on() {
    // Nothing editable on the page, so FILL is never on the list.
    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "FILL", 0.99),
        ("fill_target", "e1", 0.99),
    ])]);
    let fake = Fake::with_one_page(vec![Ok(())]);
    let outcome = pursue(&oracle, &fake, "type something", budget()).await;

    assert_eq!(outcome.actions(), 0);
    assert_eq!(outcome.status, Status::Stalled(Stuck::NotConfident));
}

/// The model cannot write a string, but choosing which of a few given strings
/// belongs in a field is a closed set, which is the one thing it does. So a
/// caller that already holds the value is never asked for it.
#[tokio::test]
async fn a_value_the_caller_offered_is_typed_without_asking() {
    let mut run = Run::new("set the delivery pincode to 600042", budget());
    run.values = vec![NamedValue::new("pincode", "600042")];

    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "FILL", 0.99),
        ("fill_target", "e2", 0.99),
        ("fill_value", "v00", 0.99),
    ])]);
    let fake = Fake::new(vec![Ok(())], vec![page_with_a_search_box()]).moving_once();
    let outcome = advance(&oracle, &fake, run).await;

    assert_eq!(
        outcome.actions(),
        1,
        "it typed, rather than parking: {:?}",
        outcome.status
    );
    let step = &outcome.trail[0];
    assert_eq!(step.operation, Operation::Fill);
    assert_eq!(
        step.value.as_deref(),
        Some("600042"),
        "the option id resolves to the caller's text, not to the id"
    );
    assert!(
        outcome.resume.is_none(),
        "nothing to continue: it did not park"
    );
}

/// The escape option is on this head like every other, and taking it means the
/// run still has to ask.
#[tokio::test]
async fn a_value_the_model_declines_still_parks() {
    let mut run = Run::new("set the delivery pincode to 600042", budget());
    run.values = vec![NamedValue::new("quantity", "2")];

    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "FILL", 0.99),
        ("fill_target", "e2", 0.99),
        ("fill_value", "none_of_these", 0.99),
    ])]);
    let fake = Fake::new(vec![], vec![page_with_a_search_box()]);
    let outcome = advance(&oracle, &fake, run).await;

    assert!(
        matches!(outcome.status, Status::NeedsValue(_)),
        "got {:?}",
        outcome.status
    );
    assert_eq!(outcome.actions(), 0);
}

/// Typing the quantity into the pincode box is as wrong as typing into the
/// wrong box, so the value is an argument the bar applies to.
#[tokio::test]
async fn a_value_matched_without_confidence_is_not_typed() {
    let mut run = Run::new("set the delivery pincode to 600042", budget());
    run.values = vec![NamedValue::new("pincode", "600042")];

    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "FILL", 0.99),
        ("fill_target", "e2", 0.99),
        ("fill_value", "v00", 0.40),
    ])]);
    let fake = Fake::new(vec![Ok(())], vec![page_with_a_search_box()]);
    let outcome = advance(&oracle, &fake, run).await;

    assert_eq!(
        outcome.actions(),
        0,
        "0.40 is under the bar, so nothing was typed: {:?}",
        outcome.status
    );
}

/// The caller answered a question about this exact control. The model only
/// matched a value to a field it chose, so the answer wins.
#[tokio::test]
async fn the_callers_own_answer_beats_the_models_match() {
    let mut run = Run::new("set the delivery pincode to 600042", budget());
    run.values = vec![NamedValue::new("guess", "560001")];
    run.supply("e2", "600042");

    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "FILL", 0.99),
        ("fill_target", "e2", 0.99),
        ("fill_value", "v00", 0.99),
    ])]);
    let fake = Fake::new(vec![Ok(())], vec![page_with_a_search_box()]).moving_once();
    let outcome = advance(&oracle, &fake, run).await;

    assert_eq!(outcome.trail[0].value.as_deref(), Some("600042"));
}

/// The id travels, not the text, so a model naming a value nobody offered
/// resolves to nothing rather than to a string it invented.
#[tokio::test]
async fn a_value_nobody_offered_resolves_to_nothing() {
    let mut run = Run::new("set the delivery pincode to 600042", budget());
    run.values = vec![NamedValue::new("pincode", "600042")];

    let oracle = Scripted::new(vec![says(&[
        (OPERATION, "FILL", 0.99),
        ("fill_target", "e2", 0.99),
        ("fill_value", "v99", 0.99),
    ])]);
    let fake = Fake::new(vec![Ok(())], vec![page_with_a_search_box()]);
    let outcome = advance(&oracle, &fake, run).await;

    assert_eq!(outcome.actions(), 0);
    assert!(
        matches!(outcome.status, Status::NeedsValue(_)),
        "got {:?}",
        outcome.status
    );
}

/// The head is only asked when there is something to ask about, so a run with
/// no values sends the request it always sent.
#[test]
fn the_value_head_is_absent_without_values() {
    let view = page_with_a_search_box();
    let space = browseros_decide::space::ActionSpace::build(&view, "type something", None);
    let operations = vec![Operation::Fill];
    let without = browseros_decide::questions::build(
        "type something",
        &space,
        &operations,
        Default::default(),
        &[],
    );
    assert!(!without.contains_key("fill_value"));

    let value = NamedValue::new("pincode", "600042");
    let offered = vec![("v00".to_string(), &value)];
    let with = browseros_decide::questions::build(
        "type something",
        &space,
        &operations,
        Default::default(),
        &offered,
    );
    assert!(with.contains_key("fill_value"));
    let rendered = serde_json::to_string(&with).expect("serialises");
    assert!(rendered.contains("600042"));
    assert!(
        rendered.contains("pincode"),
        "the name is the hint the model judges by"
    );
}
