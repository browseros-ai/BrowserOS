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

use crate::drive::Answer;
use crate::pursue::{Outcome, Status};
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
    /// The page's text, budgeted.
    pub text: String,
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
        text: view.text.clone(),
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
    /// What a report handed back, when one ended the run.
    ///
    /// The text is raw, not parsed. A model that cannot write a string can
    /// never return "312 points", so pulling the answer out of the page is the
    /// caller's job, and pretending otherwise here would just move the lie.
    pub answer: Option<Answer>,
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

    Told {
        status: outcome.status.as_str().to_string(),
        said: said(outcome, actions, changed),
        answer: outcome.answer.clone(),
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
        decided: outcome.decided.clone(),
    }
}

/// One sentence, which has to be true of what happened.
fn said(outcome: &Outcome, actions: usize, changed: usize) -> String {
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

    match &outcome.status {
        Status::Satisfied => format!(
            "{did}{moved}, and then said every requirement of the goal was visible on the page."
        ),
        // A report ends a run, it does not describe it. This arm used to assert
        // that nothing on the page had been changed, which was a claim about
        // the whole run made by the sentence for its last step. Measured live:
        // a run added an item to a cart and then reported, and told the caller
        // in the same object as `changed: 1` that it had changed nothing. The
        // caller read that and concluded the errand had failed.
        Status::Answered => {
            let what = outcome
                .answer
                .as_ref()
                .map_or("something", |answer| answer.asked.as_str());
            format!(
                "{did}{moved}, then answered the goal by choosing to {what} the page. The result \
                 is attached."
            )
        }
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
    }
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
    use crate::drive::Answer;
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
            answer: None,
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

    /// Renamed from an_answered_run_says_nothing_was_changed, which asserted
    /// the sentence said that unconditionally. It is only true when the run
    /// took no action, and a run that acted and then reported was telling the
    /// caller it had done nothing.
    #[test]
    fn an_answered_run_hands_the_text_back_raw() {
        let mut got = outcome(Status::Answered, vec![]);
        got.answer = Some(Answer {
            asked: "read".to_string(),
            text: Some("312 points by someone".to_string()),
            pictured: false,
        });
        let told = tell(&got);

        assert_eq!(told.status, "answered");
        assert!(
            told.said.starts_with("It took no action"),
            "this run genuinely took none, got {:?}",
            told.said
        );
        assert_eq!(
            told.answer
                .as_ref()
                .and_then(|answer| answer.text.as_deref()),
            Some("312 points by someone"),
            "the text travels raw; pulling the number out is the caller's job"
        );
        assert!(!told.wants_the_caller(), "the run did its job");
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

    /// Measured live: a run clicked an add-to-cart button and then reported,
    /// and the sentence said it had changed nothing in the same object that
    /// said it had changed one thing. The caller read the sentence.
    #[test]
    fn a_report_after_an_action_does_not_claim_the_run_changed_nothing() {
        let mut got = outcome(Status::Answered, vec![step(true), step(false)]);
        got.answer = Some(Answer {
            asked: "grep".to_string(),
            text: Some("two matching lines".to_string()),
            pictured: false,
        });

        let told = tell(&got);

        assert_eq!(told.actions, 2);
        assert_eq!(told.changed, 1);
        assert!(
            !told.said.contains("Nothing on the page was changed"),
            "got {:?}",
            told.said
        );
        assert!(
            told.said
                .starts_with("It took 2 actions, 1 of which changed the page"),
            "the sentence counts what the run did, got {:?}",
            told.said
        );
        assert!(
            told.said.contains("choosing to grep"),
            "got {:?}",
            told.said
        );
    }

    /// A report that was the whole run still reads as one.
    #[test]
    fn a_report_with_no_actions_before_it_says_so() {
        let mut got = outcome(Status::Answered, vec![]);
        got.answer = Some(Answer {
            asked: "read".to_string(),
            text: Some("the page".to_string()),
            pictured: false,
        });

        let told = tell(&got);

        assert!(
            told.said
                .starts_with("It took no action, then answered the goal"),
            "got {:?}",
            told.said
        );
    }
}
