//! What the caller is told, and how.
//!
//! The loop's [`Outcome`] is everything that happened. This turns it into what
//! a caller branches on, which is deliberately a smaller thing: a word for code
//! and a sentence for a person, plus the numbers needed to judge the answer
//! rather than trust it.
//!
//! The sentence matters more than it looks. A run that handed back with a
//! status and nothing beside it told a caller only that something had gone
//! wrong, and measured live, that is exactly what one got twice in a single
//! errand.

use crate::gate::Handback;
use crate::pursue::{Outcome, Status, Stuck};
use crate::view::PageView;

/// One control, as a caller needs it to compose a better goal.
///
/// Three fields and no more. A reference so the caller can name it back, a role
/// so it knows what the thing is, and a name because that is what a goal is
/// written in. Its state is folded into one word where it has one, because
/// "ticked" is what a caller acts on and a struct of five optional booleans is
/// not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub reference: String,
    pub role: String,
    pub name: String,
    pub state: Option<String>,
}

/// The page a run ended on.
///
/// The reason this exists: a status and a sentence let a caller reword its goal
/// and hope. Control names let it change the goal to something the page can
/// actually satisfy. That difference is the whole pattern.
///
/// What is deliberately not here is the whole accessibility tree. Sending it
/// would put every intermediate page back into the caller's context, which is
/// the cost this design exists to avoid, so the text is budgeted and the
/// controls are named rather than described.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub url: String,
    pub title: String,
    /// Every control the last decision could choose from.
    pub controls: Vec<Shown>,
    /// What is covering the page, if anything, because a caller told to read a
    /// page a dialog is over reads the dialog.
    pub obstructed: Option<String>,
    /// Controls that exist and are not in `controls`, so a caller knows the
    /// list is partial rather than complete.
    pub omitted: usize,
}

/// Turns the page a run ended on into what the caller is shown.
#[must_use]
fn seen(view: &PageView, omitted: usize) -> Seen {
    Seen {
        url: view.url.clone(),
        title: view.title.clone(),
        controls: view
            .controls
            .iter()
            .filter(|control| control.is_offerable())
            .map(|control| Shown {
                reference: control.reference.clone(),
                role: control.role.clone(),
                name: control.name.clone(),
                state: control.state.describe(),
            })
            .collect(),
        obstructed: view.obstructed(),
        omitted,
    }
}

/// What the caller gets when a run ends.
#[derive(Debug, Clone, PartialEq)]
pub struct Told {
    /// One word, for code to branch on.
    pub status: String,
    /// One sentence, for a person to read. True of what actually happened.
    pub said: String,
    /// Actions that actually ran, which is not the same as decisions asked for.
    pub actions: usize,
    /// How many of those changed the page.
    pub changed: usize,
    pub decisions: u32,
    /// Questions asked about a part of a page, which is more than `decisions`
    /// whenever a page did not fit in one question.
    pub batches: u32,
    /// How much of the page a decision could see, and how much it could not.
    pub offered: usize,
    pub omitted: usize,
    /// Controls retired for changing nothing when acted on.
    pub retired: usize,
    /// Images on the page carrying no accessible name.
    ///
    /// Reported because this model reads an accessibility tree, so whatever
    /// those pictures convey was invisible to every decision here. The caller
    /// has eyes and owns the screenshot tool.
    pub unlabelled_images: usize,
    /// How likely the last decision thought the goal was already satisfied.
    ///
    /// Reported, never acted on. A completion claim is not evidence, and this
    /// one is weaker than the DONE it sits beside, because DONE is chosen
    /// against competing alternatives and this has none to lose to.
    pub goal_met: Option<f64>,
    pub url: String,
    pub title: String,
    pub input_tokens: u64,
    /// The page it ended on, so a caller that has to try again can see what it
    /// is trying again against.
    pub page: Option<Seen>,
    /// One line saying what to change and try again, where there is one.
    ///
    /// Derived rather than guessed: every arm comes from what the run ended
    /// with and which judgement was weakest. This is the field that turns a
    /// failed first attempt into a better second goal, which is the whole
    /// reason a caller reads any of the rest.
    pub try_next: Option<String>,
    /// The run, in order, as sentences a person can read.
    ///
    /// `decided` is a flat array of eleven fields a decision and leaves the
    /// reading to the caller. This is the same information as prose, which is
    /// what a caller deciding how to word its next goal actually reads.
    pub story: Vec<String>,
    /// Every decision the run paid for, and what the gate made of each.
    ///
    /// The one thing a caller could not see. A run reported twelve scalars and
    /// a page, so three live failures in a row had to be diagnosed by reading
    /// the source of the thing that failed.
    pub decided: Vec<crate::pursue::Decided>,
}

