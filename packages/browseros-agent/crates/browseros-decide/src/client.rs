//! A thin client for the System One evaluation endpoint.
//!
//! Deliberately thin. The whole point of this crate is controlling the exact
//! shape of the request, so an abstraction that reshapes it would be working
//! against the design. The endpoint takes a state, a model and a map of
//! questions, and returns one answer per question plus the token usage that
//! bills us.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The documented evaluation endpoint.
pub const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";

/// The alias we ask for. It resolved to `jev-1.13.0` when last checked, which
/// matters because the published jagged edges are documented against that
/// version: the answers carry the resolved id so a caller can tell when the
/// alias has moved.
pub const DEFAULT_MODEL: &str = "jev-latest";

/// A Choice may carry at most this many options.
pub const MAX_CHOICE_OPTIONS: usize = 255;

/// A Score needs at least this many levels and at most [`MAX_SCORE_LEVELS`].
pub const MIN_SCORE_LEVELS: usize = 2;
/// The API accepts up to ten levels on a Score.
pub const MAX_SCORE_LEVELS: usize = 10;

/// One question about the state.
///
/// The three variants are the only question types the model has. `criteria`
/// carries the options for a Choice, the ordered levels for a Score, and the
/// optional true/false meanings for a Noul; all of them accept structured JSON
/// rather than only strings, which is how a candidate travels with its current
/// state attached.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    Noul {
        instructions: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        criteria: Option<Value>,
    },
    Choice {
        instructions: Value,
        /// Option id to description. An option's description may be an object.
        criteria: BTreeMap<String, Value>,
    },
    Score {
        instructions: Value,
        /// Ordered levels, lowest first.
        criteria: Vec<Value>,
    },
}

impl Question {
    /// Rejects a question the API would reject, so a bad request never leaves
    /// this process and a 422 always means something we did not anticipate.
    pub fn validate(&self, id: &str) -> Result<(), JevError> {
        match self {
            Self::Choice { criteria, .. } => {
                if criteria.is_empty() {
                    return Err(JevError::Shape(format!("choice {id} has no options")));
                }
                if criteria.len() > MAX_CHOICE_OPTIONS {
                    return Err(JevError::Shape(format!(
                        "choice {id} has {} options, over the documented maximum of {MAX_CHOICE_OPTIONS}",
                        criteria.len()
                    )));
                }
                Ok(())
            }
            Self::Score { criteria, .. } => {
                if criteria.len() < MIN_SCORE_LEVELS || criteria.len() > MAX_SCORE_LEVELS {
                    return Err(JevError::Shape(format!(
                        "score {id} has {} levels, outside the documented {MIN_SCORE_LEVELS} to {MAX_SCORE_LEVELS}",
                        criteria.len()
                    )));
                }
                Ok(())
            }
            Self::Noul { .. } => Ok(()),
        }
    }
}

/// One answer. Which fields are populated depends on the question type.
///
/// A Noul carries no confidence: the probability itself is the certainty, near
/// 0.5 meaning uncertain. Choice and Score both carry one.
#[derive(Debug, Clone, Deserialize)]
pub struct Answer {
    #[serde(rename = "type")]
    pub answer_type: String,
    /// Noul only: the probability that the answer is yes.
    #[serde(default)]
    pub noul: Option<f64>,
    /// Choice only: the selected option id.
    #[serde(default)]
    pub choice: Option<String>,
    /// Score only: a position along the levels, which may fall between two.
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub probabilities: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    pub legend: Option<Value>,
    /// Choice and Score only.
    #[serde(default)]
    pub confidence: Option<f64>,
}

/// What the request cost. Output tokens are free; input tokens are billed, so
/// this is the number every budget in this crate is measured against.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Response {
    /// The resolved model id, not the alias that was asked for.
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
    pub usage: Usage,
}

#[derive(Debug, thiserror::Error)]
pub enum JevError {
    #[error("the credential was not accepted")]
    Unauthorized,
    #[error("the request was rejected as invalid: {0}")]
    Invalid(String),
    #[error("rate limited")]
    RateLimited,
    #[error("the service is overloaded")]
    Overloaded,
    #[error("could not reach the service: {0}")]
    Transport(String),
    #[error("{0}")]
    Shape(String),
    #[error("unexpected status {status}: {body}")]
    Unexpected { status: u16, body: String },
}

impl JevError {
    /// Whether the same request is worth sending again.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::Overloaded | Self::Transport(_)
        )
    }
}

#[derive(Serialize)]
struct Request<'a> {
    state: &'a Value,
    model: &'a str,
    questions: &'a BTreeMap<String, Question>,
}

/// Holds the connection pool and the credential. A pool is the one thing in
/// this crate with a managed lifetime, so it is the one thing that is a struct
/// rather than a function.
pub struct Jev {
    http: reqwest::Client,
    token: String,
    model: String,
    endpoint: String,
}

