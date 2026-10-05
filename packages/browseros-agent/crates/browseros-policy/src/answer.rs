//! Reading an answer, and refusing to act on one that does not fit the
//! observation it was asked about.
//!
//! The provider is typed, but a target head is keyed at runtime, so the
//! returned reference is a string that has to be checked against the head this
//! observation actually offered. Anything unrecognised ends the step without
//! touching the page: a decision that cannot be resolved is not executed.

use crate::action::{ActionSpace, Head, Operation};
use crate::questions::{CLICK_TARGET, OPERATION, PROGRESS, SATISFIED, TYPE_TEXT_TARGET};
use rig::typesafeai::types::Answer;
use std::collections::BTreeMap;

/// Why a decision could not be turned into an action.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum AnswerError {
    #[error("the answer is missing the `{0}` question")]
    Missing(&'static str),
    #[error("the `{id}` answer is a {found} where a {expected} was asked")]
    WrongKind {
        id: &'static str,
        expected: &'static str,
        found: &'static str,
    },
    #[error("`{0}` is not an operation this observation offered")]
    UnknownOperation(String),
    #[error("`{operation}` needs a target and none was offered")]
    NoTargetAvailable { operation: &'static str },
    #[error("`{reference}` was not among the elements offered for `{operation}`")]
    UnofferedTarget {
        operation: &'static str,
        reference: String,
    },
}

/// What the model decided, once it has been checked against the observation.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub operation: Operation,
    /// The element to act on, already confirmed to come from this observation.
    pub target: Option<String>,
    /// Confidence in the operation choice, as reported.
    pub operation_confidence: f64,
    /// Confidence in the target choice, when a target question was asked. An
    /// implied target was never asked about, so there is nothing to report.
    pub target_confidence: Option<f64>,
    /// Probability that the goal is already visibly satisfied.
    pub satisfied: f64,
    /// Progress on the rubric, in `0..=2` for the three levels asked.
    pub progress: f64,
    /// The operation distribution, kept for the audit trail.
    pub operation_probabilities: BTreeMap<String, f64>,
}

fn kind(answer: &Answer) -> &'static str {
    match answer {
        Answer::Choice { .. } => "choice",
        Answer::Score { .. } => "score",
        Answer::Noul { .. } => "noul",
    }
}

fn choice<'a>(
    answers: &'a BTreeMap<String, Answer>,
    id: &'static str,
) -> Result<(&'a String, &'a BTreeMap<String, f64>, f64), AnswerError> {
    match answers.get(id) {
        None => Err(AnswerError::Missing(id)),
        Some(Answer::Choice {
            choice,
            probabilities,
            confidence,
        }) => Ok((choice, probabilities, *confidence)),
        Some(other) => Err(AnswerError::WrongKind {
            id,
            expected: "choice",
            found: kind(other),
        }),
    }
}

