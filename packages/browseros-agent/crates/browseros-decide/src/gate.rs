//! Reads one response into a decision, and decides whether to act on it.
//!
//! The answer says what; the confidence says whether to act. The previous
//! implementation recorded both numbers on every step and read neither, which
//! is why a low-confidence guess was indistinguishable from a sure thing.

use crate::client::Response;
use crate::operations::{Consequence, Operation};
use crate::questions::{
    CHECK_TARGET, CLICK_TARGET, FILL_TARGET, OPERATION, PRESS_KEY, PRESS_TARGET, SELECT_VALUE,
    UNCHECK_TARGET,
};
use crate::space::{ActionSpace, NONE_OF_THESE};

/// What the model chose, with the confidence behind each part.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub operation: Operation,
    pub operation_confidence: f64,
    /// The chosen control, for an operation that aims at one.
    pub target: Option<String>,
    pub target_confidence: Option<f64>,
    /// The value, for a select.
    pub value: Option<String>,
    /// The key, for a press.
    pub key: Option<String>,
    /// True when the model said none of the offered controls will do.
    pub none_of_these: bool,
    pub input_tokens: u64,
}

impl Decision {
    /// The confidence the gate judges, which is the weakest part of the answer.
    ///
    /// The minimum rather than a product or an average, following the published
    /// pattern: an operation chosen confidently but aimed at a control chosen
    /// by a coin flip is a coin flip.
    #[must_use]
    pub fn confidence(&self) -> f64 {
        match self.target_confidence {
            Some(target) => self.operation_confidence.min(target),
            None => self.operation_confidence,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// A question we asked came back missing or unreadable.
    Missing(String),
    /// The operation named is not one we offered.
    UnknownOperation(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(id) => write!(formatter, "the answer to {id} was missing"),
            Self::UnknownOperation(name) => write!(formatter, "unknown operation {name}"),
        }
    }
}

/// Reads the response into a decision, taking only the argument heads that
/// belong to the chosen operation.
pub fn read(response: &Response) -> Result<Decision, ReadError> {
    let answer = response
        .answers
        .get(OPERATION)
        .ok_or_else(|| ReadError::Missing(OPERATION.to_string()))?;
    let chosen = answer
        .choice
        .as_deref()
        .ok_or_else(|| ReadError::Missing(OPERATION.to_string()))?;
    let operation =
        Operation::parse(chosen).ok_or_else(|| ReadError::UnknownOperation(chosen.to_string()))?;

    let mut decision = Decision {
        operation,
        operation_confidence: answer.confidence.unwrap_or(0.0),
        target: None,
        target_confidence: None,
        value: None,
        key: None,
        none_of_these: false,
        input_tokens: response.usage.input_tokens,
    };

    if operation == Operation::Select {
        let answer = response
            .answers
            .get(SELECT_VALUE)
            .ok_or_else(|| ReadError::Missing(SELECT_VALUE.to_string()))?;
        let chosen = answer.choice.as_deref().unwrap_or(NONE_OF_THESE);
        decision.target_confidence = answer.confidence;
        if chosen == NONE_OF_THESE {
            decision.none_of_these = true;
        } else if let Some((reference, value)) = ActionSpace::split_select(chosen) {
            decision.target = Some(reference.to_string());
            decision.value = Some(value.to_string());
        } else {
            return Err(ReadError::Missing(SELECT_VALUE.to_string()));
        }
        return Ok(decision);
    }

    let target_id = match operation {
        Operation::Click => Some(CLICK_TARGET),
        Operation::Check => Some(CHECK_TARGET),
        Operation::Uncheck => Some(UNCHECK_TARGET),
        Operation::Press => Some(PRESS_TARGET),
        Operation::Fill => Some(FILL_TARGET),
        _ => None,
    };
    if let Some(id) = target_id {
        let answer = response
            .answers
            .get(id)
            .ok_or_else(|| ReadError::Missing(id.to_string()))?;
        let chosen = answer.choice.as_deref().unwrap_or(NONE_OF_THESE);
        decision.target_confidence = answer.confidence;
        if chosen == NONE_OF_THESE {
            decision.none_of_these = true;
        } else {
            decision.target = Some(chosen.to_string());
        }
    }

    if operation == Operation::Press {
        decision.key = response
            .answers
            .get(PRESS_KEY)
            .and_then(|answer| answer.choice.clone());
    }

    Ok(decision)
}

/// What code should do about a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Run it.
    Execute,
    /// Look at more of the page, then decide again.
    Explore(String),
    /// Return to the caller, saying what is needed.
    HandBack(String),
}

