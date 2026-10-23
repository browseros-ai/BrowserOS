use std::{fmt, time::Duration};

use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{
    config::ProviderConfig,
    providers::{VendorClient, optional},
    types::{
        ProviderAnswer, ProviderError, ProviderErrorKind, ProviderName, SolveAnswer, SolveTask,
        TileGrid,
    },
};

pub struct TwoCaptchaClient {
    config: ProviderConfig,
    http: reqwest::Client,
    poll_interval: Duration,
}

impl fmt::Debug for TwoCaptchaClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TwoCaptchaClient(<redacted>)")
    }
}

#[derive(Clone, Copy)]
enum RequestKind {
    Create,
    Poll,
    Balance,
}

impl TwoCaptchaClient {
    pub fn new(config: &ProviderConfig) -> Result<Self, ProviderError> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|_| {
                error(
                    ProviderErrorKind::Transport,
                    "HTTP_CLIENT",
                    "could not initialize HTTP client",
                    false,
                )
            })?;
        Ok(Self {
            config: config.clone(),
            http,
            poll_interval: Duration::from_secs(5),
        })
    }

    // https://2captcha.com/api-docs/get-balance
    pub async fn get_balance(
        &self,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Result<f64, ProviderError> {
        let response = self
            .request(
                "getBalance",
                json!({}),
                RequestKind::Balance,
                deadline,
                cancel,
            )
            .await?;
        number(&response["balance"]).ok_or_else(|| {
            error(
                ProviderErrorKind::BadRequest,
                "INVALID_BALANCE",
                "missing or invalid balance",
                false,
            )
        })
    }

    async fn solve_task(
        &self,
        task: &SolveTask,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Result<ProviderAnswer, ProviderError> {
        check(deadline, cancel, false)?;
        let body = task_body(task)?;
        let created = self
            .request(
                "createTask",
                json!({"task": body}),
                RequestKind::Create,
                deadline,
                cancel,
            )
            .await?;
        let task_id = created["taskId"]
            .as_u64()
            .filter(|id| *id > 0)
            .ok_or_else(|| {
                error(
                    ProviderErrorKind::Uncertain,
                    "INVALID_TASK_ID",
                    "task submission may have been accepted",
                    true,
                )
            })?;
        loop {
            // https://2captcha.com/api-docs/get-task-result: wait at least five seconds between polls.
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(error(ProviderErrorKind::Cancelled, "CANCELLED", "solve cancelled", true)),
                _ = tokio::time::sleep_until(deadline) => return Err(error(ProviderErrorKind::Timeout, "DEADLINE", "solve deadline exceeded", true)),
                _ = tokio::time::sleep(self.poll_interval) => {}
            }
            let response = self
                .request(
                    "getTaskResult",
                    json!({"taskId":task_id}),
                    RequestKind::Poll,
                    deadline,
                    cancel,
                )
                .await?;
            match response["status"].as_str() {
                Some("processing") => continue,
                Some("ready") => {
                    return Ok(ProviderAnswer {
                        answer: answer(task, &response["solution"])?,
                        provider: ProviderName::TwoCaptcha,
                        task_id: Some(task_id.to_string()),
                        // Actual USD cost, not a guessed price: https://2captcha.com/api-docs/get-task-result
                        cost_usd: number(&response["cost"]),
                    });
                }
                _ => {
                    return Err(error(
                        ProviderErrorKind::BadRequest,
                        "INVALID_STATUS",
                        "missing or invalid task status",
                        true,
                    ));
                }
            }
        }
    }

    async fn request(
        &self,
        method: &str,
        mut body: Value,
        kind: RequestKind,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Result<Value, ProviderError> {
        let paid = matches!(kind, RequestKind::Poll);
        check(deadline, cancel, paid)?;
        let url = format!(
            "{}/{method}",
            self.config.base_url.as_str().trim_end_matches('/')
        );
        body["clientKey"] = json!(self.config.key.expose());
        let submitting = matches!(kind, RequestKind::Create);
        let operation = async {
            let response = self
                .http
                .post(url)
                .json(&body)
                .send()
                .await
                .map_err(|failure| {
                    let uncertain = submitting && !failure.is_connect();
                    error(
                        if uncertain {
                            ProviderErrorKind::Uncertain
                        } else {
                            ProviderErrorKind::Transport
                        },
                        "HTTP_TRANSPORT",
                        "API request failed",
                        paid || uncertain,
                    )
                })?;
            let status = response.status();
            if !status.is_success() {
                let (kind, code, description) = match status.as_u16() {
                    429 => (
                        ProviderErrorKind::RateLimited,
                        "HTTP_429",
                        "API rate limit reached",
                    ),
                    401 | 403 => (
                        ProviderErrorKind::Auth,
                        "HTTP_AUTH",
                        "API authentication rejected",
                    ),
                    400..=499 => (
                        ProviderErrorKind::BadRequest,
                        "HTTP_REJECTED",
                        "API request rejected",
                    ),
                    _ if submitting => (
                        ProviderErrorKind::Uncertain,
                        "HTTP_SERVER",
                        "task submission may have been accepted",
                    ),
                    _ => (
                        ProviderErrorKind::Transport,
                        "HTTP_SERVER",
                        "API server failed",
                    ),
                };
                return Err(error(
                    kind,
                    code,
                    description,
                    paid || kind == ProviderErrorKind::Uncertain,
                ));
            }
            let value: Value = response.json().await.map_err(|_| {
                error(
                    if submitting {
                        ProviderErrorKind::Uncertain
                    } else {
                        ProviderErrorKind::BadRequest
                    },
                    "INVALID_RESPONSE",
                    "invalid API response",
                    paid || submitting,
                )
            })?;
            match value["errorId"].as_u64() {
                Some(0) => Ok(value),
                Some(_) => {
                    let code = value["errorCode"].as_str().unwrap_or_default();
                    let mut error = vendor_error(code);
                    error.charged = paid && code != "ERROR_CAPTCHA_UNSOLVABLE";
                    Err(error)
                }
                None => Err(error(
                    if submitting {
                        ProviderErrorKind::Uncertain
                    } else {
                        ProviderErrorKind::BadRequest
                    },
                    "INVALID_RESPONSE",
                    "missing API error status",
                    paid || submitting,
                )),
            }
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(error(ProviderErrorKind::Cancelled, "CANCELLED", "request cancelled", paid || submitting)),
            _ = tokio::time::sleep_until(deadline) => Err(error(if submitting { ProviderErrorKind::Uncertain } else { ProviderErrorKind::Timeout }, "DEADLINE", "API deadline exceeded", paid || submitting)),
            result = operation => result,
        }
    }
}

impl VendorClient for TwoCaptchaClient {
    fn name(&self) -> ProviderName {
        ProviderName::TwoCaptcha
    }
    fn supports(&self, task: &SolveTask) -> bool {
        !matches!(task, SolveTask::RecaptchaTiles(p) if p.question_text.trim().is_empty())
    }
    fn attempt_budget(&self, task: &SolveTask) -> Duration {
        match task {
            SolveTask::RecaptchaTiles(_) => Duration::from_secs(30),
            SolveTask::RecaptchaToken(_) | SolveTask::TurnstileToken(_) => Duration::from_secs(90),
        }
    }
    fn solve<'a>(
        &'a self,
        task: &'a SolveTask,
        deadline: Instant,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ProviderAnswer, ProviderError>> {
        Box::pin(self.solve_task(task, deadline, cancel))
    }
}

fn task_body(task: &SolveTask) -> Result<Value, ProviderError> {
    let body = match task {
        // https://2captcha.com/api-docs/grid (imgType requires original files; our input is a screenshot).
        SolveTask::RecaptchaTiles(p) => {
            if p.question_text.trim().is_empty() {
                return Err(error(
                    ProviderErrorKind::Unsupported,
                    "MISSING_INSTRUCTIONS",
                    "grid tasks need instruction text",
                    false,
                ));
            }
            if p.image_base64.is_empty() {
                return Err(error(
                    ProviderErrorKind::BadRequest,
                    "MISSING_IMAGE",
                    "grid tasks need an image",
                    false,
                ));
            }
            let side = match p.grid {
                TileGrid::ThreeByThree => 3,
                TileGrid::FourByFour => 4,
                TileGrid::Single => 1,
            };
            json!({"type":"GridTask","body":p.image_base64,"comment":p.question_text,"rows":side,"columns":side,"canNoAnswer":1})
        }
        // https://2captcha.com/api-docs/recaptcha-v2
        // https://2captcha.com/api-docs/recaptcha-v2-enterprise
        SolveTask::RecaptchaToken(p) => {
            let mut body = json!({"type":if p.enterprise { "RecaptchaV2EnterpriseTaskProxyless" } else { "RecaptchaV2TaskProxyless" },"websiteURL":p.website_url,"websiteKey":p.website_key,"isInvisible":p.invisible});
            if let Some(s) = p.enterprise_s.as_ref().filter(|s| !s.trim().is_empty()) {
                if p.enterprise {
                    body["enterprisePayload"] = json!({"s":s});
                } else {
                    body["recaptchaDataSValue"] = json!(s);
                }
            }
            optional(&mut body, "userAgent", &p.user_agent);
            body
        }
        // https://2captcha.com/api-docs/cloudflare-turnstile
        SolveTask::TurnstileToken(p) => {
            let mut body = json!({"type":"TurnstileTaskProxyless","websiteURL":p.website_url,"websiteKey":p.website_key});
            optional(&mut body, "action", &p.action);
            optional(&mut body, "data", &p.cdata);
            body
        }
    };
    if !matches!(task, SolveTask::RecaptchaTiles(_)) {
        let url = body["websiteURL"]
            .as_str()
            .and_then(|s| url::Url::parse(s).ok());
        if !url.is_some_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
            || body["websiteKey"]
                .as_str()
                .is_none_or(|s| s.trim().is_empty())
        {
            return Err(error(
                ProviderErrorKind::BadRequest,
                "INVALID_PARAMETERS",
                "token tasks need an HTTP URL and sitekey",
                false,
            ));
        }
    }
    Ok(body)
}

fn answer(task: &SolveTask, solution: &Value) -> Result<SolveAnswer, ProviderError> {
    let invalid = || {
        error(
            ProviderErrorKind::BadRequest,
            "INVALID_SOLUTION",
            "invalid task solution",
            true,
        )
    };
    match task {
        SolveTask::RecaptchaTiles(p) => {
            let clicks = &solution["click"];
            if clicks.as_str() == Some("No_matching_images") {
                return Ok(SolveAnswer::Tiles(vec![]));
            }
            let mut indexes = Vec::new();
            for click in clicks.as_array().ok_or_else(invalid)? {
                let index = click
                    .as_u64()
                    .filter(|n| *n >= 1 && *n <= p.grid.tiles() as u64)
                    .ok_or_else(invalid)? as usize
                    - 1;
                if indexes.contains(&index) {
                    return Err(invalid());
                }
                indexes.push(index);
            }
            Ok(SolveAnswer::Tiles(indexes))
        }
        _ => {
            let field = if matches!(task, SolveTask::RecaptchaToken(_)) {
                "gRecaptchaResponse"
            } else {
                "token"
            };
            let token = solution[field]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(invalid)?;
            Ok(SolveAnswer::Token(token.into()))
        }
    }
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|n| n.is_finite() && *n >= 0.0)
}

