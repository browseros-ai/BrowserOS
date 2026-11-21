//! The decision model behind the verify check's second tier, as a port.
//!
//! ```text
//! verify::standing ─ hints only ─▶ JudgeRequest (evidence text + questions) ─▶ dyn Judge::ask
//!                                                                              ├─ HttpJudge: Jev's systemone wire
//!                                                                              └─ a host's adapter (e.g. a licence-bearer oracle)
//! verify::read_judge ◀── JudgeReply (probabilities, noul, input tokens) ◀──────┘
//! ```
//!
//! The port is transport only: `verify.rs` owns the questions and the thresholds, so a judge
//! never decides "solved". Every failure is a [`JudgeFault`], which the check turns into an
//! `unknown` verdict. Neither the key nor the page text ever appears in a fault or a log line.

use std::{
    collections::BTreeMap,
    fmt,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use futures_util::future::BoxFuture;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::{fs, time, time::Instant};
use url::Url;

use crate::{config::ApiKey, verify::JUDGE_BUDGET};

/// Path of a file holding the Jev bearer. The same name neo-jev (PR #3764) reads, so one key
/// file serves both.
pub const JUDGE_KEY_FILE_ENV: &str = "NEO_JEV_KEY_FILE";
/// Override of the decision endpoint (a relay that speaks the same wire, or a test stub).
pub const JUDGE_URL_ENV: &str = "NEO_JEV_URL";
/// Model alias or pinned id. Images should pin one so thresholds do not drift under them.
pub const JUDGE_MODEL_ENV: &str = "NEO_JEV_MODEL";
pub const DEFAULT_JUDGE_URL: &str = "https://api.typesafe.ai/v1/systemone";
pub const DEFAULT_JUDGE_MODEL: &str = "jev-latest";

const FAULT_LIMIT: u8 = 3;
const BREAKER_COOLDOWN: Duration = Duration::from_secs(60);
const TIMED_OUT: JudgeFault = JudgeFault::Unavailable("request timed out");

/// The decision model the verify check asks when its deterministic tier has only hints.
/// Implementations send one request in which every question reads the same state, and must
/// finish within `budget`; the check does not retry.
pub trait Judge: Send + Sync {
    fn ask<'a>(
        &'a self,
        request: &'a JudgeRequest,
        budget: Duration,
    ) -> BoxFuture<'a, Result<JudgeReply, JudgeFault>>;
}

/// One request: the page evidence as tagged text, and the questions in wire order.
#[derive(Debug, Clone, PartialEq)]
pub struct JudgeRequest {
    /// Untrusted page text, already capped and stripped of input values by `evidence.rs`.
    pub evidence: String,
    pub questions: Vec<JudgeQuestion>,
}

/// A question. With no options it is a yes/no probability (Jev's "noul"); otherwise a choice.
#[derive(Debug, Clone, PartialEq)]
pub struct JudgeQuestion {
    pub id: &'static str,
    pub instructions: String,
    pub options: Vec<JudgeOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JudgeOption {
    pub id: &'static str,
    pub description: &'static str,
}

/// The judge's answers, keyed by question id. `model` and `input_tokens` are optional so a relay
/// that omits them still parses.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JudgeReply {
    pub answers: BTreeMap<String, JudgeAnswer>,
    pub input_tokens: Option<u32>,
    pub model: Option<String>,
}

/// A noul answer carries `noul`; a choice answer carries one probability per option id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JudgeAnswer {
    pub noul: Option<f64>,
    pub probabilities: BTreeMap<String, f64>,
}

/// Why the judge gave no answer. Carries no key, no page text and no response body: a 422 body
/// echoes the request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JudgeFault {
    #[error("judge unavailable: {0}")]
    Unavailable(&'static str),
    #[error("judge refused the request with HTTP {status}")]
    Refused { status: u16 },
    #[error("judge reply could not be read")]
    Malformed,
}

/// Where the default judge sends requests, and with which bearer. Built once from the
/// environment by [`crate::config::CaptchaConfig::from_lookup`].
#[derive(Clone)]
pub struct JudgeConfig {
    pub url: url::Url,
    pub model: String,
    /// Kept so the client can re-read a rotated key once after a 401.
    pub key_file: PathBuf,
    key: ApiKey,
}