impl Jev {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            token: token.into(),
            model: DEFAULT_MODEL.to_string(),
            endpoint: ENDPOINT.to_string(),
        }
    }

    /// Points this client at another endpoint, for tests against a local stub.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Asks every question about one state, in a single request.
    ///
    /// One request rather than several is not an optimisation detail: questions
    /// sharing a state are evaluated in parallel, so asking more of them barely
    /// moves the response time while splitting them repeats the state's cost.
    pub async fn ask(
        &self,
        state: &Value,
        questions: &BTreeMap<String, Question>,
    ) -> Result<Response, JevError> {
        if questions.is_empty() {
            return Err(JevError::Shape("no questions".to_string()));
        }
        for (id, question) in questions {
            question.validate(id)?;
        }
        let response = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.token)
            .json(&Request {
                state,
                model: &self.model,
                questions,
            })
            .send()
            .await
            .map_err(|error| JevError::Transport(error.to_string()))?;

        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|error| JevError::Transport(error.to_string()))?;
        match status {
            200 => serde_json::from_str(&body)
                .map_err(|error| JevError::Shape(format!("could not read the answer: {error}"))),
            401 => Err(JevError::Unauthorized),
            422 => Err(JevError::Invalid(body)),
            429 => Err(JevError::RateLimited),
            529 => Err(JevError::Overloaded),
            other => Err(JevError::Unexpected { status: other, body }),
        }
    }

    /// Whether this credential works, by asking the cheapest real question.
    ///
    /// Used when a key is entered, so a typo fails at the point of entry rather
    /// than on the first goal a user gives the browser.
    pub async fn check(&self) -> Result<String, JevError> {
        let mut questions = BTreeMap::new();
        questions.insert(
            "reachable".to_string(),
            Question::Noul {
                instructions: Value::String("Is this a test?".to_string()),
                criteria: None,
            },
        );
        self.ask(&Value::String("A connectivity check.".to_string()), &questions)
            .await
            .map(|response| response.model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice_with(count: usize) -> Question {
        Question::Choice {
            instructions: Value::String("pick".to_string()),
            criteria: (0..count)
                .map(|index| (format!("o{index}"), Value::String("an option".to_string())))
                .collect(),
        }
    }

    /// The documented ceiling is 255 options. Catching it here means a 422 from
    /// the real API always indicates something this code did not anticipate.
    #[test]
    fn a_choice_over_the_documented_option_ceiling_is_refused_locally() {
        assert!(choice_with(MAX_CHOICE_OPTIONS).validate("target").is_ok());
        let over = choice_with(MAX_CHOICE_OPTIONS + 1);
        let error = over.validate("target").expect_err("must be refused");
        assert!(
            error.to_string().contains("255"),
            "the message names the documented limit: {error}"
        );
    }

    #[test]
    fn a_choice_with_no_options_is_refused() {
        let empty = Question::Choice {
            instructions: Value::Null,
            criteria: BTreeMap::new(),
        };
        assert!(empty.validate("target").is_err());
    }

    /// A Score takes two to ten levels.
    #[test]
    fn a_score_outside_the_documented_level_range_is_refused() {
        for count in [MIN_SCORE_LEVELS, 3, MAX_SCORE_LEVELS] {
            let ok = Question::Score {
                instructions: Value::Null,
                criteria: vec![Value::String("level".to_string()); count],
            };
            assert!(ok.validate("progress").is_ok(), "{count} levels is allowed");
        }
        for count in [0, 1, MAX_SCORE_LEVELS + 1] {
            let bad = Question::Score {
                instructions: Value::Null,
                criteria: vec![Value::String("level".to_string()); count],
            };
            assert!(bad.validate("progress").is_err(), "{count} levels is not");
        }
    }

    /// Rate limiting and overload are worth retrying; a rejected credential and
    /// a malformed request are not, because the same request will fail again.
    #[test]
    fn only_transient_failures_are_retryable() {
        assert!(JevError::RateLimited.is_retryable());
        assert!(JevError::Overloaded.is_retryable());
        assert!(JevError::Transport("reset".to_string()).is_retryable());
        assert!(!JevError::Unauthorized.is_retryable());
        assert!(!JevError::Invalid("bad".to_string()).is_retryable());
        assert!(!JevError::Shape("bad".to_string()).is_retryable());
    }

    /// A Noul carries no confidence: the probability is the certainty. Parsing
    /// must not invent one.
    #[test]
    fn a_noul_answer_parses_without_a_confidence() {
        let answer: Answer = serde_json::from_str(r#"{"type":"noul","noul":0.96}"#)
            .expect("the shape the API returns");
        assert_eq!(answer.noul, Some(0.96));
        assert!(
            answer.confidence.is_none(),
            "a noul has no confidence to read"
        );
        assert!(answer.choice.is_none());
    }

    #[test]
    fn a_response_carries_the_resolved_model_and_the_billed_tokens() {
        let response: Response = serde_json::from_str(
            r#"{"model":"jev-1.13.0","answers":{"q1":{"type":"noul","noul":0.5}},
                "usage":{"input_tokens":276,"output_tokens":21}}"#,
        )
        .expect("the shape the API returns");
        assert_eq!(response.model, "jev-1.13.0");
        assert_eq!(response.usage.input_tokens, 276);
    }

    #[tokio::test]
    async fn asking_nothing_is_refused_before_any_request() {
        let jev = Jev::new("unused").with_endpoint("http://127.0.0.1:1/never");
        let error = jev
            .ask(&Value::Null, &BTreeMap::new())
            .await
            .expect_err("no questions is not a request");
        assert!(matches!(error, JevError::Shape(_)));
    }
}