fn check(
    deadline: Instant,
    cancel: &CancellationToken,
    charged: bool,
) -> Result<(), ProviderError> {
    if cancel.is_cancelled() {
        return Err(error(
            ProviderErrorKind::Cancelled,
            "CANCELLED",
            "solve cancelled",
            charged,
        ));
    }
    if Instant::now() >= deadline {
        return Err(error(
            ProviderErrorKind::Timeout,
            "DEADLINE",
            "solve deadline exceeded",
            charged,
        ));
    }
    Ok(())
}

fn error(kind: ProviderErrorKind, code: &str, description: &str, charged: bool) -> ProviderError {
    ProviderError {
        provider: Some(ProviderName::TwoCaptcha),
        kind,
        message: format!("{code}: {description}"),
        charged,
    }
}

fn vendor_error(code: &str) -> ProviderError {
    use ProviderErrorKind::*;
    // Use known descriptions: server-supplied strings can echo a key, token, or image.
    // https://2captcha.com/api-docs/error-codes (legacy WRONG_USER_KEY: https://2captcha.com/2captcha-api)
    let (kind, description) = match code {
        "ERROR_KEY_DOES_NOT_EXIST" | "ERROR_WRONG_USER_KEY" => (Auth, "API key is invalid"),
        "ERROR_IP_NOT_ALLOWED" | "ERROR_IP_BLOCKED" | "ERROR_ACCOUNT_SUSPENDED" => {
            (Auth, "API access denied")
        }
        "ERROR_ZERO_BALANCE" => (NoBalance, "account has no funds"),
        "ERROR_CAPTCHA_UNSOLVABLE" | "ERROR_BAD_DUPLICATES" => {
            (Unsolvable, "captcha could not be solved")
        }
        "ERROR_TASK_NOT_SUPPORTED" | "ERROR_NO_SUCH_METHOD" | "ERROR_IMAGE_TYPE_NOT_SUPPORTED" => {
            (Unsupported, "task is not supported")
        }
        "ERROR_NO_SLOT_AVAILABLE" => (RateLimited, "no worker capacity available"),
        "ERROR_BAD_PARAMETERS"
        | "ERROR_RECAPTCHA_INVALID_SITEKEY"
        | "ERROR_PAGEURL"
        | "ERROR_TASK_ABSENT"
        | "ERROR_ZERO_CAPTCHA_FILESIZE"
        | "ERROR_TOO_BIG_CAPTCHA_FILESIZE"
        | "ERROR_BAD_IMGINSTRUCTIONS"
        | "ERROR_NO_SUCH_CAPCHA_ID"
        | "ERROR_BAD_PROXY" => (BadRequest, "task parameters are invalid"),
        _ => {
            return error(
                BadRequest,
                "ERROR_PROVIDER",
                "unrecognized provider error",
                false,
            );
        }
    };
    error(kind, code, description, false)
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Arc, time::Duration};

    use axum::{
        Json, Router,
        extract::{Path, State},
        http::StatusCode,
        response::{IntoResponse, Response},
        routing::post,
    };
    use serde_json::{Value, json};
    use tokio::{
        sync::{Mutex, Notify},
        time::Instant,
    };
    use tokio_util::sync::CancellationToken;

    use super::super::{
        config::{CAPSOLVER_KEY_FILE_ENV, CAPSOLVER_URL_ENV, CaptchaConfig},
        providers::VendorClient,
        types::{
            ProviderAnswer, ProviderError, ProviderErrorKind, ProviderName, RecaptchaParams,
            SolveAnswer, SolveTask, TileGrid, TilePuzzle, TurnstileParams,
        },
    };
    use super::*;

    struct Reply(StatusCode, Value, Duration);
    struct MockState {
        replies: Mutex<VecDeque<Reply>>,
        calls: Mutex<Vec<(String, Value, Instant)>>,
        received: Notify,
    }
    struct Mock {
        state: Arc<MockState>,
        client: TwoCaptchaClient,
        server: tokio::task::JoinHandle<()>,
    }
    impl Drop for Mock {
        fn drop(&mut self) {
            self.server.abort();
        }
    }
    async fn respond(
        State(state): State<Arc<MockState>>,
        Path(method): Path<String>,
        Json(body): Json<Value>,
    ) -> Response {
        state
            .calls
            .lock()
            .await
            .push((method, body, Instant::now()));
        state.received.notify_one();
        let reply = state.replies.lock().await.pop_front();
        if let Some(Reply(status, body, delay)) = reply {
            tokio::time::sleep(delay).await;
            (status, Json(body)).into_response()
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({}))).into_response()
        }
    }
    async fn mock_server(replies: Vec<Value>) -> anyhow::Result<Mock> {
        let state = Arc::new(MockState {
            replies: Mutex::new(
                replies
                    .into_iter()
                    .map(|v| Reply(StatusCode::OK, v, Duration::ZERO))
                    .collect(),
            ),
            calls: Mutex::default(),
            received: Notify::new(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let router = Router::new()
            .route("/{*method}", post(respond))
            .with_state(state.clone());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        let key = tempfile::NamedTempFile::new()?;
        std::fs::write(key.path(), "secret-test-key")?;
        let config = CaptchaConfig::from_lookup(|name| match name {
            CAPSOLVER_KEY_FILE_ENV => Some(key.path().to_string_lossy().into_owned()),
            CAPSOLVER_URL_ENV => Some(url.clone()),
            _ => None,
        });
        let mut client = TwoCaptchaClient::new(
            config
                .capsolver
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing test config"))?,
        )?;
        client.poll_interval = Duration::from_millis(5);
        Ok(Mock {
            state,
            client,
            server,
        })
    }
    fn token_task(enterprise: bool) -> SolveTask {
        SolveTask::RecaptchaToken(RecaptchaParams {
            website_url: "https://example.test/".into(),
            website_key: "site-key".into(),
            invisible: true,
            enterprise,
            enterprise_s: Some("data-s".into()),
            page_action: Some("ignored-undocumented-action".into()),
            user_agent: Some("test-agent".into()),
        })
    }
    fn tiles(grid: TileGrid) -> SolveTask {
        SolveTask::RecaptchaTiles(TilePuzzle {
            image_base64: "image-body".into(),
            question_id: None,
            question_text: "Select all traffic lights".into(),
            grid,
        })
    }
    fn ready(solution: Value) -> Value {
        json!({"errorId":0,"status":"ready","solution":solution,"cost":"0.0123"})
    }
    async fn solve(
        client: &TwoCaptchaClient,
        task: &SolveTask,
    ) -> Result<ProviderAnswer, ProviderError> {
        client
            .solve(
                task,
                Instant::now() + Duration::from_secs(2),
                &CancellationToken::new(),
            )
            .await
    }

    #[tokio::test]
    async fn tests_that_two_captcha_attempt_budgets_depend_on_task() -> anyhow::Result<()> {
        let mock = mock_server(vec![]).await?;
        let client: &dyn VendorClient = &mock.client;
        for task in [
            token_task(false),
            token_task(true),
            SolveTask::TurnstileToken(TurnstileParams {
                website_url: "https://example.test/".into(),
                website_key: "site-key".into(),
                action: None,
                cdata: None,
            }),
        ] {
            assert_eq!(client.attempt_budget(&task), Duration::from_secs(90));
        }
        assert_eq!(
            client.attempt_budget(&tiles(TileGrid::ThreeByThree)),
            Duration::from_secs(30)
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_two_captcha_keeps_base_url_path_prefixes() -> anyhow::Result<()> {
        for suffix in ["", "/"] {
            let mut mock = mock_server(vec![
                json!({"errorId":0,"taskId":42}),
                ready(json!({"gRecaptchaResponse":"answer"})),
            ])
            .await?;
            mock.client
                .config
                .base_url
                .set_path(&format!("/mock/2captcha{suffix}"));
            assert_eq!(
                solve(&mock.client, &token_task(false)).await?.answer,
                SolveAnswer::Token("answer".into())
            );
            let calls = mock.state.calls.lock().await;
            assert_eq!(calls[0].0, "mock/2captcha/createTask");
            assert_eq!(calls[1].0, "mock/2captcha/getTaskResult");
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_two_captcha_omits_blank_optional_fields() -> anyhow::Result<()> {
        for blank in ["", " \t\n "] {
            for enterprise in [false, true] {
                let SolveTask::RecaptchaToken(mut params) = token_task(enterprise) else {
                    anyhow::bail!("wrong task")
                };
                params.enterprise_s = Some(blank.into());
                params.user_agent = Some(blank.into());
                params.page_action = Some(blank.into());
                let mock = mock_server(vec![
                    json!({"errorId":0,"taskId":42}),
                    ready(json!({"gRecaptchaResponse":"answer"})),
                ])
                .await?;
                assert!(
                    solve(&mock.client, &SolveTask::RecaptchaToken(params))
                        .await
                        .is_ok()
                );
                let calls = mock.state.calls.lock().await;
                let payload = &calls[0].1["task"];
                for field in [
                    "enterprisePayload",
                    "recaptchaDataSValue",
                    "userAgent",
                    "pageAction",
                ] {
                    assert!(payload.get(field).is_none(), "{field} was sent");
                }
            }
            let task = SolveTask::TurnstileToken(TurnstileParams {
                website_url: "https://example.test/".into(),
                website_key: "site-key".into(),
                action: Some(blank.into()),
                cdata: Some(blank.into()),
            });
            let mock = mock_server(vec![
                json!({"errorId":0,"taskId":42}),
                ready(json!({"token":"answer"})),
            ])
            .await?;
            assert!(solve(&mock.client, &task).await.is_ok());
            let calls = mock.state.calls.lock().await;
            assert!(calls[0].1["task"].get("action").is_none());
            assert!(calls[0].1["task"].get("data").is_none());
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_paid_task_deltas_include_both_vendors_after_charged_failover()
    -> anyhow::Result<()> {
        use super::super::providers::{CaptchaBackend, ProviderChain};
        let primary = mock_server(vec![
            ready(json!({"gRecaptchaResponse":"first"})),
            json!({"errorId":0,"taskId":"accepted"}),
            json!({"errorId":1,"errorCode":"ERROR_CAPTCHA_UNSOLVABLE"}),
        ])
        .await?;
        let backup = mock_server(vec![
            json!({"errorId":0,"taskId":42}),
            ready(json!({"gRecaptchaResponse":"backup"})),
        ])
        .await?;
        let chain = ProviderChain::new(&CaptchaConfig {
            capsolver: Some(primary.client.config.clone()),
            twocaptcha: Some(backup.client.config.clone()),
            paid_task_limit: 3,
            force_method: None,
        });
        let task = token_task(false);
        let cancel = CancellationToken::new();
        chain
            .solve(&task, Instant::now() + Duration::from_secs(10), &cancel)
            .await?;
        let before = chain.paid_tasks();
        assert_eq!(before, 1);
        let answer = chain
            .solve(&task, Instant::now() + Duration::from_secs(10), &cancel)
            .await?;
        assert_eq!(answer.answer, SolveAnswer::Token("backup".into()));
        assert_eq!(answer.provider, ProviderName::TwoCaptcha);
        assert_eq!(chain.paid_tasks() - before, 2);
        assert_eq!(chain.paid_tasks(), 3);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_all_token_types_use_documented_fields_and_poll_once_created()
    -> anyhow::Result<()> {
        let tasks = [
            (
                token_task(false),
                "RecaptchaV2TaskProxyless",
                "gRecaptchaResponse",
            ),
            (
                token_task(true),
                "RecaptchaV2EnterpriseTaskProxyless",
                "gRecaptchaResponse",
            ),
            (
                SolveTask::TurnstileToken(TurnstileParams {
                    website_url: "https://example.test/".into(),
                    website_key: "site-key".into(),
                    action: Some("login".into()),
                    cdata: Some("custom-data".into()),
                }),
                "TurnstileTaskProxyless",
                "token",
            ),
        ];
        for (task, expected_type, token_field) in tasks {
            let mut solution = json!({});
            solution[token_field] = json!("answer-token");
            let mock = mock_server(vec![
                json!({"errorId":0,"taskId":42}),
                json!({"errorId":0,"status":"processing"}),
                ready(solution),
            ])
            .await?;
            assert!(mock.client.supports(&task));
            let answer = solve(&mock.client, &task).await?;
            assert_eq!(answer.answer, SolveAnswer::Token("answer-token".into()));
            assert_eq!(answer.provider, ProviderName::TwoCaptcha);
            assert_eq!(answer.task_id.as_deref(), Some("42"));
            assert_eq!(answer.cost_usd, Some(0.0123));
            let calls = mock.state.calls.lock().await;
            assert_eq!(
                calls.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
                ["createTask", "getTaskResult", "getTaskResult"]
            );
            let payload = &calls[0].1;
            assert_eq!(payload["clientKey"], "secret-test-key");
            assert_eq!(payload["task"]["type"], expected_type);
            assert_eq!(payload["task"]["websiteURL"], "https://example.test/");
            assert_eq!(payload["task"]["websiteKey"], "site-key");
            assert!(payload["task"].get("pageAction").is_none());
            match task {
                SolveTask::RecaptchaToken(p) => {
                    assert_eq!(payload["task"]["isInvisible"], true);
                    assert_eq!(payload["task"]["userAgent"], "test-agent");
                    if p.enterprise {
                        assert_eq!(payload["task"]["enterprisePayload"]["s"], "data-s");
                    } else {
                        assert_eq!(payload["task"]["recaptchaDataSValue"], "data-s");
                    }
                }
                SolveTask::TurnstileToken(_) => {
                    assert_eq!(payload["task"]["action"], "login");
                    assert_eq!(payload["task"]["data"], "custom-data");
                }
                _ => anyhow::bail!("unexpected test task"),
            }
            assert_eq!(calls[1].1["taskId"], 42);
            assert!(calls[2].2.duration_since(calls[1].2) >= mock.client.poll_interval);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_grid_answers_are_validated_and_normalized() -> anyhow::Result<()> {
        for (grid, side) in [
            (TileGrid::ThreeByThree, 3),
            (TileGrid::FourByFour, 4),
            (TileGrid::Single, 1),
        ] {
            let last = side * side;
            let indexes = if last == 1 { vec![1] } else { vec![1, last] };
            let expected = indexes.iter().map(|i| i - 1).collect::<Vec<_>>();
            let mock = mock_server(vec![
                json!({"errorId":0,"taskId":42}),
                ready(json!({"click":indexes})),
            ])
            .await?;
            assert_eq!(
                solve(&mock.client, &tiles(grid)).await?.answer,
                SolveAnswer::Tiles(expected)
            );
            let calls = mock.state.calls.lock().await;
            let task = &calls[0].1["task"];
            assert_eq!(task["type"], "GridTask");
            assert_eq!(task["body"], "image-body");
            assert_eq!(task["comment"], "Select all traffic lights");
            assert_eq!(task["rows"], side);
            assert_eq!(task["columns"], side);
            assert_eq!(task["canNoAnswer"], 1);
            assert!(task.get("imgType").is_none());
        }
        for indexes in [
            json!([0]),
            json!([10]),
            json!([-1]),
            json!([1.5]),
            json!(["1"]),
            json!([1, 1]),
            json!(null),
        ] {
            let mock = mock_server(vec![
                json!({"errorId":0,"taskId":42}),
                ready(json!({"click":indexes})),
            ])
            .await?;
            let error = solve(&mock.client, &tiles(TileGrid::ThreeByThree))
                .await
                .err()
                .ok_or_else(|| anyhow::anyhow!("invalid indexes accepted"))?;
            assert_eq!(error.kind, ProviderErrorKind::BadRequest);
            assert!(error.charged);
        }
        for empty in [json!([]), json!("No_matching_images")] {
            let mock = mock_server(vec![
                json!({"errorId":0,"taskId":42}),
                ready(json!({"click":empty})),
            ])
            .await?;
            assert_eq!(
                solve(&mock.client, &tiles(TileGrid::ThreeByThree))
                    .await?
                    .answer,
                SolveAnswer::Tiles(vec![])
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_vendor_errors_are_classified_without_echoing_secrets() -> anyhow::Result<()>
    {
        for (code, kind) in [
            ("ERROR_KEY_DOES_NOT_EXIST", ProviderErrorKind::Auth),
            ("ERROR_WRONG_USER_KEY", ProviderErrorKind::Auth),
            ("ERROR_IP_NOT_ALLOWED", ProviderErrorKind::Auth),
            ("ERROR_ACCOUNT_SUSPENDED", ProviderErrorKind::Auth),
            ("ERROR_ZERO_BALANCE", ProviderErrorKind::NoBalance),
            ("ERROR_CAPTCHA_UNSOLVABLE", ProviderErrorKind::Unsolvable),
            ("ERROR_TASK_NOT_SUPPORTED", ProviderErrorKind::Unsupported),
            ("ERROR_NO_SUCH_METHOD", ProviderErrorKind::Unsupported),
            ("ERROR_NO_SLOT_AVAILABLE", ProviderErrorKind::RateLimited),
            ("ERROR_BAD_PARAMETERS", ProviderErrorKind::BadRequest),
            (
                "ERROR_RECAPTCHA_INVALID_SITEKEY",
                ProviderErrorKind::BadRequest,
            ),
        ] {
            let mock = mock_server(vec![json!({"errorId":1,"errorCode":code,"errorDescription":"secret-test-key image-body answer-token"})]).await?;
            let error = solve(&mock.client, &token_task(false))
                .await
                .err()
                .ok_or_else(|| anyhow::anyhow!("expected vendor error"))?;
            assert_eq!(error.kind, kind);
            assert!(error.message.contains(code));
            assert!(!error.charged);
            for secret in ["secret-test-key", "image-body", "answer-token"] {
                assert!(!format!("{error:?} {error} {:?}", mock.client).contains(secret));
            }
            assert_eq!(mock.state.calls.lock().await.len(), 1);
        }
        let mock = mock_server(vec![
            json!({"errorId":1,"errorCode":"secret-test-key","errorDescription":"image-body"}),
        ])
        .await?;
        let error = solve(&mock.client, &token_task(false))
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected error"))?;
        assert!(!format!("{error}").contains("secret-test-key"));
        assert!(!format!("{error}").contains("image-body"));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_http_rejections_and_lost_create_responses_are_distinct()
    -> anyhow::Result<()> {
        for (status, body, kind, charged) in [
            (
                StatusCode::TOO_MANY_REQUESTS,
                json!({}),
                ProviderErrorKind::RateLimited,
                false,
            ),
            (
                StatusCode::UNAUTHORIZED,
                json!({}),
                ProviderErrorKind::Auth,
                false,
            ),
            (
                StatusCode::BAD_REQUEST,
                json!({}),
                ProviderErrorKind::BadRequest,
                false,
            ),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({}),
                ProviderErrorKind::Uncertain,
                true,
            ),
            (
                StatusCode::OK,
                json!({"errorId":0}),
                ProviderErrorKind::Uncertain,
                true,
            ),
            (
                StatusCode::OK,
                json!({"errorId":0,"taskId":"not-an-integer"}),
                ProviderErrorKind::Uncertain,
                true,
            ),
        ] {
            let mock = mock_server(vec![]).await?;
            mock.state
                .replies
                .lock()
                .await
                .push_back(Reply(status, body, Duration::ZERO));
            let error = solve(&mock.client, &token_task(false))
                .await
                .err()
                .ok_or_else(|| anyhow::anyhow!("expected failure"))?;
            assert_eq!((error.kind, error.charged), (kind, charged));
            assert_eq!(mock.state.calls.lock().await.len(), 1);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_cancel_and_deadline_before_submission_do_not_spend() -> anyhow::Result<()> {
        let mock = mock_server(vec![]).await?;
        let cancel = CancellationToken::new();
        cancel.cancel();
        let error = mock
            .client
            .solve(
                &token_task(false),
                Instant::now() + Duration::from_secs(1),
                &cancel,
            )
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected cancelled"))?;
        assert_eq!(
            (error.kind, error.charged),
            (ProviderErrorKind::Cancelled, false)
        );
        let error = mock
            .client
            .solve(
                &token_task(false),
                Instant::now(),
                &CancellationToken::new(),
            )
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected timeout"))?;
        assert_eq!(
            (error.kind, error.charged),
            (ProviderErrorKind::Timeout, false)
        );
        assert!(mock.state.calls.lock().await.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_cancel_mid_poll_and_deadlines_honor_paid_submission() -> anyhow::Result<()>
    {
        let mut mock = mock_server(vec![json!({"errorId":0,"taskId":42})]).await?;
        mock.client.poll_interval = Duration::from_secs(5);
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let state = mock.state.clone();
        let trigger = tokio::spawn(async move {
            state.received.notified().await;
            tokio::time::sleep(Duration::from_millis(20)).await;
            token.cancel();
        });
        let error = mock
            .client
            .solve(
                &token_task(false),
                Instant::now() + Duration::from_secs(1),
                &cancel,
            )
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected cancelled"))?;
        assert_eq!(
            (error.kind, error.charged),
            (ProviderErrorKind::Cancelled, true)
        );
        trigger.await?;
        assert_eq!(mock.state.calls.lock().await.len(), 1);
        let error = mock
            .client
            .solve(
                &token_task(false),
                Instant::now() + Duration::from_millis(30),
                &CancellationToken::new(),
            )
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected failure"))?;
        // Replenish below for a separate accepted task bounded during its polling wait.
        assert_eq!(error.kind, ProviderErrorKind::Uncertain);
        mock.state.replies.lock().await.push_back(Reply(
            StatusCode::OK,
            json!({"errorId":0,"taskId":43}),
            Duration::ZERO,
        ));
        let error = mock
            .client
            .solve(
                &token_task(false),
                Instant::now() + Duration::from_millis(30),
                &CancellationToken::new(),
            )
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected timeout"))?;
        assert_eq!(
            (error.kind, error.charged),
            (ProviderErrorKind::Timeout, true)
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_uncertain_inflight_create_is_never_retried() -> anyhow::Result<()> {
        let mock = mock_server(vec![]).await?;
        mock.state.replies.lock().await.push_back(Reply(
            StatusCode::OK,
            json!({"errorId":0,"taskId":42}),
            Duration::from_secs(1),
        ));
        let error = mock
            .client
            .solve(
                &token_task(false),
                Instant::now() + Duration::from_millis(30),
                &CancellationToken::new(),
            )
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected uncertain"))?;
        assert_eq!(
            (error.kind, error.charged),
            (ProviderErrorKind::Uncertain, true)
        );
        assert_eq!(mock.state.calls.lock().await.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_invalid_solutions_are_rejected_and_reported_cost_is_finite()
    -> anyhow::Result<()> {
        for solution in [
            json!({}),
            json!({"gRecaptchaResponse":""}),
            json!({"gRecaptchaResponse":"   "}),
            json!({"gRecaptchaResponse":42}),
        ] {
            let mock = mock_server(vec![json!({"errorId":0,"taskId":42}), ready(solution)]).await?;
            assert_eq!(
                solve(&mock.client, &token_task(false))
                    .await
                    .err()
                    .ok_or_else(|| anyhow::anyhow!("expected invalid solution"))?
                    .kind,
                ProviderErrorKind::BadRequest
            );
        }
        for cost in [json!("NaN"), json!("-1"), json!(null)] {
            let mut reply = ready(json!({"gRecaptchaResponse":"ok"}));
            reply["cost"] = cost;
            let mock = mock_server(vec![json!({"errorId":0,"taskId":42}), reply]).await?;
            assert!(
                solve(&mock.client, &token_task(false))
                    .await?
                    .cost_usd
                    .is_none()
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_get_balance_is_free_and_bad_balance_is_rejected() -> anyhow::Result<()> {
        let mock = mock_server(vec![json!({"errorId":0,"balance":10.25})]).await?;
        assert_eq!(
            mock.client
                .get_balance(
                    Instant::now() + Duration::from_secs(1),
                    &CancellationToken::new()
                )
                .await?,
            10.25
        );
        assert_eq!(mock.state.calls.lock().await[0].0, "getBalance");
        let mock = mock_server(vec![json!({"errorId":0,"balance":-1})]).await?;
        assert_eq!(
            mock.client
                .get_balance(
                    Instant::now() + Duration::from_secs(1),
                    &CancellationToken::new()
                )
                .await
                .err()
                .ok_or_else(|| anyhow::anyhow!("expected invalid balance"))?
                .kind,
            ProviderErrorKind::BadRequest
        );
        Ok(())
    }
    #[tokio::test]
    async fn tests_that_connection_loss_after_create_is_uncertain() -> anyhow::Result<()> {
        use tokio::io::AsyncReadExt;
        let mut mock = mock_server(vec![]).await?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        mock.client.config.base_url =
            url::Url::parse(&format!("http://{}", listener.local_addr()?))?;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await?;
            let mut request = [0_u8; 4096];
            let read = socket.read(&mut request).await?;
            if read == 0 {
                return Err(std::io::Error::other("request was not received"));
            }
            Ok::<_, std::io::Error>(())
        });
        let error = solve(&mock.client, &token_task(false))
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected uncertain"))?;
        server.await??;
        assert_eq!(
            (error.kind, error.charged),
            (ProviderErrorKind::Uncertain, true)
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_connect_failure_is_unpaid_and_invalid_tasks_never_submit()
    -> anyhow::Result<()> {
        let mut mock = mock_server(vec![]).await?;
        let mut task = token_task(false);
        if let SolveTask::RecaptchaToken(p) = &mut task {
            p.website_key.clear();
        }
        assert_eq!(
            solve(&mock.client, &task)
                .await
                .err()
                .ok_or_else(|| anyhow::anyhow!("expected bad request"))?
                .kind,
            ProviderErrorKind::BadRequest
        );
        if let SolveTask::RecaptchaToken(p) = &mut task {
            p.website_key = "sitekey".into();
            p.website_url = "file:///tmp/fixture".into();
        }
        assert_eq!(
            solve(&mock.client, &task)
                .await
                .err()
                .ok_or_else(|| anyhow::anyhow!("expected bad URL"))?
                .kind,
            ProviderErrorKind::BadRequest
        );
        let mut task = tiles(TileGrid::ThreeByThree);
        if let SolveTask::RecaptchaTiles(p) = &mut task {
            p.question_text.clear();
        }
        assert!(!mock.client.supports(&task));
        assert_eq!(
            solve(&mock.client, &task)
                .await
                .err()
                .ok_or_else(|| anyhow::anyhow!("expected unsupported"))?
                .kind,
            ProviderErrorKind::Unsupported
        );
        assert!(mock.state.calls.lock().await.is_empty());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        mock.client.config.base_url =
            url::Url::parse(&format!("http://{}", listener.local_addr()?))?;
        drop(listener);
        let error = solve(&mock.client, &token_task(false))
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected connect failure"))?;
        assert_eq!(
            (error.kind, error.charged),
            (ProviderErrorKind::Transport, false)
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_production_polling_honors_the_callers_deadline() -> anyhow::Result<()> {
        let mut replies = vec![json!({"errorId":0,"taskId":42})];
        replies.extend((0..8).map(|_| json!({"errorId":0,"status":"processing"})));
        let mut mock = mock_server(replies).await?;
        mock.client = TwoCaptchaClient::new(&mock.client.config)?;
        let started = Instant::now();
        let vendor: &dyn VendorClient = &mock.client;
        assert_eq!(vendor.name(), ProviderName::TwoCaptcha);
        let error = vendor
            .solve(
                &token_task(false),
                started + Duration::from_secs(12),
                &CancellationToken::new(),
            )
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected timeout"))?;
        assert_eq!(
            (error.kind, error.charged),
            (ProviderErrorKind::Timeout, true)
        );
        assert!(started.elapsed() >= Duration::from_secs(12));
        assert!(started.elapsed() < Duration::from_secs(15));
        let calls = mock.state.calls.lock().await;
        assert_eq!(calls.iter().filter(|c| c.0 == "createTask").count(), 1);
        assert!(calls.len() >= 3);
        for pair in calls.windows(2) {
            assert!(pair[1].2.duration_since(pair[0].2) >= Duration::from_secs(5));
        }
        Ok(())
    }
    #[test]
    fn tests_that_two_captcha_configuration_uses_only_explicit_key_files() -> anyhow::Result<()> {
        use super::super::config::{TWOCAPTCHA_KEY_FILE_ENV, TWOCAPTCHA_URL_ENV};
        let key = tempfile::NamedTempFile::new()?;
        std::fs::write(key.path(), "  secret-test-key\n")?;
        let configured = CaptchaConfig::from_lookup(|name| {
            (name == TWOCAPTCHA_KEY_FILE_ENV).then(|| key.path().to_string_lossy().into_owned())
        });
        assert!(configured.has_provider());
        let provider = configured
            .twocaptcha
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing 2Captcha"))?;
        assert_eq!(provider.key.expose(), "secret-test-key");
        assert_eq!(provider.base_url.as_str(), "https://api.2captcha.com/");
        assert!(!format!("{configured:?}").contains("secret-test-key"));
        assert!(
            !CaptchaConfig::from_lookup(
                |name| (name == "BROWSEROS_2CAPTCHA_API_KEY").then(|| "inline-key".into())
            )
            .has_provider()
        );
        for bad in ["", "/missing-key-file"] {
            assert!(
                !CaptchaConfig::from_lookup(
                    |name| (name == TWOCAPTCHA_KEY_FILE_ENV).then(|| bad.into())
                )
                .has_provider()
            );
        }
        std::fs::write(key.path(), " \n")?;
        assert!(
            !CaptchaConfig::from_lookup(|name| (name == TWOCAPTCHA_KEY_FILE_ENV)
                .then(|| key.path().to_string_lossy().into_owned()))
            .has_provider()
        );
        std::fs::write(key.path(), "secret-test-key")?;
        let config = CaptchaConfig::from_lookup(|name| match name {
            TWOCAPTCHA_KEY_FILE_ENV => Some(key.path().to_string_lossy().into_owned()),
            TWOCAPTCHA_URL_ENV => Some("file:///tmp/secret-test-key".into()),
            _ => None,
        });
        assert!(!config.has_provider());
        assert_eq!(serde_json::to_value(ProviderName::TwoCaptcha)?, "2captcha");
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_registered_chain_falls_back_for_each_task_type() -> anyhow::Result<()> {
        use super::super::providers::{CaptchaBackend, ProviderChain};
        let mut tile_task = tiles(TileGrid::ThreeByThree);
        if let SolveTask::RecaptchaTiles(p) = &mut tile_task {
            p.question_id = Some("/m/015qff".into());
        }
        let tasks = [
            token_task(false),
            SolveTask::TurnstileToken(TurnstileParams {
                website_url: "https://example.test/".into(),
                website_key: "sitekey".into(),
                action: None,
                cdata: None,
            }),
            tile_task,
        ];
        for task in tasks {
            let primary = mock_server(vec![
                json!({"errorId":1,"errorCode":"ERROR_KEY_DENIED_ACCESS"}),
            ])
            .await?;
            let solution = match &task {
                SolveTask::RecaptchaToken(_) => json!({"gRecaptchaResponse":"token"}),
                SolveTask::TurnstileToken(_) => json!({"token":"token"}),
                SolveTask::RecaptchaTiles(_) => json!({"click":[1,9]}),
            };
            let backup =
                mock_server(vec![json!({"errorId":0,"taskId":42}), ready(solution)]).await?;
            let chain = ProviderChain::new(&CaptchaConfig {
                capsolver: Some(primary.client.config.clone()),
                twocaptcha: Some(backup.client.config.clone()),
                ..Default::default()
            });
            assert!(chain.is_configured());
            let answer = chain
                .solve(
                    &task,
                    Instant::now() + Duration::from_secs(10),
                    &CancellationToken::new(),
                )
                .await?;
            assert_eq!(answer.provider, ProviderName::TwoCaptcha);
            assert_eq!(answer.cost_usd, Some(0.0123));
            assert_eq!(chain.paid_tasks(), 1);
            assert_eq!(primary.state.calls.lock().await.len(), 1);
            assert_eq!(backup.state.calls.lock().await.len(), 2);
            assert_eq!(
                backup.state.calls.lock().await[0].1["task"]["type"],
                match task {
                    SolveTask::RecaptchaToken(_) => "RecaptchaV2TaskProxyless",
                    SolveTask::TurnstileToken(_) => "TurnstileTaskProxyless",
                    SolveTask::RecaptchaTiles(_) => "GridTask",
                }
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_chain_preserves_both_codes_and_cancellation_stops_failover()
    -> anyhow::Result<()> {
        use super::super::providers::{CaptchaBackend, ProviderChain};
        let primary = mock_server(vec![
            json!({"errorId":1,"errorCode":"ERROR_KEY_DENIED_ACCESS"}),
        ])
        .await?;
        let backup =
            mock_server(vec![json!({"errorId":1,"errorCode":"ERROR_ZERO_BALANCE"})]).await?;
        let chain = ProviderChain::new(&CaptchaConfig {
            capsolver: Some(primary.client.config.clone()),
            twocaptcha: Some(backup.client.config.clone()),
            ..Default::default()
        });
        let error = chain
            .solve(
                &token_task(false),
                Instant::now() + Duration::from_secs(10),
                &CancellationToken::new(),
            )
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected both failures"))?;
        assert!(error.message.contains("ERROR_KEY_DENIED_ACCESS"));
        assert!(error.message.contains("ERROR_ZERO_BALANCE"));
        assert_eq!(backup.state.calls.lock().await.len(), 1);
        let primary = mock_server(vec![]).await?;
        let backup = mock_server(vec![]).await?;
        primary.state.replies.lock().await.push_back(Reply(
            StatusCode::OK,
            json!({"errorId":0,"taskId":"42"}),
            Duration::from_secs(1),
        ));
        let chain = ProviderChain::new(&CaptchaConfig {
            capsolver: Some(primary.client.config.clone()),
            twocaptcha: Some(backup.client.config.clone()),
            ..Default::default()
        });
        let state = primary.state.clone();
        let cancel = CancellationToken::new();
        let trigger_token = cancel.clone();
        let trigger = tokio::spawn(async move {
            state.received.notified().await;
            trigger_token.cancel();
        });
        let error = chain
            .solve(
                &token_task(false),
                Instant::now() + Duration::from_secs(10),
                &cancel,
            )
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected cancellation"))?;
        trigger.await?;
        assert_eq!(error.kind, ProviderErrorKind::Cancelled);
        assert!(backup.state.calls.lock().await.is_empty());
        Ok(())
    }
    #[tokio::test]
    async fn tests_that_poll_errors_keep_spend_unless_the_vendor_refunds() -> anyhow::Result<()> {
        for (code, charged) in [
            ("ERROR_KEY_DOES_NOT_EXIST", true),
            ("ERROR_NO_SUCH_CAPCHA_ID", true),
            ("ERROR_BAD_DUPLICATES", true),
            ("ERROR_CAPTCHA_UNSOLVABLE", false),
        ] {
            let mock = mock_server(vec![
                json!({"errorId":0,"taskId":42}),
                json!({"errorId":1,"errorCode":code}),
            ])
            .await?;
            let error = solve(&mock.client, &token_task(false))
                .await
                .err()
                .ok_or_else(|| anyhow::anyhow!("expected poll error"))?;
            assert_eq!(error.charged, charged, "{code}");
        }
        Ok(())
    }
}
