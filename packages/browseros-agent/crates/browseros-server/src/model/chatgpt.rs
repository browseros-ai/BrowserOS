//! Owns the ChatGPT sign-in, including the private OAuth callback listener and token rotation.
//! The rest of the model layer receives a configured endpoint for the shared Responses wire.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::{Query, State};
use axum::http::uri::Authority;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio::time::{Instant, sleep, sleep_until};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

use super::responses::{ResponsesDialect, ResponsesEndpoint};
use crate::store::{Secret, Store, StoreError, TokenRow};
use crate::turn::{ChatError, ChatErrorCode};

const PROVIDER: &str = "chatgpt-pro";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CALLBACK_PORT: u16 = 1455;
const CALLBACK_PATH: &str = "/auth/callback";
const FLOW_TTL: Duration = Duration::from_secs(5 * 60);
const REFRESH_BUFFER_MS: i64 = 5 * 60 * 1000;
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// Owns the install's pending sign-ins and stored ChatGPT credentials.
pub struct SignIn {
    inner: Arc<SignInInner>,
}

struct SignInInner {
    store: Store,
    install_id: Option<String>,
    endpoints: OAuthEndpoints,
    http: Result<Client, reqwest::Error>,
    // Serializes token writes with each other and with sign-out. Status reads the last
    // committed row without waiting for an exchange or refresh to finish.
    session: Arc<Mutex<Option<CallbackListener>>>,
}

struct OAuthEndpoints {
    authorize: String,
    token: String,
    responses: String,
    callback_port: u16,
    flow_ttl: Duration,
}

impl Default for OAuthEndpoints {
    fn default() -> Self {
        Self {
            authorize: "https://auth.openai.com/oauth/authorize".into(),
            token: "https://auth.openai.com/oauth/token".into(),
            responses: "https://chatgpt.com/backend-api/codex/responses".into(),
            callback_port: CALLBACK_PORT,
            flow_ttl: FLOW_TTL,
        }
    }
}

/// The listener owns the PKCE flows for one bind. Dropping it stops accepting requests
/// while Axum drains any callback response already being written.
struct CallbackListener {
    id: Uuid,
    port: u16,
    stop: CancellationToken,
    pending: HashMap<String, PendingFlow>,
}

impl CallbackListener {
    fn register(&mut self, authorize: Url, inner: &Arc<SignInInner>) -> Url {
        let state = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 16]>());
        let verifier = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        let expires = Instant::now() + inner.endpoints.flow_ttl;
        let authorize = authorize_url(authorize, self.port, &state, &verifier);
        self.pending.insert(
            state.clone(),
            PendingFlow {
                verifier: Secret::new(verifier),
                expires,
            },
        );

        let inner = Arc::downgrade(inner);
        let id = self.id;
        let stop = self.stop.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = stop.cancelled() => {}
                () = sleep_until(expires) => {
                    if let Some(inner) = inner.upgrade() {
                        let mut listener = inner.session.lock().await;
                        take_flow(&mut listener, id, &state);
                    }
                }
            }
        });

        authorize
    }
}

impl Drop for CallbackListener {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

struct PendingFlow {
    verifier: Secret,
    expires: Instant,
}

/// Identifies the binding that accepted a request, including during graceful shutdown.
/// A listener must never keep the sign-in alive or mutate a later binding.
#[derive(Clone)]
struct CallbackState {
    sign_in: Weak<SignInInner>,
    id: Uuid,
    port: u16,
}

/// Why a sign-in operation could not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignInError {
    /// The callback port is held by a program that would not let go.
    PortBusy,
    /// No readable install id: tokens are keyed by it, so they could not be stored.
    NoInstallId,
    /// A failure with a safe message for the extension.
    Failed(String),
}

/// Whether tokens are stored for this install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInStatus {
    pub authenticated: bool,
    pub email: Option<String>,
}

impl SignIn {
    pub(super) fn new(store: Store, install_id: Option<String>) -> Self {
        Self::with_endpoints(store, install_id, OAuthEndpoints::default())
    }

