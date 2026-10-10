//! A thin client for the System One evaluation endpoint.
//!
//! Deliberately thin. The whole point of this crate is controlling the exact
//! shape of the request, so an abstraction that reshapes it would be working
//! against the design. The endpoint takes a state, a model and a map of
//! questions, and returns one answer per question plus the token usage that
//! bills us.

use std::collections::BTreeMap;
use std::time::Duration;

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

/// The documented retry defaults, followed rather than invented.
///
/// The published client retries 408, 429 and the whole 500 to 599 range, allows
/// two retries, and backs off exponentially from half a second, doubling, capped
/// at five, with up to a quarter of each delay subtracted as jitter.
pub const MAX_RETRIES: u32 = 2;
pub const RETRY_INITIAL: Duration = Duration::from_millis(500);
pub const RETRY_CAP: Duration = Duration::from_secs(5);
pub const RETRY_JITTER: f64 = 0.25;

/// The documented total budget for one call, covering every attempt and every
/// delay between them rather than each attempt separately.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum JevError {
    #[error("the credential was not accepted")]
    Unauthorized,
    #[error("the request was rejected as invalid: {0}")]
    Invalid(String),
    /// The rate limit was exceeded. Carries the wait the service asked for,
    /// when it named one.
    #[error("rate limited")]
    RateLimited { retry_after: Option<Duration> },
    /// The service failed to process the request. One case for the whole 5xx
    /// range, as the published taxonomy has it: 529 is not special, and
    /// treating it as the only retryable server error meant an ordinary 503
    /// ended a run the published client would have retried.
    #[error("the service failed to process the request: {status}")]
    ServerError { status: u16 },
    #[error("the request timed out after {0:?}")]
    TimedOut(Duration),
    #[error("could not reach the service: {0}")]
    Transport(String),
    #[error("{0}")]
    Shape(String),
    #[error("unexpected status {status}: {body}")]
    Unexpected { status: u16, body: String },
}

impl JevError {
    /// Whether the same request is worth sending again.
    ///
    /// Follows the published retryable set: a request timeout, a rate limit,
    /// every server error, and a failure with no response at all.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. }
                | Self::ServerError { .. }
                | Self::TimedOut(_)
                | Self::Transport(_)
        )
    }

    /// The wait the service asked for, when it named one.
    #[must_use]
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after } => *retry_after,
            _ => None,
        }
    }
}

/// The delay before a given attempt, counting from one.
///
/// Exponential from the documented half second, doubling, capped at five, with
/// jitter subtracted. A service that named its own wait overrides this, because
/// it knows better than a formula does.
#[must_use]
pub fn backoff(attempt: u32, asked_for: Option<Duration>) -> Duration {
    if let Some(asked_for) = asked_for {
        return asked_for.min(RETRY_CAP);
    }
    let shift = attempt.saturating_sub(1).min(8);
    let capped = RETRY_INITIAL.saturating_mul(1u32 << shift).min(RETRY_CAP);
    // Jitter derived from the attempt rather than a random source, so a test
    // can predict it while a retry storm still spreads.
    let fraction = RETRY_JITTER * f64::from(attempt % 4) / 3.0;
    capped.mul_f64(1.0 - fraction)
}