/// Turns what happened into what the caller is told.
#[must_use]
pub fn tell(outcome: &Outcome) -> Told {
    let actions = outcome.trail.len();
    let changed = outcome
        .trail
        .iter()
        .filter(|step| step.page_changed)
        .count();

    let try_next = try_next(outcome);
    Told {
        status: outcome.status.as_str().to_string(),
        said: said(outcome, actions, changed, try_next.as_deref()),
        actions,
        changed,
        decisions: outcome.decisions,
        batches: outcome.batches,
        offered: outcome.offered,
        omitted: outcome.omitted,
        retired: outcome.retired,
        unlabelled_images: outcome.unlabelled_images,
        goal_met: outcome.goal_met,
        url: outcome.url_after.clone(),
        title: outcome.title.clone(),
        input_tokens: outcome.input_tokens,
        page: outcome
            .ended_on
            .as_ref()
            .map(|view| seen(view, outcome.omitted)),
        try_next,
        story: story(outcome),
        decided: outcome.decided.clone(),
    }
}

/// What the caller could change, so a second attempt does better.
///
/// A mapping from what the run ended with, nothing more. Every arm is
/// reachable from a scripted model, so every arm has a test.
fn try_next(outcome: &Outcome) -> Option<String> {
    let weakest = outcome
        .decided
        .iter()
        .rev()
        .find_map(|decided| decided.weakest);
    let held_a_value = outcome
        .decided
        .iter()
        .rev()
        .any(|decided| decided.how == Some("FILL") && decided.verdict == "ran");

    let mut advice = match &outcome.status {
        Status::NeedsInput(Handback::NothingOffered, _) => {
            if outcome
                .ended_on
                .as_ref()
                .is_some_and(crate::view::PageView::can_search_itself)
            {
                "I searched this page and still found nothing. Name the control, or give me a \
                 destination."
            } else {
                "Name the control you mean, or give me a destination to go to."
            }
        }
        Status::NeedsInput(Handback::TooRiskyToGuess, _) => {
            "This one needs you to name the control. I will not guess at it."
        }
        Status::NeedsInput(Handback::NoFieldNamed, _) => {
            // Named where the run knows the field, because the caller's next
            // move is to supply a value for exactly that one.
            if let Some(field) = outcome
                .decided
                .iter()
                .rev()
                .find(|decided| decided.verdict == "handed_back")
                .and_then(|decided| decided.target_name.as_deref())
            {
                return Some(format!(
                    "Pass a value for {field:?} and run this again, and I will type it in."
                ));
            }
            "Pass the text to type as a value, and I will put it in the field."
        }
        Status::Stalled(Stuck::NotConfident) => match weakest {
            Some("which move to make") => {
                "Say what to do rather than what to find out. A goal with one verb decides faster."
            }
            Some("how to act on it") if held_a_value => {
                "That field already holds your text. Ask me to submit it."
            }
            Some("which control to use it on") => "Name the control, or narrow the page first.",
            _ => "Break the goal into one step and give me that step.",
        },
        Status::Stalled(Stuck::GoingInCircles) => {
            "It kept returning to pages it had already seen. Give it one step at a time."
        }
        Status::Stalled(Stuck::NothingChanged) => {
            "What it acted on did nothing. The control the goal needs may not be on this page."
        }
        Status::Stalled(Stuck::OnlyWaiting) => {
            "The page never settled. Try again, or give it a destination to load directly."
        }
        Status::Stalled(Stuck::BrowserKeptFailing) => {
            "The browser kept refusing. Check the tab is still the one you meant."
        }
        Status::Blocked(_) => "Nothing on this page can do that. It may need a different page.",
        Status::BudgetSpent => "Raise maxSteps, or split the goal in two.",
        Status::Satisfied | Status::Cancelled => return None,
        Status::NeedsInput(_, _) => return None,
    }
    .to_string();

    // How much of the page a decision could see is part of judging its answer,
    // and it changes what the caller should do about it.
    if outcome.omitted > 0 {
        advice.push_str(&format!(
            " I could only choose between {} of {} controls.",
            outcome.offered,
            outcome.offered + outcome.omitted
        ));
    }
    Some(advice)
}