impl fmt::Debug for JudgeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JudgeConfig")
            .field("url", &self.url.as_str())
            .field("model", &self.model)
            .field("key_file", &self.key_file)
            .field("key", &self.key)
            .finish()
    }
}

impl JudgeConfig {
    /// `None` when no readable, non-empty key file is named: the check then runs tier 1 only.
    /// An invalid URL also disables the judge rather than sending the key somewhere unexpected.
    pub fn from_lookup(lookup: &impl Fn(&str) -> Option<String>) -> Option<Self> {
        let path = lookup(JUDGE_KEY_FILE_ENV)?;
        let path = path.trim();
        if path.is_empty() {
            return None;
        }
        let Ok(contents) = std::fs::read_to_string(path) else {
            tracing::warn!(target: "captcha", variable = JUDGE_KEY_FILE_ENV, "Jev key file could not be read; verify check runs without Jev");
            return None;
        };
        let key = contents.trim();
        if key.is_empty() {
            tracing::warn!(target: "captcha", variable = JUDGE_KEY_FILE_ENV, "Jev key file is empty; verify check runs without Jev");
            return None;
        }
        let configured = lookup(JUDGE_URL_ENV);
        let configured = configured
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty());
        let url = match url::Url::parse(configured.unwrap_or(DEFAULT_JUDGE_URL)) {
            Ok(url)
                if matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none() =>
            {
                url
            }
            _ => {
                tracing::warn!(target: "captcha", variable = JUDGE_URL_ENV, "Jev URL is invalid; verify check runs without Jev");
                return None;
            }
        };
        let model = lookup(JUDGE_MODEL_ENV)
            .map(|model| model.trim().to_owned())
            .filter(|model| !model.is_empty())
            .unwrap_or_else(|| DEFAULT_JUDGE_MODEL.to_owned());
        Some(Self {
            url,
            model,
            key_file: PathBuf::from(path),
            key: ApiKey::new(key.to_owned()),
        })
    }
}

/// The default judge: Jev's `systemone` wire over HTTPS (`POST {model, state: {evidence},
/// questions}` with the key as bearer). It honours the process proxy settings, as the provider
/// clients do, so in a sandbox the call leaves through the same egress.
pub struct HttpJudge {
    http: Option<Client>,
    url: Url,
    model: String,
    key_file: PathBuf,
    state: Mutex<JudgeState>,
}

/// Requests share the credential and the breaker. Lock only for state transitions so network
/// and file reads never hold other callers up.
struct JudgeState {
    key: Option<Arc<ApiKey>>,
    consecutive_faults: u8,
    open_until: Option<Instant>,
}

impl JudgeState {
    fn admit(&mut self) -> Result<Arc<ApiKey>, JudgeFault> {
        let key = self
            .key
            .as_ref()
            .ok_or(JudgeFault::Unavailable("credential rejected"))?;
        if self.open_until.is_some_and(|until| Instant::now() < until) {
            return Err(JudgeFault::Unavailable("circuit breaker open"));
        }
        self.open_until = None;
        Ok(Arc::clone(key))
    }

    fn record(&mut self, result: &Result<JudgeReply, JudgeFault>) {
        // An older request may complete after the breaker opened. Keep the full cooldown.
        if self.open_until.is_some() {
            return;
        }
        if result.is_ok() {
            self.consecutive_faults = 0;
        } else {
            self.consecutive_faults += 1;
            if self.consecutive_faults == FAULT_LIMIT {
                self.open_until = Some(Instant::now() + BREAKER_COOLDOWN);
                self.consecutive_faults = 0;
            }
        }
    }

    fn reject(&mut self, key: &Arc<ApiKey>, status: StatusCode) {
        // A refusal for an in-flight request using the old key cannot disable its replacement.
        if self
            .key
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, key))
        {
            self.key = None;
            tracing::warn!(target: "captcha", status = status.as_u16(), "Jev rejected the credential; verify check runs without Jev");
        }
    }
}

