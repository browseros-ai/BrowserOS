//! Reads one response into a decision, and decides whether to act on it.
//!
//! The answer says what; how far it led the runner-up says whether to act on
//! it. An absolute share cannot carry that judgement, because it is not
//! comparable across questions of different sizes: spread over 254 options,
//! 0.63 against a runner-up at 0.11 is a landslide, and an absolute bar of 0.75
//! refuses it. That was measured, three times in one run.

use crate::client::Response;
use crate::questions::{
    ACT_HOW, DESTINATION, DIRECTION, FILL_VALUE, GOAL_MET, OPERATION, PRESS_KEY, SELECT_VALUE, TAB,
    TARGET,
};
use crate::space::{ActionSpace, NONE_OF_THESE};
use crate::vocabulary::{Consequence, Direction, How, Operation};

/// No share below this acts, whatever its margin.
///
/// A margin alone can be satisfied by two bad options: a leader at 0.20 over a
/// runner-up at 0.02 leads by 0.18 and knows nothing. The floor is what stops a
/// wide margin over nothing from reading as confidence.
///
/// It binds only where the required margin is under it, which is the low bar
/// alone. A margin is a share minus something non-negative, so it can never
/// exceed its own share, and a judgement clearing a 0.30 margin has cleared
/// 0.25 of share on the way. That is not a reason to drop the floor: it is the
/// reason a report, which is gated at 0.15, is the one move that needs it.
pub const FLOOR: f64 = 0.25;

/// How one judgement stood.
///
/// The two numbers travel together because neither is worth anything alone. The
/// margin says whether the answer beat its alternatives; the share says whether
/// it believed its own answer.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Lead {
    /// How far the chosen option led the runner-up.
    ///
    /// Equal to `share` when nothing was reported to compare against, which
    /// `distributed` is how to tell apart.
    pub margin: f64,
    /// What share the chosen option held.
    pub share: f64,
    /// Whether a distribution was reported at all.
    ///
    /// Without one the margin is not a margin, and judging it as though it were
    /// would make a missing signal read as a landslide.
    pub distributed: bool,
    /// How many options the question offered.
    ///
    /// Needed because the published statistic is relative to an even split, and
    /// an even split depends on how many ways the mass could have gone.
    pub options: usize,
}

impl Lead {
    /// Whether this judgement is sure enough for an action of this consequence.
    ///
    /// Two rules rather than one, because they answer to different evidence. A
    /// judgement that reported a distribution is judged on how far it led, with
    /// a floor so a wide lead between two bad options does not pass. One that
    /// reported none is judged on its share against the absolute bar, which is
    /// what the gate used before there was anything better.
    #[must_use]
    pub fn clears(self, consequence: Consequence) -> bool {
        if self.distributed {
            self.confidence() >= consequence.clarity()
        } else {
            self.share >= consequence.share_bar()
        }
    }

    /// The published statistic: how far the top sits above an even split.
    ///
    /// `(p_max - 1/n) / (1 - 1/n)`. A margin takes no account of how many
    /// options were offered, so the same 0.15 means a coin flip between two and
    /// a strong read among two hundred. This is the comparison the
    /// documentation reports, and it is what answers the objection this
    /// module's own header raises against an absolute share.
    #[must_use]
    pub fn confidence(self) -> f64 {
        if self.options < 2 {
            return self.share;
        }
        let even = 1.0 / self.options as f64;
        ((self.share - even) / (1.0 - even)).clamp(0.0, 1.0)
    }
}

/// How far the chosen option led the runner-up, and what share it held.
///
/// A question that reported no distribution falls back to its share as both,
/// which makes it behave exactly as the absolute gate did. That is the honest
/// default: there is nothing to take a margin against.
#[must_use]
pub fn lead(response: &Response, head: &str) -> Lead {
    let Some(answer) = response.answers.get(head) else {
        return Lead::default();
    };
    let share = answer.confidence.unwrap_or(0.0);
    let undistributed = Lead {
        margin: share,
        share,
        distributed: false,
        options: 0,
    };
    let Some(probabilities) = &answer.probabilities else {
        return undistributed;
    };
    let mut ranked: Vec<f64> = probabilities.values().copied().collect();
    ranked.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    match ranked.as_slice() {
        // A distribution with nothing in it is no distribution.
        [] => undistributed,
        // One option is not a judgement: there was nothing it could have lost
        // to, so its share is all there is to go on.
        [only] => Lead {
            margin: *only,
            share: *only,
            distributed: false,
            options: 1,
        },
        [top, second, ..] => Lead {
            margin: top - second,
            share: *top,
            distributed: true,
            options: ranked.len(),
        },
    }
}

