//! Reads one response into a decision, and decides whether to act on it.
//!
//! The answer says what; the confidence says whether to act. An earlier
//! implementation recorded both numbers on every step and read neither, which
//! is why a low-confidence guess was indistinguishable from a sure thing.

use crate::client::Response;
use crate::questions::{
    ACT_HOW, DESTINATION, DIRECTION, FILL_VALUE, GOAL_MET, OPERATION, PRESS_KEY, SELECT_VALUE,
    STATE_WHAT, TAB, TARGET,
};
use crate::space::{ActionSpace, NONE_OF_THESE};
use crate::vocabulary::{Consequence, Direction, How, Operation};

/// What the model chose, with the confidence behind each part.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub operation: Operation,
    pub operation_confidence: f64,
    /// How to act, for [`Operation::Act`].
    pub how: Option<How>,
    pub how_confidence: Option<f64>,
    /// The chosen control, or the destination key, or the tab id, depending on
    /// the operation. The loop resolves it against what it offered.
    pub target: Option<String>,
    pub target_confidence: Option<f64>,
    /// The value, for a select, or for a fill the model matched to one of the
    /// caller's values.
    pub value: Option<String>,
    /// How sure the model was about the value, when it chose one.
    ///
    /// `None` when the value did not come from a judgement: a select carries its
    /// control and value as one option, so the target confidence already covers
    /// it, and a value the caller supplied by name was never judged.
    pub value_confidence: Option<f64>,
    /// The key, for a press.
    pub key: Option<String>,
    pub key_confidence: Option<f64>,
    /// Which way to scroll.
    pub direction: Option<Direction>,
    /// Which state a check asks about.
    pub state: Option<String>,
    /// True when the model said none of the offered options will do.
    pub none_of_these: bool,
    /// How likely the model thought the goal was already satisfied, as a
    /// probability rather than a confidence: a Noul near 0.5 means uncertain.
    ///
    /// Carried for the caller and read by nothing in the loop. A completion
    /// claim is not evidence, and this one is weaker than the DONE operation it
    /// sits beside, because DONE is chosen against competing alternatives and
    /// this has none to lose to.
    pub goal_met: Option<f64>,
    pub input_tokens: u64,
}

/// Which judgement of an answer was the least certain.
///
/// Named rather than only counted, because "it needed 0.75 and had 0.31" does
/// not say what to do and "the key was the weak part" does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weakest {
    Operation,
    How,
    Target,
    Key,
    Value,
}

impl Weakest {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Operation => "which move to make",
            Self::How => "how to act on it",
            Self::Target => "which control to use it on",
            Self::Key => "which key to press",
            Self::Value => "which value to type",
        }
    }
}

impl Decision {
    /// A scroll the code chose rather than the model.
    ///
    /// Used when a decision was declined for want of confidence and the page has
    /// not moved: asking the identical question against the identical state
    /// would return the identical answer, so the honest response is to change
    /// what the next decision can see.
    #[must_use]
    pub fn scroll_down() -> Self {
        Self {
            operation: Operation::Act,
            // Not a judgement, so it carries no uncertainty for the gate to
            // weigh. The gate is not consulted for it either.
            operation_confidence: 1.0,
            how: Some(How::Scroll),
            how_confidence: None,
            target: None,
            target_confidence: None,
            value: None,
            value_confidence: None,
            key: None,
            key_confidence: None,
            direction: Some(Direction::Down),
            state: None,
            none_of_these: false,
            goal_met: None,
            input_tokens: 0,
        }
    }

    /// Whether this decision aims at a control in the snapshot.
    #[must_use]
    pub fn aims_at_a_control(&self) -> bool {
        match self.operation {
            Operation::Act => self.how.is_some_and(How::needs_control),
            Operation::CheckState => true,
            _ => false,
        }
    }

    /// Whether this decision needs something chosen alongside it.
    #[must_use]
    pub fn needs_target(&self) -> bool {
        self.aims_at_a_control()
            || matches!(self.operation, Operation::Navigate | Operation::Tabs)
    }