/// Reads the wait a service asked for, from either documented header.
fn retry_after_from(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    if let Some(value) = headers.get("retry-after-ms")
        && let Ok(text) = value.to_str()
        && let Ok(millis) = text.trim().parse::<u64>()
    {
        return Some(Duration::from_millis(millis));
    }
    let value = headers.get(reqwest::header::RETRY_AFTER)?;
    let seconds = value.to_str().ok()?.trim().parse::<u64>().ok()?;
    Some(Duration::from_secs(seconds))
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
        self.ask_within(state, questions, DEFAULT_TIMEOUT).await
    }

    /// Asks every question, giving up after `budget`.
    ///
    /// The budget is the whole call as the published contract defines it: every
    /// attempt and every delay between them, not each attempt separately. A
    /// caller with less time than the default should pass what it has, which is
    /// how a run's own time limit reaches the request.
    pub async fn ask_within(
        &self,
        state: &Value,
        questions: &BTreeMap<String, Question>,
        budget: Duration,
    ) -> Result<Response, JevError> {
        if questions.is_empty() {
            return Err(JevError::Shape("no questions".to_string()));
        }
        for (id, question) in questions {
            question.validate(id)?;
        }
        let send = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.token)
            .timeout(budget)
            .json(&Request {
                state,
                model: &self.model,
                questions,
            })
            .send();
        let response = match tokio::time::timeout(budget, send).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) if error.is_timeout() => return Err(JevError::TimedOut(budget)),
            Ok(Err(error)) => return Err(JevError::Transport(error.to_string())),
            Err(_elapsed) => return Err(JevError::TimedOut(budget)),
        };

        let status = response.status().as_u16();
        let retry_after = retry_after_from(response.headers());
        let body = response
            .text()
            .await
            .map_err(|error| JevError::Transport(error.to_string()))?;
        match status {
            200 => serde_json::from_str(&body)
                .map_err(|error| JevError::Shape(format!("could not read the answer: {error}"))),
            401 => Err(JevError::Unauthorized),
            408 => Err(JevError::TimedOut(Duration::ZERO)),
            422 => Err(JevError::Invalid(body)),
            429 => Err(JevError::RateLimited { retry_after }),
            // The whole range, per the published taxonomy, rather than 529
            // alone: an ordinary 503 was ending runs the published client
            // would have retried.
            500..=599 => Err(JevError::ServerError { status }),
            other => Err(JevError::Unexpected {
                status: other,
                body,
            }),
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
        self.ask(
            &Value::String("A connectivity check.".to_string()),
            &questions,
        )
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

    /// The published retryable set, which this follows rather than invents: a
    /// request timeout, a rate limit, and **every** server error. Treating 529
    /// as the only retryable one meant an ordinary 503 ended a run the
    /// published client would have retried.
    #[test]
    fn the_published_retryable_set_is_what_is_retried() {
        assert!(JevError::RateLimited { retry_after: None }.is_retryable());
        assert!(JevError::TimedOut(Duration::from_secs(1)).is_retryable());
        assert!(JevError::Transport("reset".to_string()).is_retryable());
        for status in [500, 502, 503, 529, 599] {
            assert!(
                JevError::ServerError { status }.is_retryable(),
                "{status} is inside the documented range"
            );
        }
        assert!(!JevError::Unauthorized.is_retryable());
        assert!(!JevError::Invalid("bad".to_string()).is_retryable());
        assert!(!JevError::Shape("bad".to_string()).is_retryable());
        assert!(
            !JevError::Unexpected {
                status: 418,
                body: String::new()
            }
            .is_retryable()
        );
    }

    /// Exponential from the documented half second, doubling, capped at five.
    #[test]
    fn the_backoff_grows_and_is_capped() {
        let first = backoff(1, None);
        let second = backoff(2, None);
        assert!(first >= RETRY_INITIAL.mul_f64(1.0 - RETRY_JITTER));
        assert!(first <= RETRY_INITIAL);
        assert!(second > first, "{second:?} follows {first:?}");
        for attempt in 1..12 {
            assert!(
                backoff(attempt, None) <= RETRY_CAP,
                "attempt {attempt} stays under the cap"
            );
        }
    }

    /// A service that names its own wait knows better than the formula does.
    #[test]
    fn a_service_named_wait_overrides_the_formula() {
        let asked = Duration::from_millis(1_200);
        assert_eq!(backoff(1, Some(asked)), asked);
        assert_eq!(
            backoff(1, Some(Duration::from_secs(600))),
            RETRY_CAP,
            "but not past the cap"
        );
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