    fn with_endpoints(store: Store, install_id: Option<String>, endpoints: OAuthEndpoints) -> Self {
        let http = Client::builder()
            .timeout(HTTP_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build();
        Self {
            inner: Arc::new(SignInInner {
                store,
                install_id,
                endpoints,
                http,
                session: Arc::new(Mutex::new(None)),
            }),
        }
    }

    /// Starts the loopback listener if needed and returns an authorize URL for a new PKCE flow.
    pub async fn start(&self) -> Result<Url, SignInError> {
        self.inner.install_id()?;
        self.inner.http.as_ref().map_err(|error| {
            tracing::error!(%error, "could not build the ChatGPT sign-in client");
            SignInError::Failed("Could not start ChatGPT sign-in.".into())
        })?;
        let authorize = Url::parse(&self.inner.endpoints.authorize).map_err(|error| {
            tracing::error!(%error, "invalid ChatGPT authorize URL");
            SignInError::Failed("Could not build the ChatGPT sign-in URL.".into())
        })?;

        let mut listener = self.inner.session.lock().await;
        let listener = match listener.as_mut() {
            Some(listener) => listener,
            None => listener.insert(start_listener(&self.inner).await?),
        };

        Ok(listener.register(authorize, &self.inner))
    }

    /// Reports whether a token row exists. Polling never starts a refresh.
    pub async fn status(&self) -> Result<SignInStatus, SignInError> {
        let install_id = self.inner.install_id()?;
        let row = self
            .inner
            .store
            .tokens()
            .get(install_id, PROVIDER)
            .await
            .map_err(|error| store_failure(&error))?;
        Ok(SignInStatus {
            authenticated: row.is_some(),
            email: row.and_then(|row| row.email),
        })
    }

    /// Deletes the token row and cancels pending callbacks so they cannot sign the install back in.
    pub async fn sign_out(&self) -> Result<(), SignInError> {
        let install_id = self.inner.install_id()?;
        let mut listener = self.inner.session.lock().await;
        self.inner
            .store
            .tokens()
            .delete(install_id, PROVIDER)
            .await
            .map_err(|error| store_failure(&error))?;
        *listener = None;
        Ok(())
    }

    /// Gets the credentials for a turn, refreshing once under the same lock used for callbacks
    /// and sign-out. The Codex Responses request and its body rules live in the shared wire.
    pub(super) async fn endpoint(&self) -> Result<ResponsesEndpoint, ChatError> {
        let install_id = self
            .inner
            .install_id()
            .map_err(|_| auth_failed("Could not read the BrowserOS installation id."))?;
        let guard = Arc::clone(&self.inner.session).lock_owned().await;
        let row = self
            .inner
            .store
            .tokens()
            .get(install_id, PROVIDER)
            .await
            .map_err(|error| token_store_error(&error))?
            .ok_or_else(|| auth_failed("Please sign in to ChatGPT."))?;
        if row.expires_at == 0 || row.expires_at > now_ms().saturating_add(REFRESH_BUFFER_MS) {
            return self.inner.responses_endpoint(&row);
        }

        // A stopped turn can drop its waiter. The exchange must still commit any rotated
        // refresh token before releasing the lock to another turn or to sign-out.
        let inner = Arc::clone(&self.inner);
        tokio::spawn(refresh(inner, guard, row))
            .await
            .map_err(|_| {
                chat_error(
                    ChatErrorCode::Unknown,
                    "Could not refresh the ChatGPT session.",
                )
            })?
    }
}

impl SignInInner {
    fn install_id(&self) -> Result<&str, SignInError> {
        self.install_id.as_deref().ok_or(SignInError::NoInstallId)
    }

    fn responses_endpoint(&self, row: &TokenRow) -> Result<ResponsesEndpoint, ChatError> {
        let url = Url::parse(&self.endpoints.responses).map_err(|_| {
            chat_error(
                ChatErrorCode::ProviderConfig,
                "The ChatGPT Responses URL is invalid.",
            )
        })?;
        let mut headers = vec![
            (
                "Authorization".into(),
                format!("Bearer {}", row.access.expose()),
            ),
            ("originator".into(), "browseros".into()),
            ("OpenAI-Beta".into(), "responses=experimental".into()),
        ];
        // The old server omitted this header for tokens without an account claim.
        if let Some(account_id) = row.account_id.as_ref().filter(|id| !id.is_empty()) {
            headers.push(("ChatGPT-Account-Id".into(), account_id.clone()));
        }
        Ok(ResponsesEndpoint {
            url,
            headers,
            dialect: ResponsesDialect::ChatGptCodex,
        })
    }