/// Turns a raw answer set into a decision, or refuses it.
///
/// Every refusal here means no action runs, which is the point: an answer that
/// does not fit the observation is a reason to stop, never a reason to guess.
pub fn interpret(
    answers: &BTreeMap<String, Answer>,
    space: &ActionSpace,
) -> Result<Decision, AnswerError> {
    let (operation_name, operation_probabilities, operation_confidence) =
        choice(answers, OPERATION)?;

    let operation = Operation::parse(operation_name)
        .filter(|operation| space.available_operations().contains(operation))
        .ok_or_else(|| AnswerError::UnknownOperation(operation_name.clone()))?;

    let (target, target_confidence) = if operation.needs_target() {
        let id = match operation {
            Operation::Click => CLICK_TARGET,
            Operation::TypeText => TYPE_TEXT_TARGET,
            _ => unreachable!("only click and type_text need a target"),
        };
        match space.head(operation) {
            None | Some(Head::Unavailable) => {
                return Err(AnswerError::NoTargetAvailable {
                    operation: operation.as_str(),
                });
            }
            // Asked about, so the answer has to name an element that was offered.
            Some(Head::Question { options, .. }) => {
                let (reference, _, confidence) = choice(answers, id)?;
                if !options.contains_key(reference) {
                    return Err(AnswerError::UnofferedTarget {
                        operation: operation.as_str(),
                        reference: reference.clone(),
                    });
                }
                (Some(reference.clone()), Some(confidence))
            }
            // Never asked about, so there is no answer to read and nothing to
            // validate: one candidate is the target by construction.
            Some(Head::Implied(reference)) => (Some(reference.clone()), None),
        }
    } else {
        (None, None)
    };

    let satisfied = match answers.get(SATISFIED) {
        None => return Err(AnswerError::Missing(SATISFIED)),
        Some(Answer::Noul { noul }) => *noul,
        Some(other) => {
            return Err(AnswerError::WrongKind {
                id: SATISFIED,
                expected: "noul",
                found: kind(other),
            });
        }
    };
    let progress = match answers.get(PROGRESS) {
        None => return Err(AnswerError::Missing(PROGRESS)),
        Some(Answer::Score { score, .. }) => *score,
        Some(other) => {
            return Err(AnswerError::WrongKind {
                id: PROGRESS,
                expected: "score",
                found: kind(other),
            });
        }
    };

    Ok(Decision {
        operation,
        target,
        operation_confidence,
        target_confidence,
        satisfied,
        progress,
        operation_probabilities: operation_probabilities.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Element;
    use serde_json::{Value, json};

    fn page(roles: &[(&str, &str)]) -> Vec<Element> {
        roles
            .iter()
            .enumerate()
            .map(|(index, (role, name))| Element::new(format!("e{}", index + 1), *role, *name))
            .collect()
    }

    fn answer(value: Value) -> Answer {
        serde_json::from_value(value).unwrap_or_else(|error| panic!("bad fixture: {error}"))
    }

    fn choice_answer(selected: &str, options: &[&str]) -> Answer {
        let share = 1.0 / options.len() as f64;
        let probabilities: serde_json::Map<String, Value> = options
            .iter()
            .map(|option| ((*option).to_string(), json!(share)))
            .collect();
        answer(json!({
            "type": "choice",
            "choice": selected,
            "probabilities": probabilities,
            "confidence": 0.7,
        }))
    }

    fn base(extra: Vec<(&str, Answer)>) -> BTreeMap<String, Answer> {
        let mut answers = BTreeMap::new();
        answers.insert(
            SATISFIED.to_string(),
            answer(json!({ "type": "noul", "noul": 0.2 })),
        );
        answers.insert(
            PROGRESS.to_string(),
            answer(json!({
                "type": "score", "score": 1.0,
                "probabilities": {}, "legend": {}, "confidence": 0.8
            })),
        );
        for (id, value) in extra {
            answers.insert(id.to_string(), value);
        }
        answers
    }

    #[test]
    fn a_target_from_a_question_is_accepted_when_it_was_offered() {
        let space = ActionSpace::new(
            page(&[("textbox", "From"), ("textbox", "To")]),
            false,
            false,
        );
        let answers = base(vec![
            (
                OPERATION,
                choice_answer("TYPE_TEXT", &["TYPE_TEXT", "DONE"]),
            ),
            (TYPE_TEXT_TARGET, choice_answer("e2", &["e1", "e2"])),
        ]);
        let decision = interpret(&answers, &space).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(decision.operation, Operation::TypeText);
        assert_eq!(decision.target.as_deref(), Some("e2"));
        assert_eq!(decision.target_confidence, Some(0.7));
        assert!((decision.satisfied - 0.2).abs() < f64::EPSILON);
    }

    /// The guarantee: a reference the observation never offered is refused, and
    /// refusing means nothing is executed.
    #[test]
    fn a_target_that_was_not_offered_is_refused() {
        let space = ActionSpace::new(
            page(&[("textbox", "From"), ("textbox", "To")]),
            false,
            false,
        );
        let answers = base(vec![
            (
                OPERATION,
                choice_answer("TYPE_TEXT", &["TYPE_TEXT", "DONE"]),
            ),
            (TYPE_TEXT_TARGET, choice_answer("e99", &["e1", "e2"])),
        ]);
        assert_eq!(
            interpret(&answers, &space),
            Err(AnswerError::UnofferedTarget {
                operation: "TYPE_TEXT",
                reference: "e99".to_string()
            })
        );
    }

    /// An implied head was never asked about, so there is no answer for it and
    /// the single candidate is used directly.
    #[test]
    fn an_implied_target_needs_no_answer() {
        let space = ActionSpace::new(page(&[("button", "Search")]), false, false);
        let answers = base(vec![(
            OPERATION,
            choice_answer("CLICK", &["CLICK", "DONE"]),
        )]);
        let decision = interpret(&answers, &space).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(decision.operation, Operation::Click);
        assert_eq!(decision.target.as_deref(), Some("e1"));
        assert_eq!(
            decision.target_confidence, None,
            "nothing was asked, so nothing is reported"
        );
    }

    #[test]
    fn an_operation_this_observation_did_not_offer_is_refused() {
        // Nothing editable, so TYPE_TEXT was never on the menu.
        let space = ActionSpace::new(page(&[("button", "A"), ("button", "B")]), false, false);
        let answers = base(vec![(
            OPERATION,
            choice_answer("TYPE_TEXT", &["TYPE_TEXT", "DONE"]),
        )]);
        assert_eq!(
            interpret(&answers, &space),
            Err(AnswerError::UnknownOperation("TYPE_TEXT".to_string()))
        );
    }

    /// Code execution can never arrive as an operation, whatever comes back.
    #[test]
    fn an_operation_that_would_execute_code_is_refused() {
        let space = ActionSpace::new(page(&[("button", "A"), ("button", "B")]), false, false);
        for forged in ["RUN", "EVALUATE", "eval", ""] {
            let answers = base(vec![(OPERATION, choice_answer(forged, &[forged, "DONE"]))]);
            assert_eq!(
                interpret(&answers, &space),
                Err(AnswerError::UnknownOperation(forged.to_string())),
                "{forged} must never resolve to an operation"
            );
        }
    }

    #[test]
    fn terminal_operations_need_no_target() {
        let space = ActionSpace::new(page(&[("button", "A"), ("button", "B")]), false, false);
        for name in ["DONE", "BLOCKED", "WAIT"] {
            let answers = base(vec![(OPERATION, choice_answer(name, &[name, "CLICK"]))]);
            let decision = interpret(&answers, &space).unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(decision.target, None);
        }
    }

    #[test]
    fn a_missing_or_mistyped_question_is_refused_rather_than_guessed() {
        let space = ActionSpace::new(page(&[("button", "A"), ("button", "B")]), false, false);

        let mut answers = base(vec![
            (OPERATION, choice_answer("CLICK", &["CLICK", "DONE"])),
            (CLICK_TARGET, choice_answer("e1", &["e1", "e2"])),
        ]);
        answers.remove(SATISFIED);
        assert_eq!(
            interpret(&answers, &space),
            Err(AnswerError::Missing(SATISFIED))
        );

        let mut answers = base(vec![
            (OPERATION, choice_answer("CLICK", &["CLICK", "DONE"])),
            (CLICK_TARGET, choice_answer("e1", &["e1", "e2"])),
        ]);
        answers.insert(
            SATISFIED.to_string(),
            answer(json!({
                "type": "score", "score": 1.0,
                "probabilities": {}, "legend": {}, "confidence": 0.8
            })),
        );
        assert_eq!(
            interpret(&answers, &space),
            Err(AnswerError::WrongKind {
                id: SATISFIED,
                expected: "noul",
                found: "score"
            })
        );
    }

    #[test]
    fn the_operation_distribution_is_kept_for_the_trail() {
        let space = ActionSpace::new(page(&[("button", "A"), ("button", "B")]), false, false);
        let answers = base(vec![
            (
                OPERATION,
                choice_answer("CLICK", &["CLICK", "WAIT", "DONE"]),
            ),
            (CLICK_TARGET, choice_answer("e1", &["e1", "e2"])),
        ]);
        let decision = interpret(&answers, &space).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(decision.operation_probabilities.len(), 3);
        assert!(decision.operation_probabilities.contains_key("WAIT"));
    }
}
