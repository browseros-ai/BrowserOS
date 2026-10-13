//! The loop, driven end to end by a scripted model and a fake browser.
//!
//! Every scenario here is one that actually happened against a real page. The
//! fake browser exists so they can be replayed in milliseconds and so a
//! regression is caught by `cargo test` rather than by a live errand.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::Duration;

use browseros_decide::client::{Answer as RawAnswer, JevError, Question, Response, Usage};
use browseros_decide::drive::{ActError, Answer, Driver, Move, Performed};
use browseros_decide::gate::Handback;
use browseros_decide::offer::{Destination, NamedValue};
use browseros_decide::pursue::{Oracle, Status, Stuck, advance};
use browseros_decide::run::{Budget, Offered, Run};
use browseros_decide::view::{Control, ControlState, PageView};
use browseros_decide::vocabulary::Operation;

/// A model that answers from a script, so a scenario is written rather than
/// hoped for.
struct Scripted {
    turns: RefCell<Vec<Vec<(String, RawAnswer)>>>,
    /// What was asked each turn, so a test can assert on the request as well as
    /// on the outcome.
    asked: RefCell<Vec<Vec<String>>>,
}

impl Scripted {
    fn new(turns: Vec<Vec<(&str, RawAnswer)>>) -> Self {
        Self {
            turns: RefCell::new(
                turns
                    .into_iter()
                    .map(|turn| {
                        turn.into_iter()
                            .map(|(id, answer)| (id.to_string(), answer))
                            .collect()
                    })
                    .collect(),
            ),
            asked: RefCell::new(Vec::new()),
        }
    }

    fn heads_asked(&self) -> Vec<Vec<String>> {
        self.asked.borrow().clone()
    }
}

impl Oracle for Scripted {
    async fn ask_within(
        &self,
        _state: &serde_json::Value,
        questions: &BTreeMap<String, Question>,
        _budget: Duration,
    ) -> Result<Response, JevError> {
        self.asked
            .borrow_mut()
            .push(questions.keys().cloned().collect());
        let mut turns = self.turns.borrow_mut();
        if turns.is_empty() {
            // Running out of script means the loop asked more than the scenario
            // expected, which is itself worth failing on.
            return Err(JevError::Shape("the script ran out".to_string()));
        }
        let turn = turns.remove(0);
        Ok(Response {
            model: "jev-1.13.0".to_string(),
            answers: turn.into_iter().collect(),
            usage: Usage {
                input_tokens: 1_000,
                output_tokens: 0,
            },
        })
    }
}

fn choice(value: &str, confidence: f64) -> RawAnswer {
    RawAnswer {
        answer_type: "choice".to_string(),
        noul: None,
        choice: Some(value.to_string()),
        score: None,
        probabilities: None,
        legend: None,
        confidence: Some(confidence),
    }
}

fn control(reference: &str, role: &str, name: &str) -> Control {
    Control {
        reference: reference.to_string(),
        clickable: false,
        role: role.to_string(),
        name: name.to_string(),
        value: None,
        state: ControlState {
            editable: matches!(role, "textbox" | "searchbox"),
            ..ControlState::default()
        },
        options: Vec::new(),
        guard: 0,
    }
}

fn page(url: &str, fingerprint: u64, controls: Vec<Control>) -> PageView {
    PageView {
        url: url.to_string(),
        title: "A page".to_string(),
        text: "some visible text".to_string(),
        controls,
        fingerprint,
        unlabelled_images: 0,
    }
}

/// A browser that does what it is told, and records it.
struct Fake {
    page: RefCell<PageView>,
    ran: RefCell<Vec<Move>>,
    /// References whose click this page swallows: the action reports success
    /// and nothing changes.
    inert: Vec<String>,
    /// Tools this browser does not have.
    missing: Vec<Operation>,
    /// What a report hands back.
    reports: String,
}

impl Fake {
    fn new(page: PageView) -> Self {
        Self {
            page: RefCell::new(page),
            ran: RefCell::new(Vec::new()),
            inert: Vec::new(),
            missing: Vec::new(),
            reports: "312 points by someone 2 hours ago".to_string(),
        }
    }

    fn swallowing(mut self, references: &[&str]) -> Self {
        self.inert = references.iter().map(|r| (*r).to_string()).collect();
        self
    }

    fn ran(&self) -> Vec<Move> {
        self.ran.borrow().clone()
    }