/// Judges a decision against the bar its consequence sets.
///
/// The bar rises with what it costs to be wrong, which is the published
/// guidance: a scroll and a submit do not deserve the same threshold.
#[must_use]
pub fn verdict(decision: &Decision, target_name: Option<&str>) -> Verdict {
    if decision.none_of_these {
        return Verdict::Explore(
            "the model said none of the offered controls can advance the goal".to_string(),
        );
    }
    if decision.operation == Operation::Fill {
        return Verdict::HandBack(
            "this step needs text typed into a field, which the caller supplies".to_string(),
        );
    }
    if decision.operation.needs_target() && decision.target.is_none() {
        return Verdict::Explore(
            "no control was chosen for an operation that needs one".to_string(),
        );
    }

    let consequence = decision.operation.consequence(target_name);
    let confidence = decision.confidence();
    if confidence >= consequence.bar() {
        return Verdict::Execute;
    }
    // Below the floor nothing acts, and a high-stakes action short of its bar
    // is handed back rather than explored: looking at more of the page does not
    // make a submit any safer.
    if consequence == Consequence::High {
        return Verdict::HandBack(format!(
            "{} on {:?} needs confidence {:.2} and had {confidence:.2}",
            decision.operation.as_str(),
            target_name.unwrap_or("an unnamed control"),
            consequence.bar()
        ));
    }
    Verdict::Explore(format!(
        "{} had confidence {confidence:.2}, under the {:.2} this needs",
        decision.operation.as_str(),
        consequence.bar()
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
                input_tokens: 2380,
                output_tokens: 20,
            },
        }
    }

    /// Only the argument head belonging to the chosen operation is read. The
    /// others were asked speculatively and their answers mean nothing.
    #[test]
    fn only_the_chosen_operations_arguments_are_read() {
        let response = response(&[
            (OPERATION, answer("CLICK", 0.9)),
            (CLICK_TARGET, answer("e4", 0.8)),
            (CHECK_TARGET, answer("e9", 0.95)),
            (PRESS_KEY, answer("Enter", 0.99)),
        ]);
        let decision = read(&response).expect("reads");
        assert_eq!(decision.operation, Operation::Click);
        assert_eq!(decision.target.as_deref(), Some("e4"));
        assert_eq!(
            decision.key, None,
            "the press key was speculative and CLICK won"
        );
    }

    /// A select's control and value arrive as one option and split back apart.
    #[test]
    fn a_select_splits_into_a_control_and_a_value() {
        let response = response(&[
            (OPERATION, answer("SELECT", 0.92)),
            (SELECT_VALUE, answer("e9::Price: Low to High", 0.88)),
        ]);
        let decision = read(&response).expect("reads");
        assert_eq!(decision.target.as_deref(), Some("e9"));
        assert_eq!(decision.value.as_deref(), Some("Price: Low to High"));
    }

    /// The judged confidence is the weakest part of the answer, because an
    /// operation chosen confidently but aimed by a coin flip is a coin flip.
    #[test]
    fn the_judged_confidence_is_the_weakest_part() {
        let response = response(&[
            (OPERATION, answer("CLICK", 0.99)),
            (CLICK_TARGET, answer("e4", 0.41)),
        ]);
        let decision = read(&response).expect("reads");
        assert!((decision.confidence() - 0.41).abs() < f64::EPSILON);
    }

    /// Choosing the escape option never executes: code looks further instead.
    #[test]
    fn saying_none_of_these_explores_rather_than_acting() {
        let response = response(&[
            (OPERATION, answer("CLICK", 0.99)),
            (CLICK_TARGET, answer(NONE_OF_THESE, 1.0)),
        ]);
        let decision = read(&response).expect("reads");
        assert!(decision.none_of_these);
        assert!(matches!(verdict(&decision, None), Verdict::Explore(_)));
    }

    /// A confident ordinary action runs.
    #[test]
    fn a_confident_medium_action_executes() {
        let response = response(&[
            (OPERATION, answer("CHECK", 0.95)),
            (CHECK_TARGET, answer("e9", 0.9)),
        ]);
        let decision = read(&response).expect("reads");
        assert_eq!(verdict(&decision, Some("Corsair")), Verdict::Execute);
    }

    /// Under its bar, an ordinary action is not run.
    #[test]
    fn an_unsure_medium_action_does_not_execute() {
        let response = response(&[
            (OPERATION, answer("CLICK", 0.9)),
            (CLICK_TARGET, answer("e4", 0.5)),
        ]);
        let decision = read(&response).expect("reads");
        assert!(matches!(
            verdict(&decision, Some("Corsair")),
            Verdict::Explore(_)
        ));
    }

    /// The same confidence that runs a filter click does not run a submit. The
    /// bar follows what being wrong costs.
    #[test]
    fn the_same_confidence_runs_a_filter_but_not_a_submit() {
        let response = response(&[
            (OPERATION, answer("CLICK", 0.80)),
            (CLICK_TARGET, answer("e4", 0.80)),
        ]);
        let decision = read(&response).expect("reads");
        assert_eq!(
            verdict(&decision, Some("Corsair brand filter")),
            Verdict::Execute
        );
        assert!(
            matches!(
                verdict(&decision, Some("Place your order")),
                Verdict::HandBack(_)
            ),
            "a high-stakes action short of its bar is handed back, because looking at more of \
             the page does not make a submit safer"
        );
    }

    /// Scrolling is cheap to get wrong, so it clears a lower bar.
    #[test]
    fn scrolling_clears_a_lower_bar_than_clicking() {
        let scroll = read(&response(&[(OPERATION, answer("SCROLL_DOWN", 0.65))])).expect("reads");
        assert_eq!(verdict(&scroll, None), Verdict::Execute);

        let click = read(&response(&[
            (OPERATION, answer("CLICK", 0.65)),
            (CLICK_TARGET, answer("e4", 0.65)),
        ]))
        .expect("reads");
        assert!(matches!(
            verdict(&click, Some("Corsair")),
            Verdict::Explore(_)
        ));
    }

    /// Typing is handed back whatever the confidence: the caller owns the text,
    /// because this model is not trained to generate it.
    #[test]
    fn typing_is_always_handed_back() {
        let response = response(&[
            (OPERATION, answer("FILL", 1.0)),
            (FILL_TARGET, answer("e5", 1.0)),
        ]);
        let decision = read(&response).expect("reads");
        assert!(matches!(
            verdict(&decision, Some("Search")),
            Verdict::HandBack(_)
        ));
    }

    #[test]
    fn an_operation_we_did_not_offer_is_refused() {
        let response = response(&[(OPERATION, answer("LAUNCH_MISSILES", 1.0))]);
        assert_eq!(
            read(&response),
            Err(ReadError::UnknownOperation("LAUNCH_MISSILES".to_string()))
        );
    }

    #[test]
    fn a_missing_answer_is_refused_rather_than_guessed() {
        let response = response(&[(OPERATION, answer("CLICK", 0.9))]);
        assert_eq!(
            read(&response),
            Err(ReadError::Missing(CLICK_TARGET.to_string()))
        );
    }
}
