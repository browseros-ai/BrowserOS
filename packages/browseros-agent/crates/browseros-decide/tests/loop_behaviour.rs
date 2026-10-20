//! The loop, driven end to end by a scripted model and a fake browser.
//!
//! Every scenario here is one that actually happened against a real page. The
//! fake browser exists so they can be replayed in milliseconds and so a
//! regression is caught by `cargo test` rather than by a live errand.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::time::Duration;

use browseros_decide::client::{Answer as RawAnswer, JevError, Question, Response, Usage};
use browseros_decide::drive::{ActError, Driver, Move, Performed};
use browseros_decide::gate::Handback;
use browseros_decide::offer::{Destination, NamedValue};
use browseros_decide::pursue::{Oracle, Status, Stuck, advance};
use browseros_decide::run::{Budget, Offered, Run};
use browseros_decide::view::{Control, ControlState, PageView};
use browseros_decide::vocabulary::{How, Operation};

/// A model that answers from a script, so a scenario is written rather than
/// hoped for.
struct Scripted {
    turns: RefCell<Vec<Vec<(String, RawAnswer)>>>,
    /// What was asked each turn, so a test can assert on the request as well as
    /// on the outcome.
    asked: RefCell<Vec<Vec<String>>>,
    /// What every move is judged worth doing, unless a scenario says otherwise.
    ///
    /// The loop asks this about the control it has settled on, in a request of
    /// its own. Answering it without consuming a turn is what lets a scenario
    /// be written about the decisions it cares about rather than about every
    /// call the loop happens to make.
    worth: Cell<f64>,
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
            worth: Cell::new(0.9),
        }
    }

    /// Judges every move worth this much, for a scenario about the approval.
    fn judging_moves_worth(self, worth: f64) -> Self {
        self.worth.set(worth);
        self
    }

    /// The decision requests, which is what a scenario is written about.
    ///
    /// The approval is asked on its own about a control already chosen, so it
    /// is left out here: a scenario counting how many times the page was
    /// searched is not counting that.
    fn heads_asked(&self) -> Vec<Vec<String>> {
        self.asked
            .borrow()
            .iter()
            .filter(|heads| heads.as_slice() != ["worth_acting_on"])
            .cloned()
            .collect()
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
        // The approval is asked on its own and answered without spending a
        // turn, so every scenario below still reads as the decisions it writes.
        if questions.len() == 1 && questions.contains_key("worth_acting_on") {
            return Ok(Response {
                model: "jev-1.13.0".to_string(),
                answers: [("worth_acting_on".to_string(), noul(self.worth.get()))]
                    .into_iter()
                    .collect(),
                usage: Usage {
                    input_tokens: 100,
                    output_tokens: 0,
                },
            });
        }
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

fn noul(probability: f64) -> RawAnswer {
    RawAnswer {
        answer_type: "noul".to_string(),
        noul: Some(probability),
        choice: None,
        score: None,
        probabilities: None,
        legend: None,
        confidence: None,
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

/// A scripted answer that reports its distribution, which is what the margin
/// gate and the pagination stop both read.
/// A scripted answer that reports a distribution shaped like a real one.
///
/// The leftover mass is spread over further options rather than dropped,
/// because the published confidence is measured against an even split and an
/// even split depends on how many ways the mass could have gone. A two entry
/// distribution says the question had two options, which is a different
/// question from the one these scenarios mean.
fn spread(value: &str, share: f64, runner_up: f64) -> RawAnswer {
    let mut probabilities = BTreeMap::new();
    probabilities.insert(value.to_string(), share);
    probabilities.insert("__runner_up".to_string(), runner_up);
    let left = (1.0 - share - runner_up).max(0.0);
    if left > 0.0 {
        let each = left / 7.0;
        for index in 0..7 {
            probabilities.insert(format!("__other_{index}"), each);
        }
    }
    RawAnswer {
        answer_type: "choice".to_string(),
        noul: None,
        choice: Some(value.to_string()),
        score: None,
        probabilities: Some(probabilities),
        legend: None,
        confidence: Some(share),
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
        group: None,
    }
}

fn page(url: &str, fingerprint: u64, controls: Vec<Control>) -> PageView {
    PageView {
        url: url.to_string(),
        title: "A page".to_string(),
        text: "some visible text".to_string(),
        controls,
        fingerprint,
        arrangement: fingerprint,
        unlabelled_images: 0,
    }
}

/// A browser that does what it is told, and records it.
struct Fake {
    page: RefCell<PageView>,
    ran: RefCell<Vec<Move>>,
    /// How many times the loop read the page.
    observed: RefCell<usize>,
    /// The look at which a slow control's effect becomes visible.
    pending: RefCell<Option<usize>>,
    /// References whose click this page swallows: the action reports success
    /// and nothing changes.
    inert: Vec<String>,
    /// Controls that move the fingerprint and nothing else.
    toggles: Vec<String>,
    /// How many arrangements this page cycles through, if it cycles.
    cycle: Option<u64>,
    /// Which action this is, for a cycling page.
    acted: RefCell<u64>,
    /// Controls whose effect appears only after this many further looks, which
    /// is what a panel behind a timeout looks like from here.
    slow: Vec<(String, usize)>,
}

impl Fake {
    fn new(page: PageView) -> Self {
        Self {
            page: RefCell::new(page),
            ran: RefCell::new(Vec::new()),
            observed: RefCell::new(0),
            pending: RefCell::new(None),
            inert: Vec::new(),
            slow: Vec::new(),
            toggles: Vec::new(),
            cycle: None,
            acted: RefCell::new(0),
        }
    }

    /// A control that works, late. `after` is how many looks pass before the
    /// page shows it.
    fn slow_to_react(mut self, reference: &str, after: usize) -> Self {
        self.slow.push((reference.to_string(), after));
        self
    }

    /// Cycles the page through `states` arrangements rather than inventing a
    /// new one on every action, which is what a dialog opening and closing over
    /// the same two pages does.
    fn cycling(mut self, states: u64) -> Self {
        self.cycle = Some(states);
        self
    }

    /// Controls whose action changes the page without going anywhere: it flips
    /// between two arrangements, which is what opening and closing a panel
    /// looks like, and is a change that is also a return.
    fn toggling(mut self, references: &[&str]) -> Self {
        self.toggles = references.iter().map(|r| (*r).to_string()).collect();
        self
    }

    fn swallowing(mut self, references: &[&str]) -> Self {
        self.inert = references.iter().map(|r| (*r).to_string()).collect();
        self
    }

    fn ran(&self) -> Vec<Move> {
        self.ran.borrow().clone()
    }

    fn observes(&self) -> usize {
        *self.observed.borrow()
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
        *self.observed.borrow_mut() += 1;
        // A slow control's effect lands on the look it was scheduled for, not
        // on the one right after the act.
        let due = *self.pending.borrow();
        if let Some(due) = due
            && *self.observed.borrow() >= due
        {
            *self.pending.borrow_mut() = None;
            let mut page = self.page.borrow_mut();
            page.fingerprint += 1;
            page.controls
                .push(control("late", "checkbox", "Brand: Corsair"));
        }
        Ok(self.page.borrow().clone())
    }

    async fn perform(&self, act: &Move) -> Result<Performed, ActError> {
        self.ran.borrow_mut().push(act.clone());
        match act {
            Move::Act(decision) => {
                let aimed = decision.target.clone().unwrap_or_default();
                if self.inert.contains(&aimed) {
                    // The click lands, the browser says it worked, and the page
                    // is untouched.
                    return Ok(Performed::Unmoved);
                }
                if let Some((_, looks)) = self.slow.iter().find(|(slow, _)| *slow == aimed) {
                    // The click landed and the page will show it, later. Until
                    // then it is a perfectly quiet page that looks dead.
                    *self.pending.borrow_mut() = Some(*self.observed.borrow() + looks);
                    return Ok(Performed::Moved);
                }
                let mut page = self.page.borrow_mut();
                if let Some(states) = self.cycle {
                    // Round and round the same few arrangements, each one a
                    // real change from the last.
                    *self.acted.borrow_mut() += 1;
                    let step = *self.acted.borrow();
                    page.fingerprint = 1 + step % states;
                    page.url = format!("https://example.test/docs/{}", step % states);
                    return Ok(Performed::Moved);
                }
                if self.toggles.contains(&aimed) {
                    // A real change that is not progress, and a real return:
                    // a panel that opens and closes comes back to the
                    // arrangement it started from.
                    *self.acted.borrow_mut() += 1;
                    page.fingerprint = 1 + *self.acted.borrow() % 2;
                    return Ok(Performed::Moved);
                }
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
            Move::Settle => Ok(Performed::Unmoved),
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

    let outcome = advance(
        &oracle,
        &driver,
        run_for("open the Floyd-Steinberg article"),
    )
    .await;

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
        // Up to two calls per step now: the decision, and the approval asked
        // about the control it settled on.
        outcome.decisions <= Budget::default().max_steps * 2 + 2,
        "it asked {} times, which is not bounded by the budget",
        outcome.decisions
    );
}

/// A move the model picked and then judged not worth making does not run.
///
/// This is the whole point of asking: a control can lead its alternatives and
/// still be the wrong thing to do, and on a page where several controls each
/// serve the goal a lead cannot say which.
#[tokio::test]
async fn a_move_judged_not_worth_making_does_not_run() {
    let view = page(
        "https://example.test/start",
        1,
        vec![
            control("e1", "link", "Go"),
            control("e2", "link", "Elsewhere"),
        ],
    );
    let driver = Fake::new(view);
    let oracle = Scripted::new(vec![
        vec![
            ("operation", choice("ACT", 0.95)),
            ("act_how", choice("CLICK", 0.95)),
            ("target", choice("e1", 0.95)),
        ];
        6
    ])
    .judging_moves_worth(0.20);

    let outcome = advance(&oracle, &driver, run_for("go somewhere")).await;

    assert!(
        outcome.trail.is_empty(),
        "nothing should have been acted on, trail was {:?}",
        outcome.trail
    );
    assert!(
        matches!(outcome.status, Status::Stalled(_) | Status::NeedsInput(..)),
        "ended as {:?}",
        outcome.status
    );
    // And it did not answer a refusal about the move by moving the page. The
    // control sits on screen; scrolling past it is what used to take the one
    // control that worked out of reach.
    let scrolled = driver.ran().iter().any(|act| match act {
        Move::Act(decision) => decision.how == Some(How::Scroll),
        _ => false,
    });
    assert!(
        !scrolled,
        "a refusal about the move must not scroll the page"
    );
}

/// The live failure the vocabulary was widened for, restated now that the
/// widening has been undone. Asked how many points a story has, the loop can
/// say the answer is on this page and stop; counting it is the caller's job,
/// and the caller is the only one of the two that can read a number anyway.
#[tokio::test]
async fn a_question_about_the_page_ends_on_the_page_that_answers_it() {
    let view = page(
        "https://news.example.test/",
        1,
        vec![control("e1", "link", "A story")],
    );
    let driver = Fake::new(view);
    let oracle = Scripted::new(vec![vec![("operation", choice("DONE", 0.88))]]);

    let outcome = advance(
        &oracle,
        &driver,
        run_for("find out how many points the top story has"),
    )
    .await;

    assert_eq!(outcome.status, Status::Satisfied);
    assert_eq!(
        outcome.url_after, "https://news.example.test/",
        "and it says which page to read"
    );
    assert!(
        browseros_decide::report::tell(&outcome).try_next.is_none(),
        "a run that finished has nothing to advise"
    );
}

/// An ending ends the run, so nothing runs after it and nothing is asked of
/// the browser for it either: it touches no page.
#[tokio::test]
async fn nothing_happens_after_an_ending() {
    let view = page("https://example.test/", 1, vec![control("e1", "link", "A")]);
    let driver = Fake::new(view);
    let oracle = Scripted::new(vec![vec![("operation", choice("DONE", 0.9))]]);

    advance(&oracle, &driver, run_for("read the page")).await;

    assert!(
        driver.ran().is_empty(),
        "the loop had nowhere to continue to"
    );
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
        vec![
            control("e1", "link", "Something"),
            control("e2", "link", "Else"),
        ],
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
        vec![
            control("e1", "link", "Go"),
            control("e2", "link", "Elsewhere"),
        ],
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

    // Three model calls: the decision that clicked, the approval asked about
    // the control it chose, and the decision that ended the run. The approval
    // costs tokens, so it is counted rather than hidden.
    assert_eq!(outcome.decisions, 3);
    assert_eq!(outcome.trail.len(), 1, "one action was taken");
    assert_eq!(outcome.trail[0].operation, Operation::Act);
    assert!(outcome.trail[0].page_changed);
    assert_eq!(outcome.url_before, "https://example.test/start");
    assert!(outcome.url_after.contains("after-e1"));
    assert_eq!(
        outcome.input_tokens, 2_100,
        "every call is billed, the approval included"
    );
}

/// The loop used to read the page twice per step: once at the top and once
/// after acting, with nothing touching it in between. A snapshot is the most
/// expensive thing the loop does, and now that it waits for the page to settle
/// first, it is more expensive still.
#[tokio::test]
async fn one_step_reads_the_page_once() {
    let browser = Fake::new(page(
        "https://example.test/",
        1,
        vec![control("e1", "link", "Open it")],
    ));
    let model = Scripted::new(vec![
        vec![
            ("operation", spread("ACT", 0.90, 0.02)),
            ("act_how", spread("CLICK", 0.95, 0.01)),
            ("target", spread("e1", 0.90, 0.02)),
        ],
        vec![("operation", spread("DONE", 0.95, 0.01))],
    ]);
    let outcome = advance(&model, &browser, run_for("open it")).await;
    assert_eq!(outcome.status, Status::Satisfied);
    assert_eq!(
        browser.observes(),
        2,
        "once to start, once after the only action; not four"
    );
}

/// A page bigger than one question is asked about in batches. The answer sat
/// past the first 254 on the measured page, and the shortlist that used to cut
/// the list scored its page at zero.
#[tokio::test]
async fn a_control_past_the_first_question_is_still_reachable() {
    let mut controls: Vec<Control> = (0..300)
        .map(|index| control(&format!("e{index}"), "link", &format!("Link {index}")))
        .collect();
    // The one that matters, last on the page and so last in the order.
    controls.push(control("target", "button", "Sort by price"));
    let browser = Fake::new(page("https://example.test/", 1, controls));
    let model = Scripted::new(vec![
        // The first batch does not hold it, and says so quietly: a thin lead.
        vec![
            ("operation", spread("ACT", 0.90, 0.02)),
            ("act_how", spread("CLICK", 0.95, 0.01)),
            ("target", spread("e4", 0.20, 0.18)),
        ],
        // The second batch does.
        vec![
            ("operation", spread("ACT", 0.90, 0.02)),
            ("act_how", spread("CLICK", 0.95, 0.01)),
            ("target", spread("target", 0.93, 0.01)),
        ],
        vec![("operation", spread("DONE", 0.95, 0.01))],
    ]);
    let outcome = advance(&model, &browser, run_for("sort these by price")).await;
    assert_eq!(
        browser.targets_acted_on(),
        vec!["target".to_string()],
        "the better answer from the second batch is the one acted on"
    );
    assert_eq!(
        outcome.batches, 3,
        "two questions to search the page, then one that needed no target"
    );
    assert!(
        outcome.batches > outcome.decisions - outcome.batches,
        "and the batches are reported, not folded into the decision count"
    );
    assert_eq!(outcome.status, Status::Satisfied);
}

/// Pagination makes every control reachable, which also means a run can pay for
/// every batch of a page it has already answered. On one measured page, 7 of
/// the 8 batches asked were a re-search of a page answered by the first.
#[tokio::test]
async fn a_batch_that_settles_stops_the_search() {
    let controls: Vec<Control> = (0..600)
        .map(|index| control(&format!("e{index}"), "link", &format!("Link {index}")))
        .collect();
    let browser = Fake::new(page("https://example.test/", 1, controls));
    let model = Scripted::new(vec![
        vec![
            ("operation", spread("ACT", 0.90, 0.02)),
            ("act_how", spread("CLICK", 0.95, 0.01)),
            ("target", spread("e3", 0.91, 0.01)),
        ],
        vec![("operation", spread("DONE", 0.95, 0.01))],
    ]);
    let outcome = advance(&model, &browser, run_for("click a link")).await;
    assert_eq!(outcome.status, Status::Satisfied);
    assert_eq!(
        model.heads_asked().len(),
        2,
        "three batches existed and one answer settled it, so two questions were \
         asked in total rather than four"
    );
    assert_eq!(outcome.batches, 2, "and the run says how many it paid for");
}

/// The measured case the margin gate exists for, driven end to end: an answer
/// leading 0.63 to 0.11 over many options runs, where an absolute bar of 0.75
/// refused it three times in one run.
#[tokio::test]
async fn a_landslide_over_many_options_acts() {
    let controls: Vec<Control> = (0..60)
        .map(|index| control(&format!("e{index}"), "link", &format!("Link {index}")))
        .collect();
    let browser = Fake::new(page("https://news.example.test/", 1, controls));
    let model = Scripted::new(vec![
        vec![
            ("operation", spread("ACT", 0.63, 0.11)),
            ("act_how", spread("CLICK", 0.88, 0.04)),
            ("target", spread("e30", 0.76, 0.05)),
        ],
        vec![("operation", spread("DONE", 0.95, 0.01))],
    ]);
    let outcome = advance(&model, &browser, run_for("open the world section")).await;
    assert_eq!(
        browser.targets_acted_on(),
        vec!["e30".to_string()],
        "0.63 against 0.11 is not a coin toss"
    );
    assert_eq!(outcome.status, Status::Satisfied);
}

/// The reply has to carry the page, or a caller that gets a failure can only
/// reword its goal and hope.
#[tokio::test]
async fn a_run_hands_back_the_controls_it_was_looking_at() {
    let browser = Fake::new(page(
        "https://example.test/results",
        1,
        vec![
            control("e1", "button", "Filters"),
            control("e2", "checkbox", "In stock"),
        ],
    ));
    let model = Scripted::new(vec![vec![("operation", spread("BLOCKED", 0.90, 0.02))]]);
    let outcome = advance(&model, &browser, run_for("sort by price")).await;
    let told = browseros_decide::report::tell(&outcome);
    let seen = told.page.expect("the page it stopped on");
    let named: Vec<&str> = seen
        .controls
        .iter()
        .map(|control| control.name.as_str())
        .collect();
    assert_eq!(named, vec!["Filters", "In stock"]);
    assert_eq!(seen.url, "https://example.test/results");
}

/// Choosing to look again costs a whole request to obtain what the next step
/// does for nothing, now that every look is of a settled page.
#[tokio::test]
async fn looking_again_is_not_one_of_the_moves_offered() {
    let browser = Fake::new(page(
        "https://example.test/",
        1,
        vec![control("e1", "link", "Open it")],
    ));
    let model = Scripted::new(vec![vec![("operation", spread("DONE", 0.95, 0.01))]]);
    let _ = advance(&model, &browser, run_for("read the page")).await;
    let asked = model.heads_asked();
    let operation = asked.first().expect("one question was asked");
    assert!(
        operation.contains(&"operation".to_string()),
        "the move question is always asked"
    );
}

/// A button whose panel appears 400ms after the click is indistinguishable
/// from a dead one at the instant the act returns, and the page is perfectly
/// quiet in between, so a quiet check returns at once.
///
/// Measured live against exactly that button: the act tool returned in under
/// 10ms, the settle found a quiet page and returned, the loop saw nothing
/// change, and a control that worked perfectly was retired.
#[tokio::test]
async fn a_control_that_works_late_is_not_retired_as_dead() {
    let browser = Fake::new(page(
        "https://example.test/",
        1,
        vec![control("e1", "button", "Filters")],
    ))
    .slow_to_react("e1", 3);
    let model = Scripted::new(vec![
        vec![
            ("operation", spread("ACT", 0.90, 0.02)),
            ("act_how", spread("CLICK", 0.95, 0.01)),
            ("target", spread("e1", 0.90, 0.02)),
        ],
        vec![("operation", spread("DONE", 0.95, 0.01))],
    ]);
    let outcome = advance(&model, &browser, run_for("open the filters")).await;
    assert_eq!(
        outcome.retired, 0,
        "it worked, so nothing should have been set aside"
    );
    assert!(
        outcome.trail.first().is_some_and(|step| step.page_changed),
        "and the step is recorded as having changed the page"
    );
    assert_eq!(outcome.status, Status::Satisfied);
}

/// The waiting is only ever paid for on the unchanged path, so a page that
/// reacts at once costs nothing extra.
#[tokio::test]
async fn a_page_that_reacts_at_once_is_not_waited_on() {
    let browser = Fake::new(page(
        "https://example.test/",
        1,
        vec![control("e1", "link", "Open it")],
    ));
    let model = Scripted::new(vec![
        vec![
            ("operation", spread("ACT", 0.90, 0.02)),
            ("act_how", spread("CLICK", 0.95, 0.01)),
            ("target", spread("e1", 0.90, 0.02)),
        ],
        vec![("operation", spread("DONE", 0.95, 0.01))],
    ]);
    let began = std::time::Instant::now();
    let outcome = advance(&model, &browser, run_for("open it")).await;
    assert_eq!(outcome.status, Status::Satisfied);
    assert_eq!(browser.observes(), 2, "no extra looks were needed");
    assert!(
        began.elapsed() < Duration::from_millis(100),
        "and no time was spent waiting for a change that had already happened"
    );
}

/// A control that genuinely does nothing still gets retired, after the wait
/// rather than instead of it.
#[tokio::test]
async fn a_dead_control_is_still_retired_once_the_waiting_is_done() {
    let browser = Fake::new(page(
        "https://example.test/",
        1,
        vec![
            control("e1", "link", "Swallows clicks"),
            control("e2", "link", "Works"),
        ],
    ))
    .swallowing(&["e1"]);
    let model = Scripted::new(vec![
        vec![
            ("operation", spread("ACT", 0.90, 0.02)),
            ("act_how", spread("CLICK", 0.95, 0.01)),
            ("target", spread("e1", 0.90, 0.02)),
        ],
        vec![
            ("operation", spread("ACT", 0.90, 0.02)),
            ("act_how", spread("CLICK", 0.95, 0.01)),
            ("target", spread("e2", 0.90, 0.02)),
        ],
        vec![("operation", spread("DONE", 0.95, 0.01))],
    ]);
    let outcome = advance(&model, &browser, run_for("open something")).await;
    assert_eq!(outcome.retired, 1, "the dead one was set aside");
    assert_eq!(
        browser.targets_acted_on(),
        vec!["e1".to_string(), "e2".to_string()]
    );
}

/// The model picks a value by name, because a name is something it can choose
/// between and a string is something it cannot write. The name must not be what
/// lands in the field.
///
/// Measured by reading the search box after a live run: given
/// {"search term": "32GB DDR5"}, the box held the words "search term".
#[tokio::test]
async fn the_caller_s_text_is_typed_rather_than_the_name_it_was_given() {
    let browser = Fake::new(page(
        "https://example.test/",
        1,
        vec![control("e1", "searchbox", "Search parts")],
    ));
    let model = Scripted::new(vec![
        vec![
            ("operation", spread("ACT", 0.92, 0.02)),
            ("act_how", spread("FILL", 0.90, 0.03)),
            ("target", spread("e1", 0.93, 0.02)),
            ("fill_value", spread("search term", 0.95, 0.01)),
        ],
        vec![("operation", spread("DONE", 0.95, 0.01))],
    ]);
    let run = Run::new("put the search term in the box", Budget::default()).offering(Offered {
        values: vec![NamedValue::new("search term", "32GB DDR5")],
        ..Offered::default()
    });
    let outcome = advance(&model, &browser, run).await;
    assert_eq!(outcome.status, Status::Satisfied);
    let typed: Vec<String> = browser
        .ran()
        .iter()
        .filter_map(|act| match act {
            Move::Act(decision) => decision.value.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(
        typed,
        vec!["32GB DDR5".to_string()],
        "the caller's text, not the key it was filed under"
    );
}

/// A select is not the same case: its value came from the control's own
/// options, so it is already the literal thing the page expects.
#[tokio::test]
async fn a_selects_own_value_is_passed_through_untouched() {
    let mut sort = control("e1", "combobox", "Sort by");
    sort.options = vec!["Featured".to_string(), "Price: Low to High".to_string()];
    let browser = Fake::new(page("https://example.test/", 1, vec![sort]));
    let model = Scripted::new(vec![
        vec![
            ("operation", spread("ACT", 0.92, 0.02)),
            ("act_how", spread("SELECT", 0.90, 0.03)),
            ("select_value", spread("e1::Price: Low to High", 0.94, 0.02)),
        ],
        vec![("operation", spread("DONE", 0.95, 0.01))],
    ]);
    let outcome = advance(&model, &browser, run_for("sort by price")).await;
    assert_eq!(outcome.status, Status::Satisfied);
    let chosen: Vec<String> = browser
        .ran()
        .iter()
        .filter_map(|act| match act {
            Move::Act(decision) => decision.value.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(chosen, vec!["Price: Low to High".to_string()]);
}

/// The amazon.in case, replayed. A run typed the caller's search term into the
/// box and then had no way to search with it, so the step that should have
/// submitted was asked which of 254 controls to aim at and answered that none
/// of them would do, which ended the run one move from working.
///
/// The second turn here answers `none_of_these` on the target head on purpose:
/// that is what the live model answered, and this passes only if that answer no
/// longer decides anything for a submit.
#[tokio::test]
async fn a_run_that_typed_can_submit_what_it_typed() {
    let view = page(
        "https://example.test/",
        1,
        vec![
            control("e1", "searchbox", "Search Amazon.in"),
            control("e2", "button", "Go"),
            control("e3", "link", "Today's Deals"),
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
        vec![
            ("operation", choice("ACT", 0.95)),
            ("act_how", choice("SUBMIT", 0.95)),
            ("target", choice("none_of_these", 0.95)),
        ],
        vec![("operation", choice("DONE", 0.95))],
    ]);
    let run = run_for("search for sensodyne toothpaste and add it to the cart").offering(Offered {
        values: vec![NamedValue::new("search term", "sensodyne toothpaste")],
        ..Offered::default()
    });

    let outcome = advance(&oracle, &driver, run).await;

    assert_eq!(
        outcome.status,
        Status::Satisfied,
        "a submit must not be ended by an answer about the target head"
    );

    let acted: Vec<_> = driver
        .ran()
        .into_iter()
        .filter_map(|act| match act {
            Move::Act(decision) => Some(decision),
            _ => None,
        })
        .collect();
    assert_eq!(acted.len(), 2, "the fill and the submit both ran");
    assert_eq!(acted[0].how, Some(How::Fill));
    assert_eq!(
        acted[0].value.as_deref(),
        Some("sensodyne toothpaste"),
        "the caller's text is typed, not the name it was filed under"
    );

    assert_eq!(acted[1].how, Some(How::Submit));
    assert_eq!(
        acted[1].target.as_deref(),
        Some("e1"),
        "the field comes from what the last step filled, not from the answer"
    );
    assert_eq!(acted[1].key.as_deref(), Some("Enter"));

    assert_eq!(
        outcome.retired, 0,
        "submitting is not aiming at the same control twice, so nothing is set aside"
    );

    // The target head is still asked on this step, because one request cannot
    // know the answer to `act_how` before it is built. It is asked and not read,
    // which is the whole of the fix.
    let heads = oracle.heads_asked();
    assert!(
        heads[1].contains(&"target".to_string()),
        "the target head is still asked, it is simply not read for a submit"
    );
}

/// The reason three live failures in a row had to be diagnosed by reading
/// source. A declined decision produces no step, so a run reported three
/// decisions, no actions, and no trace of what it tried three times. The gate
/// composed the sentence that explains it and the loop dropped it on the floor.
#[tokio::test]
async fn a_declined_decision_says_what_it_tried_and_why_it_was_refused() {
    let view = page(
        "https://example.test/",
        1,
        vec![
            control("e1", "link", "Something"),
            control("e2", "link", "Something else"),
        ],
    );
    let driver = Fake::new(view);
    // A thin lead on the operation head, three times: the shape of a run that
    // comes back having done nothing.
    let thin = || {
        vec![
            ("operation", spread("ACT", 0.30, 0.28)),
            ("act_how", spread("CLICK", 0.90, 0.05)),
            ("target", spread("e1", 0.90, 0.05)),
        ]
    };
    let oracle = Scripted::new(vec![thin(), thin(), thin()]);

    let outcome = advance(&oracle, &driver, run_for("do the thing")).await;

    assert_eq!(outcome.status, Status::Stalled(Stuck::NotConfident));
    assert!(
        outcome.trail.is_empty(),
        "nothing ran, which is what made this invisible"
    );
    assert_eq!(
        outcome.decided.len(),
        3,
        "every decision it paid for is on the record"
    );

    let first = &outcome.decided[0];
    assert_eq!(first.operation, "ACT");
    assert_eq!(first.how, Some("CLICK"));
    assert_eq!(first.verdict, "explored");
    assert_eq!(
        first.weakest,
        Some("which move to make"),
        "it names the head that was least certain"
    );
    assert!(
        first.margin.is_some_and(|margin| margin < 0.05),
        "it reports how far the weakest judgement actually led, got {:?}",
        first.margin
    );
    assert_eq!(
        first.needed,
        Some(0.30),
        "and the bar it had to clear, because a margin alone says nothing"
    );
    assert!(
        first.margin < first.needed,
        "which is the whole of why it did not run"
    );
    assert!(
        first
            .why
            .as_deref()
            .is_some_and(|why| why.contains("which move to make")),
        "the gate's own sentence travels, got {:?}",
        first.why
    );
    assert_eq!(
        first.batch, 1,
        "and which part of the page it was asked about"
    );
}

/// A decision that ran is on the record too, so a caller can see what the run
/// chose and not only what it refused.
#[tokio::test]
async fn a_decision_that_ran_is_on_the_record_as_having_run() {
    let view = page(
        "https://example.test/",
        1,
        vec![
            control("e1", "link", "Open it"),
            control("e2", "link", "Something else"),
        ],
    );
    let driver = Fake::new(view);
    let oracle = Scripted::new(vec![
        vec![
            ("operation", choice("ACT", 0.95)),
            ("act_how", choice("CLICK", 0.95)),
            ("target", choice("e1", 0.95)),
        ],
        vec![("operation", choice("DONE", 0.95))],
    ]);

    let outcome = advance(&oracle, &driver, run_for("open it")).await;

    let ran: Vec<&browseros_decide::pursue::Decided> = outcome
        .decided
        .iter()
        .filter(|decided| decided.verdict == "ran")
        .collect();
    assert_eq!(ran.len(), 2, "the click and the ending both ran");
    assert_eq!(ran[0].target.as_deref(), Some("e1"));
    assert_eq!(
        ran[0].target_name.as_deref(),
        Some("Open it"),
        "named, because a name is what a caller can act on"
    );
    assert!(
        ran[0].why.is_none(),
        "nothing to explain about a move that ran"
    );
}

/// The OpenAI docs case, the other half. The page offered
/// `button "Search the API docs"`, the run was asked for something that page
/// did not list, and it answered that none of the listed controls would do.
/// That ended the run. It is the one situation where a page's own search is
/// the obvious next move, and the goal never mentions it.
#[tokio::test]
async fn nothing_listed_fitting_is_a_reason_to_search_the_page() {
    let mut search = control("e9", "searchbox", "Search the API docs");
    search.state.editable = true;
    let view = page(
        "https://example.test/docs",
        1,
        vec![
            control("e1", "link", "Home"),
            control("e2", "link", "Guides"),
            search,
        ],
    );
    let driver = Fake::new(view);
    let oracle = Scripted::new(vec![
        // Nothing here will do.
        vec![
            ("operation", spread("ACT", 0.90, 0.03)),
            ("act_how", spread("CLICK", 0.90, 0.03)),
            ("target", spread("none_of_these", 0.90, 0.03)),
        ],
        // Asked again, with the search field in front.
        vec![
            ("operation", spread("ACT", 0.95, 0.02)),
            ("act_how", spread("FILL", 0.95, 0.02)),
            ("target", spread("e9", 0.95, 0.02)),
            ("fill_value", spread("search term", 0.95, 0.02)),
        ],
        vec![("operation", spread("DONE", 0.95, 0.02))],
    ]);
    let run = run_for("find the page about the secure tunnel").offering(Offered {
        values: vec![NamedValue::new("search term", "secure tunnel")],
        ..Offered::default()
    });

    let outcome = advance(&oracle, &driver, run).await;

    assert_eq!(
        outcome.status,
        Status::Satisfied,
        "the run carried on instead of ending on nothing_offered"
    );
    assert_eq!(
        driver.targets_acted_on(),
        vec!["e9".to_string()],
        "and what it acted on was the page's own search field"
    );
    assert!(
        outcome
            .decided
            .iter()
            .any(|decided| decided.verdict == "handed_back"),
        "the refusal is still on the record, it just is not the ending"
    );
}

/// Once per page. A search that does not help must not be tried again, or the
/// run trades one dead end for a loop.
#[tokio::test]
async fn a_page_is_searched_once_and_then_handed_back() {
    let mut search = control("e9", "searchbox", "Search");
    search.state.editable = true;
    let view = page(
        "https://example.test/docs",
        1,
        vec![control("e1", "link", "Home"), search],
    );
    let driver = Fake::new(view);
    let nothing_fits = || {
        vec![
            ("operation", spread("ACT", 0.90, 0.03)),
            ("act_how", spread("CLICK", 0.90, 0.03)),
            ("target", spread("none_of_these", 0.90, 0.03)),
        ]
    };
    let oracle = Scripted::new(vec![nothing_fits(), nothing_fits()]);

    let outcome = advance(&oracle, &driver, run_for("find something not here")).await;

    let Status::NeedsInput(why, _) = outcome.status else {
        panic!("expected a hand-back, got {:?}", outcome.status);
    };
    assert_eq!(why, Handback::NothingOffered);
    assert_eq!(
        outcome.decided.len(),
        2,
        "it asked twice and then stopped, rather than asking forever"
    );
}

/// A page with no way to search itself hands back as it always did, so the
/// change is additive rather than a new way to spend a budget.
#[tokio::test]
async fn a_page_that_cannot_search_itself_still_hands_back_at_once() {
    let view = page(
        "https://example.test/",
        1,
        vec![
            control("e1", "link", "Home"),
            control("e2", "link", "About"),
        ],
    );
    let driver = Fake::new(view);
    let oracle = Scripted::new(vec![vec![
        ("operation", spread("ACT", 0.90, 0.03)),
        ("act_how", spread("CLICK", 0.90, 0.03)),
        ("target", spread("none_of_these", 0.90, 0.03)),
    ]]);

    let outcome = advance(&oracle, &driver, run_for("find something not here")).await;

    let Status::NeedsInput(why, _) = outcome.status else {
        panic!("expected a hand-back, got {:?}", outcome.status);
    };
    assert_eq!(why, Handback::NothingOffered);
    assert_eq!(outcome.decided.len(), 1, "one question, one answer, done");
}

/// The docs-site loop, replayed. Eleven actions, eleven changes, nothing
/// retired, and the run opened the same dialog three times. Every guard the
/// loop had was built on the page changing, and the page changed every time.
#[tokio::test]
async fn a_run_that_changes_the_page_and_gets_nowhere_stops() {
    let view = page(
        "https://example.test/docs",
        1,
        vec![
            control("e1", "button", "Open the dialog"),
            control("e2", "button", "Close the dialog"),
        ],
    );
    // Acting toggles the fingerprint and leaves the url and the control count
    // alone, which is what opening and closing a panel looks like.
    let driver = Fake::new(view).toggling(&["e1", "e2"]);
    let toggle = |target: &'static str| {
        vec![
            ("operation", spread("ACT", 0.95, 0.02)),
            ("act_how", spread("CLICK", 0.95, 0.02)),
            ("target", spread(target, 0.95, 0.02)),
        ]
    };
    let oracle = Scripted::new(vec![
        toggle("e1"),
        toggle("e2"),
        toggle("e1"),
        toggle("e2"),
        toggle("e1"),
        toggle("e2"),
    ]);

    let outcome = advance(&oracle, &driver, run_for("find the thing")).await;

    assert_eq!(outcome.status, Status::Stalled(Stuck::GoingInCircles));
    assert_eq!(
        outcome.retired, 0,
        "nothing was dead, which is why the old guards could not see this"
    );
    assert!(
        outcome.trail.iter().all(|step| step.page_changed),
        "every step changed the page"
    );
    assert!(
        outcome.trail.len() < 6,
        "and it stopped before spending the whole script, got {} steps",
        outcome.trail.len()
    );
}

/// A run that is getting somewhere is not stopped. Each step here moves the
/// url, which is what following a trail of links looks like.
#[tokio::test]
async fn a_run_that_keeps_arriving_somewhere_is_left_alone() {
    let view = page(
        "https://example.test/one",
        1,
        vec![
            control("e1", "link", "Next"),
            control("e2", "link", "Other"),
        ],
    );
    let driver = Fake::new(view);
    let step = || {
        vec![
            ("operation", spread("ACT", 0.95, 0.02)),
            ("act_how", spread("CLICK", 0.95, 0.02)),
            ("target", spread("e1", 0.95, 0.02)),
        ]
    };
    let oracle = Scripted::new(vec![
        step(),
        step(),
        step(),
        step(),
        step(),
        vec![("operation", spread("DONE", 0.95, 0.02))],
    ]);

    let outcome = advance(&oracle, &driver, run_for("follow the trail")).await;

    assert_eq!(
        outcome.status,
        Status::Satisfied,
        "the url moved every step, so none of them was circling"
    );
}

/// The per-page flag is forgotten when the page changes, and an active page
/// changes on every action. Measured live: a run was refused three times on a
/// moving page and reached for a search all three. A run-wide bound is what
/// makes "once per page" mean something.
#[tokio::test]
async fn a_run_reaches_for_a_search_a_bounded_number_of_times() {
    let mut search = control("e9", "searchbox", "Search");
    search.state.editable = true;
    let view = page(
        "https://example.test/",
        1,
        vec![control("e1", "link", "Something"), search],
    );
    // Acting moves the page, which is what re-armed the per-page flag.
    let driver = Fake::new(view);
    let nothing_fits = || {
        vec![
            ("operation", spread("ACT", 0.90, 0.03)),
            ("act_how", spread("CLICK", 0.90, 0.03)),
            ("target", spread("none_of_these", 0.90, 0.03)),
        ]
    };
    let acts = || {
        vec![
            ("operation", spread("ACT", 0.95, 0.02)),
            ("act_how", spread("CLICK", 0.95, 0.02)),
            ("target", spread("e1", 0.95, 0.02)),
        ]
    };
    // Refused, acts, refused, acts, refused: without a bound the third refusal
    // would reach for a search again.
    let oracle = Scripted::new(vec![
        nothing_fits(),
        acts(),
        nothing_fits(),
        acts(),
        nothing_fits(),
        acts(),
        nothing_fits(),
    ]);

    let outcome = advance(&oracle, &driver, run_for("find something")).await;

    let Status::NeedsInput(why, _) = outcome.status else {
        panic!("expected a hand-back, got {:?}", outcome.status);
    };
    assert_eq!(why, Handback::NothingOffered);
    let refusals = outcome
        .decided
        .iter()
        .filter(|decided| decided.verdict == "handed_back")
        .count();
    assert_eq!(
        refusals, 3,
        "two refusals were answered by searching and the third ended the run"
    );
}

/// The docs-site loop, as the caller-side tests recorded it. Four moves to a
/// lap, walked three times, having reached the page it was sent for on the
/// fourth action. Both guards missed it: no two consecutive moves aimed at the
/// same control, and the url and control count moved on every single step.
#[tokio::test]
async fn a_four_move_cycle_is_a_cycle() {
    let view = page(
        "https://example.test/docs",
        1,
        vec![
            control("e1", "button", "Search the API docs"),
            control("e2", "searchbox", "Search docs"),
            control("e3", "link", "Webhooks, API docs"),
        ],
    );
    // Acting cycles the page through a handful of arrangements rather than
    // inventing a new one each time, which is what a search dialog opening and
    // closing over the same two pages does.
    let driver = Fake::new(view).cycling(3);
    let lap = || {
        vec![
            vec![
                ("operation", spread("ACT", 0.95, 0.02)),
                ("act_how", spread("CLICK", 0.95, 0.02)),
                ("target", spread("e1", 0.95, 0.02)),
            ],
            vec![
                ("operation", spread("ACT", 0.95, 0.02)),
                ("act_how", spread("FILL", 0.95, 0.02)),
                ("target", spread("e2", 0.95, 0.02)),
                ("fill_value", spread("search term", 0.95, 0.02)),
            ],
            vec![
                ("operation", spread("ACT", 0.95, 0.02)),
                ("act_how", spread("CLICK", 0.95, 0.02)),
                ("target", spread("e3", 0.95, 0.02)),
            ],
        ]
    };
    let mut script = Vec::new();
    for _ in 0..4 {
        script.extend(lap());
    }
    let oracle = Scripted::new(script);
    let run = run_for("search the docs for webhooks").offering(Offered {
        values: vec![NamedValue::new("search term", "webhooks")],
        ..Offered::default()
    });

    let outcome = advance(&oracle, &driver, run).await;

    assert_eq!(outcome.status, Status::Stalled(Stuck::GoingInCircles));
    assert!(
        outcome.trail.len() <= 8,
        "it stopped inside the second lap rather than walking twelve, got {} steps",
        outcome.trail.len()
    );
    assert!(
        outcome.trail.iter().all(|step| step.page_changed),
        "every step changed the page, which is what hid this"
    );
    assert_eq!(
        outcome.retired, 0,
        "nothing was dead either, so neither old guard could fire"
    );
}

/// A run following a trail of new pages is left alone, because each step lands
/// somewhere it has not stood before.
#[tokio::test]
async fn a_run_reaching_new_pages_is_not_called_a_circle() {
    let view = page(
        "https://example.test/one",
        1,
        vec![
            control("e1", "link", "Next"),
            control("e2", "link", "Other"),
        ],
    );
    let driver = Fake::new(view);
    let step = || {
        vec![
            ("operation", spread("ACT", 0.95, 0.02)),
            ("act_how", spread("CLICK", 0.95, 0.02)),
            ("target", spread("e1", 0.95, 0.02)),
        ]
    };
    let oracle = Scripted::new(vec![
        step(),
        step(),
        step(),
        step(),
        step(),
        step(),
        vec![("operation", spread("DONE", 0.95, 0.02))],
    ]);

    let outcome = advance(&oracle, &driver, run_for("follow the trail")).await;

    assert_eq!(
        outcome.status,
        Status::Satisfied,
        "six steps, six new pages, no circle"
    );
}

/// The record has to say what was typed, not the name the caller filed it
/// under. The model picks a value by name because it cannot write a string,
/// and a story that reports the name tells the caller it typed "search term".
#[tokio::test]
async fn the_story_says_what_was_typed_not_what_it_was_called() {
    let view = page(
        "https://example.test/",
        1,
        vec![control("e1", "searchbox", "Search")],
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
    let run = run_for("search for the tunnel docs").offering(Offered {
        values: vec![NamedValue::new("search term", "MCP tunnel")],
        ..Offered::default()
    });

    let outcome = advance(&oracle, &driver, run).await;

    let told = browseros_decide::report::tell(&outcome);
    assert_eq!(
        told.story[0], "typed \"MCP tunnel\" into Search",
        "the text, not the key it was chosen by"
    );
}