    fn targets_acted_on(&self) -> Vec<String> {
        self.ran()
            .iter()
            .filter_map(|act| match act {
                Move::Act(decision) => decision.target.clone(),
                _ => None,
            })
            .collect()
    }
}

impl Driver for Fake {
    fn stopped(&self) -> bool {
        false
    }

    async fn observe(&self) -> Result<PageView, String> {
        Ok(self.page.borrow().clone())
    }

    async fn perform(&self, act: &Move) -> Result<Performed, ActError> {
        self.ran.borrow_mut().push(act.clone());
        match act {
            Move::Report(ask) => {
                if self.missing.contains(&Operation::Grep) && ask.as_str() == "grep" {
                    return Err(ActError::NotOffered(Operation::Grep));
                }
                Ok(Performed::Answer(Answer {
                    asked: ask.as_str().to_string(),
                    text: Some(self.reports.clone()),
                    pictured: ask.as_str() == "screenshot",
                }))
            }
            Move::Act(decision) => {
                let aimed = decision.target.clone().unwrap_or_default();
                if self.inert.contains(&aimed) {
                    // The click lands, the browser says it worked, and the page
                    // is untouched.
                    return Ok(Performed::Unmoved);
                }
                let mut page = self.page.borrow_mut();
                page.fingerprint += 1;
                page.url = format!("{}/after-{aimed}", page.url);
                Ok(Performed::Moved)
            }
            Move::Travel(_) => {
                let mut page = self.page.borrow_mut();
                page.fingerprint += 1;
                page.url = "https://example.test/elsewhere".to_string();
                Ok(Performed::Moved)
            }
            Move::Resnapshot | Move::Settle => Ok(Performed::Unmoved),
        }
    }
}

fn run_for(goal: &str) -> Run {
    Run::new(goal, Budget::default())
}

/// The Wikipedia case. The highest ranked link sat inside an image caption
/// whose click the page's own photo viewer intercepted, so the click reported
/// success, nothing moved, and the loop chose the same reference every step
/// until the budget ran out.
#[tokio::test]
async fn a_control_that_swallows_its_click_is_retired_and_the_next_one_is_tried() {
    let view = page(
        "https://example.test/dither",
        1,
        vec![
            control("e483", "link", "Floyd-Steinberg dithering"),
            control("e224", "link", "Floyd-Steinberg dithering in the body"),
        ],
    );
    let driver = Fake::new(view).swallowing(&["e483"]);
    let oracle = Scripted::new(vec![
        vec![
            ("operation", choice("ACT", 0.95)),
            ("act_how", choice("CLICK", 0.95)),
            ("target", choice("e483", 0.93)),
        ],
        vec![
            ("operation", choice("ACT", 0.95)),
            ("act_how", choice("CLICK", 0.95)),
            ("target", choice("e224", 0.90)),
        ],
        vec![("operation", choice("DONE", 0.95))],
    ]);

    let outcome = advance(&oracle, &driver, run_for("open the Floyd-Steinberg article")).await;

    assert_eq!(outcome.status, Status::Satisfied);
    assert_eq!(
        driver.targets_acted_on(),
        vec!["e483".to_string(), "e224".to_string()],
        "the dead control is tried once, then never again"
    );
    assert_eq!(outcome.retired, 1);
}

/// Without retirement the same reference is chosen until the budget is gone.
#[tokio::test]
async fn a_run_that_cannot_retire_anything_still_stops_rather_than_looping() {
    let view = page(
        "https://example.test/dither",
        1,
        vec![control("e1", "link", "The only link")],
    );
    let driver = Fake::new(view).swallowing(&["e1"]);
    let turn = vec![
        ("operation", choice("ACT", 0.95)),
        ("act_how", choice("CLICK", 0.95)),
        ("target", choice("e1", 0.93)),
    ];
    // Generously more script than the run should need, so running out is not
    // what stops it.
    let oracle = Scripted::new(vec![turn.clone(); 40]);

    let outcome = advance(&oracle, &driver, run_for("open the only link")).await;

    assert!(
        matches!(outcome.status, Status::Stalled(_) | Status::BudgetSpent),
        "ended as {:?}",
        outcome.status
    );
    assert!(
        outcome.decisions <= Budget::default().max_steps + 2,
        "it asked {} times, which is not bounded by the budget",
        outcome.decisions
    );
}