    async fn refresh_grant(&self, token: &Secret) -> Result<TokenResponse, ExchangeError> {
        if token.expose().is_empty() {
            return Err(ExchangeError::Refused);
        }
        self.exchange(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", token.expose()),
        ])
        .await
    }

    async fn exchange(&self, form: &[(&str, &str)]) -> Result<TokenResponse, ExchangeError> {
        let http = self.http.as_ref().map_err(|error| {
            tracing::error!(%error, "could not build the ChatGPT sign-in client");
            ExchangeError::Unavailable
        })?;
        let response = http.post(&self.endpoints.token).form(form).send().await
            .map_err(|error| {
                tracing::warn!(error = %error.without_url(), "ChatGPT token service request failed");
                ExchangeError::Unavailable
            })?;
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "ChatGPT token service refused the request");
            return Err(ExchangeError::Refused);
        }
        let tokens: TokenResponse = response.json().await.map_err(|error| {
            tracing::warn!(error = %error.without_url(), "invalid ChatGPT token response");
            ExchangeError::InvalidResponse
        })?;
        if tokens.access_token.is_empty() {
            return Err(ExchangeError::InvalidResponse);
        }
        Ok(tokens)
    }
}

async fn start_listener(inner: &Arc<SignInInner>) -> Result<CallbackListener, SignInError> {
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, inner.endpoints.callback_port);
    let listener = bind_callback(address).await?;
    let port = listener.local_addr().map_err(bind_failure)?.port();
    let id = Uuid::new_v4();
    let stop = CancellationToken::new();
    let routes = Router::new()
        .route(CALLBACK_PATH, get(callback))
        .route("/cancel", get(cancel))
        .with_state(CallbackState {
            sign_in: Arc::downgrade(inner),
            id,
            port,
        });
    let shutdown = stop.clone();
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, routes)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
        {
            tracing::warn!(%error, "ChatGPT callback listener stopped");
        }
    });
    Ok(CallbackListener {
        id,
        port,
        stop,
        pending: HashMap::new(),
    })
}

async fn bind_callback(address: SocketAddrV4) -> Result<TcpListener, SignInError> {
    for attempt in 0..5 {
        match TcpListener::bind(address).await {
            Ok(listener) => return Ok(listener),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {}
            Err(error) => return Err(bind_failure(error)),
        }
        if attempt == 0 {
            // Ask the current loopback owner to release the port, as the Codex CLI does.
            // This request is local only, even on machines with a configured HTTP proxy.
            if let Ok(http) = Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(2))
                .build()
            {
                let _ = http.get(format!("http://{address}/cancel")).send().await;
            }
        }
        if attempt < 4 {
            sleep(Duration::from_millis(300)).await;
        }
    }
    Err(SignInError::PortBusy)
}

fn bind_failure(error: std::io::Error) -> SignInError {
    tracing::error!(%error, "could not bind the ChatGPT callback listener");
    SignInError::Failed("Could not open the ChatGPT callback listener.".into())
}

fn authorize_url(mut authorize: Url, port: u16, state: &str, verifier: &str) -> Url {
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    authorize.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", &redirect_uri(port)),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        ("scope", "openid profile email offline_access"),
        ("state", state),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", "browseros"),
    ]);
    authorize
}

fn redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}{CALLBACK_PATH}")
}

fn take_flow(
    listener: &mut Option<CallbackListener>,
    id: Uuid,
    state: &str,
) -> Option<PendingFlow> {
    let binding = listener.as_mut().filter(|binding| binding.id == id)?;
    let flow = binding.pending.remove(state);
    if binding.pending.is_empty() {
        *listener = None;
    }
    flow
}

fn is_callback_host(headers: &HeaderMap, port: u16) -> bool {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<Authority>().ok())
    else {
        return false;
    };
    host.port_u16() == Some(port)
        && (host.host().eq_ignore_ascii_case("localhost") || host.host() == "127.0.0.1")
}

