//! Runs whole errands against the live model, with a browser made of fixtures.
//!
//! Everything above the browser is real: the real endpoint, the real questions,
//! the real gate, the real loop. What is faked is the page, so an errand runs in
//! seconds and the same errand runs the same way tomorrow.
//!
//! The pages are shaped like the ones that beat the previous design. The point
//! is not that the model answers well in the abstract; it is that these four
//! specific failures do not come back.
//!
//!   cargo run --example live_errand
//!
//! Reads JEV_TOKEN from the environment and never prints it.

use std::cell::RefCell;

use browseros_decide::client::Jev;
use browseros_decide::drive::{ActError, Answer, Driver, Move, Performed};
use browseros_decide::offer::NamedValue;
use browseros_decide::pursue::{Status, advance};
use browseros_decide::report::{Told, tell};
use browseros_decide::run::{Budget, Offered, Run};
use browseros_decide::view::{Control, ControlState, PageView};
use browseros_decide::vocabulary::How;

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

fn page(url: &str, title: &str, text: &str, controls: Vec<Control>) -> PageView {
    PageView {
        url: url.to_string(),
        title: title.to_string(),
        text: text.to_string(),
        controls,
        fingerprint: 1,
        unlabelled_images: 0,
    }
}

/// A browser made of fixtures, which behaves like one.
///
/// The first version of this bumped a fingerprint and left the controls where
/// they were, so a click that "navigated" left its own link on the page and the
/// loop quite reasonably clicked it again. That was the fixture lying, not the
/// loop failing, and it would have hidden every regression this harness exists
/// to catch. A fake browser has to take the page away when a link is followed.
struct Fixture {
    page: RefCell<PageView>,
    /// References whose click this page swallows: the action reports success and
    /// nothing changes. Wikipedia's photo viewer does this to links inside image
    /// captions.
    inert: Vec<&'static str>,
    text: String,
    acted: RefCell<Vec<String>>,
}

impl Fixture {
    fn new(page: PageView, text: &str) -> Self {
        Self {
            page: RefCell::new(page),
            inert: Vec::new(),
            text: text.to_string(),
            acted: RefCell::new(Vec::new()),
        }
    }

    fn swallowing(mut self, inert: &[&'static str]) -> Self {
        self.inert = inert.to_vec();
        self
    }

    fn acted_on(&self) -> Vec<String> {
        self.acted.borrow().clone()
    }

    /// Following a link takes the page away, which is what a browser does.
    fn arrive_at(&self, from: &str) {
        let mut page = self.page.borrow_mut();
        page.fingerprint += 1;
        page.url = format!("https://example.test/after-{from}");
        page.title = format!("The page after {from}");
        page.text = "This is a different page.".to_string();
        page.controls = vec![control("n1", "link", "Back to where you were")];
    }
}

impl Driver for Fixture {
    fn stopped(&self) -> bool {
        false
    }

    async fn observe(&self) -> Result<PageView, String> {
        Ok(self.page.borrow().clone())
    }

