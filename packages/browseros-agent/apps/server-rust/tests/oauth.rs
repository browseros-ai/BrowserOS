//! Behavioral port tests use fake loopback auth servers and a temporary real SQLite database.
//! Every callback port is ephemeral: tests never occupy or evict the registered port 1455.
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Request, State},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use http::{StatusCode, header};
use serde_json::{Value, json};
use server_rust::{
    db::{Database, OAuthTokenStore, OAuthTokens},
    http::oauth::{start_with_manager, store_token_with_manager},
    oauth::{OAuthProvider, ProviderSpec, TokenInput, TokenManager},
    paths::BrowserosPaths,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    error::Error,
    future::IntoFuture,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::TcpListener,
    sync::{Mutex, mpsc},
};
use tokio_util::sync::CancellationToken;
type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;
struct Observed {
    path: String,
    content_type: String,
    accept: String,
    body: Value,
    at: tokio::time::Instant,
}
type Reply = (StatusCode, &'static str, Value);
struct FakeInner {
    requests: Mutex<Vec<Observed>>,
    replies: Mutex<HashMap<String, VecDeque<Reply>>>,
    hits: mpsc::UnboundedSender<()>,
    delay: Duration,
}
/// Failed assertions must not leave fake listeners alive.
struct FakeAuth {
    base: String,
    inner: Arc<FakeInner>,
    hits: mpsc::UnboundedReceiver<()>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for FakeAuth {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl FakeAuth {
    async fn new(delay: Duration) -> TestResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let (sender, hits) = mpsc::unbounded_channel();
        let inner = Arc::new(FakeInner {
            requests: Mutex::new(vec![]),
            replies: Mutex::new(HashMap::new()),
            hits: sender,
            delay,
        });
        let router = Router::new().fallback(fake_auth).with_state(inner.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Ok(Self {
            base,
            inner,
            hits,
            task,
        })
    }
    async fn reply(&self, path: &str, status: StatusCode, ct: &'static str, value: Value) {
        self.inner
            .replies
            .lock()
            .await
            .entry(path.to_owned())
            .or_default()
            .push_back((status, ct, value));
    }
    fn specs(&self) -> [ProviderSpec; 3] {
        OAuthProvider::ALL.map(|provider| {
            let mut spec = provider.spec();
            spec.auth_endpoint = format!("{}/{}/authorize", self.base, provider.as_str());
            spec.token_endpoint = format!("{}/{}/token", self.base, provider.as_str());
            spec
        })
    }
    async fn next_hit(&mut self) -> TestResult {
        self.hits.recv().await.ok_or("fake server closed".into())
    }
}
async fn fake_auth(State(state): State<Arc<FakeInner>>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    let ct = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let accept = request
        .headers()
        .get(header::ACCEPT)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = to_bytes(request.into_body(), 1024 * 1024)
        .await
        .unwrap_or_default();
    let body = if ct.contains("application/json") {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    } else {
        Value::Object(
            url::form_urlencoded::parse(&bytes)
                .map(|(k, v)| (k.into_owned(), Value::String(v.into_owned())))
                .collect(),
        )
    };
    state.requests.lock().await.push(Observed {
        path: path.clone(),
        content_type: ct,
        accept,
        body,
        at: tokio::time::Instant::now(),
    });
    let _ = state.hits.send(());
    if !state.delay.is_zero() {
        tokio::time::sleep(state.delay).await;
    }
    match state
        .replies
        .lock()
        .await
        .get_mut(&path)
        .and_then(VecDeque::pop_front)
    {
        Some((status, ct, value)) => {
            (status, [(header::CONTENT_TYPE, ct)], value.to_string()).into_response()
        }
        None => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"access_denied"})),
        )
            .into_response(),
    }
}
struct Context {
    _dir: tempfile::TempDir,
    paths: BrowserosPaths,
    store: OAuthTokenStore,
    manager: Arc<TokenManager>,
}
impl Context {
    async fn new(fake: &FakeAuth, port: u16) -> TestResult<Self> {
        assert_ne!(port, 1455);
        let dir = tempfile::tempdir()?;
        let paths = BrowserosPaths::new(dir.path().join("browseros"));
        let (db, _) = Database::open(&paths).await?;
        let store = db.oauth_tokens();
        let manager = TokenManager::with_provider_specs(
            store.clone(),
            "installation-a".into(),
            reqwest::Client::new(),
            fake.specs(),
            port,
        );
        Ok(Self {
            _dir: dir,
            paths,
            store,
            manager,
        })
    }
    async fn put(&self, provider: OAuthProvider, expires_at: i64, refresh: &str) -> TestResult {
        self.store
            .upsert(
                "installation-a",
                provider.as_str(),
                &OAuthTokens {
                    access_token: "old".into(),
                    refresh_token: refresh.into(),
                    expires_at,
                    email: Some("retained@example.test".into()),
                    account_id: Some("retained-account".into()),
                },
            )
            .await?;
        Ok(())
    }
}
async fn body(response: Response) -> TestResult<String> {
    Ok(String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024).await?.to_vec(),
    )?)
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn jwt(claims: Value) -> String {
    format!("e30.{}.test", URL_SAFE_NO_PAD.encode(claims.to_string()))
}
async fn pkce(manager: &Arc<TokenManager>) -> TestResult<HashMap<String, String>> {
    let response = start_with_manager(manager, "chatgpt-pro").await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let url = url::Url::parse(
        response
            .headers()
            .get(header::LOCATION)
            .ok_or("missing Location")?
            .to_str()?,
    )?;
    assert_eq!(url.path(), "/chatgpt-pro/authorize");
    Ok(url.query_pairs().into_owned().collect())
}
fn callback_url(params: &HashMap<String, String>) -> TestResult<url::Url> {
    let mut url = url::Url::parse(params.get("redirect_uri").ok_or("missing redirect_uri")?)?;
    url.set_host(Some("127.0.0.1"))?;
    Ok(url)
}