#[derive(Deserialize)]
struct CallbackQuery {
    state: String,
    code: Option<String>,
    error: Option<String>,
}

async fn callback(
    State(state): State<CallbackState>,
    method: Method,
    headers: HeaderMap,
    query: Result<Query<CallbackQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    if method != Method::GET {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    if !is_callback_host(&headers, state.port) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(Query(query)) = query else {
        return callback_failure(
            StatusCode::BAD_REQUEST,
            "Missing or invalid sign-in parameters.",
        );
    };
    let Some(inner) = state.sign_in.upgrade() else {
        return callback_failure(StatusCode::BAD_REQUEST, "Invalid or expired sign-in state.");
    };
    let mut listener = Arc::clone(&inner.session).lock_owned().await;
    let Some(flow) = take_flow(&mut listener, state.id, &query.state)
        .filter(|flow| flow.expires > Instant::now())
    else {
        return callback_failure(StatusCode::BAD_REQUEST, "Invalid or expired sign-in state.");
    };
    if query.error.is_some() {
        return callback_failure(
            StatusCode::BAD_REQUEST,
            "You did not authorize the sign-in.",
        );
    }
    let Some(code) = query.code.filter(|code| !code.is_empty()) else {
        return callback_failure(StatusCode::BAD_REQUEST, "Missing authorization code.");
    };

    // The browser can close the callback tab once the exchange has started. The task owns
    // the one-time grant and the lock until the row is committed, even if the handler is dropped.
    tokio::spawn(complete_sign_in(inner, listener, state.port, code, flow))
        .await
        .unwrap_or_else(|_| {
            callback_failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not complete the ChatGPT sign-in.",
            )
        })
}

async fn complete_sign_in(
    inner: Arc<SignInInner>,
    _guard: OwnedMutexGuard<Option<CallbackListener>>,
    port: u16,
    code: String,
    flow: PendingFlow,
) -> Response {
    let Ok(install_id) = inner.install_id() else {
        return callback_failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "Could not read the BrowserOS installation id.",
        );
    };

    let tokens = inner
        .exchange(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", &code),
            ("redirect_uri", &redirect_uri(port)),
            ("code_verifier", flow.verifier.expose()),
        ])
        .await;
    let Ok(tokens) = tokens else {
        return callback_failure(
            StatusCode::BAD_GATEWAY,
            "Could not exchange the ChatGPT authorization code.",
        );
    };
    if let Err(error) = inner
        .store
        .tokens()
        .put(install_id, PROVIDER, tokens.into_row(None))
        .await
    {
        let _ = store_failure(&error);
        return callback_failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not save the ChatGPT sign-in.",
        );
    }
    callback_page(
        StatusCode::OK,
        "Authentication Successful",
        "You can close this tab and return to BrowserOS.",
    )
}

async fn cancel(
    State(state): State<CallbackState>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    if method != Method::GET {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    // This handoff is for local programs. A browser visit must not cancel a pending login.
    if !is_callback_host(&headers, state.port)
        || headers.contains_key(header::ORIGIN)
        || headers.contains_key("sec-fetch-site")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(inner) = state.sign_in.upgrade() {
        let mut listener = inner.session.lock().await;
        if listener
            .as_ref()
            .is_some_and(|listener| listener.id == state.id)
        {
            *listener = None;
        }
    }
    "Login cancelled".into_response()
}

fn callback_failure(status: StatusCode, message: &'static str) -> Response {
    callback_page(status, "Authentication Failed", message)
}

fn callback_page(status: StatusCode, title: &'static str, message: &'static str) -> Response {
    // Only fixed text enters the page. The callback URL carries a one-time authorization code.
    let page = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\">\
         <title>BrowserOS | {title}</title><h1>{title}</h1><p>{message}</p></html>"
    );
    (
        status,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        Html(page),
    )
        .into_response()
}

#[derive(Debug)]
enum ExchangeError {
    Refused,
    Unavailable,
    InvalidResponse,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: i64,
}