/// The run as sentences, built from what was already recorded.
///
/// A pure function of the outcome, so the whole thing is testable without a
/// browser or a model. Walks the decisions and the steps together: a decision
/// that ran has a step beside it, and a decision that was refused does not.
fn story(outcome: &Outcome) -> Vec<String> {
    let mut lines = Vec::new();
    let mut steps = outcome.trail.iter();

    for decided in &outcome.decided {
        if decided.verdict != "ran" {
            lines.push(refused(decided, outcome));
            continue;
        }
        let step = steps.next();
        lines.push(ran(decided, step));
    }
    lines
}

/// What a refused decision says about itself.
fn refused(decided: &crate::pursue::Decided, outcome: &Outcome) -> String {
    let what = decided.operation.to_lowercase();
    match (decided.verdict, decided.weakest) {
        ("handed_back", _) => format!(
            "none of the {} controls it could choose between would do",
            outcome.offered
        ),
        (_, Some(weakest)) => format!(
            "{what} was too close to call on {weakest}{}",
            match (decided.margin, decided.needed) {
                (Some(margin), Some(needed)) =>
                    format!(", leading by {margin:.2} where it needed {needed:.2}"),
                _ => String::new(),
            }
        ),
        _ => format!("{what} was refused"),
    }
}

/// What a decision that ran says about itself, with what it moved.
fn ran(decided: &crate::pursue::Decided, step: Option<&crate::pursue::Step>) -> String {
    let on = decided
        .target_name
        .as_deref()
        .filter(|name| !name.trim().is_empty());
    let moved = step.map_or(String::new(), |step| {
        step.moved
            .iter()
            .map(|moved| format!(", and {:?} became {:?}", moved.was, moved.now))
            .collect::<Vec<_>>()
            .join("")
    });

    let did = match (decided.operation, decided.how) {
        ("ACT", Some("FILL")) => match (&step.and_then(|step| step.value.clone()), on) {
            (Some(text), Some(name)) => format!("typed {text:?} into {name}"),
            (Some(text), None) => format!("typed {text:?}"),
            (None, Some(name)) => format!("typed into {name}"),
            (None, None) => "typed something".to_string(),
        },
        ("ACT", Some("SUBMIT")) => "submitted it".to_string(),
        ("ACT", Some("SCROLL")) => "scrolled the page".to_string(),
        ("ACT", Some(how)) => {
            // Past tense, because the story is a record of what happened. The
            // vocabulary's own words are imperatives.
            let did = match how {
                "CLICK" => "clicked",
                "CHECK" => "ticked",
                "UNCHECK" => "unticked",
                "HOVER" => "hovered over",
                "SELECT" => "set",
                "PRESS" => "pressed a key on",
                other => other,
            };
            match on {
                Some(name) => format!("{did} {name}"),
                None => did.to_string(),
            }
        }
        ("NAVIGATE", _) => match on {
            Some(name) => format!("went to {name}"),
            None => "went to another page".to_string(),
        },
        ("DONE", _) => "said the goal was met".to_string(),
        ("BLOCKED", _) => "said nothing on this page could do it".to_string(),
        (operation, _) => operation.to_lowercase(),
    };

    let arrived = step
        .map(|step| step.url_after.as_str())
        .filter(|url| !url.is_empty())
        .filter(|_| decided.how == Some("SUBMIT") || decided.operation == "NAVIGATE")
        .map_or(String::new(), |url| format!(", and the page became {url}"));

    format!("{did}{arrived}{moved}")
}

