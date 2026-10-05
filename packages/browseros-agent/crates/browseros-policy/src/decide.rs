//! One decision: build the questions, ask once, check the answer.
//!
//! The credential arrives from the server's own settings rather than the
//! environment, and the client redacts it when printed, so a stored token
//! cannot reach a log line through an accidental debug format.

use crate::action::ActionSpace;
use crate::answer::{AnswerError, Decision, interpret};
use crate::questions::{Observation, PastAction, questions, state};
use rig::driver::Model;
use rig::typesafeai::types::Answer;
use rig::typesafeai::{DynamicQuery, Evaluate, Jev, JevConfig};
use std::collections::BTreeMap;
use std::time::Instant;

/// Why a decision could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum DecideError {
    /// The question set itself was rejected, which is a bug in how it was
    /// built rather than anything the provider decided.
    #[error("the question set was rejected before sending: {0}")]
    Questions(String),
    #[error("the decision provider failed: {0}")]
    Provider(String),
    #[error(transparent)]
    Answer(#[from] AnswerError),
}

/// What one decision cost and what it concluded.
#[derive(Debug, Clone)]
pub struct Step {
    pub decision: Decision,
    pub latency_ms: u64,
    pub model: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Candidates the cap removed from a target head, so a capped page is
    /// distinguishable from a small one in the trail.
    pub dropped_targets: usize,
}

/// Asks a decision model for the next operation.
#[derive(Clone)]
pub struct Decider {
    model: Model<JevConfig>,
}

impl std::fmt::Debug for Decider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Decider")
    }
}

impl Decider {
    /// A decider for a credential the server holds.
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            model: Jev::new(token).evaluation(),
        }
    }

    /// A decider against a different endpoint, for tests and self-hosted
    /// backends speaking the same protocol.
    #[must_use]
    pub fn with_endpoint(token: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            model: JevConfig::new(token)
                .with_endpoint(endpoint)
                .client()
                .evaluation(),
        }
    }

    /// Asks the provider one trivial question, to prove a credential works.
    ///
    /// A credential field that reports success on a syntax check teaches the
    /// user nothing: a wrong value, a blocked network and a suspended account
    /// are indistinguishable until something is actually asked. The error is
    /// the provider's own wording, because that is what tells them which of
    /// the three it is.
    pub async fn check(&self) -> Result<(), String> {
        let mut definitions = std::collections::BTreeMap::new();
        definitions.insert(
            "reachable".to_string(),
            rig::typesafeai::types::Question::Noul {
                instructions: serde_json::json!("Answer true."),
                criteria: None,
            },
        );
        let query = DynamicQuery::new(definitions).map_err(|error| error.to_string())?;
        self.model
            .evaluate(&serde_json::json!({ "check": true }), query)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Decides the next operation for one observation.
    pub async fn decide(
        &self,
        goal: &str,
        observation: &Observation,
        history: &[PastAction],
    ) -> Result<Step, DecideError> {
        let built = questions(goal, observation);
        let dropped_targets = dropped(&observation.space);
        let query =
            DynamicQuery::new(built).map_err(|error| DecideError::Questions(error.to_string()))?;
        let shared = state(goal, observation, history);

        let started = Instant::now();
        let evaluation = self
            .model
            .evaluate(&shared, query)
            .await
            .map_err(|error| DecideError::Provider(error.to_string()))?;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        let answers: BTreeMap<String, Answer> = evaluation.answers;
        let decision = interpret(&answers, &observation.space)?;
        Ok(Step {
            decision,
            latency_ms,
            model: evaluation.model,
            input_tokens: evaluation
                .usage
                .as_ref()
                .and_then(|usage| usage.input_tokens),
            output_tokens: evaluation
                .usage
                .as_ref()
                .and_then(|usage| usage.output_tokens),
            dropped_targets,
        })
    }
}

fn dropped(space: &ActionSpace) -> usize {
    [&space.click, &space.type_text]
        .into_iter()
        .map(|head| match head {
            crate::action::Head::Question { dropped, .. } => *dropped,
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Element;

    fn observation(elements: Vec<Element>) -> Observation {
        Observation {
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            tree: "- button \"Go\" [ref=e1]".to_string(),
            space: ActionSpace::new(elements, false, false),
        }
    }

    /// The credential must not be reachable through a debug format, since a
    /// decider ends up inside other structures that derive Debug.
    #[test]
    fn the_credential_is_not_printable() {
        let decider = Decider::new("a-real-looking-secret-value");
        let rendered = format!("{decider:?}");
        assert!(
            !rendered.contains("a-real-looking-secret-value"),
            "{rendered}"
        );
    }

    #[test]
    fn a_capped_page_reports_how_many_targets_were_dropped() {
        let many: Vec<Element> = (1..=260)
            .map(|index| Element::new(format!("e{index}"), "button", format!("Button {index}")))
            .collect();
        assert_eq!(dropped(&observation(many).space), 10);
        assert_eq!(
            dropped(&observation(vec![Element::new("e1", "button", "Go")]).space),
            0
        );
    }

    /// A provider that cannot be reached must surface as an error rather than a
    /// decision, because the caller's next move is to hand back, not to act.
    #[tokio::test]
    async fn an_unreachable_provider_produces_no_decision() {
        let decider = Decider::with_endpoint("token", "http://127.0.0.1:1/v1/systemone");
        let result = decider
            .decide(
                "Click the button.",
                &observation(vec![
                    Element::new("e1", "button", "Go"),
                    Element::new("e2", "button", "Stop"),
                ]),
                &[],
            )
            .await;
        assert!(
            matches!(result, Err(DecideError::Provider(_))),
            "expected a provider error, got {result:?}"
        );
    }
}
