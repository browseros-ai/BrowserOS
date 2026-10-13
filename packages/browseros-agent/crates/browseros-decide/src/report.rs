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
        offered: outcome.offered,
        omitted: outcome.omitted,
        retired: outcome.retired,
        unlabelled_images: outcome.unlabelled_images,
        goal_met: outcome.goal_met,
        url: outcome.url_after.clone(),
        title: outcome.title.clone(),
        input_tokens: outcome.input_tokens,
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
        Status::Answered => {
            let what = outcome
                .answer
                .as_ref()
                .map_or("something", |answer| answer.asked.as_str());
            format!(
                "It answered the goal by choosing to {what} the page, and the result is attached. \
                 Nothing on the page was changed."
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
            input_tokens: 2_000,
            trail,
            url_before: "https://example.test/".to_string(),
            url_after: "https://example.test/after".to_string(),
            title: "A page".to_string(),
            answer: None,
            goal_met: Some(0.81),
            unlabelled_images: 3,
        }
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
        assert!(told.said.contains("1 of which changed the page"), "{}", told.said);
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

    #[test]
    fn an_answered_run_says_nothing_was_changed() {
        let mut got = outcome(Status::Answered, vec![]);
        got.answer = Some(Answer {
            asked: "read".to_string(),
            text: Some("312 points by someone".to_string()),
            pictured: false,
        });
        let told = tell(&got);

        assert_eq!(told.status, "answered");
        assert!(told.said.contains("Nothing on the page was changed"));
        assert_eq!(
            told.answer.as_ref().and_then(|answer| answer.text.as_deref()),
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

    #[test]
    fn a_run_that_did_nothing_does_not_claim_it_did() {
        let told = tell(&outcome(Status::BudgetSpent, vec![]));
        assert!(told.said.starts_with("It took no action"), "{}", told.said);
    }
}
