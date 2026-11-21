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

use std::{collections::BTreeMap, fmt, path::PathBuf, time::Duration};

use futures_util::future::BoxFuture;

use crate::config::ApiKey;

/// Path of a file holding the Jev bearer. The same name neo-jev (PR #3764) reads, so one key
/// file serves both.
pub const JUDGE_KEY_FILE_ENV: &str = "NEO_JEV_KEY_FILE";
/// Override of the decision endpoint (a relay that speaks the same wire, or a test stub).
pub const JUDGE_URL_ENV: &str = "NEO_JEV_URL";
/// Model alias or pinned id. Images should pin one so thresholds do not drift under them.
pub const JUDGE_MODEL_ENV: &str = "NEO_JEV_MODEL";
pub const DEFAULT_JUDGE_URL: &str = "https://api.typesafe.ai/v1/systemone";
pub const DEFAULT_JUDGE_MODEL: &str = "jev-latest";

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
    config: JudgeConfig,
}

impl HttpJudge {
    pub fn new(config: JudgeConfig) -> Self {
        Self { config }
    }

    /// The endpoint and model this judge uses, for the host's startup log line. Never the key.
    pub fn describe(&self) -> String {
        format!(
            "{} ({})",
            self.config.url.host_str().unwrap_or_default(),
            self.config.model
        )
    }
}

impl Judge for HttpJudge {
    // Wave 2 (piece V2) sends the request: redirects off, one attempt within `budget`, a tolerant
    // reply parser, one key-file re-read after a 401, and a three-fault breaker. Until then the
    // judge is honestly unavailable, so an unclear check reads `unknown`.
    fn ask<'a>(
        &'a self,
        _request: &'a JudgeRequest,
        _budget: Duration,
    ) -> BoxFuture<'a, Result<JudgeReply, JudgeFault>> {
        Box::pin(std::future::ready(Err(JudgeFault::Unavailable(
            "the HTTP judge is not built yet",
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn tests_that_the_unbuilt_http_judge_is_unavailable_not_a_guess() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("key");
        std::fs::write(&file, "k")?;
        let config = JudgeConfig::from_lookup(&lookup(&[(
            JUDGE_KEY_FILE_ENV,
            file.to_string_lossy().into_owned(),
        )]))
        .ok_or_else(|| anyhow::anyhow!("configured"))?;
        let judge = HttpJudge::new(config);
        let request = JudgeRequest {
            evidence: String::new(),
            questions: Vec::new(),
        };
        assert!(matches!(
            judge.ask(&request, Duration::from_secs(2)).await,
            Err(JudgeFault::Unavailable(_))
        ));
        Ok(())
    }
}