/// One sentence, which has to be true of what happened.
fn said(outcome: &Outcome, actions: usize, changed: usize, try_next: Option<&str>) -> String {
    let did = match actions {
        0 => "It took no action".to_string(),
        1 => "It took one action".to_string(),
        many => format!("It took {many} actions"),
    };
    let moved = match (actions, changed) {
        (0, _) => String::new(),
        (_, 0) => ", none of which changed the page".to_string(),
        (a, c) if a == c => String::new(),
        (_, c) => format!(", {c} of which changed the page"),
    };

    let ended = if outcome.url_after.is_empty() {
        String::new()
    } else {
        format!(" It ended on {}.", outcome.url_after)
    };
    let next = try_next.map_or(String::new(), |line| format!(" {line}"));
    let sentence = match &outcome.status {
        Status::Satisfied => format!(
            "{did}{moved}, and then said every requirement of the goal was visible on the page."
        ),
        Status::Blocked(why) => format!("{did}{moved}, then stopped: {why}."),
        Status::Stalled(stuck) => format!(
            "{did}{moved}, then stopped making progress ({}).",
            stuck.as_str()
        ),
        Status::NeedsInput(_, why) => format!("{did}{moved}, then came back to you: {why}."),
        Status::BudgetSpent => {
            format!("{did}{moved}, and ran out of steps before the goal was met.")
        }
        Status::Cancelled => format!(
            "{did}{moved}, then something outside the run stopped it before the goal was met."
        ),
    };
    format!("{sentence}{ended}{next}")
}