impl HttpJudge {
    pub fn new(config: JudgeConfig) -> Self {
        // Redirects can disclose the evidence and retries can repeat a billable call.
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .ok();
        Self {
            http,
            url: config.url,
            model: config.model,
            key_file: config.key_file,
            state: Mutex::new(JudgeState {
                key: Some(Arc::new(config.key)),
                consecutive_faults: 0,
                open_until: None,
            }),
        }
    }

    /// The endpoint and model this judge uses, for the host's startup log line. Never the key.
    pub fn describe(&self) -> String {
        format!(
            "{} ({})",
            self.url.host_str().unwrap_or_default(),
            self.model
        )
    }

    fn state(&self) -> MutexGuard<'_, JudgeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn refresh_key(&self, rejected: &Arc<ApiKey>) -> Result<Arc<ApiKey>, JudgeFault> {
        let contents = fs::read_to_string(&self.key_file).await.ok();
        let mut state = self.state();
        if let Some(current) = &state.key {
            if !Arc::ptr_eq(current, rejected) {
                return Ok(Arc::clone(current));
            }
            if let Some(key) = contents
                .as_deref()
                .map(str::trim)
                .filter(|key| !key.is_empty() && *key != rejected.expose())
            {
                let key = Arc::new(ApiKey::new(key.to_owned()));
                state.key = Some(Arc::clone(&key));
                return Ok(key);
            }
        }
        state.reject(rejected, StatusCode::UNAUTHORIZED);
        Err(refused(StatusCode::UNAUTHORIZED))
    }

    async fn exchange(
        &self,
        client: &Client,
        request: &JudgeRequest,
        mut key: Arc<ApiKey>,
    ) -> Result<JudgeReply, JudgeFault> {
        let body = WireRequest::new(request, &self.model);
        let send = |key: &ApiKey| {
            client
                .post(self.url.clone())
                .bearer_auth(key.expose())
                .json(&body)
                .send()
        };
        let mut response = send(&key).await.map_err(transport_fault)?;
        if response.status() == StatusCode::UNAUTHORIZED {
            key = self.refresh_key(&key).await?;
            response = send(&key).await.map_err(transport_fault)?;
        }
        let status = response.status();
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            self.state().reject(&key, status);
        }
        if status != StatusCode::OK {
            // Refusal bodies can echo evidence. Only the status crosses this boundary.
            return Err(refused(status));
        }
        let bytes = response.bytes().await.map_err(transport_fault)?;
        let reply: WireReply = serde_json::from_slice(&bytes).map_err(|_| JudgeFault::Malformed)?;
        reply.into_reply(request)
    }
}

impl Judge for HttpJudge {
    fn ask<'a>(
        &'a self,
        request: &'a JudgeRequest,
        budget: Duration,
    ) -> BoxFuture<'a, Result<JudgeReply, JudgeFault>> {
        // Start before the caller wraps this future in its own timeout. The breaker can then
        // count a transport timeout even when the caller uses the same budget.
        let deadline = Instant::now() + budget.min(JUDGE_BUDGET);
        Box::pin(async move {
            if Instant::now() >= deadline {
                return Err(TIMED_OUT);
            }
            let client = self
                .http
                .as_ref()
                .ok_or(JudgeFault::Unavailable("HTTP client unavailable"))?;
            let key = self.state().admit()?;
            let result = time::timeout_at(deadline, self.exchange(client, request, key))
                .await
                .unwrap_or(Err(TIMED_OUT));
            self.state().record(&result);
            result
        })
    }
}

fn refused(status: StatusCode) -> JudgeFault {
    JudgeFault::Refused {
        status: status.as_u16(),
    }
}

fn transport_fault(error: reqwest::Error) -> JudgeFault {
    if error.is_timeout() {
        TIMED_OUT
    } else {
        JudgeFault::Unavailable("HTTP request failed")
    }
}

#[derive(Serialize)]
struct WireRequest<'a> {
    state: WireState<'a>,
    model: &'a str,
    questions: BTreeMap<&'a str, WireQuestion<'a>>,
}

