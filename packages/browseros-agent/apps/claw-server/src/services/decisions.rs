//! The decisions worker: the hosted proxy that holds the decision model's and the writer's keys.
//!
//! The server proves the plan by sending the licence the billing gateway signed, which the worker
//! verifies offline. The install secret and the licence key never go to the worker.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use browseros_decide::{
    client::{Jev, JevError, Question, Response},
    pursue::Oracle,
    writer::{Field, Writer, WriterError},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use url::Url;

use crate::services::billing::Billing;

/// The staging worker, for builds with the `billing-staging` feature.
const STAGING_URL: Option<&str> = Some("https://neo-decisions-staging.openrebase.com");

/// The production worker.
const RELEASE_URL: Option<&str> = Some("https://neo-decisions.openrebase.com");

/// A debug build may point at another worker, for a stub in tests or a local QA run.
const URL_OVERRIDE_ENV: &str = "CLAW_DECISIONS_URL";

const WRITER_TIMEOUT: Duration = Duration::from_secs(20);

/// Where this build reaches the decisions worker, if anywhere.
pub struct Decisions {
    url: Option<Url>,
    http: reqwest::Client,
}

impl Decisions {
    #[must_use]
    pub fn new(url: Option<Url>) -> Self {
        Self {
            url,
            http: reqwest::Client::builder()
                .timeout(WRITER_TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }

    /// The worker this build uses: none in hosted mode, which has no plan to prove.
    #[must_use]
    pub fn for_build(hosted: bool) -> Self {
        if hosted {
            return Self::new(None);
        }
        let overridden = cfg!(debug_assertions)
            .then(|| std::env::var(URL_OVERRIDE_ENV).ok())
            .flatten()
            .filter(|url| !url.trim().is_empty());
        let chosen = overridden
            .as_deref()
            .or(if cfg!(feature = "billing-staging") {
                STAGING_URL
            } else {
                RELEASE_URL
            });
        Self::new(chosen.and_then(|url| Url::parse(url).ok()))
    }

    #[must_use]
    pub fn url(&self) -> Option<&Url> {
        self.url.as_ref()
    }

    /// The decision model, reached through the worker with the current licence.
    pub async fn oracle(&self, billing: Arc<Billing>) -> Option<LicencedOracle> {
        let endpoint = self.url.as_ref()?.join("v1/jev/systemone").ok()?;
        let bearer = billing.signed_licence().await?.bearer();
        Some(LicencedOracle {
            jev: Mutex::new((bearer.clone(), jev_for(&bearer, &endpoint))),
            endpoint,
            billing,
        })
    }

    /// The writer, reached through the worker with the current licence.
    #[must_use]
    pub fn writer(&self, billing: Arc<Billing>) -> Option<WorkerWriter> {
        Some(WorkerWriter {
            endpoint: self.url.as_ref()?.join("v1/writer/field-values").ok()?,
            http: self.http.clone(),
            billing,
        })
    }
}

fn jev_for(bearer: &str, endpoint: &Url) -> Jev {
    Jev::new(bearer).with_endpoint(endpoint.as_str())
}

/// jev through the worker. A refused licence is refreshed once and the request asked again, since
/// a computer that slept past the licence's expiry holds a stale one.
pub struct LicencedOracle {
    jev: Mutex<(String, Jev)>,
    endpoint: Url,
    billing: Arc<Billing>,
}

impl Oracle for LicencedOracle {
    async fn ask_within(
        &self,
        state: &Value,
        questions: &BTreeMap<String, Question>,
        budget: Duration,
    ) -> Result<Response, JevError> {
        let mut jev = self.jev.lock().await;
        match jev.1.ask_within(state, questions, budget).await {
            Err(JevError::Unauthorized) => {
                let Some(fresh) = refreshed_bearer(&self.billing, &jev.0).await else {
                    return Err(JevError::Unauthorized);
                };
                *jev = (fresh.clone(), jev_for(&fresh, &self.endpoint));
                jev.1.ask_within(state, questions, budget).await
            }
            answered => answered,
        }
    }
}

/// A newer licence than `used`, after asking the gateway for one.
async fn refreshed_bearer(billing: &Billing, used: &str) -> Option<String> {
    let _ = billing.refresh().await;
    let fresh = billing.signed_licence().await?.bearer();
    (fresh != used).then_some(fresh)
}

/// The field writer on the worker. Only the goal and the field descriptor travel; the worker owns
/// the prompt and the model.
pub struct WorkerWriter {
    endpoint: Url,
    http: reqwest::Client,
    billing: Arc<Billing>,
}

#[derive(Deserialize)]
struct Written {
    #[serde(default)]
    values: Vec<String>,
}

impl WorkerWriter {
    async fn send(&self, bearer: &str, body: &Value) -> Result<reqwest::Response, WriterError> {
        self.http
            .post(self.endpoint.clone())
            .bearer_auth(bearer)
            .json(body)
            .send()
            .await
            .map_err(|error| WriterError(format!("could not reach the writer: {error}")))
    }
}

impl Writer for WorkerWriter {
    async fn candidates(&self, goal: &str, field: &Field) -> Result<Vec<String>, WriterError> {
        let body = json!({
            "goal": goal,
            "field": {
                "role": field.role,
                "name": field.name,
                "value": field.value,
                "site": field.site,
                "title": field.title,
            },
        });
        let Some(bearer) = self
            .billing
            .signed_licence()
            .await
            .map(|signed| signed.bearer())
        else {
            return Err(WriterError("this computer has no licence".to_string()));
        };
        let mut response = self.send(&bearer, &body).await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            && let Some(fresh) = refreshed_bearer(&self.billing, &bearer).await
        {
            response = self.send(&fresh, &body).await?;
        }
        if !response.status().is_success() {
            return Err(WriterError(format!(
                "the writer answered {}",
                response.status().as_u16()
            )));
        }
        response
            .json::<Written>()
            .await
            .map(|written| written.values)
            .map_err(|_| WriterError("the writer's answer could not be read".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_compiled_in_worker_url_parses() {
        for url in [STAGING_URL, RELEASE_URL].into_iter().flatten() {
            assert!(Url::parse(url).is_ok(), "{url}");
        }
    }
}
