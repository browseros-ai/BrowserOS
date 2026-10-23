use std::{
    future::Future,
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use futures_util::future::BoxFuture;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{
    config::{CaptchaConfig, ProviderConfig},
    types::{
        ProviderAnswer, ProviderError, ProviderErrorKind, ProviderName, SolveAnswer, SolveTask,
        TileGrid,
    },
};

const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_SUBMISSION_BUDGET: Duration = Duration::from_secs(5);

pub trait CaptchaBackend: Send + Sync {
    fn is_configured(&self) -> bool;
    /// With a `ProviderChain`, read `paid_tasks()` before and after this call to
    /// count charged attempts, including failed vendors before a successful fallback.
    /// Deltas are per-call only without overlapping solves; the counter also includes
    /// in-flight reservations. `ProviderAnswer` carries the last vendor's task ID only.
    fn solve(
        &self,
        task: &SolveTask,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> impl Future<Output = Result<ProviderAnswer, ProviderError>> + Send;
}

pub trait VendorClient: Send + Sync {
    fn name(&self) -> ProviderName;
    fn supports(&self, task: &SolveTask) -> bool;
    fn attempt_budget(&self, task: &SolveTask) -> Duration {
        match task {
            SolveTask::RecaptchaTiles(_) => ATTEMPT_TIMEOUT,
            SolveTask::RecaptchaToken(_) | SolveTask::TurnstileToken(_) => Duration::from_secs(60),
        }
    }
    fn solve<'a>(
        &'a self,
        task: &'a SolveTask,
        deadline: Instant,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ProviderAnswer, ProviderError>>;
}

pub struct ProviderChain {
    clients: Vec<Box<dyn VendorClient>>,
    paid: AtomicU32,
    limit: u32,
}

impl ProviderChain {
    pub fn new(config: &CaptchaConfig) -> Self {
        // A redirect or automatic replay could disclose credentials or buy a second task.
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .ok();
        let mut clients: Vec<Box<dyn VendorClient>> = Vec::new();
        if let Some(config) = &config.capsolver {
            clients.push(Box::new(CapsolverClient {
                config: config.clone(),
                http,
            }));
        }
        if let Some(config) = &config.twocaptcha {
            match super::twocaptcha::TwoCaptchaClient::new(config) {
                Ok(client) => clients.push(Box::new(client)),
                Err(error) => {
                    tracing::warn!(target: "captcha", kind = ?error.kind, "2Captcha client unavailable")
                }
            }
        }
        Self::with_clients(clients, config.paid_task_limit)
    }
    pub fn with_clients(clients: Vec<Box<dyn VendorClient>>, limit: u32) -> Self {
        Self {
            clients,
            paid: AtomicU32::new(0),
            limit,
        }
    }
    pub fn paid_tasks(&self) -> u32 {
        self.paid.load(Ordering::Relaxed)
    }
}

impl CaptchaBackend for ProviderChain {
    fn is_configured(&self) -> bool {
        !self.clients.is_empty()
    }

    async fn solve(
        &self,
        task: &SolveTask,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Result<ProviderAnswer, ProviderError> {
        let mut errors = Vec::new();
        let mut charged = false;
        let mut prior_cost = 0.0;
        for (index, client) in self.clients.iter().enumerate() {
            if !client.supports(task) {
                continue;
            }
            let provider = client.name();
            let started = Instant::now();
            if cancel.is_cancelled()
                || deadline.saturating_duration_since(started) < MIN_SUBMISSION_BUDGET
            {
                errors.push("provider attempt cancelled or insufficient time remaining".into());
                return Err(ProviderError {
                    provider: Some(provider),
                    kind: if cancel.is_cancelled() {
                        ProviderErrorKind::Cancelled
                    } else {
                        ProviderErrorKind::Timeout
                    },
                    message: errors.join("; "),
                    charged,
                });
            }
            let result = async {
                let mut slot = PaidSlot::reserve(&self.paid, self.limit).ok_or_else(|| ProviderError {provider:Some(provider),kind:ProviderErrorKind::SpendLimit,message:"paid-task limit reached".into(),charged:false})?;
                tracing::debug!(target: "captcha", provider = ?provider, task_type = task.label(), "provider task started");
                // Keep the reservation if the caller drops this future after submission.
                slot.charged = true;
                let result = client.solve(task, deadline.min(started + client.attempt_budget(task)), cancel).await;
                if let Err(error) = &result { slot.charged = error.charged; }
                tracing::debug!(target: "captcha", provider = ?provider, task_type = task.label(), latency_ms = started.elapsed().as_millis() as u64, success = result.is_ok(), "provider task finished");
                result
            }.await;
            match result {
                Ok(mut answer) => {
                    answer.cost_usd = Some(prior_cost + answer.cost_usd.unwrap_or(0.0));
                    return Ok(answer);
                }
                Err(mut error) => {
                    charged |= error.charged;
                    if error.charged {
                        prior_cost += estimate(task);
                    }
                    errors.push(format!("{provider:?} {:?}: {}", error.kind, error.message));
                    let next = self
                        .clients
                        .iter()
                        .skip(index + 1)
                        .any(|client| client.supports(task));
                    if error.kind.may_fallback() && next && self.paid_tasks() >= self.limit {
                        error.kind = ProviderErrorKind::SpendLimit;
                        errors.push("paid-task limit reached".into());
                    }
                    if !error.kind.may_fallback() || !next {
                        error.message = errors.join("; ");
                        error.charged = charged;
                        return Err(error);
                    }
                }
            }
        }
        Err(ProviderError {
            provider: None,
            kind: ProviderErrorKind::NotConfigured,
            message: "no configured provider supports this task".into(),
            charged: false,
        })
    }
}

struct PaidSlot<'a> {
    paid: &'a AtomicU32,
    charged: bool,
}

impl<'a> PaidSlot<'a> {
    fn reserve(paid: &'a AtomicU32, limit: u32) -> Option<Self> {
        paid.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
            (count < limit).then(|| count + 1)
        })
        .ok()?;
        Some(Self {
            paid,
            charged: false,
        })
    }
}