#[derive(Serialize)]
struct WireState<'a> {
    evidence: &'a str,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum WireQuestion<'a> {
    Noul {
        instructions: &'a str,
    },
    Choice {
        instructions: &'a str,
        criteria: BTreeMap<&'a str, &'a str>,
    },
}

impl<'a> WireRequest<'a> {
    fn new(request: &'a JudgeRequest, model: &'a str) -> Self {
        let mut questions = BTreeMap::new();
        for question in &request.questions {
            let instructions = &question.instructions;
            let wire = if question.options.is_empty() {
                WireQuestion::Noul { instructions }
            } else {
                WireQuestion::Choice {
                    instructions,
                    criteria: question
                        .options
                        .iter()
                        .map(|option| (option.id, option.description))
                        .collect(),
                }
            };
            questions.insert(question.id, wire);
        }
        Self {
            state: WireState {
                evidence: &request.evidence,
            },
            model,
            questions,
        }
    }
}

#[derive(Deserialize)]
struct WireReply {
    answers: BTreeMap<String, WireAnswer>,
    model: Option<String>,
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireUsage {
    input_tokens: Option<u32>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum WireAnswer {
    Noul {
        noul: f64,
    },
    Choice {
        probabilities: BTreeMap<String, f64>,
    },
}

impl WireReply {
    fn into_reply(mut self, request: &JudgeRequest) -> Result<JudgeReply, JudgeFault> {
        // Match the returned fields to what we sent. The verifier owns probability thresholds.
        let mut answers = BTreeMap::new();
        for question in &request.questions {
            let (id, answer) = self
                .answers
                .remove_entry(question.id)
                .ok_or(JudgeFault::Malformed)?;
            let answer = match answer {
                WireAnswer::Noul { noul } if question.options.is_empty() => JudgeAnswer {
                    noul: Some(noul),
                    ..JudgeAnswer::default()
                },
                WireAnswer::Choice { probabilities }
                    if !question.options.is_empty()
                        && question
                            .options
                            .iter()
                            .all(|option| probabilities.contains_key(option.id)) =>
                {
                    JudgeAnswer {
                        probabilities,
                        ..JudgeAnswer::default()
                    }
                }
                _ => return Err(JudgeFault::Malformed),
            };
            answers.insert(id, answer);
        }
        Ok(JudgeReply {
            answers,
            model: self.model,
            input_tokens: self.usage.and_then(|usage| usage.input_tokens),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        fs,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use axum::{
        Json, Router,
        body::{Body, Bytes},
        http::{HeaderMap, StatusCode, header},
        routing::{MethodRouter, post},
    };
    use serde_json::{Value, json};
    use tempfile::NamedTempFile;
    use tokio::{net::TcpListener, task::JoinHandle, time};

    use super::*;
    use crate::verify::{self, JUDGE_BUDGET, Standing};

    const KEY: &str = "fixture-jev-key";

    struct Stub {
        url: String,
        server: JoinHandle<()>,
    }

    impl Stub {
        async fn start(route: MethodRouter) -> anyhow::Result<Self> {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let path = "/v1/systemone";
            let url = format!("http://{}{path}", listener.local_addr()?);
            let app = Router::new().route(path, route);
            let server = tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
            });
            Ok(Self { url, server })
        }

        fn judge(&self, file: &NamedTempFile) -> anyhow::Result<HttpJudge> {
            fs::write(file.path(), format!("  {KEY}\n"))?;
            let config = JudgeConfig::from_lookup(&lookup(&[
                (
                    JUDGE_KEY_FILE_ENV,
                    file.path().to_string_lossy().into_owned(),
                ),
                (JUDGE_URL_ENV, self.url.clone()),
                (JUDGE_MODEL_ENV, "jev-fixture".into()),
            ]))
            .ok_or_else(|| anyhow::anyhow!("stub judge must be configured"))?;
            Ok(HttpJudge::new(config))
        }
    }

    impl Drop for Stub {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    fn request() -> JudgeRequest {
        JudgeRequest {
            evidence: "synthetic page evidence".into(),
            questions: vec![
                JudgeQuestion {
                    id: "standing",
                    instructions: "Choose the page status".into(),
                    options: vec![
                        JudgeOption {
                            id: "clear",
                            description: "The page is usable",
                        },
                        JudgeOption {
                            id: "challenge",
                            description: "A challenge is visible",
                        },
                    ],
                },
                JudgeQuestion {
                    id: "blocked",
                    instructions: "Is the page blocked?".into(),
                    options: Vec::new(),
                },
            ],
        }
    }

    fn answers() -> Value {
        json!({
            "standing": {
                "type": "choice",
                "choice": "clear",
                "probabilities": {"clear": 0.875, "challenge": 0.125}
            },
            "blocked": {"type": "noul", "noul": 0.25}
        })
    }

    fn lookup<'a>(vars: &'a [(&'a str, String)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn tests_that_the_judge_reads_the_neo_jev_names_and_defaults() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("jev-key");
        std::fs::write(&file, "  fixture-jev-key \n")?;
        let path = file.to_string_lossy().into_owned();
        let config = JudgeConfig::from_lookup(&lookup(&[(JUDGE_KEY_FILE_ENV, path.clone())]))
            .ok_or_else(|| anyhow::anyhow!("a readable key file configures the judge"))?;
        assert_eq!(config.url.as_str(), DEFAULT_JUDGE_URL);
        assert_eq!(config.model, DEFAULT_JUDGE_MODEL);
        assert_eq!(config.key.expose(), "fixture-jev-key");
        assert!(
            !format!("{config:?}").contains("fixture-jev-key"),
            "the key must never reach a log line"
        );
        let pinned = JudgeConfig::from_lookup(&lookup(&[
            (JUDGE_KEY_FILE_ENV, path.clone()),
            (JUDGE_URL_ENV, "http://127.0.0.1:9/v1/systemone".into()),
            (JUDGE_MODEL_ENV, "jev-1.14.0".into()),
        ]))
        .ok_or_else(|| anyhow::anyhow!("an override keeps the judge on"))?;
        assert_eq!(pinned.url.as_str(), "http://127.0.0.1:9/v1/systemone");
        assert_eq!(pinned.model, "jev-1.14.0");
        Ok(())
    }

    #[test]
    fn tests_that_a_missing_key_or_a_bad_url_leaves_the_judge_off() -> anyhow::Result<()> {
        assert!(JudgeConfig::from_lookup(&lookup(&[])).is_none());
        let dir = tempfile::tempdir()?;
        let empty = dir.path().join("empty");
        std::fs::write(&empty, " \n")?;
        assert!(
            JudgeConfig::from_lookup(&lookup(&[(
                JUDGE_KEY_FILE_ENV,
                empty.to_string_lossy().into_owned()
            )]))
            .is_none()
        );
        let file = dir.path().join("key");
        std::fs::write(&file, "k")?;
        assert!(
            JudgeConfig::from_lookup(&lookup(&[
                (JUDGE_KEY_FILE_ENV, file.to_string_lossy().into_owned()),
                (JUDGE_URL_ENV, "https://user:pw@example.test/x".into()),
            ]))
            .is_none(),
            "a URL with credentials could leak the key to a third party"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_the_judge_sends_and_reads_the_systemone_wire() -> anyhow::Result<()> {
        let stub = Stub::start(post(|headers: HeaderMap, Json(body): Json<Value>| async move {
            assert_eq!(
                headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()),
                Some("Bearer fixture-jev-key")
            );
            assert_eq!(body, json!({
                "state": {"evidence": "synthetic page evidence"},
                "model": "jev-fixture",
                "questions": {
                    "standing": {"type": "choice", "instructions": "Choose the page status", "criteria": {
                        "clear": "The page is usable", "challenge": "A challenge is visible"
                    }},
                    "blocked": {"type": "noul", "instructions": "Is the page blocked?"}
                }
            }));
            Json(json!({
                "answers": answers(), "model": "jev-1.14.0",
                "usage": {"input_tokens": 123, "output_tokens": 10}
            }))
        })).await?;
        let file = NamedTempFile::new()?;
        let reply = stub.judge(&file)?.ask(&request(), JUDGE_BUDGET).await?;
        assert_eq!(reply.model.as_deref(), Some("jev-1.14.0"));
        assert_eq!(reply.input_tokens, Some(123));
        assert_eq!(
            reply.answers.get("blocked").and_then(|answer| answer.noul),
            Some(0.25)
        );
        assert_eq!(
            reply
                .answers
                .get("standing")
                .and_then(|answer| answer.probabilities.get("clear")),
            Some(&0.875)
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_401_reloads_and_retains_the_rotated_key() -> anyhow::Result<()> {
        let file = NamedTempFile::new()?;
        let path = file.path().to_owned();
        let calls = Arc::new(AtomicUsize::new(0));
        let server_calls = Arc::clone(&calls);
        let stub = Stub::start(post(move |headers: HeaderMap| async move {
            let attempt = server_calls.fetch_add(1, Ordering::Relaxed);
            let bearer = headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok());
            if attempt == 0 && bearer == Some("Bearer fixture-jev-key") {
                fs::write(path, " new-fixture-key\n")
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                return Err(StatusCode::UNAUTHORIZED);
            }
            if bearer != Some("Bearer new-fixture-key") {
                return Err(StatusCode::FORBIDDEN);
            }
            Ok(Json(json!({"answers": answers()})))
        }))
        .await?;
        let judge = stub.judge(&file)?;
        for _ in 0..2 {
            let reply = judge.ask(&request(), JUDGE_BUDGET).await?;
            assert_eq!(
                reply.answers.get("blocked").and_then(|answer| answer.noul),
                Some(0.25)
            );
        }
        assert_eq!(
            calls.load(Ordering::Relaxed),
            3,
            "one retry, then use the rotated key directly"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_unchanged_401_or_a_403_turns_the_judge_off() -> anyhow::Result<()> {
        for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            let calls = Arc::new(AtomicUsize::new(0));
            let server_calls = Arc::clone(&calls);
            let stub = Stub::start(post(move || async move {
                server_calls.fetch_add(1, Ordering::Relaxed);
                status
            }))
            .await?;
            let file = NamedTempFile::new()?;
            let judge = stub.judge(&file)?;
            assert_eq!(
                judge.ask(&request(), JUDGE_BUDGET).await,
                Err(JudgeFault::Refused {
                    status: status.as_u16()
                })
            );
            assert!(matches!(
                judge.ask(&request(), JUDGE_BUDGET).await,
                Err(JudgeFault::Unavailable(_))
            ));
            assert_eq!(
                calls.load(Ordering::Relaxed),
                1,
                "a rejected credential stays off"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_422_discards_the_echoed_body() -> anyhow::Result<()> {
        let stub = Stub::start(post(|| async {
            (StatusCode::UNPROCESSABLE_ENTITY, "synthetic page evidence")
        }))
        .await?;
        let file = NamedTempFile::new()?;
        let log = NamedTempFile::new()?;
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(log.reopen()?)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        assert_eq!(
            stub.judge(&file)?.ask(&request(), JUDGE_BUDGET).await,
            Err(JudgeFault::Refused { status: 422 })
        );
        let logged = fs::read_to_string(log.path())?;
        assert!(
            !logged.contains("synthetic page evidence"),
            "a refusal body can echo the page text"
        );
        assert!(!logged.contains(KEY), "the bearer must stay out of logs");
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_the_judge_does_not_follow_redirects() -> anyhow::Result<()> {
        let stub = Stub::start(post(|| async {
            (
                StatusCode::TEMPORARY_REDIRECT,
                [(header::LOCATION, "/other")],
            )
        }))
        .await?;
        let file = NamedTempFile::new()?;
        assert_eq!(
            stub.judge(&file)?.ask(&request(), JUDGE_BUDGET).await,
            Err(JudgeFault::Refused { status: 307 })
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_the_budget_covers_reading_the_response_body() -> anyhow::Result<()> {
        let stub = Stub::start(post(|| async {
            Body::from_stream(futures_util::stream::pending::<Result<Bytes, Infallible>>())
        }))
        .await?;
        let file = NamedTempFile::new()?;
        let judge = stub.judge(&file)?;
        let request = request();
        let reply = time::timeout(
            Duration::from_secs(1),
            judge.ask(&request, Duration::from_millis(80)),
        )
        .await?;
        assert!(
            matches!(reply, Err(JudgeFault::Unavailable(reason)) if reason.contains("timed out"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_model_and_usage_can_be_missing() -> anyhow::Result<()> {
        let stub = Stub::start(post(|| async { Json(json!({"answers": answers()})) })).await?;
        let file = NamedTempFile::new()?;
        let reply = stub.judge(&file)?.ask(&request(), JUDGE_BUDGET).await?;
        assert_eq!(reply.model, None);
        assert_eq!(reply.input_tokens, None);
        assert_eq!(
            reply.answers.get("blocked").and_then(|answer| answer.noul),
            Some(0.25)
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_bad_reply_shape_is_malformed() -> anyhow::Result<()> {
        for body in [
            json!({"model": "jev-fixture"}),
            json!({"answers": []}),
            json!({"answers": {"standing": {"type": "choice", "probabilities": []}, "blocked": {"type": "noul", "noul": 0.25}}}),
        ] {
            let stub = Stub::start(post(move || async { Json(body) })).await?;
            let file = NamedTempFile::new()?;
            assert_eq!(
                stub.judge(&file)?.ask(&request(), JUDGE_BUDGET).await,
                Err(JudgeFault::Malformed)
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_three_consecutive_faults_pause_requests_for_sixty_seconds()
    -> anyhow::Result<()> {
        let calls = Arc::new(AtomicUsize::new(0));
        let server_calls = Arc::clone(&calls);
        let stub = Stub::start(post(move || async move {
            match server_calls.fetch_add(1, Ordering::Relaxed) {
                2 | 6 => Ok(Json(json!({"answers": answers()}))),
                _ => Err(StatusCode::SERVICE_UNAVAILABLE),
            }
        }))
        .await?;
        let file = NamedTempFile::new()?;
        let judge = stub.judge(&file)?;
        let request = request();
        for expected in [false, false, true, false, false, false] {
            let reply = judge.ask(&request, JUDGE_BUDGET).await;
            if expected {
                assert!(reply.is_ok(), "success resets the fault count");
            } else {
                assert_eq!(reply, Err(JudgeFault::Refused { status: 503 }));
            }
        }
        time::pause();
        time::advance(Duration::from_secs(59)).await;
        assert!(matches!(
            judge.ask(&request, JUDGE_BUDGET).await,
            Err(JudgeFault::Unavailable(_))
        ));
        assert_eq!(
            calls.load(Ordering::Relaxed),
            6,
            "an open breaker skips the request"
        );
        time::advance(Duration::from_secs(1)).await;
        time::resume();
        judge.ask(&request, JUDGE_BUDGET).await?;
        assert_eq!(calls.load(Ordering::Relaxed), 7);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "makes one live Jev request using NEO_JEV_KEY_FILE"]
    #[expect(
        clippy::print_stdout,
        reason = "the ignored live test reports only latency and model id"
    )]
    async fn tests_that_live_jev_answers_the_verify_questions() -> anyhow::Result<()> {
        let config = JudgeConfig::from_lookup(&|name| {
            (name == JUDGE_KEY_FILE_ENV)
                .then(|| std::env::var(name).ok())
                .flatten()
        })
        .ok_or_else(|| anyhow::anyhow!("set NEO_JEV_KEY_FILE to a readable key file"))?;
        let judge = HttpJudge::new(config);
        let request = JudgeRequest {
            evidence: "[SITE] example.test\n[TITLE] Library search results\n[HEADING] Search results\n[TEXT] 12 books about local history. Each result has a visible title and a link.\n[BUTTON] Next page".into(),
            questions: verify::questions(),
        };
        let started = time::Instant::now();
        let reply = judge.ask(&request, JUDGE_BUDGET).await?;
        let elapsed = started.elapsed();
        let model = reply
            .model
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("live reply omitted its model id"))?;
        println!("Jev answered in {} ms with {model}", elapsed.as_millis());
        assert_eq!(verify::read_judge(&reply), Standing::Clear);
        Ok(())
    }
}