    async fn perform(&self, act: &Move) -> Result<Performed, ActError> {
        match act {
            Move::Report(ask) => Ok(Performed::Answer(Answer {
                asked: ask.as_str().to_string(),
                text: Some(self.text.clone()),
                pictured: ask.as_str() == "screenshot",
            })),
            Move::Act(decision) => {
                let Some(how) = decision.how else {
                    return Ok(Performed::Unmoved);
                };
                let aimed = decision
                    .target
                    .clone()
                    .unwrap_or_else(|| "the page".to_string());
                self.acted.borrow_mut().push(aimed.clone());

                if self.inert.contains(&aimed.as_str()) {
                    return Ok(Performed::Unmoved);
                }
                match how {
                    How::Scroll => Ok(Performed::Unmoved),
                    // Typing leaves the text in the box, so the next decision can
                    // see that the field is no longer empty.
                    How::Fill => {
                        let mut page = self.page.borrow_mut();
                        page.fingerprint += 1;
                        let typed = decision.value.clone().unwrap_or_default();
                        if let Some(control) = page
                            .controls
                            .iter_mut()
                            .find(|control| control.reference == aimed)
                        {
                            control.value = Some(typed);
                        }
                        Ok(Performed::Moved)
                    }
                    How::Check | How::Uncheck => {
                        let mut page = self.page.borrow_mut();
                        page.fingerprint += 1;
                        if let Some(control) = page
                            .controls
                            .iter_mut()
                            .find(|control| control.reference == aimed)
                        {
                            control.state.checked = Some(how == How::Check);
                        }
                        Ok(Performed::Moved)
                    }
                    How::Click | How::Select | How::Press | How::Hover => {
                        self.arrive_at(&aimed);
                        Ok(Performed::Moved)
                    }
                }
            }
            Move::Travel(_) => {
                self.arrive_at("a destination");
                Ok(Performed::Moved)
            }
            Move::Resnapshot | Move::Settle => Ok(Performed::Unmoved),
        }
    }
}

/// One errand, and what it is checking for.
struct Errand {
    name: &'static str,
    goal: &'static str,
    tests: &'static str,
    driver: Fixture,
    offered: Offered,
    /// What a pass looks like, judged from the outcome and what was acted on.
    passes: fn(&Told, &[String]) -> Result<(), String>,
}

/// A Hacker News shaped page: two hundred stories and a link called More, last.
fn hacker_news() -> PageView {
    let mut controls: Vec<Control> = (0..200)
        .map(|index| {
            control(
                &format!("e{index}"),
                "link",
                &format!("Story number {index} about something or other"),
            )
        })
        .collect();
    controls.push(control("more", "link", "More"));
    page(
        "https://news.example.test/",
        "Hacker News",
        "312 points by someone 2 hours ago | 88 comments",
        controls,
    )
}

fn errands() -> Vec<Errand> {
    vec![
        Errand {
            name: "a link whose click the page swallows",
            goal: "open the article about Floyd-Steinberg dithering",
            tests: "the dead control is tried once, retired, and the next one chosen",
            driver: Fixture::new(
                page(
                    "https://example.test/dither",
                    "Dither",
                    "Dither is an intentionally applied form of noise.",
                    vec![
                        control("e483", "link", "Floyd-Steinberg dithering"),
                        control("e224", "link", "Floyd-Steinberg dithering explained"),
                        control("e99", "link", "Ordered dithering"),
                    ],
                ),
                "",
            )
            .swallowing(&["e483"]),
            offered: Offered::default(),
            passes: |told, acted| {
                let on_controls: Vec<&String> =
                    acted.iter().filter(|aimed| *aimed != "the page").collect();
                let twice = on_controls.windows(2).any(|pair| pair[0] == pair[1]);
                if twice {
                    return Err(format!("it acted on the same control twice: {acted:?}"));
                }
                if told.retired == 0 && on_controls.len() > 1 {
                    return Err("it moved on without retiring anything".to_string());
                }
                Ok(())
            },
        },
        Errand {
            name: "a question whose answer is printed on the page",
            goal: "find out how many points the top story has",
            tests: "it reads the page rather than claiming the goal is met",
            driver: Fixture::new(hacker_news(), "312 points by someone 2 hours ago"),
            offered: Offered::default(),
            passes: |told, _| match told.status.as_str() {
                "answered" => Ok(()),
                "satisfied" => {
                    Err("it claimed the goal was met and returned nothing".to_string())
                }
                other => Err(format!("ended as {other}")),
            },
        },
        Errand {
            name: "a control at the very bottom of a long page",
            goal: "go to the second page of stories",
            tests: "the More link is offered despite sharing no word with the goal",
            driver: Fixture::new(hacker_news(), ""),
            offered: Offered::default(),
            passes: |_, acted| {
                if acted.iter().any(|reference| reference == "more") {
                    Ok(())
                } else {
                    Err(format!("it never reached the More link, acted on {acted:?}"))
                }
            },
        },
        Errand {
            name: "a value the caller supplied",
            goal: "search this site for dithering",
            tests: "it types the caller's value rather than inventing one",
            driver: Fixture::new(
                page(
                    "https://example.test/",
                    "Example",
                    "A site with a search box.",
                    vec![
                        control("e1", "searchbox", "Search"),
                        control("e2", "link", "About us"),
                        control("e3", "link", "Contact"),
                    ],
                ),
                "",
            ),
            offered: Offered {
                values: vec![NamedValue::new("search term", "dithering")],
                ..Offered::default()
            },
            passes: |told, acted| {
                if told.status == "no_field_named" {
                    return Err("it had a value and still asked for one".to_string());
                }
                if acted.is_empty() {
                    return Err(format!("it never acted, ending as {}", told.status));
                }
                let repeats = acted.windows(2).filter(|pair| pair[0] == pair[1]).count();
                if repeats > 0 {
                    return Err(format!(
                        "it typed into the same field twice running: {acted:?}"
                    ));
                }
                Ok(())
            },
        },
    ]
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = std::env::var("JEV_TOKEN")
        .map_err(|_| "JEV_TOKEN is not set in this environment")?;
    if token.trim().is_empty() {
        return Err("JEV_TOKEN is empty".into());
    }
    let jev = Jev::new(token);

    println!("Four errands against the live model, on pages shaped like the ones");
    println!("that beat the previous design.\n");

    let mut failed = 0usize;
    let mut tokens = 0u64;

    for errand in errands() {
        let run = Run::new(
            errand.goal,
            Budget {
                max_steps: 6,
                max_seconds: 90,
            },
        )
        .offering(errand.offered.clone());

        let started = std::time::Instant::now();
        let outcome = advance(&jev, &errand.driver, run).await;
        let told = tell(&outcome);
        let acted = errand.driver.acted_on();
        tokens += told.input_tokens;

        let verdict = (errand.passes)(&told, &acted);
        let mark = if verdict.is_ok() { "ok  " } else { "FAIL" };
        if verdict.is_err() {
            failed += 1;
        }

        println!("{mark}  {}", errand.name);
        println!("      checking: {}", errand.tests);
        println!("      goal:     {}", errand.goal);
        println!(
            "      ended:    {} after {} decisions, {} actions, {} tokens, {:.1}s",
            told.status,
            told.decisions,
            told.actions,
            told.input_tokens,
            started.elapsed().as_secs_f64()
        );
        println!("      said:     {}", told.said);
        if !acted.is_empty() {
            println!("      acted on: {}", acted.join(", "));
        }
        if told.retired > 0 {
            println!("      retired:  {} control(s) that changed nothing", told.retired);
        }
        if let Some(answer) = &told.answer {
            let text = answer.text.clone().unwrap_or_default();
            println!(
                "      handed back ({}): {}",
                answer.asked,
                text.chars().take(70).collect::<String>()
            );
        }
        if let Err(why) = verdict {
            println!("      WHY IT FAILED: {why}");
        }
        println!();
    }

    println!(
        "{} of {} errands behaved, {} input tokens in total, about ${:.5}",
        errands().len() - failed,
        errands().len(),
        tokens,
        tokens as f64 * 0.042 / 1_000_000.0
    );
    if failed > 0 {
        std::process::exit(1);
    }
    let _ = Status::Satisfied;
    Ok(())
}