impl Drop for PaidSlot<'_> {
    fn drop(&mut self) {
        if !self.charged {
            self.paid.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

pub struct CapsolverClient {
    config: ProviderConfig,
    http: Option<Client>,
}

impl CapsolverClient {
    pub fn new(config: &ProviderConfig, http: Client) -> Self {
        Self {
            config: config.clone(),
            http: Some(http),
        }
    }

    fn error(
        &self,
        kind: ProviderErrorKind,
        message: impl Into<String>,
        charged: bool,
    ) -> ProviderError {
        ProviderError {
            provider: Some(ProviderName::Capsolver),
            kind,
            message: message.into(),
            charged,
        }
    }

    fn check_budget(
        &self,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Result<(), ProviderError> {
        if cancel.is_cancelled() {
            return Err(self.error(ProviderErrorKind::Cancelled, "cancelled", false));
        }
        if Instant::now() >= deadline {
            return Err(self.error(
                ProviderErrorKind::Timeout,
                "provider deadline exceeded",
                false,
            ));
        }
        Ok(())
    }

    fn task_payload(&self, task: &SolveTask) -> Result<Value, ProviderError> {
        let bad = || {
            self.error(
                ProviderErrorKind::BadRequest,
                "missing or invalid CAPTCHA parameters",
                false,
            )
        };
        Ok(match task {
            SolveTask::RecaptchaTiles(puzzle) => {
                if puzzle.image_base64.trim().is_empty() || puzzle.image_base64.starts_with("data:")
                {
                    return Err(bad());
                }
                // https://docs.capsolver.com/en/guide/recognition/ReCaptchaClassification/
                let id = puzzle
                    .question_id
                    .as_deref()
                    .filter(|id| id.starts_with("/m/") && id.len() > 3)
                    .ok_or_else(|| {
                        self.error(
                            ProviderErrorKind::NotConfigured,
                            "image classification requires a question ID",
                            false,
                        )
                    })?;
                json!({"type":"ReCaptchaV2Classification","image":puzzle.image_base64,"question":id})
            }
            SolveTask::RecaptchaToken(params) => {
                validate_site(&params.website_url, &params.website_key).map_err(|()| bad())?;
                // https://docs.capsolver.com/en/guide/captcha/ReCaptchaV2/
                let kind = if params.enterprise {
                    "ReCaptchaV2EnterpriseTaskProxyLess"
                } else {
                    "ReCaptchaV2TaskProxyLess"
                };
                let mut payload = json!({"type":kind,"websiteURL":params.website_url,"websiteKey":params.website_key,"isInvisible":params.invisible});
                if params.enterprise {
                    if let Some(s) = &params.enterprise_s {
                        payload["enterprisePayload"] = json!({"s":s});
                    }
                } else {
                    optional(&mut payload, "recaptchaDataSValue", &params.enterprise_s);
                }
                optional(&mut payload, "pageAction", &params.page_action);
                payload
            }
            SolveTask::TurnstileToken(params) => {
                validate_site(&params.website_url, &params.website_key).map_err(|()| bad())?;
                // https://docs.capsolver.com/en/guide/captcha/cloudflare_turnstile/
                let mut metadata = json!({});
                optional(&mut metadata, "action", &params.action);
                optional(&mut metadata, "cdata", &params.cdata);
                json!({"type":"AntiTurnstileTaskProxyLess","websiteURL":params.website_url,"websiteKey":params.website_key,"metadata":metadata})
            }
        })
    }

    async fn solve_task(
        &self,
        task: &SolveTask,
        payload: Value,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Result<ProviderAnswer, ProviderError> {
        let mut response = self
            .request(
                "createTask",
                json!({"task":payload}),
                Some(task),
                deadline,
                cancel,
            )
            .await?;
        let task_id = response
            .get("taskId")
            .filter(|id| valid_task_id(id))
            .cloned();
        let mut cost = amount(&response["cost"]);
        let mut delay = Duration::from_secs(1);
        loop {
            if cancel.is_cancelled() {
                return Err(self.error(ProviderErrorKind::Cancelled, "cancelled", true));
            }
            if response["status"] == "ready"
                || (response["status"].is_null()
                    && response.get("solution").is_some_and(Value::is_object))
            {
                let answer = normalize(task, &response["solution"]).map_err(|()| {
                    self.error(
                        ProviderErrorKind::BadRequest,
                        "malformed CAPTCHA solution",
                        true,
                    )
                })?;
                return Ok(ProviderAnswer {
                    answer,
                    provider: ProviderName::Capsolver,
                    task_id: task_id.as_ref().map(|id| {
                        self.redact(
                            id.as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| id.to_string()),
                            Some(task),
                        )
                    }),
                    cost_usd: Some(cost.unwrap_or_else(|| estimate(task))),
                });
            }
            let id = task_id.as_ref().ok_or_else(|| {
                self.error(
                    ProviderErrorKind::Uncertain,
                    "createTask returned no task ID or solution",
                    true,
                )
            })?;
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(self.error(ProviderErrorKind::Cancelled, "cancelled", true)),
                _ = tokio::time::sleep_until(deadline) => return Err(self.error(ProviderErrorKind::Timeout, "provider deadline exceeded", true)),
                _ = tokio::time::sleep(delay) => {}
            }
            response = self
                .request(
                    "getTaskResult",
                    json!({"taskId":id}),
                    Some(task),
                    deadline,
                    cancel,
                )
                .await
                .map_err(|mut error| {
                    error.charged = true;
                    error
                })?;
            cost = amount(&response["cost"]).or(cost);
            if !matches!(
                response["status"].as_str(),
                Some("idle" | "processing" | "ready")
            ) {
                return Err(self.error(ProviderErrorKind::BadRequest, "invalid task status", true));
            }
            delay = Duration::from_secs(2);
        }
    }

    pub async fn get_balance(&self) -> Result<f64, ProviderError> {
        let response = self
            .request(
                "getBalance",
                json!({}),
                None,
                Instant::now() + ATTEMPT_TIMEOUT,
                &CancellationToken::new(),
            )
            .await?;
        amount(&response["balance"])
            .ok_or_else(|| self.error(ProviderErrorKind::BadRequest, "invalid balance", false))
    }

    async fn request(
        &self,
        method: &str,
        mut body: Value,
        task: Option<&SolveTask>,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Result<Value, ProviderError> {
        self.check_budget(deadline, cancel)?;
        let creating = method == "createTask";
        let failed = || {
            self.error(
                if creating {
                    ProviderErrorKind::Uncertain
                } else {
                    ProviderErrorKind::Transport
                },
                "provider response could not be read",
                creating,
            )
        };
        let http = self.http.as_ref().ok_or_else(|| {
            self.error(
                ProviderErrorKind::Transport,
                "HTTP client initialization failed",
                false,
            )
        })?;
        let endpoint = format!(
            "{}/{method}",
            self.config.base_url.as_str().trim_end_matches('/')
        );
        body["clientKey"] = json!(self.config.key.expose());
        let send = async {
            let response = http
                .post(endpoint)
                .timeout(deadline.saturating_duration_since(Instant::now()))
                .json(&body)
                .send()
                .await
                .map_err(|error| {
                    if error.is_connect() {
                        self.error(
                            ProviderErrorKind::Transport,
                            "provider connection failed",
                            false,
                        )
                    } else if error.is_timeout() {
                        self.error(
                            if creating {
                                ProviderErrorKind::Uncertain
                            } else {
                                ProviderErrorKind::Timeout
                            },
                            "provider request timed out",
                            creating,
                        )
                    } else {
                        failed()
                    }
                })?;
            let status = response.status();
            let value = response.json::<Value>().await;
            if let Ok(value) = &value
                && value["errorId"].as_i64().is_some_and(|id| id != 0)
            {
                let code = value["errorCode"].as_str().unwrap_or("VENDOR_ERROR");
                let description = value["errorDescription"]
                    .as_str()
                    .unwrap_or("request rejected");
                let kind = classify(status, code);
                let message = self.redact(
                    format!("{code}: {description} (HTTP {})", status.as_u16()),
                    task,
                );
                return Err(self.error(kind, message, false));
            }
            if !status.is_success() {
                let kind = classify(status, "");
                let uncertain = creating && status.is_server_error();
                return Err(self.error(
                    if uncertain {
                        ProviderErrorKind::Uncertain
                    } else {
                        kind
                    },
                    format!("provider HTTP {}", status.as_u16()),
                    uncertain,
                ));
            }
            let value = value.map_err(|_| failed())?;
            if value["errorId"].as_u64() != Some(0) {
                return Err(failed());
            }
            Ok(value)
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(self.error(ProviderErrorKind::Cancelled, "cancelled", creating)),
            _ = tokio::time::sleep_until(deadline) => Err(self.error(if creating {ProviderErrorKind::Uncertain} else {ProviderErrorKind::Timeout}, "provider deadline exceeded", creating)),
            result = send => result,
        }
    }

    fn redact(&self, mut text: String, task: Option<&SolveTask>) -> String {
        let mut secrets = vec![self.config.key.expose()];
        match task {
            Some(SolveTask::RecaptchaTiles(puzzle)) => secrets.push(&puzzle.image_base64),
            Some(SolveTask::RecaptchaToken(params)) => {
                secrets.extend(params.enterprise_s.as_deref())
            }
            Some(SolveTask::TurnstileToken(params)) => secrets.extend(params.cdata.as_deref()),
            None => {}
        }
        for secret in secrets.into_iter().filter(|secret| !secret.is_empty()) {
            text = text.replace(secret, "<redacted>");
        }
        text
    }
}
impl VendorClient for CapsolverClient {
    fn name(&self) -> ProviderName {
        ProviderName::Capsolver
    }
    fn supports(&self, task: &SolveTask) -> bool {
        !matches!(task, SolveTask::RecaptchaTiles(puzzle)
            if !puzzle.question_id.as_deref().is_some_and(|id| id.starts_with("/m/") && id.len()>3))
    }
    fn solve<'a>(
        &'a self,
        task: &'a SolveTask,
        deadline: Instant,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ProviderAnswer, ProviderError>> {
        Box::pin(async move {
            let payload = self.task_payload(task)?;
            self.solve_task(task, payload, deadline, cancel).await
        })
    }
}

fn validate_site(url: &str, key: &str) -> Result<(), ()> {
    let url = url::Url::parse(url).map_err(|_| ())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || key.trim().is_empty()
    {
        return Err(());
    }
    Ok(())
}

pub(super) fn optional(payload: &mut Value, name: &str, value: &Option<String>) {
    if let Some(value) = value.as_ref().filter(|value| !value.trim().is_empty()) {
        payload[name] = json!(value);
    }
}

fn valid_task_id(id: &Value) -> bool {
    id.as_u64().is_some_and(|id| id > 0) || id.as_str().is_some_and(|id| !id.trim().is_empty())
}

fn amount(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|amount| amount.is_finite() && *amount >= 0.0)
}

fn normalize(task: &SolveTask, solution: &Value) -> Result<SolveAnswer, ()> {
    if let SolveTask::RecaptchaTiles(puzzle) = task {
        if let Some(size) = solution.get("size") {
            let expected = match puzzle.grid {
                TileGrid::Single => 1,
                TileGrid::ThreeByThree => 3,
                TileGrid::FourByFour => 4,
            };
            if size.as_u64() != Some(expected) {
                return Err(());
            }
        }
        let mut indexes = if puzzle.grid == TileGrid::Single && solution.get("hasObject").is_some()
        {
            if solution["hasObject"].as_bool().ok_or(())? {
                vec![0]
            } else {
                vec![]
            }
        } else {
            let objects = solution["objects"].as_array().ok_or(())?;
            if objects.len() > puzzle.grid.tiles() {
                return Err(());
            }
            objects
                .iter()
                .map(|value| usize::try_from(value.as_u64().ok_or(())?).map_err(|_| ()))
                .collect::<Result<Vec<_>, _>>()?
        };
        if indexes.iter().any(|&index| index >= puzzle.grid.tiles()) {
            return Err(());
        }
        indexes.sort_unstable();
        if indexes.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(());
        }
        Ok(SolveAnswer::Tiles(indexes))
    } else {
        let field = if matches!(task, SolveTask::RecaptchaToken(_)) {
            "gRecaptchaResponse"
        } else {
            "token"
        };
        let token = solution[field]
            .as_str()
            .filter(|token| !token.trim().is_empty())
            .ok_or(())?;
        Ok(SolveAnswer::Token(token.to_owned()))
    }
}

fn classify(status: StatusCode, code: &str) -> ProviderErrorKind {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return ProviderErrorKind::RateLimited;
    }
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
        || matches!(
            code,
            "ERROR_KEY_DENIED_ACCESS"
                | "ERROR_KEY_DOES_NOT_EXIST"
                | "ERROR_WRONG_USER_KEY"
                | "ERROR_INVALID_KEY"
                | "ERROR_KEY_INVALID"
                | "ERROR_INVALID_CLIENT_KEY"
                | "ERROR_IP_NOT_ALLOWED"
        )
    {
        return ProviderErrorKind::Auth;
    }
    match code {
        "ERROR_ZERO_BALANCE" => ProviderErrorKind::NoBalance,
        "ERROR_CAPTCHA_UNSOLVABLE" => ProviderErrorKind::Unsolvable,
        "ERROR_TASK_NOT_SUPPORTED"
        | "ERROR_TYPE_NOT_SUPPORTED"
        | "ERROR_TASK_NOT_ALLOWED"
        | "ERROR_TASK_ABSENT"
        | "ERROR_BAD_PARAMETERS"
        | "ERROR_PARAMETERS" => ProviderErrorKind::Unsupported,
        _ if status.is_server_error() => ProviderErrorKind::Transport,
        _ => ProviderErrorKind::BadRequest,
    }
}