#[test]
fn production_configuration_matches_ts() {
    for (provider, client, auth, token, scope) in [
        (
            OAuthProvider::ChatGptPro,
            "app_EMoamEEZ73f0CkXaXp7hrann",
            "https://auth.openai.com/oauth/authorize",
            "https://auth.openai.com/oauth/token",
            "openid profile email offline_access",
        ),
        (
            OAuthProvider::GitHubCopilot,
            "Ov23li8tweQw6odWQebz",
            "https://github.com/login/device/code",
            "https://github.com/login/oauth/access_token",
            "read:user",
        ),
        (
            OAuthProvider::QwenCode,
            "f0304373b74a44d2b584a3fb70ca9e56",
            "https://chat.qwen.ai/api/v1/oauth2/device/code",
            "https://chat.qwen.ai/api/v1/oauth2/token",
            "openid profile email model.completion",
        ),
    ] {
        let spec = provider.spec();
        assert_eq!(
            (
                spec.client_id,
                spec.auth_endpoint.as_str(),
                spec.token_endpoint.as_str(),
                spec.scope
            ),
            (client, auth, token, scope)
        );
    }
}
#[tokio::test]
async fn pkce_redirect_exchange_persistence_and_one_use_state() -> TestResult {
    let fake = FakeAuth::new(Duration::ZERO).await?;
    let ctx = Context::new(&fake, 0).await?;
    let params = pkce(&ctx.manager).await?;
    let second = pkce(&ctx.manager).await?;
    for (key, value) in [
        ("response_type", "code"),
        ("client_id", "app_EMoamEEZ73f0CkXaXp7hrann"),
        ("scope", "openid profile email offline_access"),
        ("code_challenge_method", "S256"),
        ("originator", "browseros"),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
    ] {
        assert_eq!(params[key], value);
    }
    assert_eq!(params["redirect_uri"], second["redirect_uri"]);
    assert_ne!(params["state"], second["state"]);
    assert_ne!(params["code_challenge"], second["code_challenge"]);
    assert_eq!(URL_SAFE_NO_PAD.decode(&params["state"])?.len(), 16);
    let token = jwt(
        json!({"https://api.openai.com/auth":{"chatgpt_account_id":"account-1"},"https://api.openai.com/profile":{"email":"person@example.test"}}),
    );
    fake.reply(
        "/chatgpt-pro/token",
        StatusCode::OK,
        "application/json",
        json!({"access_token":token,"refresh_token":"refresh-1","expires_in":3600}),
    )
    .await;
    let mut callback = callback_url(&params)?;
    callback
        .query_pairs_mut()
        .append_pair("code", "auth-code")
        .append_pair("state", &params["state"]);
    let response = reqwest::get(callback.clone()).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert_eq!(
        response.text().await?,
        include_str!("../src/oauth/success.html")
    );
    let saved = ctx
        .store
        .get("installation-a", "chatgpt-pro")
        .await?
        .ok_or("missing token")?;
    assert_eq!(saved.access_token, token);
    assert_eq!(saved.email.as_deref(), Some("person@example.test"));
    assert_eq!(saved.account_id.as_deref(), Some("account-1"));
    assert!((now_ms() + 3_590_000..=now_ms() + 3_600_000).contains(&saved.expires_at));
    assert!(
        ctx.store
            .get("another-installation", "chatgpt-pro")
            .await?
            .is_none()
    );
    let requests = fake.inner.requests.lock().await;
    let request = requests.first().ok_or("missing exchange")?;
    assert_eq!(request.content_type, "application/x-www-form-urlencoded");
    assert_eq!(request.body["grant_type"], "authorization_code");
    assert_eq!(request.body["code"], "auth-code");
    assert_eq!(request.body["redirect_uri"], params["redirect_uri"]);
    let verifier = request.body["code_verifier"]
        .as_str()
        .ok_or("missing verifier")?;
    assert_eq!(URL_SAFE_NO_PAD.decode(verifier)?.len(), 32);
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        params["code_challenge"]
    );
    drop(requests);
    // The second pending flow keeps the listener alive, making replay observable.
    assert!(
        reqwest::get(callback)
            .await?
            .text()
            .await?
            .contains("Invalid or expired OAuth state")
    );
    assert_eq!(
        ctx.manager
            .try_status("chatgpt-pro")
            .await?
            .email
            .as_deref(),
        Some("person@example.test")
    );
    ctx.manager.shutdown().await;
    Ok(())
}
#[tokio::test]
async fn callback_errors_escaping_failed_exchange_retry_and_port_release() -> TestResult {
    let fake = FakeAuth::new(Duration::ZERO).await?;
    let ctx = Context::new(&fake, 0).await?;
    let params = pkce(&ctx.manager).await?;
    let callback = callback_url(&params)?;
    assert_eq!(
        reqwest::get(callback.join("/unknown")?).await?.status(),
        StatusCode::NOT_FOUND
    );
    assert!(
        reqwest::get(callback.clone())
            .await?
            .text()
            .await?
            .contains("Missing authorization code or state")
    );
    let mut denied = callback.clone();
    denied
        .query_pairs_mut()
        .append_pair("error", "denied")
        .append_pair("error_description", "<unsafe>&");
    assert_eq!(
        reqwest::get(denied).await?.text().await?,
        include_str!("../src/oauth/error.html")
            .replace(concat!("$", "{escaped}"), "&lt;unsafe&gt;&amp;")
    );
    let mut exchange = callback.clone();
    exchange
        .query_pairs_mut()
        .append_pair("code", "code")
        .append_pair("state", &params["state"]);
    fake.reply(
        "/chatgpt-pro/token",
        StatusCode::UNAUTHORIZED,
        "application/json",
        json!({}),
    )
    .await;
    assert!(
        reqwest::get(exchange.clone())
            .await?
            .text()
            .await?
            .contains("Token exchange failed: 401")
    );
    fake.reply(
        "/chatgpt-pro/token",
        StatusCode::OK,
        "application/json",
        json!({"access_token":"opaque","expires_in":3600}),
    )
    .await;
    assert_eq!(
        reqwest::get(exchange).await?.text().await?,
        include_str!("../src/oauth/success.html")
    );
    let saved = ctx
        .store
        .get("installation-a", "chatgpt-pro")
        .await?
        .ok_or("missing token")?;
    assert!(saved.refresh_token.is_empty());
    assert!(saved.email.is_none());
    for _ in 0..100 {
        if let Ok(listener) =
            TcpListener::bind(("127.0.0.1", callback.port().ok_or("missing port")?)).await
        {
            drop(listener);
            return Ok(());
        }
        tokio::task::yield_now().await;
    }
    Err("listener not released".into())
}
#[tokio::test(start_paused = true)]
async fn callback_expires_after_five_minutes() -> TestResult {
    let fake = FakeAuth::new(Duration::ZERO).await?;
    let ctx = Context::new(&fake, 0).await?;
    let params = pkce(&ctx.manager).await?;
    tokio::time::advance(Duration::from_secs(301)).await;
    let mut callback = callback_url(&params)?;
    callback
        .query_pairs_mut()
        .append_pair("code", "code")
        .append_pair("state", &params["state"]);
    assert!(
        reqwest::get(callback.clone())
            .await?
            .text()
            .await?
            .contains("OAuth flow expired. Please try again.")
    );
    assert!(
        reqwest::get(callback)
            .await?
            .text()
            .await?
            .contains("Invalid or expired OAuth state")
    );
    assert!(fake.inner.requests.lock().await.is_empty());
    ctx.manager.shutdown().await;
    Ok(())
}
#[tokio::test]
async fn cancel_evicts_holder_and_allows_listener_restart() -> TestResult {
    let fake = FakeAuth::new(Duration::ZERO).await?;
    let holder = TcpListener::bind("127.0.0.1:0").await?;
    let port = holder.local_addr()?.port();
    let cancel = CancellationToken::new();
    let state = cancel.clone();
    let router = Router::new().fallback(move |request: Request| {
        let state = state.clone();
        async move {
            assert_eq!(request.uri().path(), "/cancel");
            state.cancel();
            "Login cancelled"
        }
    });
    let task = tokio::spawn(
        axum::serve(holder, router)
            .with_graceful_shutdown(cancel.cancelled_owned())
            .into_future(),
    );
    let ctx = Context::new(&fake, port).await?;
    let params = pkce(&ctx.manager).await?;
    task.await??;
    assert_eq!(callback_url(&params)?.port(), Some(port));
    assert_eq!(
        reqwest::Client::new()
            .delete(format!("http://127.0.0.1:{port}/cancel"))
            .send()
            .await?
            .text()
            .await?,
        "Login cancelled"
    );
    pkce(&ctx.manager).await?;
    ctx.manager.shutdown().await;
    Ok(())
}
#[tokio::test]
async fn occupied_callback_port_is_503() -> TestResult {
    let fake = FakeAuth::new(Duration::ZERO).await?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let task = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            Router::new().fallback(|| async { StatusCode::NOT_FOUND }),
        )
        .await;
    });
    let ctx = Context::new(&fake, port).await?;
    let response = start_with_manager(&ctx.manager, "chatgpt-pro").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body(response).await?,json!({"error":format!("OAuth callback port {port} is in use by another process. Close other BrowserOS instances or CLI tools and try again.")}).to_string());
    task.abort();
    ctx.manager.shutdown().await;
    Ok(())
}
#[tokio::test(start_paused = true)]
async fn device_json_and_form_pkce_pending_waf_and_slowdown() -> TestResult {
    for provider in [OAuthProvider::GitHubCopilot, OAuthProvider::QwenCode] {
        let mut fake = FakeAuth::new(Duration::ZERO).await?;
        let ctx = Context::new(&fake, 0).await?;
        let auth_path = format!("/{}/authorize", provider.as_str());
        let token_path = format!("/{}/token", provider.as_str());
        fake.reply(&auth_path,StatusCode::OK,"application/json",json!({"device_code":"device","user_code":"ABCD","verification_uri":"https://verify.test/base","verification_uri_complete":"https://verify.test/complete","expires_in":600,"interval":1})).await;
        fake.reply(&token_path, StatusCode::OK, "text/html", json!("WAF"))
            .await;
        fake.reply(
            &token_path,
            StatusCode::BAD_REQUEST,
            "application/json",
            json!({"error":"authorization_pending"}),
        )
        .await;
        fake.reply(
            &token_path,
            StatusCode::BAD_REQUEST,
            "application/json",
            json!({"error":"slow_down","interval":2}),
        )
        .await;
        fake.reply(
            &token_path,
            StatusCode::OK,
            "application/json",
            json!({"access_token":"device-token","refresh_token":"device-refresh"}),
        )
        .await;
        let response = start_with_manager(&ctx.manager, provider.as_str()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body(response).await?,
            r#"{"userCode":"ABCD","verificationUri":"https://verify.test/complete","expiresIn":600}"#
        );
        for _ in 0..5 {
            fake.next_hit().await?;
        }
        for _ in 0..1000 {
            if ctx
                .manager
                .try_status(provider.as_str())
                .await?
                .authenticated
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        let saved = ctx
            .store
            .get("installation-a", provider.as_str())
            .await?
            .ok_or("device token not persisted")?;
        assert_eq!(saved.access_token, "device-token");
        assert_eq!(saved.expires_at, 0);
        assert!(saved.account_id.is_none());
        let requests = fake.inner.requests.lock().await;
        assert_eq!(requests.len(), 5);
        assert_eq!(requests[0].path, auth_path);
        assert_eq!(requests[0].body["scope"], provider.spec().scope);
        for request in requests.iter() {
            assert_eq!(request.accept, "application/json");
            assert_eq!(
                request.content_type,
                if provider == OAuthProvider::QwenCode {
                    "application/x-www-form-urlencoded"
                } else {
                    "application/json"
                }
            );
        }
        for request in &requests[1..] {
            assert_eq!(request.body["device_code"], "device");
            assert_eq!(
                request.body["grant_type"],
                "urn:ietf:params:oauth:grant-type:device_code"
            );
        }
        assert!(requests[1].at.duration_since(requests[0].at) >= Duration::from_secs(4));
        assert!(requests[4].at.duration_since(requests[3].at) >= Duration::from_secs(10));
        if provider == OAuthProvider::QwenCode {
            let verifier = requests[1].body["code_verifier"]
                .as_str()
                .ok_or("missing verifier")?;
            assert_eq!(
                URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
                requests[0].body["code_challenge"]
            );
            assert_eq!(requests[0].body["code_challenge_method"], "S256");
        } else {
            assert!(requests[0].body.get("code_challenge").is_none());
        }
        drop(requests);
        ctx.manager.shutdown().await;
    }
    Ok(())
}
#[tokio::test]
async fn device_start_errors_match_ts() -> TestResult {
    let fake = FakeAuth::new(Duration::ZERO).await?;
    let ctx = Context::new(&fake, 0).await?;
    for (status, ct, data, expected) in [
        (
            StatusCode::FORBIDDEN,
            "text/html",
            json!("WAF"),
            "Authentication service temporarily unavailable. Please try again in a few minutes.",
        ),
        (
            StatusCode::FORBIDDEN,
            "application/json",
            json!({}),
            "Failed to request device code: 403",
        ),
        (
            StatusCode::OK,
            "application/json",
            json!({"error":"blocked"}),
            "Device code error: blocked",
        ),
        (
            StatusCode::OK,
            "application/json",
            json!({}),
            "Invalid device code response",
        ),
    ] {
        fake.reply("/github-copilot/authorize", status, ct, data)
            .await;
        let response = start_with_manager(&ctx.manager, "github-copilot").await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body(response).await?, json!({"error":expected}).to_string());
    }
    ctx.manager.shutdown().await;
    Ok(())
}
#[tokio::test]
async fn refresh_single_flight_rotation_and_claim_retention() -> TestResult {
    let fake = FakeAuth::new(Duration::from_millis(50)).await?;
    let ctx = Context::new(&fake, 0).await?;
    ctx.put(
        OAuthProvider::ChatGptPro,
        now_ms() + 120_000,
        "original-refresh",
    )
    .await?;
    fake.reply(
        "/chatgpt-pro/token",
        StatusCode::OK,
        "application/json",
        json!({"access_token":"new-opaque","expires_in":3600}),
    )
    .await;
    let mut tasks = vec![];
    for _ in 0..16 {
        let manager = ctx.manager.clone();
        tasks.push(tokio::spawn(async move {
            manager.credentials(OAuthProvider::ChatGptPro).await
        }));
    }
    for task in tasks {
        let creds = task.await??;
        assert_eq!(creds.access_token, "new-opaque");
        assert_eq!(creds.account_id.as_deref(), Some("retained-account"));
    }
    let saved = ctx
        .store
        .get("installation-a", "chatgpt-pro")
        .await?
        .ok_or("missing refreshed token")?;
    assert_eq!(saved.refresh_token, "original-refresh");
    assert_eq!(saved.email.as_deref(), Some("retained@example.test"));
    let requests = fake.inner.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body["grant_type"], "refresh_token");
    assert_eq!(requests[0].body["refresh_token"], "original-refresh");
    drop(requests);
    ctx.put(OAuthProvider::QwenCode, 1, "qwen-refresh").await?;
    fake.reply("/qwen-code/token",StatusCode::OK,"application/json",json!({"access_token":jwt(json!({"account_id":"new-account","email":"new@example.test"})),"refresh_token":"rotated","expires_in":3600})).await;
    assert_eq!(
        ctx.manager
            .credentials(OAuthProvider::QwenCode)
            .await?
            .account_id
            .as_deref(),
        Some("new-account")
    );
    assert_eq!(
        ctx.store
            .get("installation-a", "qwen-code")
            .await?
            .ok_or("missing Qwen")?
            .refresh_token,
        "rotated"
    );
    ctx.manager.shutdown().await;
    Ok(())
}
#[tokio::test]
async fn refresh_rejection_deletes_and_malformed_success_preserves_tokens() -> TestResult {
    let fake = FakeAuth::new(Duration::ZERO).await?;
    let ctx = Context::new(&fake, 0).await?;
    ctx.put(OAuthProvider::ChatGptPro, 1, "").await?;
    assert_eq!(
        ctx.manager
            .credentials(OAuthProvider::ChatGptPro)
            .await
            .err()
            .ok_or("expected failure")?
            .to_string(),
        "chatgpt-pro session expired (no refresh token). Please re-login."
    );
    assert!(!ctx.manager.try_status("chatgpt-pro").await?.authenticated);
    ctx.put(OAuthProvider::ChatGptPro, 1, "bad").await?;
    fake.reply(
        "/chatgpt-pro/token",
        StatusCode::UNAUTHORIZED,
        "application/json",
        json!({}),
    )
    .await;
    assert_eq!(
        ctx.manager
            .credentials(OAuthProvider::ChatGptPro)
            .await
            .err()
            .ok_or("expected failure")?
            .to_string(),
        "ChatGPT session expired. Please re-login."
    );
    assert!(!ctx.manager.try_status("chatgpt-pro").await?.authenticated);
    ctx.put(OAuthProvider::ChatGptPro, 1, "valid").await?;
    fake.reply(
        "/chatgpt-pro/token",
        StatusCode::OK,
        "text/html",
        json!("not a token"),
    )
    .await;
    assert!(
        ctx.manager
            .credentials(OAuthProvider::ChatGptPro)
            .await
            .is_err()
    );
    assert!(ctx.manager.try_status("chatgpt-pro").await?.authenticated);
    ctx.manager.shutdown().await;
    Ok(())
}
#[tokio::test]
async fn copilot_no_refresh_durable_storage_and_logout() -> TestResult {
    let fake = FakeAuth::new(Duration::ZERO).await?;
    let ctx = Context::new(&fake, 0).await?;
    for (provider, expiry) in [
        (OAuthProvider::GitHubCopilot, 1),
        (OAuthProvider::ChatGptPro, 0),
        (OAuthProvider::QwenCode, now_ms() + 3_600_000),
    ] {
        ctx.put(provider, expiry, "").await?;
        assert_eq!(ctx.manager.credentials(provider).await?.access_token, "old");
    }
    assert!(fake.inner.requests.lock().await.is_empty());
    assert_eq!(
        serde_json::to_value(ctx.manager.try_status("unknown").await?)?,
        json!({"authenticated":false,"provider":"unknown"})
    );
    let (reopened, _) = Database::open(&ctx.paths).await?;
    assert_eq!(
        reopened
            .oauth_tokens()
            .get("installation-a", "chatgpt-pro")
            .await?
            .ok_or("not durable")?
            .access_token,
        "old"
    );
    ctx.manager.try_logout("chatgpt-pro").await?;
    assert!(!ctx.manager.try_status("chatgpt-pro").await?.authenticated);
    assert!(ctx.manager.try_status("qwen-code").await?.authenticated);
    assert_eq!(
        ctx.manager
            .credentials(OAuthProvider::ChatGptPro)
            .await
            .err()
            .ok_or("expected missing")?
            .to_string(),
        "Not authenticated with ChatGPT. Please login first."
    );
    ctx.manager.shutdown().await;
    Ok(())
}
#[tokio::test]
async fn route_untyped_body_and_text_errors_match_bun_goldens() -> TestResult {
    let fake = FakeAuth::new(Duration::ZERO).await?;
    let ctx = Context::new(&fake, 0).await?;
    // Captured by executing the TS route with Bun, including null versus primitive JSON.
    for (provider, input, status, expected) in [
        (
            "unknown",
            "{}",
            StatusCode::BAD_REQUEST,
            "Unknown OAuth provider: unknown",
        ),
        (
            "chatgpt-pro",
            "{",
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to store token",
        ),
        (
            "chatgpt-pro",
            "null",
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to store token",
        ),
        (
            "chatgpt-pro",
            "{}",
            StatusCode::BAD_REQUEST,
            "Missing accessToken",
        ),
        (
            "chatgpt-pro",
            "false",
            StatusCode::BAD_REQUEST,
            "Missing accessToken",
        ),
        (
            "chatgpt-pro",
            r#"{"accessToken":0}"#,
            StatusCode::BAD_REQUEST,
            "Missing accessToken",
        ),
        (
            "chatgpt-pro",
            r#"{"accessToken":{}}"#,
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to store token",
        ),
    ] {
        let response = store_token_with_manager(&ctx.manager, provider, input.as_bytes()).await;
        assert_eq!(response.status(), status);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/plain; charset=UTF-8"
        );
        assert_eq!(body(response).await?, expected);
    }
    for (input, expected) in [
        (r#"{"accessToken":true}"#, "1"),
        (r#"{"accessToken":42}"#, "42"),
        (r#"{"accessToken":"token","expiresIn":"0"}"#, "token"),
    ] {
        let response =
            store_token_with_manager(&ctx.manager, "chatgpt-pro", input.as_bytes()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body(response).await?, r#"{"ok":true}"#);
        let token = ctx
            .store
            .get("installation-a", "chatgpt-pro")
            .await?
            .ok_or("missing submitted token")?;
        assert_eq!(token.access_token, expected);
        assert!(token.email.is_none());
        assert!(token.account_id.is_none());
        if expected == "token" {
            assert!(token.expires_at >= now_ms() - 1000)
        } else {
            assert_eq!(token.expires_at, 0)
        }
    }
    ctx.manager
        .store_token(
            OAuthProvider::ChatGptPro,
            TokenInput {
                access_token: jwt(json!({"email":"untrusted","account_id":"untrusted"})),
                refresh_token: None,
                expires_at: None,
            },
        )
        .await?;
    assert!(ctx.manager.try_status("chatgpt-pro").await?.email.is_none());
    let unknown = start_with_manager(&ctx.manager, "unknown").await;
    assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body(unknown).await?, "Unknown OAuth provider: unknown");
    ctx.manager.shutdown().await;
    Ok(())
}