/// The Hacker News case. Asked for a number printed on the page, the old
/// vocabulary had no way to read, so it answered DONE at high confidence and
/// returned nothing.
#[tokio::test]
async fn a_question_about_the_page_is_answered_with_text_rather_than_a_claim() {
    let view = page(
        "https://news.example.test/",
        1,
        vec![control("e1", "link", "A story")],
    );
    let driver = Fake::new(view);
    let oracle = Scripted::new(vec![vec![("operation", choice("READ", 0.88))]]);

    let outcome = advance(
        &oracle,
        &driver,
        run_for("find out how many points the top story has"),
    )
    .await;

    assert_eq!(outcome.status, Status::Answered);
    let answer = outcome.answer.expect("a report hands something back");
    assert_eq!(answer.asked, "read");
    assert!(
        answer.text.unwrap().contains("312 points"),
        "the caller gets the text, which is the only thing that can carry a number"
    );
}

/// A report ends the run, so nothing runs after it.
#[tokio::test]
async fn nothing_happens_after_a_report() {
    let view = page("https://example.test/", 1, vec![control("e1", "link", "A")]);
    let driver = Fake::new(view);
    let oracle = Scripted::new(vec![vec![("operation", choice("READ", 0.9))]]);

    advance(&oracle, &driver, run_for("read the page")).await;

    assert_eq!(driver.ran().len(), 1, "the loop had nowhere to continue to");
}

/// Looking at more of the page does not make a submit any safer.
#[tokio::test]
async fn an_unsure_expensive_action_hands_back_rather_than_guessing() {
    let view = page(
        "https://shop.example.test/",
        1,
        vec![control("e9", "button", "Place order")],
    );
    let driver = Fake::new(view);
    let oracle = Scripted::new(vec![vec![
        ("operation", choice("ACT", 0.80)),
        ("act_how", choice("CLICK", 0.80)),
        ("target", choice("e9", 0.70)),
    ]]);

    let outcome = advance(&oracle, &driver, run_for("place the order")).await;

    let Status::NeedsInput(why, _) = outcome.status else {
        panic!("expected a hand-back, got {:?}", outcome.status);
    };
    assert_eq!(why, Handback::TooRiskyToGuess);
    assert!(driver.ran().is_empty(), "and nothing was clicked");
}

/// The same action on an ordinary control is not held to the same bar, which is
/// the regression that made a run refuse every Sony listing on a search page.
#[tokio::test]
async fn the_same_confidence_on_an_ordinary_control_runs() {
    let view = page(
        "https://shop.example.test/",
        1,
        vec![control(
            "e9",
            "link",
            "SONY WH-1000XM5 Wireless Noise Cancellation Headphones",
        )],
    );
    let driver = Fake::new(view);
    let oracle = Scripted::new(vec![
        vec![
            ("operation", choice("ACT", 0.80)),
            ("act_how", choice("CLICK", 0.80)),
            ("target", choice("e9", 0.78)),
        ],
        vec![("operation", choice("DONE", 0.9))],
    ]);

    let outcome = advance(&oracle, &driver, run_for("open the Sony listing")).await;

    assert_eq!(outcome.status, Status::Satisfied);
    assert_eq!(driver.targets_acted_on(), vec!["e9".to_string()]);
}

/// A run that keeps being declined stops rather than asking forever.
#[tokio::test]
async fn a_run_the_gate_keeps_declining_stops() {
    let view = page(
        "https://example.test/",
        1,
        vec![control("e1", "link", "Something"), control("e2", "link", "Else")],
    );
    let driver = Fake::new(view);
    let unsure = vec![
        ("operation", choice("ACT", 0.40)),
        ("act_how", choice("CLICK", 0.40)),
        ("target", choice("e1", 0.40)),
    ];
    let oracle = Scripted::new(vec![unsure.clone(), unsure.clone(), unsure]);

    let outcome = advance(&oracle, &driver, run_for("do something")).await;

    assert_eq!(outcome.status, Status::Stalled(Stuck::NotConfident));
}

/// A browser without a tool never gets asked for it, and the rest of the loop
/// still works.
#[tokio::test]
async fn a_browser_missing_a_tool_is_never_offered_it() {
    let view = page("https://example.test/", 1, vec![control("e1", "link", "A")]);
    let driver = Fake::new(view);
    let oracle = Scripted::new(vec![vec![("operation", choice("READ", 0.9))]]);
    let run = run_for("read the page").offering(Offered {
        unavailable: vec![Operation::Grep, Operation::Evaluate],
        ..Offered::default()
    });

    let outcome = advance(&oracle, &driver, run).await;

    assert_eq!(outcome.status, Status::Answered);
    let heads = oracle.heads_asked();
    assert!(heads[0].contains(&"operation".to_string()));
}