/// What the model chose, with how each part of it stood.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub operation: Operation,
    pub operation_lead: Lead,
    /// How to act, for [`Operation::Act`].
    pub how: Option<How>,
    pub how_lead: Option<Lead>,
    /// The chosen control, or the destination key, or the tab id, depending on
    /// the operation. The loop resolves it against what it offered.
    pub target: Option<String>,
    pub target_lead: Option<Lead>,
    /// The value, for a select, or for a fill the model matched to one of the
    /// caller's values.
    pub value: Option<String>,
    /// How the value judgement stood, when there was one.
    ///
    /// `None` when the value did not come from a judgement: a select carries its
    /// control and value as one option, so the target lead already covers it,
    /// and a value the caller supplied by name was never judged.
    pub value_lead: Option<Lead>,
    /// The key, for a press.
    pub key: Option<String>,
    pub key_lead: Option<Lead>,
    /// Which way to scroll.
    pub direction: Option<Direction>,
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
/// Named rather than only counted, because "it needed 0.30 and led by 0.04"
/// does not say what to do and "the key was the weak part" does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weakest {
    Operation,
    How,
    Target,
    Key,
    Value,
    /// Whether the move was worth making at all.
    Worth,
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
            Self::Worth => "whether this move was worth making",
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
            operation_lead: Lead {
                margin: 1.0,
                share: 1.0,
                distributed: true,
                options: 2,
            },
            how: Some(How::Scroll),
            how_lead: None,
            target: None,
            target_lead: None,
            value: None,
            value_lead: None,
            key: None,
            key_lead: None,
            direction: Some(Direction::Down),
            none_of_these: false,
            goal_met: None,
            input_tokens: 0,
        }
    }

    /// Whether this decision aims at a control in the snapshot.
    #[must_use]
    pub fn aims_at_a_control(&self) -> bool {
        self.operation == Operation::Act && self.how.is_some_and(How::needs_control)
    }

    /// Whether this decision needs something chosen alongside it.
    #[must_use]
    pub fn needs_target(&self) -> bool {
        self.aims_at_a_control() || matches!(self.operation, Operation::Navigate | Operation::Tabs)
    }

    /// Every judgement the execution of this decision depends on.
    ///
    /// The gate takes the weakest of these rather than a product or an average,
    /// following the published pattern: one wrong argument is enough to spoil
    /// the result, and a product answers a different question and falls as a
    /// call takes more arguments whether or not any judgement is shaky.
    fn judgements(&self) -> Vec<(Weakest, Lead)> {
        let mut judgements = vec![(Weakest::Operation, self.operation_lead)];
        // How to act is a judgement the execution depends on exactly as much as
        // the move is: ticking a box the goal wanted unticked is as wrong as
        // picking the wrong box.
        if self.operation == Operation::Act && self.how_lead.is_some() {
            judgements.push((Weakest::How, self.how_lead.unwrap_or_default()));
        }
        // The target's lead is still measured and still reported. For a
        // control it is no longer what decides, because a lead cannot answer
        // the question that matters on a page where several controls each
        // serve the goal: two good options agreeing and two bad ones tying
        // look identical to it. `worth_acting_on` answers it instead.
        //
        // A destination key and a tab id are not controls, so no approval can
        // be asked about them: nothing on the page describes them. They keep
        // the lead check, or a confident decision to navigate would follow a
        // barely chosen destination with nothing weighing it at all.
        if self.needs_target() && !self.aims_at_a_control() {
            judgements.push((Weakest::Target, self.target_lead.unwrap_or_default()));
        }
        if self.how == Some(How::Press) {
            judgements.push((Weakest::Key, self.key_lead.unwrap_or_default()));
        }
        // A value the model matched to a field is an argument the execution
        // depends on exactly as much as the field is. Typing the quantity into
        // the pincode box is as wrong as typing into the wrong box.
        if let Some(value) = self.value_lead {
            judgements.push((Weakest::Value, value));
        }
        judgements
    }

    /// The judgement that led its runner-up by the least, and by how much.
    ///
    /// A Choice always reports a distribution and only a Noul does not, so a
    /// Choice answer that arrives without one counts as zero, which is the safe
    /// direction.
    #[must_use]
    pub fn weakest(&self) -> (Weakest, f64) {
        self.judgements()
            .into_iter()
            .map(|(which, lead)| (which, lead.margin))
            .min_by(|left, right| {
                left.1
                    .partial_cmp(&right.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or((Weakest::Operation, 0.0))
    }

    /// The first judgement that is not sure enough for this consequence.
    ///
    /// `None` means every one of them cleared, which is the only case that
    /// executes. Returned rather than a bare boolean so the caller can say
    /// which part was weak, because "it needed 0.30 and led by 0.04" does not
    /// say what to do and "the key was the weak part" does.
    #[must_use]
    pub fn short_of(&self, consequence: Consequence) -> Option<(Weakest, Lead)> {
        self.judgements()
            .into_iter()
            .find(|(_, lead)| !lead.clears(consequence))
    }

    /// The smallest share any judgement this decision depends on held.
    ///
    /// Judged against [`FLOOR`] separately from the margin, because a wide
    /// margin between two options that both know nothing is not confidence.
    #[must_use]
    pub fn thinnest(&self) -> f64 {
        self.judgements()
            .into_iter()
            .map(|(_, lead)| lead.share)
            .fold(f64::INFINITY, f64::min)
            .min(1.0)
    }

    /// The weakest share, for reporting rather than for gating.
    #[must_use]
    pub fn confidence(&self) -> f64 {
        self.thinnest()
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
        operation_lead: lead(response, OPERATION),
        how: None,
        how_lead: None,
        target: None,
        target_lead: None,
        value: None,
        value_lead: None,
        key: None,
        key_lead: None,
        direction: None,
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
        if let Some((named, _)) = chosen(response, ACT_HOW) {
            decision.how = How::parse(named);
            decision.how_lead = Some(lead(response, ACT_HOW));
        }
        if decision.how == Some(How::Scroll) {
            decision.direction = chosen(response, DIRECTION)
                .and_then(|(named, _)| Direction::parse(named))
                .or(Some(Direction::Down));
            return Ok(decision);
        }
        // A submit aims at nothing the model has to name: the field is the one
        // the last step typed into, which the loop knows. Returning before the
        // target head is read is what makes that true, because reading it is
        // what sets `none_of_these`, and an answer about a question this move
        // does not depend on must not be able to end the run.
        if decision.how == Some(How::Submit) {
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
            decision.target_lead = Some(lead(response, SELECT_VALUE));
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
        Operation::Act => Some(TARGET),
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
        decision.target_lead = Some(lead(response, head));
        if picked == NONE_OF_THESE {
            decision.none_of_these = true;
        } else {
            decision.target = Some(picked.to_string());
        }
    }

    // Only read for a fill, and only when the head was asked at all: a run with
    // no values offered has no value question in the request.
    if decision.how == Some(How::Fill)
        && let Some((named, _)) = chosen(response, FILL_VALUE)
        && named != NONE_OF_THESE
    {
        decision.value = Some(named.to_string());
        decision.value_lead = Some(lead(response, FILL_VALUE));
    }

    if decision.how == Some(How::Press)
        && let Some((named, _)) = chosen(response, PRESS_KEY)
    {
        decision.key = Some(named.to_string());
        decision.key_lead = Some(lead(response, PRESS_KEY));
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
/// guidance: a scroll and a submit do not deserve the same threshold. What
/// rises is the margin it must lead by, not the share it must hold, because a
/// share is not comparable between a question over eleven options and one over
/// two hundred and fifty four.
///
/// `text` is what the caller has already supplied for the chosen control, if
/// anything. With text in hand a fill is judged like any other move rather than
/// refused: it is the one move whose argument comes from outside, and refusing
/// it even when the argument has arrived is what made the hand-back a dead end.
#[must_use]
pub fn verdict(
    decision: &Decision,
    target_name: Option<&str>,
    text: Option<&str>,
    worth: Option<f64>,
) -> Verdict {
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
        // Name the field. Stopping here is right, because only the caller has
        // the text, but a caller told only that something was missing has to
        // guess which field to supply and may well supply the wrong one. Named,
        // it can run the errand again with one more value.
        return Verdict::HandBack(
            Handback::NoFieldNamed,
            target_name.map_or_else(
                || "this step needs text typed into a field, and no value was supplied".to_string(),
                |field| {
                    format!(
                        "this step needs text typed into {field:?}, and no value supplied matches                          it; pass one named for that field and run this again"
                    )
                },
            ),
        );
    }
    if decision.needs_target() && decision.target.is_none() {
        return Verdict::Explore("no control was chosen for a move that needs one".to_string());
    }

    let consequence = decision.operation.consequence(decision.how, target_name);

    // Whether this move is worth making, asked about the one control the run
    // settled on. A second control that would also have served cannot lower
    // this, which is the property the target's lead lacks.
    if decision.needs_target()
        && let Some(worth) = worth
        && worth < consequence.approval()
    {
        let said = format!(
            "{} on {} was judged worth doing at only {worth:.2}, under the {:.2} this needs",
            decision.operation.as_str(),
            target_name.unwrap_or("the chosen control"),
            consequence.approval(),
        );
        // Looking at more of the page does not make a submit any safer, which
        // is the rule this gate already held for a lead short of the high bar.
        return if consequence == Consequence::High {
            Verdict::HandBack(Handback::TooRiskyToGuess, said)
        } else {
            Verdict::Explore(said)
        };
    }

    let Some((which, lead)) = decision.short_of(consequence) else {
        return Verdict::Execute;
    };
    // What to say depends on which rule it failed, because a thin lead, an
    // answer nobody believes, and a missing distribution are three different
    // problems and only the first is worth looking at more of the page for.
    let said = if !lead.distributed {
        format!(
            "{} held {:.2} of the answer and reported nothing to compare it against, under the \
             {:.2} that needs; the least certain judgement was {}",
            decision.operation.as_str(),
            lead.share,
            consequence.share_bar(),
            which.as_str()
        )
    } else if lead.share < FLOOR {
        format!(
            "{} held only {:.2} of the answer, under the {FLOOR:.2} floor; the least certain \
             judgement was {}",
            decision.operation.as_str(),
            lead.share,
            which.as_str()
        )
    } else {
        format!(
            "{} led by {:.2}, under the {:.2} this needs; the least certain judgement was {}",
            decision.operation.as_str(),
            lead.margin,
            consequence.margin(),
            which.as_str()
        )
    };
    // A high-stakes action short of its bar is handed back rather than
    // explored: looking at more of the page does not make a submit any safer.
    if consequence == Consequence::High {
        return Verdict::HandBack(
            Handback::TooRiskyToGuess,
            format!(
                "{} on {:?} is not safe to guess at. {said}",
                decision.operation.as_str(),
                target_name.unwrap_or("an unnamed control")
            ),
        );
    }
    Verdict::Explore(said)
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

    /// An answer that reports its distribution, which is what a real Choice
    /// does and what the margin is taken from.
    fn spread(choice: &str, probabilities: &[(&str, f64)]) -> Answer {
        let share = probabilities
            .iter()
            .find(|(option, _)| *option == choice)
            .map(|(_, probability)| *probability)
            .unwrap_or(0.0);
        Answer {
            answer_type: "choice".to_string(),
            noul: None,
            choice: Some(choice.to_string()),
            score: None,
            probabilities: Some(
                probabilities
                    .iter()
                    .map(|(option, probability)| ((*option).to_string(), *probability))
                    .collect::<BTreeMap<_, _>>(),
            ),
            legend: None,
            confidence: Some(share),
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

    /// An ending takes no arguments at all, which is what makes it an ending.
    #[test]
    fn an_ending_takes_no_arguments_at_all() {
        let decision = read(&response(&[(OPERATION, answer("DONE", 0.8))])).expect("reads");
        assert_eq!(decision.operation, Operation::Done);
        assert!(decision.target.is_none());
        assert!(!decision.needs_target());
    }

    /// The whole point of the gate: the weakest judgement decides, not the one
    /// that happens to be reported first.
    #[test]
    fn the_least_certain_judgement_is_the_one_that_counts() {
        let decision = read(&response(&[
            (OPERATION, answer("ACT", 0.99)),
            (ACT_HOW, answer("CLICK", 0.31)),
            (TARGET, answer("e1", 0.99)),
        ]))
        .expect("reads");
        let (weakest, confidence) = decision.weakest();
        assert_eq!(weakest, Weakest::How);
        assert!((confidence - 0.31).abs() < f64::EPSILON);
    }

    /// The target's lead is still measured and reported, and no longer decides.
    /// A control chosen at 0.31 among many is not a reason to refuse: that is
    /// what `worth_acting_on` is asked instead.
    #[test]
    fn the_targets_lead_is_reported_and_does_not_gate() {
        let decision = clicking(0.31);
        assert!(decision.target_lead.is_some(), "still measured");
        assert_ne!(decision.weakest().0, Weakest::Target);
        assert_eq!(
            verdict(&decision, Some("Next page"), None, Some(0.82)),
            Verdict::Execute
        );
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
            verdict(&clicking(0.9), Some("Next page"), None, None),
            Verdict::Execute
        );
    }

    /// Looking at more of the page does not make a submit any safer.
    #[test]
    fn an_unsure_click_on_something_expensive_is_handed_back_not_explored() {
        // The operation itself is short of the high bar, which is what this
        // has always been about: the target's own lead no longer gates.
        let Verdict::HandBack(why, _) =
            verdict(&clicking(0.99), Some("Place order"), None, Some(0.70))
        else {
            panic!("a high stakes action short of its bar must not be explored");
        };
        assert_eq!(why, Handback::TooRiskyToGuess);
    }

    /// And the same control, with the move judged not worth making.
    #[test]
    fn a_move_judged_not_worth_making_is_explored_rather_than_run() {
        let Verdict::Explore(said) = verdict(&clicking(0.99), Some("Next page"), None, Some(0.31))
        else {
            panic!("a move under its approval must not run");
        };
        assert!(said.contains("0.31"), "{said}");
    }

    /// Stopping is right, because only the caller has the text. Stopping
    /// without saying which field is what makes the caller guess.
    #[test]
    fn a_fill_with_no_value_names_the_field_it_needed() {
        let filling = read(&response(&[
            (OPERATION, answer("ACT", 0.95)),
            (ACT_HOW, answer("FILL", 0.95)),
            (TARGET, answer("e17", 0.95)),
        ]))
        .expect("reads");
        let Verdict::HandBack(why, said) = verdict(&filling, Some("Where to? "), None, None) else {
            panic!("a fill with nothing to type must hand back");
        };
        assert_eq!(why, Handback::NoFieldNamed);
        assert!(said.contains("Where to?"), "{said}");
    }

    /// A destination is not a control, so no approval can be asked about it.
    /// Without its lead it would be gated by nothing at all.
    #[test]
    fn a_target_that_is_not_a_control_keeps_its_lead_check() {
        let going = read(&response(&[
            (
                OPERATION,
                spread("NAVIGATE", &[("NAVIGATE", 0.95), ("ACT", 0.01)]),
            ),
            (
                DESTINATION,
                spread("checkout", &[("checkout", 0.30), ("basket", 0.28)]),
            ),
        ]))
        .expect("reads");
        assert!(going.needs_target() && !going.aims_at_a_control());
        let judged = verdict(&going, None, None, None);
        assert!(
            matches!(judged, Verdict::Explore(_) | Verdict::HandBack(..)),
            "a barely chosen destination must not run: {judged:?}"
        );
    }

    /// And an approval that could not be asked is a refusal rather than a free
    /// pass, now that the target's lead no longer gates a control.
    #[test]
    fn an_unavailable_approval_does_not_let_a_move_through() {
        let refused = verdict(&clicking(0.99), Some("Place order"), None, Some(0.0));
        assert!(matches!(refused, Verdict::HandBack(..)), "{refused:?}");
    }

    /// An expensive control needs more than a coin flip to be worth doing.
    #[test]
    fn what_a_move_has_to_be_worth_rises_with_what_it_would_cost() {
        assert_eq!(Consequence::Low.approval(), 0.50);
        assert_eq!(Consequence::Medium.approval(), 0.50);
        assert_eq!(Consequence::High.approval(), 0.80);
        // 0.70 clears a click and does not clear a submit.
        assert_eq!(
            verdict(&clicking(0.99), Some("Next page"), None, Some(0.70)),
            Verdict::Execute
        );
        let refused = verdict(&clicking(0.99), Some("Place your order"), None, Some(0.70));
        assert!(matches!(refused, Verdict::HandBack(..)), "{refused:?}");
    }

    /// The measured regression that made a run refuse every Sony listing on a
    /// search page.
    #[test]
    fn noise_cancellation_does_not_raise_the_bar() {
        let listing = "SONY WH-1000XM5 Wireless Noise Cancellation Headphones";
        assert_eq!(
            verdict(&clicking(0.80), Some(listing), None, None),
            Verdict::Execute
        );
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
        let Verdict::HandBack(why, _) = verdict(&decision, None, None, None) else {
            panic!("there is nothing to act on");
        };
        assert_eq!(why, Handback::NothingOffered);
    }

    /// A report ends the run, so a hunch is expensive: the caller gets a blob
    /// of text instead of progress and there is no second chance. Gated at the
    /// low bar rather than waved through.
    #[test]
    fn a_low_consequence_move_on_a_hunch_is_explored_rather_than_run() {
        let decision = read(&response(&[(OPERATION, answer("WAIT", 0.20))])).expect("reads");
        let Verdict::Explore(_) = verdict(&decision, None, None, None) else {
            panic!("0.20 is under the 0.60 floor, and reading ends the run");
        };
    }

    #[test]
    fn a_confident_low_consequence_move_runs() {
        let decision = read(&response(&[(OPERATION, answer("WAIT", 0.88))])).expect("reads");
        assert_eq!(verdict(&decision, None, None, None), Verdict::Execute);
    }

    /// A completion claim is the caller's to judge, not the gate's.
    #[test]
    fn an_ending_is_never_refused() {
        for named in ["DONE", "BLOCKED"] {
            let decision = read(&response(&[(OPERATION, answer(named, 0.11))])).expect("reads");
            assert_eq!(
                verdict(&decision, None, None, None),
                Verdict::Execute,
                "{named}"
            );
        }
    }

    #[test]
    fn leaving_the_page_is_judged_at_the_high_bar() {
        let decision = read(&response(&[
            (OPERATION, answer("NAVIGATE", 0.80)),
            (DESTINATION, answer("the results page", 0.80)),
        ]))
        .expect("reads");
        let Verdict::HandBack(why, _) = verdict(&decision, Some("the results page"), None, None)
        else {
            panic!("0.80 is under the 0.85 that leaving the page needs");
        };
        assert_eq!(why, Handback::TooRiskyToGuess);
    }

    #[test]
    fn a_scroll_the_code_chose_is_not_judged_at_all() {
        let decision = Decision::scroll_down();
        assert_eq!(decision.how, Some(How::Scroll));
        assert_eq!(verdict(&decision, None, None, None), Verdict::Execute);
    }

    #[test]
    fn an_unknown_move_is_an_error_rather_than_a_guess() {
        let got = read(&response(&[(OPERATION, answer("TELEPORT", 0.99))]));
        assert_eq!(
            got.unwrap_err(),
            ReadError::UnknownOperation("TELEPORT".to_string())
        );
    }

    /// The measured case the margin gate exists for. Eleven options, the leader
    /// at 0.63 and the runner-up at 0.11: seven times chance, leading by 0.52.
    /// An absolute bar of 0.75 refused this three times in one run.
    #[test]
    fn a_landslide_over_many_options_runs_where_an_absolute_bar_refused_it() {
        let mut spread_out: Vec<(&str, f64)> = vec![("ACT", 0.63), ("WAIT", 0.11)];
        // The rest sharing what is left, so no single one is the runner-up.
        for name in ["BACK", "RELOAD", "NAVIGATE", "TABS", "DONE", "BLOCKED"] {
            spread_out.push((name, 0.26 / 6.0));
        }
        let decision = read(&response(&[
            (OPERATION, spread(spread_out[0].0, &spread_out)),
            (
                ACT_HOW,
                spread("CLICK", &[("CLICK", 0.91), ("HOVER", 0.05)]),
            ),
            (TARGET, spread("e30", &[("e30", 0.74), ("e12", 0.08)])),
        ]))
        .expect("reads");
        assert!(decision.operation_lead.distributed);
        assert!((decision.operation_lead.margin - 0.52).abs() < 0.01);
        assert_eq!(
            verdict(&decision, Some("World"), None, None),
            Verdict::Execute,
            "0.63 against 0.11 is not a coin toss"
        );
    }

    /// A margin alone is not confidence: a leader at 0.20 over a runner-up at
    /// 0.02 leads by 0.18 and knows nothing.
    ///
    /// The floor only ever binds where the required margin is under it, which
    /// is the low bar alone, because a margin can never exceed its own share. A
    /// report is judged at the low bar, so this is where it bites.
    #[test]
    fn a_wide_lead_over_nothing_is_still_refused() {
        let decision = read(&response(&[(
            OPERATION,
            spread("WAIT", &[("WAIT", 0.20), ("RELOAD", 0.02)]),
        )]))
        .expect("reads");
        assert!(
            decision.operation_lead.margin > Consequence::Low.margin(),
            "it leads by more than a report needs"
        );
        assert!(decision.operation_lead.share < FLOOR);
        assert!(
            matches!(verdict(&decision, None, None, None), Verdict::Explore(_)),
            "0.20 of the answer is not enough however far ahead of the rest it is"
        );
    }

    /// Stated as a test because it is a property of the two numbers rather than
    /// a choice: a margin is the share minus something non-negative, so it can
    /// never exceed the share. Any required margin at or above the floor makes
    /// the floor redundant for that consequence.
    #[test]
    fn the_floor_only_binds_at_the_low_bar() {
        assert!(Consequence::Low.margin() < FLOOR, "so the floor can bite");
        assert!(Consequence::Medium.margin() >= FLOOR);
        assert!(Consequence::High.margin() >= FLOOR);
    }

    /// The bar rises with the consequence, and what rises is the margin.
    #[test]
    fn a_submit_needs_a_wider_lead_than_a_click_on_a_filter() {
        let narrow = |target: &str| {
            read(&response(&[
                (OPERATION, spread("ACT", &[("ACT", 0.80), ("WAIT", 0.05)])),
                (
                    ACT_HOW,
                    spread("CLICK", &[("CLICK", 0.95), ("HOVER", 0.01)]),
                ),
                (TARGET, spread(target, &[(target, 0.60), ("e9", 0.25)])),
            ]))
            .expect("reads")
        };
        // The same evidence, judged against what it would cost to be wrong.
        assert_eq!(
            verdict(&narrow("e1"), Some("Brand: Corsair"), None, Some(0.70)),
            Verdict::Execute,
            "0.70 is worth doing on a filter"
        );
        let Verdict::HandBack(why, _) =
            verdict(&narrow("e2"), Some("Place order"), None, Some(0.70))
        else {
            panic!("the same evidence on a submit must not run");
        };
        assert_eq!(why, Handback::TooRiskyToGuess);
    }

    /// A judgement that reported no distribution falls back to the absolute
    /// bar, so a missing signal is never more permissive than the bar it
    /// replaced.
    #[test]
    fn a_judgement_with_nothing_to_compare_against_keeps_the_old_bar() {
        let decision = read(&response(&[
            (OPERATION, answer("NAVIGATE", 0.80)),
            (DESTINATION, answer("the results page", 0.80)),
        ]))
        .expect("reads");
        assert!(!decision.operation_lead.distributed);
        let (_, lead) = decision
            .short_of(Consequence::High)
            .expect("0.80 is under the 0.85 a high stakes move needs");
        assert!(!lead.distributed);
    }

    /// Words that mean an expensive mistake, matched whole rather than as
    /// substrings. "purchase" belongs with them and was missing.
    #[test]
    fn purchase_is_treated_as_expensive() {
        let decision = read(&response(&[
            (OPERATION, spread("ACT", &[("ACT", 0.80), ("WAIT", 0.05)])),
            (
                ACT_HOW,
                spread("CLICK", &[("CLICK", 0.95), ("HOVER", 0.01)]),
            ),
            (TARGET, spread("e1", &[("e1", 0.60), ("e9", 0.25)])),
        ]))
        .expect("reads");
        let Verdict::HandBack(why, _) =
            verdict(&decision, Some("Complete purchase"), None, Some(0.70))
        else {
            panic!("a purchase is not a filter");
        };
        assert_eq!(why, Handback::TooRiskyToGuess);
    }
}