    /// The confidence the gate judges: the least certain judgement the execution
    /// depends on.
    ///
    /// The minimum rather than a product or an average, following the published
    /// pattern, which reports "the least certain judgement in the call" because
    /// one wrong argument is enough to spoil the result, and argues against a
    /// product on the grounds that it answers a different question and falls as
    /// a call takes more arguments whether or not any judgement is shaky.
    #[must_use]
    pub fn confidence(&self) -> f64 {
        self.weakest().1
    }

    /// The least certain judgement and its confidence.
    ///
    /// A Choice always carries a confidence and only a Noul does not, so a
    /// Choice answer that arrives without one is an anomaly rather than a
    /// permission: it counts as zero, which is the safe direction.
    #[must_use]
    pub fn weakest(&self) -> (Weakest, f64) {
        let mut weakest = (Weakest::Operation, self.operation_confidence);
        let mut consider = |which: Weakest, confidence: f64| {
            if confidence < weakest.1 {
                weakest = (which, confidence);
            }
        };

        // How to act is a judgement the execution depends on exactly as much as
        // the move is: ticking a box the goal wanted unticked is as wrong as
        // picking the wrong box.
        if self.operation == Operation::Act && self.how_confidence.is_some() {
            consider(Weakest::How, self.how_confidence.unwrap_or(0.0));
        }
        if self.needs_target() {
            consider(Weakest::Target, self.target_confidence.unwrap_or(0.0));
        }
        if self.how == Some(How::Press) {
            consider(Weakest::Key, self.key_confidence.unwrap_or(0.0));
        }
        // A value the model matched to a field is an argument the execution
        // depends on exactly as much as the field is. Typing the quantity into
        // the pincode box is as wrong as typing into the wrong box.
        if let Some(value) = self.value_confidence {
            consider(Weakest::Value, value);
        }
        weakest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// A question we asked came back missing or unreadable.
    Missing(String),
    /// The move named is not one we offered.
    UnknownOperation(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(id) => write!(formatter, "the answer to {id} was missing"),
            Self::UnknownOperation(name) => write!(formatter, "unknown move {name}"),
        }
    }
}

fn chosen<'a>(response: &'a Response, head: &str) -> Option<(&'a str, Option<f64>)> {
    let answer = response.answers.get(head)?;
    Some((answer.choice.as_deref()?, answer.confidence))
}

/// Reads the response into a decision, taking only the argument heads that
/// belong to the chosen move.
pub fn read(response: &Response) -> Result<Decision, ReadError> {
    let answer = response
        .answers
        .get(OPERATION)
        .ok_or_else(|| ReadError::Missing(OPERATION.to_string()))?;
    let named = answer
        .choice
        .as_deref()
        .ok_or_else(|| ReadError::Missing(OPERATION.to_string()))?;
    let operation =
        Operation::parse(named).ok_or_else(|| ReadError::UnknownOperation(named.to_string()))?;

    let mut decision = Decision {
        operation,
        operation_confidence: answer.confidence.unwrap_or(0.0),
        how: None,
        how_confidence: None,
        target: None,
        target_confidence: None,
        value: None,
        value_confidence: None,
        key: None,
        key_confidence: None,
        direction: None,
        state: None,
        none_of_these: false,
        goal_met: response
            .answers
            .get(GOAL_MET)
            .and_then(|answer| answer.noul),
        input_tokens: response.usage.input_tokens,
    };

    if operation == Operation::Act {
        // A page with one way of acting has no how question in the request,
        // because a one-option Choice is not a judgement.
        if let Some((named, confidence)) = chosen(response, ACT_HOW) {
            decision.how = How::parse(named);
            decision.how_confidence = confidence;
        }
        if decision.how == Some(How::Scroll) {
            decision.direction = chosen(response, DIRECTION)
                .and_then(|(named, _)| Direction::parse(named))
                .or(Some(Direction::Down));
            return Ok(decision);
        }
        // A select carries its control and its value as one option, because a
        // question cannot read another question's answer and so cannot be told
        // which control was chosen.
        if decision.how == Some(How::Select) {
            let answer = response
                .answers
                .get(SELECT_VALUE)
                .ok_or_else(|| ReadError::Missing(SELECT_VALUE.to_string()))?;
            let picked = answer.choice.as_deref().unwrap_or(NONE_OF_THESE);
            decision.target_confidence = answer.confidence;
            if picked == NONE_OF_THESE {
                decision.none_of_these = true;
            } else if let Some((reference, value)) = ActionSpace::split_select(picked) {
                decision.target = Some(reference.to_string());
                decision.value = Some(value.to_string());
            } else {
                return Err(ReadError::Missing(SELECT_VALUE.to_string()));
            }
            return Ok(decision);
        }
    }

    let head = match operation {
        Operation::Act | Operation::CheckState => Some(TARGET),
        // The destination key, not a control reference. The loop resolves it
        // against what the caller authored.
        Operation::Navigate => Some(DESTINATION),
        Operation::Tabs => Some(TAB),
        _ => None,
    };
    if let Some(head) = head {
        let answer = response
            .answers
            .get(head)
            .ok_or_else(|| ReadError::Missing(head.to_string()))?;
        let picked = answer.choice.as_deref().unwrap_or(NONE_OF_THESE);
        decision.target_confidence = answer.confidence;
        if picked == NONE_OF_THESE {
            decision.none_of_these = true;
        } else {
            decision.target = Some(picked.to_string());
        }
    }

    // Only read for a fill, and only when the head was asked at all: a run with
    // no values offered has no value question in the request.
    if decision.how == Some(How::Fill)
        && let Some((named, confidence)) = chosen(response, FILL_VALUE)
        && named != NONE_OF_THESE
    {
        decision.value = Some(named.to_string());
        decision.value_confidence = confidence;
    }

    if decision.how == Some(How::Press)
        && let Some((named, confidence)) = chosen(response, PRESS_KEY)
    {
        decision.key = Some(named.to_string());
        decision.key_confidence = confidence;
    }

    if operation == Operation::CheckState {
        decision.state = chosen(response, STATE_WHAT).map(|(named, _)| named.to_string());
    }

    Ok(decision)
}