impl Told {
    /// Whether the caller still has something to do.
    #[must_use]
    pub fn wants_the_caller(&self) -> bool {
        !matches!(self.status.as_str(), "satisfied" | "answered")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::Handback;
    use crate::pursue::{Status, Step, Stuck};
    use crate::vocabulary::{How, Operation};

    fn step(changed: bool) -> Step {
        Step {
            operation: Operation::Act,
            how: Some(How::Click),
            target: Some("e1".to_string()),
            value: None,
            confidence: 0.9,
            page_changed: changed,
            moved: Vec::new(),
            url_after: "https://example.test/".to_string(),
            input_tokens: 1_000,
        }
    }

    fn filled(text: &str, url: &str) -> Step {
        Step {
            how: Some(How::Fill),
            value: Some(text.to_string()),
            url_after: url.to_string(),
            ..step(true)
        }
    }

    fn submitted(url: &str) -> Step {
        Step {
            how: Some(How::Submit),
            target: None,
            url_after: url.to_string(),
            ..step(true)
        }
    }

    fn clicked_and_moved(was: &str, now: &str) -> Step {
        Step {
            moved: vec![crate::view::Moved {
                reference: "e63".to_string(),
                was: was.to_string(),
                now: now.to_string(),
            }],
            ..step(true)
        }
    }

    fn decision_that_ran(
        operation: &'static str,
        how: Option<&'static str>,
        target_name: Option<&str>,
    ) -> crate::pursue::Decided {
        crate::pursue::Decided {
            operation,
            how,
            target: Some("e1".to_string()),
            target_name: target_name.map(str::to_string),
            verdict: "ran",
            weakest: None,
            margin: None,
            needed: None,
            share: None,
            why: None,
            batch: 1,
            input_tokens: 1_000,
        }
    }

    fn outcome(status: Status, trail: Vec<Step>) -> Outcome {
        Outcome {
            status,
            offered: 12,
            omitted: 180,
            retired: 0,
            decisions: 2,
            batches: 2,
            input_tokens: 2_000,
            trail,
            decided: Vec::new(),
            url_before: "https://example.test/".to_string(),
            url_after: "https://example.test/after".to_string(),
            title: "A page".to_string(),
            goal_met: Some(0.81),
            unlabelled_images: 3,
            ended_on: None,
        }
    }

    /// A page with a control on it. The point of the fixture is that the
    /// control survives into the reply, so the caller can name it back.
    fn ended_on(status: Status) -> Outcome {
        let mut controls = vec![
            crate::view::Control {
                reference: "e7".to_string(),
                clickable: false,
                role: "button".to_string(),
                name: "Filters".to_string(),
                value: None,
                state: crate::view::ControlState::default(),
                options: Vec::new(),
                guard: 0,
                group: None,
            },
            crate::view::Control {
                reference: "e8".to_string(),
                clickable: false,
                role: "checkbox".to_string(),
                name: "In stock".to_string(),
                value: None,
                state: crate::view::ControlState {
                    checked: Some(false),
                    ..crate::view::ControlState::default()
                },
                options: Vec::new(),
                guard: 0,
                group: None,
            },
        ];
        // Disabled controls are not offered, so they are not shown either.
        controls.push(crate::view::Control {
            reference: "e9".to_string(),
            clickable: false,
            role: "button".to_string(),
            name: "Apply".to_string(),
            value: None,
            state: crate::view::ControlState {
                disabled: true,
                ..crate::view::ControlState::default()
            },
            options: Vec::new(),
            guard: 0,
            group: None,
        });
        let view = PageView::from_snapshot_parts(
            "https://example.test/results",
            "Results",
            "two hundred results",
            controls,
        );
        Outcome {
            ended_on: Some(view),
            ..outcome(status, vec![step(false)])
        }
    }

    /// The field the previous attempt did not have. Without it a caller that
    /// gets `not_confident` can only reword its goal and hope; with it, the
    /// caller can see that a Filters button exists and say so.
    #[test]
    fn a_run_that_could_not_finish_hands_back_the_page_it_stopped_on() {
        let told = tell(&ended_on(Status::Stalled(
            crate::pursue::Stuck::NotConfident,
        )));
        let page = told.page.expect("the page it stopped on");
        assert_eq!(page.url, "https://example.test/results");
        assert_eq!(page.title, "Results");
        let named: Vec<&str> = page
            .controls
            .iter()
            .map(|control| control.name.as_str())
            .collect();
        assert_eq!(named, vec!["Filters", "In stock"]);
        assert!(
            !named.contains(&"Apply"),
            "a disabled control was never offered, so showing it would invite a \
             goal that cannot be carried out"
        );
        assert_eq!(
            page.controls[1].state.as_deref(),
            Some("unchecked"),
            "a word, because that is what a caller acts on"
        );
        assert_eq!(page.omitted, 180, "and it says the list is partial");
    }

    /// A run that never read a page has no page to hand back, and says so
    /// rather than inventing an empty one.
    #[test]
    fn a_run_that_never_saw_a_page_hands_back_none() {
        let told = tell(&outcome(Status::Cancelled, Vec::new()));
        assert!(told.page.is_none());
    }

    #[test]
    fn a_satisfied_run_says_what_it_did_before_claiming_anything() {
        let told = tell(&outcome(Status::Satisfied, vec![step(true), step(true)]));
        assert_eq!(told.status, "satisfied");
        assert!(told.said.starts_with("It took 2 actions"));
        assert!(told.said.contains("said every requirement"));
    }

    /// A run that acted and changed nothing must not read like one that worked.
    #[test]
    fn a_run_that_changed_nothing_says_so() {
        let told = tell(&outcome(
            Status::Stalled(Stuck::NothingChanged),
            vec![step(false), step(false)],
        ));
        assert!(
            told.said.contains("none of which changed the page"),
            "{}",
            told.said
        );
        assert_eq!(told.changed, 0);
    }

    #[test]
    fn a_partly_effective_run_counts_only_what_landed() {
        let told = tell(&outcome(Status::BudgetSpent, vec![step(true), step(false)]));
        assert!(
            told.said.contains("1 of which changed the page"),
            "{}",
            told.said
        );
        assert_eq!(told.actions, 2);
        assert_eq!(told.changed, 1);
    }

    /// The hand-back that measured badly: a status with nothing beside it told
    /// a caller only that something had gone wrong.
    #[test]
    fn a_hand_back_carries_the_reason_as_well_as_the_word() {
        let told = tell(&outcome(
            Status::NeedsInput(
                Handback::TooRiskyToGuess,
                "ACT on \"Place order\" needs confidence 0.85 and had 0.70".to_string(),
            ),
            vec![],
        ));
        assert_eq!(told.status, "too_risky_to_guess");
        assert!(told.said.contains("needs confidence 0.85"));
        assert!(told.wants_the_caller());
    }

    /// How much of the page a decision could not see is part of judging its
    /// answer, so it travels on every ending.
    #[test]
    fn what_the_decision_could_not_see_is_always_reported() {
        let told = tell(&outcome(Status::Satisfied, vec![step(true)]));
        assert_eq!(told.offered, 12);
        assert_eq!(told.omitted, 180);
        assert_eq!(told.unlabelled_images, 3);
    }

    /// A completion claim travels as a number for the caller to weigh, never as
    /// permission.
    #[test]
    fn the_completion_claim_is_reported_not_acted_on() {
        let told = tell(&outcome(Status::BudgetSpent, vec![step(false)]));
        assert_eq!(told.goal_met, Some(0.81));
        assert_eq!(
            told.status, "budget_spent",
            "a confident claim does not change how the run ended"
        );
    }

    /// Being stopped from outside is not the same as running out of steps, and
    /// saying so plainly is what would have made a day of diagnosis an hour.
    #[test]
    fn being_cancelled_does_not_read_as_running_out_of_steps() {
        let told = tell(&outcome(Status::Cancelled, vec![]));
        assert_eq!(told.status, "cancelled");
        assert!(
            !told.said.contains("ran out of steps"),
            "a run that took no steps cannot have run out of them: {}",
            told.said
        );
        assert!(told.said.contains("stopped it"), "{}", told.said);
    }

    #[test]
    fn a_run_that_did_nothing_does_not_claim_it_did() {
        let told = tell(&outcome(Status::BudgetSpent, vec![]));
        assert!(told.said.starts_with("It took no action"), "{}", told.said);
    }

    /// A refused decision says which judgement was close and by how much,
    /// which is the thing a caller needs in order to reword a goal.
    #[test]
    fn the_story_says_why_a_decision_was_refused() {
        let mut got = outcome(Status::Stalled(Stuck::NotConfident), vec![]);
        got.decided = vec![crate::pursue::Decided {
            operation: "ACT",
            how: Some("CLICK"),
            target: None,
            target_name: None,
            verdict: "explored",
            weakest: Some("how to act on it"),
            margin: Some(0.06),
            needed: Some(0.30),
            share: Some(0.44),
            why: None,
            batch: 1,
            input_tokens: 100,
        }];

        let told = tell(&got);

        assert_eq!(
            told.story,
            vec![
                "act was too close to call on how to act on it, leading by 0.06 where it needed 0.30"
                    .to_string()
            ]
        );
    }

    /// A hand-back says how much it was choosing between, because "none of
    /// these" means something different over eight controls than over 254.
    #[test]
    fn the_story_says_how_much_it_was_choosing_between() {
        let mut got = outcome(
            Status::NeedsInput(Handback::NothingOffered, "none will do".to_string()),
            vec![],
        );
        got.offered = 254;
        got.decided = vec![crate::pursue::Decided {
            operation: "ACT",
            how: Some("CLICK"),
            target: None,
            target_name: None,
            verdict: "handed_back",
            weakest: None,
            margin: None,
            needed: None,
            share: None,
            why: Some("none of the options offered for this step can advance the goal".to_string()),
            batch: 1,
            input_tokens: 100,
        }];

        let told = tell(&got);

        assert_eq!(
            told.story,
            vec!["none of the 254 controls it could choose between would do".to_string()]
        );
    }

    fn refused_on(
        weakest: &'static str,
        how: Option<&'static str>,
        verdict: &'static str,
    ) -> crate::pursue::Decided {
        crate::pursue::Decided {
            operation: "ACT",
            how,
            target: None,
            target_name: None,
            verdict,
            weakest: Some(weakest),
            margin: Some(0.06),
            needed: Some(0.30),
            share: Some(0.44),
            why: None,
            batch: 1,
            input_tokens: 100,
        }
    }

    /// The case from the caller-side tests: it typed into the box and then
    /// could not choose between filling it again and submitting it.
    #[test]
    fn a_filled_field_is_told_to_be_submitted() {
        let mut got = outcome(Status::Stalled(Stuck::NotConfident), vec![]);
        got.omitted = 0;
        got.decided = vec![
            decision_that_ran("ACT", Some("FILL"), Some("Search Amazon.in")),
            refused_on("how to act on it", Some("FILL"), "explored"),
        ];

        let told = tell(&got);

        assert_eq!(
            told.try_next.as_deref(),
            Some("That field already holds your text. Ask me to submit it.")
        );
    }

    /// An informational goal that lost its operation head is told to say what
    /// to do instead of what to find out.
    #[test]
    fn a_thin_operation_head_is_told_to_name_an_action() {
        let mut got = outcome(Status::Stalled(Stuck::NotConfident), vec![]);
        got.decided = vec![refused_on("which move to make", None, "explored")];

        let told = tell(&got);

        assert!(
            told.try_next
                .as_deref()
                .is_some_and(|line| line.starts_with("Say what to do")),
            "got {:?}",
            told.try_next
        );
    }

    /// A refusal on a page that can search itself has already been searched,
    /// so the advice is different from one on a page that cannot.
    #[test]
    fn a_refusal_names_what_the_caller_can_supply() {
        let mut got = outcome(
            Status::NeedsInput(Handback::NothingOffered, "none will do".to_string()),
            vec![],
        );
        got.omitted = 0;
        got.decided = vec![refused_on(
            "which control to use it on",
            Some("CLICK"),
            "handed_back",
        )];

        let told = tell(&got);

        assert_eq!(
            told.try_next.as_deref(),
            Some("Name the control you mean, or give me a destination to go to.")
        );
    }

    /// A loop is told to be given one step at a time.
    #[test]
    fn going_in_circles_is_told_to_go_one_step_at_a_time() {
        let got = outcome(Status::Stalled(Stuck::GoingInCircles), vec![]);

        let told = tell(&got);

        assert!(
            told.try_next
                .as_deref()
                .is_some_and(|line| line.contains("one step at a time")),
            "got {:?}",
            told.try_next
        );
    }

    /// How much of the page it could see changes what the caller should do, so
    /// it is appended wherever it applies.
    #[test]
    fn a_page_it_could_only_partly_see_says_so() {
        let mut got = outcome(Status::BudgetSpent, vec![]);
        got.offered = 254;
        got.omitted = 46;

        let told = tell(&got);

        assert_eq!(
            told.try_next.as_deref(),
            Some(
                "Raise maxSteps, or split the goal in two. I could only choose between 254 of 300 controls."
            )
        );
    }

    /// A run that finished has nothing to advise, and saying nothing is the
    /// honest answer rather than filling the field.
    #[test]
    fn a_run_that_finished_is_given_no_advice() {
        assert!(tell(&outcome(Status::Satisfied, vec![])).try_next.is_none());
        assert!(tell(&outcome(Status::Cancelled, vec![])).try_next.is_none());
    }

    /// The amazon case as the caller would read it back: what it typed, that
    /// it submitted, and that the cart moved.
    #[test]
    fn the_story_reads_as_what_the_run_did() {
        let mut got = outcome(Status::Satisfied, vec![]);
        got.trail = vec![
            filled("sensodyne toothpaste", "https://amazon.test/"),
            submitted("https://amazon.test/s?k=sensodyne+toothpaste"),
            clicked_and_moved("0 items in cart", "2 items in cart"),
        ];
        got.decided = vec![
            decision_that_ran("ACT", Some("FILL"), Some("Search Amazon.in")),
            decision_that_ran("ACT", Some("SUBMIT"), None),
            decision_that_ran("ACT", Some("CLICK"), Some("Add to cart")),
        ];

        let told = tell(&got);

        assert_eq!(
            told.story,
            vec![
                "typed \"sensodyne toothpaste\" into Search Amazon.in".to_string(),
                "submitted it, and the page became https://amazon.test/s?k=sensodyne+toothpaste"
                    .to_string(),
                "clicked Add to cart, and \"0 items in cart\" became \"2 items in cart\""
                    .to_string(),
            ]
        );
    }
}

#[cfg(test)]
mod naming_the_missing_field {
    use super::*;
    use crate::pursue::Decided;