/// A value the caller supplied is what gets typed. The model picks which one,
/// never what it says.
#[tokio::test]
async fn a_fill_uses_a_value_the_caller_supplied() {
    let view = page(
        "https://example.test/",
        1,
        vec![
            control("e1", "textbox", "Search"),
            control("e2", "link", "Something else"),
        ],
    );
    let driver = Fake::new(view);
    let oracle = Scripted::new(vec![
        vec![
            ("operation", choice("ACT", 0.95)),
            ("act_how", choice("FILL", 0.95)),
            ("target", choice("e1", 0.95)),
            ("fill_value", choice("search term", 0.95)),
        ],
        vec![("operation", choice("DONE", 0.95))],
    ]);
    let run = run_for("search for dithering").offering(Offered {
        values: vec![NamedValue::new("search term", "dithering")],
        ..Offered::default()
    });

    let outcome = advance(&oracle, &driver, run).await;

    assert_eq!(outcome.status, Status::Satisfied);
    let heads = oracle.heads_asked();
    assert!(
        heads[0].contains(&"fill_value".to_string()),
        "the value head is asked when the caller offered values"
    );
}

/// Choosing to type with nothing to type is a hand-back, not a guess.
#[tokio::test]
async fn a_fill_with_no_value_available_hands_back() {
    let view = page(
        "https://example.test/",
        1,
        vec![
            control("e1", "textbox", "Search"),
            control("e2", "link", "Something"),
        ],
    );
    let driver = Fake::new(view);
    let oracle = Scripted::new(vec![vec![
        ("operation", choice("ACT", 0.95)),
        ("act_how", choice("FILL", 0.95)),
        ("target", choice("e1", 0.95)),
    ]]);

    let outcome = advance(&oracle, &driver, run_for("type something")).await;

    let Status::NeedsInput(why, _) = outcome.status else {
        panic!("expected a hand-back, got {:?}", outcome.status);
    };
    assert_eq!(why, Handback::NoFieldNamed);
}

/// A run may only move between pages so many times, so a goal that is really a
/// tour does not wander.
#[tokio::test]
async fn a_run_stops_moving_between_pages_after_its_limit() {
    let view = page("https://example.test/", 1, vec![control("e1", "link", "A")]);
    let driver = Fake::new(view);
    let going = vec![
        ("operation", choice("NAVIGATE", 0.95)),
        ("destination", choice("the_other_page", 0.95)),
    ];
    let oracle = Scripted::new(vec![
        going.clone(),
        going.clone(),
        going.clone(),
        going.clone(),
    ]);
    let run = run_for("go to the other page").offering(Offered {
        destinations: vec![Destination {
            label: "the other page".to_string(),
            url: "https://example.test/other".to_string(),
            new_tab: false,
        }],
        ..Offered::default()
    });

    let outcome = advance(&oracle, &driver, run).await;

    assert!(
        matches!(outcome.status, Status::Blocked(_)),
        "ended as {:?}",
        outcome.status
    );
}

/// Everything the run did is on the outcome, so a caller can check it rather
/// than trust it.
#[tokio::test]
async fn the_outcome_reports_what_actually_happened() {
    let view = page(
        "https://example.test/start",
        1,
        vec![control("e1", "link", "Go"), control("e2", "link", "Elsewhere")],
    );
    let driver = Fake::new(view);
    let oracle = Scripted::new(vec![
        vec![
            ("operation", choice("ACT", 0.95)),
            ("act_how", choice("CLICK", 0.95)),
            ("target", choice("e1", 0.91)),
        ],
        vec![("operation", choice("DONE", 0.97))],
    ]);

    let outcome = advance(&oracle, &driver, run_for("go somewhere")).await;

    assert_eq!(outcome.decisions, 2);
    assert_eq!(outcome.trail.len(), 1, "one action was taken");
    assert_eq!(outcome.trail[0].operation, Operation::Act);
    assert!(outcome.trail[0].page_changed);
    assert_eq!(outcome.url_before, "https://example.test/start");
    assert!(outcome.url_after.contains("after-e1"));
    assert_eq!(outcome.input_tokens, 2_000, "both decisions are billed");
}
