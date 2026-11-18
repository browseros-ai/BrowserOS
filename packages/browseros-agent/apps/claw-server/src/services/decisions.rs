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

use claw_api::models::{
    SkillDraft, SkillDraftFailure, SkillDraftFailureCode, SkillDraftLimitScope,
};

use crate::services::{billing::Billing, skill_draft::digest::DraftRequest};

/// The staging worker, for builds with the `billing-staging` feature.
const STAGING_URL: Option<&str> = Some("https://neo-decisions-staging.openrebase.com");

/// The production worker.
const RELEASE_URL: Option<&str> = Some("https://neo-decisions.openrebase.com");

/// A debug build may point at another worker, for a stub in tests or a local QA run.
const URL_OVERRIDE_ENV: &str = "CLAW_DECISIONS_URL";

const WRITER_TIMEOUT: Duration = Duration::from_secs(20);

/// The worker gives OpenAI 30 s, retries once after a short backoff, and answers within about a
/// minute; waiting a little longer means a slow draft is still read rather than paid for twice.
const SKILL_DRAFT_TIMEOUT: Duration = Duration::from_secs(75);

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

impl Decisions {
    /// The skill drafter, reached through the worker with the current licence.
    #[must_use]
    pub fn skill_drafter(&self, billing: Arc<Billing>) -> Option<WorkerSkillDrafter> {
        Some(WorkerSkillDrafter {
            endpoint: self.url.as_ref()?.join("v1/skills/draft").ok()?,
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

/// The skill drafter on the worker. Only the redacted digest travels; the worker owns the prompt,
/// the model and the per-licence limits, and counts each draft by its request id.
pub struct WorkerSkillDrafter {
    endpoint: Url,
    http: reqwest::Client,
    billing: Arc<Billing>,
}

impl WorkerSkillDrafter {
    /// A draft for this digest, or why the worker could not write one. A refused licence is
    /// refreshed once and the same request (same request id) sent again.
    pub async fn draft(&self, request: &DraftRequest) -> Result<SkillDraft, SkillDraftFailure> {
        let Some(bearer) = self
            .billing
            .signed_licence()
            .await
            .map(|signed| signed.bearer())
        else {
            return Err(SkillDraftFailure::new(SkillDraftFailureCode::UpstreamError));
        };
        let mut response = send_skill_draft(&self.http, &self.endpoint, &bearer, request).await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            && let Some(fresh) = refreshed_bearer(&self.billing, &bearer).await
        {
            response = send_skill_draft(&self.http, &self.endpoint, &fresh, request).await?;
        }
        read_skill_draft(response).await
    }
}

async fn send_skill_draft(
    http: &reqwest::Client,
    endpoint: &Url,
    bearer: &str,
    request: &DraftRequest,
) -> Result<reqwest::Response, SkillDraftFailure> {
    http.post(endpoint.clone())
        .bearer_auth(bearer)
        .timeout(SKILL_DRAFT_TIMEOUT)
        .json(request)
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, "skill draft worker unreachable");
            SkillDraftFailure::new(SkillDraftFailureCode::UpstreamError)
        })
}

#[derive(Deserialize)]
struct Drafted {
    draft: SkillDraft,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Refusal {
    #[serde(default)]
    code: String,
    scope: Option<SkillDraftLimitScope>,
    resets_at: Option<String>,
}

/// Reads the worker's answer into a draft or one of the failures the app explains.
async fn read_skill_draft(response: reqwest::Response) -> Result<SkillDraft, SkillDraftFailure> {
    let status = response.status();
    if status.is_success() {
        return response
            .json::<Drafted>()
            .await
            .map(|drafted| drafted.draft)
            .map_err(|_| SkillDraftFailure::new(SkillDraftFailureCode::DraftUnusable));
    }
    let refusal = response.json::<Refusal>().await.ok();
    let code = refusal.as_ref().map_or("", |refusal| refusal.code.as_str());
    tracing::info!(status = status.as_u16(), code, "skill draft refused");
    let mut failure = SkillDraftFailure::new(match (status.as_u16(), code) {
        (429, _) | (_, "limit_reached") => SkillDraftFailureCode::LimitReached,
        (_, "drafts_paused" | "paused") => SkillDraftFailureCode::DraftsPaused,
        (409, _) | (_, "in_progress") => SkillDraftFailureCode::InProgress,
        (422, _) | (_, "draft_unusable") => SkillDraftFailureCode::DraftUnusable,
        _ => SkillDraftFailureCode::UpstreamError,
    });
    if failure.code == SkillDraftFailureCode::LimitReached
        && let Some(refusal) = refusal
    {
        failure.scope = refusal.scope;
        failure.resets_at = refusal.resets_at;
    }
    Err(failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::{Json, Router, http::StatusCode, routing::post};
    use serde_json::Value;

    /// A stand-in worker that answers every draft with `status` and `body`, and hands back the
    /// bearer and body it received.
    async fn stub_worker(
        status: StatusCode,
        body: Value,
    ) -> anyhow::Result<(Url, tokio::sync::mpsc::UnboundedReceiver<(String, Value)>)> {
        let (seen, received) = tokio::sync::mpsc::unbounded_channel();
        let app = Router::new().route(
            "/v1/skills/draft",
            post(
                move |headers: axum::http::HeaderMap, Json(sent): Json<Value>| {
                    let seen = seen.clone();
                    let body = body.clone();
                    async move {
                        let bearer = headers
                            .get("authorization")
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or_default()
                            .to_string();
                        let _ = seen.send((bearer, sent));
                        (status, Json(body))
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok((url, received))
    }

    fn digest() -> DraftRequest {
        DraftRequest {
            request_id: "0123456789abcdef".into(),
            goal: "Find phones under 30000 on Flipkart".into(),
            outcome: "done",
            site: "flipkart.com".into(),
            agent: "claude-code",
            steps: vec!["Opened Flipkart".into(), "Read Mobiles".into()],
            neo: None,
            pages: vec![],
            fields: vec![],
        }
    }

    async fn ask(
        status: StatusCode,
        body: Value,
    ) -> anyhow::Result<Result<SkillDraft, SkillDraftFailure>> {
        let (url, mut received) = stub_worker(status, body).await?;
        let endpoint = url.join("v1/skills/draft")?;
        let http = reqwest::Client::new();
        let response = send_skill_draft(&http, &endpoint, "licence.sig.kid", &digest()).await;
        let answer = match response {
            Ok(response) => read_skill_draft(response).await,
            Err(failure) => Err(failure),
        };
        let (bearer, sent) = received
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("the worker was not called"))?;
        assert_eq!(bearer, "Bearer licence.sig.kid");
        assert_eq!(sent["requestId"], "0123456789abcdef");
        assert_eq!(sent["outcome"], "done");
        assert!(sent.get("neo").is_none(), "absent parts are left out");
        Ok(answer)
    }

    #[tokio::test]
    async fn a_written_draft_comes_back() -> anyhow::Result<()> {
        let draft = serde_json::json!({
            "name": "flipkart-phones-under-budget",
            "title": "Find phones under a budget on Flipkart",
            "description": "Lists popular phones under a price you choose.",
            "site": "flipkart.com",
            "steps": ["Open flipkart.com.", "Set the price filter to {budget}."],
            "inputs": [{ "name": "budget", "label": "Highest price", "example": "30000", "kind": "number" }],
            "notes": [],
            "effects": "read",
            "confidence": "high"
        });
        let answer = ask(
            StatusCode::OK,
            serde_json::json!({ "draft": draft, "model": "gpt-6-luna" }),
        )
        .await?;
        assert_eq!(
            answer
                .map_err(|failure| anyhow::anyhow!("{failure:?}"))?
                .name,
            "flipkart-phones-under-budget"
        );
        Ok(())
    }

    #[tokio::test]
    async fn refusals_map_to_what_the_app_explains() -> anyhow::Result<()> {
        let limited = ask(
            StatusCode::TOO_MANY_REQUESTS,
            serde_json::json!({
                "code": "limit_reached",
                "message": "limit",
                "scope": "month",
                "resetsAt": "2026-12-01T00:00:00.000Z"
            }),
        )
        .await?
        .err()
        .ok_or_else(|| anyhow::anyhow!("not limited"))?;
        assert_eq!(limited.code, SkillDraftFailureCode::LimitReached);
        assert_eq!(limited.scope, Some(SkillDraftLimitScope::Month));
        assert_eq!(
            limited.resets_at.as_deref(),
            Some("2026-12-01T00:00:00.000Z")
        );

        for (status, code, expected) in [
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "drafts_paused",
                SkillDraftFailureCode::DraftsPaused,
            ),
            (
                StatusCode::CONFLICT,
                "in_progress",
                SkillDraftFailureCode::InProgress,
            ),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                "draft_unusable",
                SkillDraftFailureCode::DraftUnusable,
            ),
            (
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                SkillDraftFailureCode::UpstreamError,
            ),
            (
                StatusCode::FORBIDDEN,
                "not_entitled",
                SkillDraftFailureCode::UpstreamError,
            ),
        ] {
            let failure = ask(status, serde_json::json!({ "code": code, "message": "no" }))
                .await?
                .err()
                .ok_or_else(|| anyhow::anyhow!("{code} was not refused"))?;
            assert_eq!(failure.code, expected, "{code}");
            assert_eq!(failure.scope, None);
        }
        Ok(())
    }

    #[tokio::test]
    async fn an_unreachable_worker_is_an_upstream_error() -> anyhow::Result<()> {
        let endpoint = Url::parse("http://127.0.0.1:9/v1/skills/draft")?;
        let failure = send_skill_draft(&reqwest::Client::new(), &endpoint, "b", &digest())
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("nothing listens on port 9"))?;
        assert_eq!(failure.code, SkillDraftFailureCode::UpstreamError);
        Ok(())
    }

    #[test]
    fn every_compiled_in_worker_url_parses() {
        for url in [STAGING_URL, RELEASE_URL].into_iter().flatten() {
            assert!(Url::parse(url).is_ok(), "{url}");
        }
    }
}