impl TokenResponse {
    fn into_row(self, previous: Option<TokenRow>) -> TokenRow {
        let claims = token_claims(&self.access_token);
        let (refresh, email, account_id) = previous.map_or_else(
            || (Secret::new(""), None, None),
            |row| (row.refresh, row.email, row.account_id),
        );
        TokenRow {
            access: Secret::new(self.access_token),
            refresh: self.refresh_token.map_or(refresh, Secret::new),
            expires_at: now_ms().saturating_add(self.expires_in.saturating_mul(1000)),
            email: claims
                .profile
                .and_then(|profile| profile.email)
                .or(claims.email)
                .or(email),
            account_id: claims
                .auth
                .and_then(|auth| auth.chatgpt_account_id)
                .or(claims.chatgpt_account_id)
                .or(claims.account_id)
                .or(account_id),
        }
    }
}

#[derive(Default, Deserialize)]
struct TokenClaims {
    #[serde(rename = "https://api.openai.com/auth")]
    auth: Option<AuthClaims>,
    #[serde(rename = "https://api.openai.com/profile")]
    profile: Option<ProfileClaims>,
    chatgpt_account_id: Option<String>,
    account_id: Option<String>,
    email: Option<String>,
}

#[derive(Deserialize)]
struct AuthClaims {
    chatgpt_account_id: Option<String>,
}

#[derive(Deserialize)]
struct ProfileClaims {
    email: Option<String>,
}

fn token_claims(access_token: &str) -> TokenClaims {
    // The token came from the token endpoint over HTTPS. These claims supply metadata for
    // display and routing. The Responses backend validates the bearer itself.
    let mut segments = access_token.split('.');
    match (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) {
        (Some(_), Some(payload), Some(_), None) => URL_SAFE_NO_PAD
            .decode(payload)
            .ok()
            .and_then(|json| serde_json::from_slice(&json).ok())
            .unwrap_or_default(),
        _ => TokenClaims::default(),
    }
}

async fn refresh(
    inner: Arc<SignInInner>,
    _guard: OwnedMutexGuard<Option<CallbackListener>>,
    row: TokenRow,
) -> Result<ResponsesEndpoint, ChatError> {
    let install_id = inner
        .install_id()
        .map_err(|_| auth_failed("Could not read the BrowserOS installation id."))?;
    let tokens = inner.store.tokens();
    match inner.refresh_grant(&row.refresh).await {
        Ok(response) => {
            let row = response.into_row(Some(row));
            // Persist the pair before another turn can try to use the old refresh token.
            let endpoint = inner.responses_endpoint(&row);
            tokens
                .put(install_id, PROVIDER, row)
                .await
                .map_err(|error| token_store_error(&error))?;
            endpoint
        }
        Err(ExchangeError::Refused) => {
            tokens
                .delete(install_id, PROVIDER)
                .await
                .map_err(|error| token_store_error(&error))?;
            Err(auth_failed(
                "ChatGPT session expired. Please sign in again.",
            ))
        }
        Err(ExchangeError::Unavailable | ExchangeError::InvalidResponse) => Err(chat_error(
            ChatErrorCode::ProviderUnavailable,
            "Could not refresh the ChatGPT session. Please try again.",
        )),
    }
}

fn now_ms() -> i64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
}

fn store_failure(error: &StoreError) -> SignInError {
    tracing::error!(%error, "could not access the ChatGPT token store");
    SignInError::Failed("Could not access the ChatGPT sign-in store.".into())
}

fn token_store_error(error: &StoreError) -> ChatError {
    tracing::error!(%error, "could not access the ChatGPT token store");
    chat_error(
        ChatErrorCode::Unknown,
        "Could not access the ChatGPT sign-in store.",
    )
}

fn auth_failed(message: &'static str) -> ChatError {
    chat_error(ChatErrorCode::AuthFailed, message)
}

fn chat_error(code: ChatErrorCode, message: &'static str) -> ChatError {
    let mut error = ChatError::new(code, "ChatGPT sign-in", message);
    error.provider = Some(PROVIDER.into());
    error
}

#[cfg(test)]
#[path = "../../tests/chatgpt_model/mod.rs"]
mod tests;