fn estimate(task: &SolveTask) -> f64 {
    // Static USD/task estimates; a vendor-reported cost takes precedence.
    match task {
        SolveTask::RecaptchaTiles(_) => 0.0004,
        SolveTask::RecaptchaToken(_) => 0.0008,
        SolveTask::TurnstileToken(_) => 0.0012,
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Arc, time::Duration};

    use axum::{
        Json, Router,
        extract::State,
        http::{StatusCode, Uri},
        routing::post,
    };
    use serde_json::{Value, json};
    use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

    use super::*;
    use crate::services::captcha::{
        config::{CAPSOLVER_KEY_FILE_ENV, CAPSOLVER_URL_ENV, PAID_TASK_LIMIT_ENV},
        types::{
            ProviderName, RecaptchaParams, SolveAnswer, TileGrid, TilePuzzle, TurnstileParams,
        },
    };

    const KEY: &str = "provider-unit-test-secret";
    const IMAGE: &str = "aW1hZ2UtdGhhdC1tdXN0LW5ldmVyLWJlLWxvZ2dlZA==";

    struct Reply {
        body: Value,
        status: StatusCode,
        delay: Duration,
    }

    impl From<Value> for Reply {
        fn from(body: Value) -> Self {
            Self {
                body,
                status: StatusCode::OK,
                delay: Duration::ZERO,
            }
        }
    }

    #[derive(Default)]
    struct MockState {
        replies: VecDeque<Reply>,
        requests: Vec<(String, Value, Instant)>,
    }

    struct MockVendor {
        url: String,
        state: Arc<Mutex<MockState>>,
        server: JoinHandle<()>,
    }

    impl Drop for MockVendor {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    async fn vendor(replies: Vec<Reply>) -> anyhow::Result<MockVendor> {
        async fn handle(
            State(state): State<Arc<Mutex<MockState>>>,
            uri: Uri,
            Json(body): Json<Value>,
        ) -> (StatusCode, Json<Value>) {
            let reply = {
                let mut state = state.lock().await;
                state
                    .requests
                    .push((uri.path().into(), body, Instant::now()));
                state.replies.pop_front().unwrap_or_else(|| Reply {
                    body: json!({"errorId":1, "errorCode":"UNEXPECTED_REQUEST"}),
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    delay: Duration::ZERO,
                })
            };
            tokio::time::sleep(reply.delay).await;
            (reply.status, Json(reply.body))
        }
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        let state = Arc::new(Mutex::new(MockState {
            replies: replies.into(),
            ..Default::default()
        }));
        let app = Router::new()
            .route("/{*method}", post(handle))
            .with_state(state.clone());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(MockVendor { url, state, server })
    }

    fn config(mock: Option<&MockVendor>, limit: u32) -> anyhow::Result<CaptchaConfig> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), KEY)?;
        Ok(CaptchaConfig::from_lookup(|name| match name {
            CAPSOLVER_KEY_FILE_ENV if mock.is_some() => Some(file.path().to_string_lossy().into()),
            CAPSOLVER_URL_ENV => mock.map(|mock| mock.url.clone()),
            PAID_TASK_LIMIT_ENV => Some(limit.to_string()),
            _ => None,
        }))
    }

    fn tiles(grid: TileGrid) -> SolveTask {
        SolveTask::RecaptchaTiles(TilePuzzle {
            image_base64: IMAGE.into(),
            question_id: Some("/m/015qff".into()),
            question_text: "Click on traffic lights".into(),
            grid,
        })
    }

    fn recaptcha() -> SolveTask {
        SolveTask::RecaptchaToken(RecaptchaParams {
            website_url: "https://example.test/captcha".into(),
            website_key: "site-key".into(),
            invisible: true,
            enterprise: false,
            enterprise_s: Some("data-s".into()),
            page_action: None,
            user_agent: Some("test-agent".into()),
        })
    }

    fn turnstile() -> SolveTask {
        SolveTask::TurnstileToken(TurnstileParams {
            website_url: "https://example.test/captcha".into(),
            website_key: "site-key".into(),
            action: Some("login".into()),
            cdata: Some("context".into()),
        })
    }

    async fn solve(
        chain: &ProviderChain,
        task: &SolveTask,
    ) -> Result<ProviderAnswer, ProviderError> {
        chain
            .solve(
                task,
                Instant::now() + Duration::from_secs(10),
                &CancellationToken::new(),
            )
            .await
    }

    #[tokio::test]
    async fn tests_that_capsolver_tiles_are_normalized_and_charged_once() -> anyhow::Result<()> {
        let mock = vendor(vec![
            json!({"errorId":0,"status":"ready","solution":{"objects":[8,0,4]},"cost":"0.0007"})
                .into(),
        ])
        .await?;
        let chain = ProviderChain::new(&config(Some(&mock), 100)?);
        let answer = solve(&chain, &tiles(TileGrid::ThreeByThree)).await?;
        assert_eq!(answer.answer, SolveAnswer::Tiles(vec![0, 4, 8]));
        assert_eq!(answer.provider, ProviderName::Capsolver);
        assert_eq!(answer.cost_usd, Some(0.0007));
        assert_eq!(chain.paid_tasks(), 1);
        let state = mock.state.lock().await;
        assert_eq!(state.requests.len(), 1);
        assert_eq!(
            state.requests[0].1["task"],
            json!({"type":"ReCaptchaV2Classification","image":IMAGE,"question":"/m/015qff"})
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_capsolver_keeps_base_url_path_prefixes() -> anyhow::Result<()> {
        for suffix in ["", "/"] {
            let mut mock = vendor(vec![
                json!({"errorId":0,"taskId":"job"}).into(),
                json!({"errorId":0,"status":"ready","solution":{"token":"answer"}}).into(),
            ])
            .await?;
            mock.url = format!("{}mock/capsolver{suffix}", mock.url);
            let chain = ProviderChain::new(&config(Some(&mock), 1)?);
            let answer = solve(&chain, &turnstile()).await?;
            assert_eq!(answer.answer, SolveAnswer::Token("answer".into()));
            let state = mock.state.lock().await;
            assert_eq!(state.requests[0].0, "/mock/capsolver/createTask");
            assert_eq!(state.requests[1].0, "/mock/capsolver/getTaskResult");
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_single_tile_has_object_is_a_valid_boolean_answer() -> anyhow::Result<()> {
        for (has_object, expected) in [(true, vec![0]), (false, vec![])] {
            let mock = vendor(vec![
                json!({"errorId":0,"status":"ready","solution":{"hasObject":has_object}}).into(),
            ])
            .await?;
            let chain = ProviderChain::new(&config(Some(&mock), 100)?);
            assert_eq!(
                solve(&chain, &tiles(TileGrid::Single)).await?.answer,
                SolveAnswer::Tiles(expected)
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_capsolver_alone_is_not_configured_for_text_only_tiles() -> anyhow::Result<()>
    {
        let mock = vendor(vec![]).await?;
        let chain = ProviderChain::new(&config(Some(&mock), 1)?);
        let SolveTask::RecaptchaTiles(mut puzzle) = tiles(TileGrid::Single) else {
            anyhow::bail!("wrong test task")
        };
        puzzle.question_id = None;
        let Err(error) = solve(&chain, &SolveTask::RecaptchaTiles(puzzle)).await else {
            anyhow::bail!("text-only task was submitted")
        };
        assert_eq!(error.kind, ProviderErrorKind::NotConfigured);
        assert!(mock.state.lock().await.requests.is_empty());
        assert_eq!(chain.paid_tasks(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_processing_solutions_are_not_returned_before_ready() -> anyhow::Result<()> {
        let mock = vendor(vec![json!({"errorId":0,"taskId":"job","status":"processing","solution":{"token":"partial"}}).into(), json!({"errorId":0,"status":"ready","solution":{"token":"final"}}).into()]).await?;
        let chain = ProviderChain::new(&config(Some(&mock), 1)?);
        assert_eq!(
            solve(&chain, &turnstile()).await?.answer,
            SolveAnswer::Token("final".into())
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_idle_tasks_keep_polling_until_ready() -> anyhow::Result<()> {
        let mock = vendor(vec![
            json!({"errorId":0,"taskId":"queued","status":"idle"}).into(),
            json!({"errorId":0,"status":"idle"}).into(),
            json!({"errorId":0,"status":"ready","solution":{"gRecaptchaResponse":"ready-token"}})
                .into(),
        ])
        .await?;
        let chain = ProviderChain::new(&config(Some(&mock), 1)?);
        let answer = solve(&chain, &recaptcha()).await?;
        assert_eq!(answer.answer, SolveAnswer::Token("ready-token".into()));
        assert_eq!(chain.paid_tasks(), 1);
        let state = mock.state.lock().await;
        assert_eq!(state.requests.len(), 3);
        assert_eq!(state.requests[2].0, "/getTaskResult");
        assert_eq!(state.requests[2].1["taskId"], "queued");
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_lost_create_response_is_charged_and_never_retried() -> anyhow::Result<()>
    {
        let primary = vendor(vec![Reply {
            body: json!({"errorId":0,"taskId":"accepted"}),
            status: StatusCode::OK,
            delay: Duration::from_secs(2),
        }])
        .await?;
        let provider = config(Some(&primary), 100)?
            .capsolver
            .ok_or_else(|| anyhow::anyhow!("missing provider"))?;
        let chain = ProviderChain::with_clients(
            vec![Box::new(BudgetVendor(
                CapsolverClient::new(&provider, Client::new()),
                Duration::from_millis(150),
            ))],
            100,
        );
        let result = chain
            .solve(
                &recaptcha(),
                Instant::now() + Duration::from_secs(10),
                &CancellationToken::new(),
            )
            .await;
        let Err(error) = result else {
            anyhow::bail!("lost createTask response was accepted")
        };
        assert_eq!(error.kind, ProviderErrorKind::Uncertain);
        assert!(error.charged);
        assert_eq!(chain.paid_tasks(), 1);
        assert_eq!(primary.state.lock().await.requests.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_cancellation_mid_poll_keeps_the_paid_count() -> anyhow::Result<()> {
        let primary = vendor(vec![json!({"errorId":0,"taskId":"accepted"}).into()]).await?;
        let chain = ProviderChain::new(&config(Some(&primary), 100)?);
        let cancel = CancellationToken::new();
        let stopped = cancel.clone();
        let interrupt = async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            stopped.cancel();
        };
        let task = recaptcha();
        let (result, ()) = tokio::join!(
            chain.solve(&task, Instant::now() + Duration::from_secs(10), &cancel),
            interrupt
        );
        let Err(error) = result else {
            anyhow::bail!("cancelled task succeeded")
        };
        assert_eq!(error.kind, ProviderErrorKind::Cancelled);
        assert!(error.charged);
        assert_eq!(chain.paid_tasks(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_the_spend_limit_is_atomic_across_concurrent_solves() -> anyhow::Result<()> {
        let mock = vendor(vec![
            json!({"errorId":0,"status":"ready","solution":{"hasObject":true}}).into(),
        ])
        .await?;
        let chain = ProviderChain::new(&config(Some(&mock), 1)?);
        let task = tiles(TileGrid::Single);
        let (a, b) = tokio::join!(solve(&chain, &task), solve(&chain, &task));
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        let error = match (a, b) {
            (Err(error), _) | (_, Err(error)) => error,
            _ => anyhow::bail!("spend limit was ignored"),
        };
        assert_eq!(error.kind, ProviderErrorKind::SpendLimit);
        assert_eq!(chain.paid_tasks(), 1);
        assert_eq!(mock.state.lock().await.requests.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_deadlines_stop_polling_and_expired_calls_do_not_submit()
    -> anyhow::Result<()> {
        let mock = vendor(vec![json!({"errorId":0,"taskId":"accepted"}).into()]).await?;
        let provider = config(Some(&mock), 100)?
            .capsolver
            .ok_or_else(|| anyhow::anyhow!("missing provider"))?;
        let chain = ProviderChain::with_clients(
            vec![Box::new(BudgetVendor(
                CapsolverClient::new(&provider, Client::new()),
                Duration::from_millis(100),
            ))],
            100,
        );
        for elapsed in [true, false] {
            let deadline = Instant::now()
                + if elapsed {
                    Duration::ZERO
                } else {
                    Duration::from_secs(10)
                };
            let Err(error) = chain
                .solve(&recaptcha(), deadline, &CancellationToken::new())
                .await
            else {
                anyhow::bail!("deadline ignored")
            };
            assert_eq!(error.kind, ProviderErrorKind::Timeout);
            assert_eq!(error.charged, !elapsed);
        }
        assert_eq!(mock.state.lock().await.requests.len(), 1);
        Ok(())
    }

    #[test]
    fn tests_that_puzzle_debug_does_not_expose_image_data() {
        let debug = format!("{:?}", tiles(TileGrid::Single));
        assert!(!debug.contains(IMAGE));
        assert!(debug.contains("base64 chars>"));
    }

    struct FakeVendor {
        code: &'static str,
        failure: Option<ProviderErrorKind>,
        charged: bool,
        supported: bool,
        calls: Arc<AtomicU32>,
    }

    impl VendorClient for FakeVendor {
        fn name(&self) -> ProviderName {
            ProviderName::Capsolver
        }
        fn supports(&self, _task: &SolveTask) -> bool {
            self.supported
        }
        fn solve<'a>(
            &'a self,
            _task: &'a SolveTask,
            _deadline: Instant,
            _cancel: &'a CancellationToken,
        ) -> futures_util::future::BoxFuture<'a, Result<ProviderAnswer, ProviderError>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::Relaxed);
                match self.failure {
                    Some(kind) => Err(ProviderError {
                        provider: Some(self.name()),
                        kind,
                        message: self.code.into(),
                        charged: self.charged,
                    }),
                    None => Ok(ProviderAnswer {
                        provider: self.name(),
                        answer: SolveAnswer::Token("fake-token".into()),
                        task_id: None,
                        cost_usd: Some(0.001),
                    }),
                }
            })
        }
    }

    fn fake(
        code: &'static str,
        failure: Option<ProviderErrorKind>,
        charged: bool,
        supported: bool,
    ) -> (Box<dyn VendorClient>, Arc<AtomicU32>) {
        let calls = Arc::new(AtomicU32::new(0));
        (
            Box::new(FakeVendor {
                code,
                failure,
                charged,
                supported,
                calls: calls.clone(),
            }),
            calls,
        )
    }

    #[tokio::test]
    async fn tests_that_the_generic_chain_falls_back_to_a_fake_vendor() -> anyhow::Result<()> {
        let (primary, first) = fake("PRIMARY_AUTH", Some(ProviderErrorKind::Auth), false, true);
        let (backup, second) = fake("FAKE_BACKUP", None, true, true);
        let chain = ProviderChain::with_clients(vec![primary, backup], 1);
        let answer = solve(&chain, &recaptcha()).await?;
        assert_eq!(answer.answer, SolveAnswer::Token("fake-token".into()));
        assert_eq!(first.load(Ordering::Relaxed), 1);
        assert_eq!(second.load(Ordering::Relaxed), 1);
        assert_eq!(chain.paid_tasks(), 1);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_short_deadlines_skip_paid_attempts_before_reserving() -> anyhow::Result<()>
    {
        for (remaining_ms, limit, succeeds) in [
            (0, 1, false),
            (4999, 0, false),
            (4999, 1, false),
            (5000, 1, true),
        ] {
            let (client, calls) = fake("READY", None, true, true);
            let (backup, backup_calls) = fake("BACKUP", None, true, true);
            let chain = ProviderChain::with_clients(vec![client, backup], limit);
            let result = chain
                .solve(
                    &recaptcha(),
                    Instant::now() + Duration::from_millis(remaining_ms),
                    &CancellationToken::new(),
                )
                .await;
            if succeeds {
                assert_eq!(result?.answer, SolveAnswer::Token("fake-token".into()));
            } else {
                let error = result
                    .err()
                    .ok_or_else(|| anyhow::anyhow!("late task was submitted"))?;
                assert_eq!(
                    (error.kind, error.charged),
                    (ProviderErrorKind::Timeout, false)
                );
            }
            assert_eq!(calls.load(Ordering::Relaxed), u32::from(succeeds));
            assert_eq!(backup_calls.load(Ordering::Relaxed), 0);
            assert_eq!(chain.paid_tasks(), u32::from(succeeds));
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_short_deadlines_never_reach_the_mock_vendor() -> anyhow::Result<()> {
        let mock = vendor(vec![]).await?;
        let chain = ProviderChain::new(&config(Some(&mock), 1)?);
        let error = chain
            .solve(
                &recaptcha(),
                Instant::now() + Duration::from_millis(4999),
                &CancellationToken::new(),
            )
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("late task was submitted"))?;
        assert_eq!(
            (error.kind, error.charged),
            (ProviderErrorKind::Timeout, false)
        );
        assert!(mock.state.lock().await.requests.is_empty());
        assert_eq!(chain.paid_tasks(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_the_generic_chain_keeps_every_error_code() -> anyhow::Result<()> {
        let (primary, _) = fake("PRIMARY_AUTH", Some(ProviderErrorKind::Auth), false, true);
        let (backup, _) = fake(
            "BACKUP_EMPTY",
            Some(ProviderErrorKind::NoBalance),
            false,
            true,
        );
        let chain = ProviderChain::with_clients(vec![primary, backup], 2);
        let Err(error) = solve(&chain, &recaptcha()).await else {
            anyhow::bail!("expected failure")
        };
        assert_eq!(error.kind, ProviderErrorKind::NoBalance);
        assert!(error.message.contains("PRIMARY_AUTH"));
        assert!(error.message.contains("BACKUP_EMPTY"));
        assert_eq!(chain.paid_tasks(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_unsupported_vendors_are_skipped_without_an_attempt() -> anyhow::Result<()> {
        let (primary, first) = fake("UNSUPPORTED", None, true, false);
        let (backup, second) = fake("SUPPORTED", None, true, true);
        let chain = ProviderChain::with_clients(vec![primary, backup], 1);
        assert!(solve(&chain, &recaptcha()).await.is_ok());
        assert_eq!(first.load(Ordering::Relaxed), 0);
        assert_eq!(second.load(Ordering::Relaxed), 1);
        assert_eq!(chain.paid_tasks(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_terminal_errors_or_exhausted_budget_never_fall_back() -> anyhow::Result<()>
    {
        for (kind, charged, limit, expected) in [
            (
                ProviderErrorKind::Cancelled,
                false,
                2,
                ProviderErrorKind::Cancelled,
            ),
            (
                ProviderErrorKind::Uncertain,
                true,
                2,
                ProviderErrorKind::Uncertain,
            ),
            (
                ProviderErrorKind::Unsolvable,
                true,
                1,
                ProviderErrorKind::SpendLimit,
            ),
        ] {
            let (primary, _) = fake("PRIMARY_FAILED", Some(kind), charged, true);
            let (backup, second) = fake("BACKUP", None, true, true);
            let chain = ProviderChain::with_clients(vec![primary, backup], limit);
            let Err(error) = solve(&chain, &recaptcha()).await else {
                anyhow::bail!("expected failure")
            };
            assert_eq!(error.kind, expected);
            assert_eq!(second.load(Ordering::Relaxed), 0);
            assert_eq!(chain.paid_tasks(), u32::from(charged));
            assert!(error.message.contains("PRIMARY_FAILED"));
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_vendor_errors_are_classified_without_charging_rejected_tasks()
    -> anyhow::Result<()> {
        for (code, status, expected) in [
            (
                "ERROR_KEY_DOES_NOT_EXIST",
                StatusCode::OK,
                ProviderErrorKind::Auth,
            ),
            (
                "ERROR_TASK_NOT_SUPPORTED",
                StatusCode::OK,
                ProviderErrorKind::Unsupported,
            ),
            (
                "ERROR_CAPTCHA_UNSOLVABLE",
                StatusCode::OK,
                ProviderErrorKind::Unsolvable,
            ),
            (
                "ERROR_ZERO_BALANCE",
                StatusCode::OK,
                ProviderErrorKind::NoBalance,
            ),
            (
                "SLOW_DOWN",
                StatusCode::TOO_MANY_REQUESTS,
                ProviderErrorKind::RateLimited,
            ),
        ] {
            let mock = vendor(vec![Reply {
                body: json!({"errorId":1,"errorCode":code,"errorDescription":"rejected"}),
                status,
                delay: Duration::ZERO,
            }])
            .await?;
            let chain = ProviderChain::new(&config(Some(&mock), 100)?);
            let Err(error) = solve(&chain, &recaptcha()).await else {
                anyhow::bail!("rejected task succeeded")
            };
            assert_eq!(error.kind, expected);
            assert!(error.message.contains(code));
            assert!(error.message.contains(&format!("HTTP {}", status.as_u16())));
            assert!(!error.charged);
            assert_eq!(chain.paid_tasks(), 0);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_tokens_poll_until_ready_and_use_documented_fields() -> anyhow::Result<()> {
        for task in [recaptcha(), turnstile()] {
            let mock=vendor(vec![json!({"errorId":0,"taskId":"job"}).into(),json!({"errorId":0,"status":"processing"}).into(),json!({"errorId":0,"status":"ready","cost":0.002,"solution":{"gRecaptchaResponse":"secret-token","token":"secret-token"}}).into()]).await?;
            let chain = ProviderChain::new(&config(Some(&mock), 1)?);
            let result = solve(&chain, &task).await?;
            assert_eq!(result.answer, SolveAnswer::Token("secret-token".into()));
            assert_eq!(result.cost_usd, Some(0.002));
            assert!(!format!("{result:?}").contains("secret-token"));
            let state = mock.state.lock().await;
            assert_eq!(state.requests.len(), 3);
            assert_eq!(state.requests[1].1["taskId"], "job");
            assert!(
                state.requests[1].2.duration_since(state.requests[0].2)
                    >= Duration::from_millis(900)
            );
            assert!(
                state.requests[2].2.duration_since(state.requests[1].2)
                    >= Duration::from_millis(1900)
            );
            let sent = &state.requests[0].1["task"];
            assert_eq!(sent["websiteKey"], "site-key");
            assert_eq!(sent["websiteURL"], "https://example.test/captcha");
            if matches!(task, SolveTask::RecaptchaToken(_)) {
                assert_eq!(sent["type"], "ReCaptchaV2TaskProxyLess");
                assert_eq!(sent["recaptchaDataSValue"], "data-s");
                assert_eq!(sent["isInvisible"], true);
            } else {
                assert_eq!(sent["type"], "AntiTurnstileTaskProxyLess");
                assert_eq!(
                    sent["metadata"],
                    json!({"action":"login","cdata":"context"})
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_enterprise_payload_uses_the_documented_variant() -> anyhow::Result<()> {
        let mock=vendor(vec![json!({"errorId":0,"status":"ready","solution":{"gRecaptchaResponse":"enterprise-token"}}).into()]).await?;
        let chain = ProviderChain::new(&config(Some(&mock), 1)?);
        let SolveTask::RecaptchaToken(mut params) = recaptcha() else {
            anyhow::bail!("wrong task")
        };
        params.enterprise = true;
        params.page_action = Some("verify".into());
        assert!(
            solve(&chain, &SolveTask::RecaptchaToken(params))
                .await
                .is_ok()
        );
        let state = mock.state.lock().await;
        let sent = &state.requests[0].1["task"];
        assert_eq!(sent["type"], "ReCaptchaV2EnterpriseTaskProxyLess");
        assert_eq!(sent["enterprisePayload"], json!({"s":"data-s"}));
        assert_eq!(sent["pageAction"], "verify");
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_malformed_answers_never_become_clicks_or_tokens() -> anyhow::Result<()> {
        for (task, solution) in [
            (tiles(TileGrid::ThreeByThree), json!({"objects":[9]})),
            (tiles(TileGrid::ThreeByThree), json!({"objects":[-1]})),
            (tiles(TileGrid::ThreeByThree), json!({"objects":[1.5]})),
            (tiles(TileGrid::ThreeByThree), json!({"objects":[0,0]})),
            (
                tiles(TileGrid::ThreeByThree),
                json!({"objects":[0],"size":4}),
            ),
            (
                tiles(TileGrid::ThreeByThree),
                json!({"hasObject":false,"size":0}),
            ),
            (recaptcha(), json!({"gRecaptchaResponse":" "})),
            (turnstile(), json!({"token":null})),
        ] {
            let mock = vendor(vec![
                json!({"errorId":0,"status":"ready","solution":solution}).into(),
            ])
            .await?;
            let chain = ProviderChain::new(&config(Some(&mock), 1)?);
            let Err(error) = solve(&chain, &task).await else {
                anyhow::bail!("malformed answer accepted")
            };
            assert_eq!(error.kind, ProviderErrorKind::BadRequest);
            assert!(error.charged);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_vendor_errors_redact_keys_and_images() -> anyhow::Result<()> {
        let mock=vendor(vec![json!({"errorId":1,"errorCode":"ERROR_KEY_DENIED_ACCESS","errorDescription":format!("denied {KEY} {IMAGE}")}).into()]).await?;
        let chain = ProviderChain::new(&config(Some(&mock), 1)?);
        let Err(error) = solve(&chain, &tiles(TileGrid::ThreeByThree)).await else {
            anyhow::bail!("expected auth error")
        };
        assert_eq!(error.kind, ProviderErrorKind::Auth);
        assert!(error.message.contains("ERROR_KEY_DENIED_ACCESS"));
        assert!(!format!("{error:?} {error}").contains(KEY));
        assert!(!format!("{error:?} {error}").contains(IMAGE));
        assert_eq!(chain.paid_tasks(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_dropping_a_submitted_future_keeps_the_paid_reservation()
    -> anyhow::Result<()> {
        let mock = vendor(vec![Reply {
            body: json!({"errorId":0,"taskId":"accepted"}),
            status: StatusCode::OK,
            delay: Duration::from_secs(5),
        }])
        .await?;
        let chain = ProviderChain::new(&config(Some(&mock), 1)?);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), solve(&chain, &recaptcha()))
                .await
                .is_err()
        );
        assert_eq!(mock.state.lock().await.requests.len(), 1);
        assert_eq!(chain.paid_tasks(), 1);
        let Err(error) = solve(&chain, &recaptcha()).await else {
            anyhow::bail!("cap was bypassed")
        };
        assert_eq!(error.kind, ProviderErrorKind::SpendLimit);
        Ok(())
    }
    struct SlowVendor(Duration);

    impl VendorClient for SlowVendor {
        fn name(&self) -> ProviderName {
            ProviderName::Capsolver
        }
        fn supports(&self, _task: &SolveTask) -> bool {
            true
        }
        fn solve<'a>(
            &'a self,
            task: &'a SolveTask,
            deadline: Instant,
            cancel: &'a CancellationToken,
        ) -> BoxFuture<'a, Result<ProviderAnswer, ProviderError>> {
            Box::pin(async move {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => Err(ProviderError { provider: Some(self.name()), kind: ProviderErrorKind::Cancelled, message: "cancelled".into(), charged: true }),
                    _ = tokio::time::sleep_until(deadline) => Err(ProviderError { provider: Some(self.name()), kind: ProviderErrorKind::Timeout, message: "deadline".into(), charged: true }),
                    _ = tokio::time::sleep(self.0) => Ok(ProviderAnswer {
                        provider: self.name(),
                        answer: if matches!(task,SolveTask::RecaptchaTiles(_)) { SolveAnswer::Tiles(vec![0]) } else { SolveAnswer::Token("delayed-token".into()) },
                        task_id: Some("slow".into()), cost_usd: Some(0.001),
                    }),
                }
            })
        }
    }

    struct BudgetVendor<T>(T, Duration);

    impl<T: VendorClient> VendorClient for BudgetVendor<T> {
        fn name(&self) -> ProviderName {
            self.0.name()
        }
        fn supports(&self, task: &SolveTask) -> bool {
            self.0.supports(task)
        }
        fn attempt_budget(&self, _task: &SolveTask) -> Duration {
            self.1
        }
        fn solve<'a>(
            &'a self,
            task: &'a SolveTask,
            deadline: Instant,
            cancel: &'a CancellationToken,
        ) -> BoxFuture<'a, Result<ProviderAnswer, ProviderError>> {
            self.0.solve(task, deadline, cancel)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_vendor_attempt_budgets_are_used_and_capped_by_the_overall_deadline()
    -> anyhow::Result<()> {
        for (task, delay, overall, elapsed, succeeds) in [
            (recaptcha(), 65, 120, 65, true),
            (turnstile(), 65, 120, 65, true),
            (recaptcha(), 95, 120, 90, false),
            (turnstile(), 95, 80, 80, false),
        ] {
            let chain = ProviderChain::with_clients(
                vec![Box::new(BudgetVendor(
                    SlowVendor(Duration::from_secs(delay)),
                    Duration::from_secs(90),
                ))],
                1,
            );
            let started = Instant::now();
            let result = chain
                .solve(
                    &task,
                    started + Duration::from_secs(overall),
                    &CancellationToken::new(),
                )
                .await;
            if succeeds {
                assert_eq!(result?.answer, SolveAnswer::Token("delayed-token".into()));
            } else {
                let error = result
                    .err()
                    .ok_or_else(|| anyhow::anyhow!("deadline ignored"))?;
                assert_eq!(
                    (error.kind, error.charged),
                    (ProviderErrorKind::Timeout, true)
                );
            }
            assert_eq!(started.elapsed(), Duration::from_secs(elapsed));
            assert_eq!(chain.paid_tasks(), 1);
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_late_failover_keeps_prior_charges_without_submitting_again()
    -> anyhow::Result<()> {
        for (overall, succeeds) in [(34, false), (35, true)] {
            let (backup, calls) = fake("BACKUP", None, true, true);
            let chain = ProviderChain::with_clients(
                vec![Box::new(SlowVendor(Duration::from_secs(35))), backup],
                2,
            );
            let started = Instant::now();
            let result = chain
                .solve(
                    &tiles(TileGrid::ThreeByThree),
                    started + Duration::from_secs(overall),
                    &CancellationToken::new(),
                )
                .await;
            if succeeds {
                assert!(result.is_ok());
            } else {
                let error = result
                    .err()
                    .ok_or_else(|| anyhow::anyhow!("late backup was submitted"))?;
                assert_eq!(
                    (error.kind, error.charged),
                    (ProviderErrorKind::Timeout, true)
                );
            }
            assert_eq!(calls.load(Ordering::Relaxed), u32::from(succeeds));
            assert_eq!(chain.paid_tasks(), 1 + u32::from(succeeds));
            assert_eq!(started.elapsed(), Duration::from_secs(30));
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_attempt_budgets_depend_on_task_and_respect_the_overall_deadline()
    -> anyhow::Result<()> {
        for (task, delay, overall, elapsed, succeeds) in [
            (tiles(TileGrid::ThreeByThree), 35, 120, 30, false),
            (recaptcha(), 35, 120, 35, true),
            (turnstile(), 35, 120, 35, true),
            (recaptcha(), 65, 120, 60, false),
            (turnstile(), 35, 20, 20, false),
        ] {
            let chain = ProviderChain::with_clients(
                vec![Box::new(SlowVendor(Duration::from_secs(delay)))],
                1,
            );
            let start = Instant::now();
            let result = chain
                .solve(
                    &task,
                    start + Duration::from_secs(overall),
                    &CancellationToken::new(),
                )
                .await;
            assert_eq!(result.is_ok(), succeeds, "{}", task.label());
            if !succeeds {
                assert_eq!(
                    result
                        .err()
                        .ok_or_else(|| anyhow::anyhow!("expected timeout"))?
                        .kind,
                    ProviderErrorKind::Timeout
                );
            }
            assert_eq!(start.elapsed(), Duration::from_secs(elapsed));
        }
        Ok(())
    }
}