/// Why a decision went back to the caller.
///
/// Named rather than left as prose, because the caller branches on structured
/// content, and a status of `needs_input` with nothing beside it tells it only
/// that something went wrong. Measured on a live run, that is exactly what a
/// caller got twice in one errand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handback {
    /// None of the options offered for this step can advance the goal.
    NothingOffered,
    /// A consequential action was short of the bar its consequence sets, and
    /// looking at more of the page does not make a submit any safer.
    TooRiskyToGuess,
    /// It chose to type but named no field, so there is no question to ask.
    NoFieldNamed,
    /// It asked for text until the limit and the field kept changing.
    AskedTooOften,
    /// A completion claim could not be checked, because the page could not be
    /// read back.
    Unverifiable,
}

impl Handback {
    /// A short word a caller can branch on.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NothingOffered => "nothing_offered",
            Self::TooRiskyToGuess => "too_risky_to_guess",
            Self::NoFieldNamed => "no_field_named",
            Self::AskedTooOften => "asked_too_often",
            Self::Unverifiable => "unverifiable",
        }
    }
}

/// What code should do about a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Run it.
    Execute,
    /// Look at more of the page, then decide again.
    Explore(String),
    /// Return to the caller, saying what is needed and why.
    HandBack(Handback, String),
}

/// Judges a decision against the bar its consequence sets.
///
/// An ending is not judged. The published pattern gates whether to **act**, and
/// a completion claim is a report rather than an action; the reference
/// implementation re-validates the page instead and states plainly that such a
/// claim is never independent evidence. So the confidence behind one travels out
/// to the caller, which is the documented "ask for review" branch.
///
/// A report **is** judged, at the low bar. Handing back a page's text changes
/// nothing the caller could regret, but it ends the run, so acting on a hunch
/// burns the whole errand and returns a blob instead of progress. The measured
/// case for gating is the same one that argues for the widened vocabulary: the
/// wrong answers it gives arrive at around 0.29 confidence, and that is only
/// worth anything if something is reading the number.
///
/// The bar rises with what it costs to be wrong, which is the published
/// guidance: a scroll and a submit do not deserve the same threshold.
///
/// `text` is what the caller has already supplied for the chosen control, if
/// anything. With text in hand a fill is judged like any other move rather than
/// refused: it is the one move whose argument comes from outside, and refusing
/// it even when the argument has arrived is what made the hand-back a dead end.
#[must_use]
pub fn verdict(decision: &Decision, target_name: Option<&str>, text: Option<&str>) -> Verdict {
    // An ending touches nothing, so there is nothing to gate.
    if decision.operation.is_ending() {
        return Verdict::Execute;
    }
    if decision.none_of_these {
        return Verdict::HandBack(
            Handback::NothingOffered,
            "none of the options offered for this step can advance the goal".to_string(),
        );
    }
    if decision.how == Some(How::Fill) && text.is_none() && decision.value.is_none() {
        return Verdict::HandBack(
            Handback::NoFieldNamed,
            "this step needs text typed into a field, which the caller supplies".to_string(),
        );
    }
    if decision.needs_target() && decision.target.is_none() {
        return Verdict::Explore("no control was chosen for a move that needs one".to_string());
    }

    let consequence = decision.operation.consequence(decision.how, target_name);
    let (weakest, confidence) = decision.weakest();
    if confidence >= consequence.bar() {
        return Verdict::Execute;
    }
    // Below the floor nothing acts, and a high-stakes action short of its bar is
    // handed back rather than explored: looking at more of the page does not
    // make a submit any safer.
    if consequence == Consequence::High {
        return Verdict::HandBack(
            Handback::TooRiskyToGuess,
            format!(
                "{} on {:?} needs confidence {:.2} and had {confidence:.2}; the least certain \
                 judgement was {}",
                decision.operation.as_str(),
                target_name.unwrap_or("an unnamed control"),
                consequence.bar(),
                weakest.as_str()
            ),
        );
    }
    Verdict::Explore(format!(
        "{} had confidence {confidence:.2}, under the {:.2} this needs; the least certain \
         judgement was {}",
        decision.operation.as_str(),
        consequence.bar(),
        weakest.as_str()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Answer, Usage};
    use std::collections::BTreeMap;

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

    fn response(answers: &[(&str, Answer)]) -> Response {
        Response {
            model: "jev-1.13.0".to_string(),
            answers: answers
                .iter()
                .map(|(id, answer)| ((*id).to_string(), answer.clone()))
                .collect::<BTreeMap<_, _>>(),
            usage: Usage {
                input_tokens: 100,
                output_tokens: 0,
            },
        }
    }

    fn clicking(target_confidence: f64) -> Decision {
        read(&response(&[
            (OPERATION, answer("ACT", 0.95)),
            (ACT_HOW, answer("CLICK", 0.95)),
            (TARGET, answer("e1", target_confidence)),
        ]))
        .expect("a well formed response reads")
    }

    #[test]
    fn a_click_reads_its_how_and_its_target() {
        let decision = clicking(0.9);
        assert_eq!(decision.operation, Operation::Act);
        assert_eq!(decision.how, Some(How::Click));
        assert_eq!(decision.target.as_deref(), Some("e1"));
    }

    #[test]
    fn a_read_takes_no_arguments_at_all() {
        let decision = read(&response(&[(OPERATION, answer("READ", 0.8))])).expect("reads");
        assert_eq!(decision.operation, Operation::Read);
        assert!(decision.target.is_none());
        assert!(!decision.needs_target());
    }

    /// The whole point of the gate: the weakest judgement decides, not the one
    /// that happens to be reported first.
    #[test]
    fn the_least_certain_judgement_is_the_one_that_counts() {
        let decision = clicking(0.31);
        let (weakest, confidence) = decision.weakest();
        assert_eq!(weakest, Weakest::Target);
        assert!((confidence - 0.31).abs() < f64::EPSILON);
    }

    /// Enter and Escape do opposite things, so a certain press with an unsure
    /// key is not a certain decision.
    #[test]
    fn an_unsure_key_makes_the_whole_decision_unsure() {
        let decision = read(&response(&[
            (OPERATION, answer("ACT", 0.99)),
            (ACT_HOW, answer("PRESS", 0.99)),
            (TARGET, answer("e1", 0.99)),
            (PRESS_KEY, answer("Escape", 0.40)),
        ]))
        .expect("reads");
        assert_eq!(decision.weakest().0, Weakest::Key);
    }

    #[test]
    fn a_sure_click_on_a_harmless_control_runs() {
        assert_eq!(
            verdict(&clicking(0.9), Some("Next page"), None),
            Verdict::Execute
        );
    }

    /// Looking at more of the page does not make a submit any safer.
    #[test]
    fn an_unsure_click_on_something_expensive_is_handed_back_not_explored() {
        let Verdict::HandBack(why, _) = verdict(&clicking(0.70), Some("Place order"), None) else {
            panic!("a high stakes action short of its bar must not be explored");
        };
        assert_eq!(why, Handback::TooRiskyToGuess);
    }

    /// The measured regression that made a run refuse every Sony listing on a
    /// search page.
    #[test]
    fn noise_cancellation_does_not_raise_the_bar() {
        let listing = "SONY WH-1000XM5 Wireless Noise Cancellation Headphones";
        assert_eq!(verdict(&clicking(0.80), Some(listing), None), Verdict::Execute);
    }

    #[test]
    fn saying_none_of_these_hands_back_rather_than_guessing() {
        let decision = read(&response(&[
            (OPERATION, answer("ACT", 0.9)),
            (ACT_HOW, answer("CLICK", 0.9)),
            (TARGET, answer(NONE_OF_THESE, 0.9)),
        ]))
        .expect("reads");
        assert!(decision.none_of_these);
        let Verdict::HandBack(why, _) = verdict(&decision, None, None) else {
            panic!("there is nothing to act on");
        };
        assert_eq!(why, Handback::NothingOffered);
    }

    /// A report ends the run, so a hunch is expensive: the caller gets a blob
    /// of text instead of progress and there is no second chance. Gated at the
    /// low bar rather than waved through.
    #[test]
    fn a_report_on_a_hunch_is_explored_rather_than_run() {
        let decision = read(&response(&[(OPERATION, answer("READ", 0.20))])).expect("reads");
        let Verdict::Explore(_) = verdict(&decision, None, None) else {
            panic!("0.20 is under the 0.60 floor, and reading ends the run");
        };
    }

    #[test]
    fn a_confident_report_runs() {
        let decision = read(&response(&[(OPERATION, answer("READ", 0.88))])).expect("reads");
        assert_eq!(verdict(&decision, None, None), Verdict::Execute);
    }

    /// A completion claim is the caller's to judge, not the gate's.
    #[test]
    fn an_ending_is_never_refused() {
        for named in ["DONE", "BLOCKED"] {
            let decision = read(&response(&[(OPERATION, answer(named, 0.11))])).expect("reads");
            assert_eq!(verdict(&decision, None, None), Verdict::Execute, "{named}");
        }
    }

    #[test]
    fn leaving_the_page_is_judged_at_the_high_bar() {
        let decision = read(&response(&[
            (OPERATION, answer("NAVIGATE", 0.80)),
            (DESTINATION, answer("the results page", 0.80)),
        ]))
        .expect("reads");
        let Verdict::HandBack(why, _) = verdict(&decision, Some("the results page"), None) else {
            panic!("0.80 is under the 0.85 that leaving the page needs");
        };
        assert_eq!(why, Handback::TooRiskyToGuess);
    }

    #[test]
    fn a_scroll_the_code_chose_is_not_judged_at_all() {
        let decision = Decision::scroll_down();
        assert_eq!(decision.how, Some(How::Scroll));
        assert_eq!(verdict(&decision, None, None), Verdict::Execute);
    }

    #[test]
    fn an_unknown_move_is_an_error_rather_than_a_guess() {
        let got = read(&response(&[(OPERATION, answer("TELEPORT", 0.99))]));
        assert_eq!(
            got.unwrap_err(),
            ReadError::UnknownOperation("TELEPORT".to_string())
        );
    }

    #[test]
    fn a_state_check_reads_which_state_it_was_asked_about() {
        let decision = read(&response(&[
            (OPERATION, answer("CHECK_STATE", 0.9)),
            (TARGET, answer("e4", 0.9)),
            (STATE_WHAT, answer("checked", 0.9)),
        ]))
        .expect("reads");
        assert_eq!(decision.state.as_deref(), Some("checked"));
        assert_eq!(decision.target.as_deref(), Some("e4"));
    }
}