    fn handed_back_on(field: Option<&str>) -> Outcome {
        let mut outcome = Outcome {
            status: Status::NeedsInput(
                Handback::NoFieldNamed,
                "this step needs text typed into a field".to_string(),
            ),
            offered: 12,
            omitted: 0,
            retired: 0,
            decisions: 2,
            batches: 1,
            input_tokens: 2_000,
            trail: Vec::new(),
            decided: Vec::new(),
            url_before: "https://example.test/".to_string(),
            url_after: "https://example.test/".to_string(),
            title: "A page".to_string(),
            goal_met: None,
            unlabelled_images: 0,
            ended_on: None,
        };
        outcome.decided = vec![Decided {
            operation: "ACT",
            how: Some("FILL"),
            target: Some("e17".to_string()),
            target_name: field.map(str::to_string),
            verdict: "handed_back",
            weakest: None,
            margin: None,
            needed: None,
            share: None,
            why: None,
            batch: 1,
            input_tokens: 1_000,
        }];
        outcome
    }

    /// A caller told only that something was missing has to guess which field.
    #[test]
    fn the_advice_names_the_field_to_supply() {
        let told = tell(&handed_back_on(Some("Where to? ")));
        let advice = told.try_next.expect("a hand-back carries advice");
        assert!(advice.contains("Where to?"), "{advice}");
    }

    #[test]
    fn an_unnamed_field_still_gets_the_general_advice() {
        let told = tell(&handed_back_on(None));
        let advice = told.try_next.expect("a hand-back carries advice");
        assert!(advice.contains("value"), "{advice}");
    }
}
