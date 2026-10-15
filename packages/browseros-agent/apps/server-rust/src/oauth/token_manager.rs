//! Owns provider authentication, durable credentials and single-flight refresh for chat.
//! Interactive flows outlive their initiating HTTP request; the callback holds a weak manager
//! and device pollers are cancelled on shutdown, avoiding application/task ownership cycles.

use super::{CallbackServer, OAuthProvider, ProviderSpec};
use crate::db::{OAuthTokenStore, OAuthTokens};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const FLOW_TTL: Duration = Duration::from_secs(300);
const EXPIRY_BUFFER_MS: i64 = 300_000;

#[derive(Debug)]
pub enum StartFlow {
    Device {
        user_code: String,
        verification_uri: String,
        expires_in: u64,
    },
    /// The route returns 302, preserving the extension's redirect-following flow.
    Redirect(url::Url),
}

/// Caller-supplied credentials; expires_at is an absolute epoch-millisecond value.
/// The HTTP adapter translates the existing expiresIn seconds before crossing this seam.
#[derive(Debug, Clone)]
pub struct TokenInput {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OAuthStatus {
    pub authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub provider: String,
}

#[derive(Debug, Clone)]
pub struct OAuthCredentials {
    pub access_token: String,
    pub account_id: Option<String>,
}

/// Text is surfaced verbatim by the existing HTTP and chat error dialects.
#[derive(Debug, Clone, thiserror::Error)]
pub enum OAuthError {
    #[error("{0}")]
    NotAuthenticated(String),
    #[error("{0}")]
    Expired(String),
    #[error("{0}")]
    Upstream(String),
    #[error("{0}")]
    Unavailable(String),
}

struct PendingFlow {
    provider: OAuthProvider,
    verifier: String,
    created_at: tokio::time::Instant,
    redirect_uri: String,
}

type RefreshResult = Result<OAuthTokens, OAuthError>;
type RefreshFuture = Shared<BoxFuture<'static, RefreshResult>>;

/// One manager per BrowserOS installation; every database operation includes its installation
/// id. Shared refresh results include failures so concurrent turns observe the same outcome.
pub struct TokenManager {
    store: OAuthTokenStore,
    browseros_id: String,
    pub(super) http: reqwest::Client,
    callback: CallbackServer,
    callback_port: u16,
    providers: HashMap<OAuthProvider, ProviderSpec>,
    pending: Mutex<HashMap<String, PendingFlow>>,
    refreshes: Arc<Mutex<HashMap<OAuthProvider, (Uuid, RefreshFuture)>>>,
    stopped: CancellationToken,
}

impl TokenManager {
    pub fn new(store: OAuthTokenStore, browseros_id: String, http: reqwest::Client) -> Arc<Self> {
        Self::with_provider_specs(
            store,
            browseros_id,
            http,
            OAuthProvider::ALL.map(OAuthProvider::spec),
            CallbackServer::PORT,
        )
    }

    /// Injection seam for fake auth servers. Production uses new; tests use loopback upstreams
    /// and callback port zero so they never bind or evict the real registered port 1455.
    pub fn with_provider_specs(
        store: OAuthTokenStore,
        browseros_id: String,
        http: reqwest::Client,
        providers: [ProviderSpec; 3],
        callback_port: u16,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            browseros_id,
            http,
            callback: CallbackServer::new(),
            callback_port,
            providers: providers.into_iter().map(|s| (s.provider, s)).collect(),
            pending: Mutex::new(HashMap::new()),
            refreshes: Arc::new(Mutex::new(HashMap::new())),
            stopped: CancellationToken::new(),
        })
    }

    fn spec(&self, provider: OAuthProvider) -> Result<&ProviderSpec, OAuthError> {
        self.providers
            .get(&provider)
            .ok_or_else(|| upstream(format!("Unknown OAuth provider: {}", provider.as_str())))
    }

    pub async fn start(self: &Arc<Self>, provider: OAuthProvider) -> Result<StartFlow, OAuthError> {
        let spec = self.spec(provider)?;
        if provider != OAuthProvider::ChatGptPro {
            return self.start_device(spec).await;
        }
        let port = self
            .callback
            .ensure_running(self, self.callback_port)
            .await?;
        let verifier = random_verifier();
        let state = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 16]>());
        let redirect_uri = format!("http://localhost:{port}/auth/callback");
        let mut url = url::Url::parse(&spec.auth_endpoint).map_err(upstream)?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", spec.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("code_challenge", &challenge(&verifier))
            .append_pair("code_challenge_method", "S256")
            .append_pair("scope", spec.scope)
            .append_pair("state", &state)
            .append_pair("id_token_add_organizations", "true")
            .append_pair("codex_cli_simplified_flow", "true")
            .append_pair("originator", "browseros");
        let mut pending = self.pending.lock().await;
        let now = tokio::time::Instant::now();
        pending.retain(|_, flow| now.duration_since(flow.created_at) <= FLOW_TTL);
        pending.insert(
            state,
            PendingFlow {
                provider,
                verifier,
                created_at: now,
                redirect_uri,
            },
        );
        Ok(StartFlow::Redirect(url))
    }

    pub(super) async fn handle_callback(&self, code: &str, state: &str) -> Result<(), OAuthError> {
        // Hold the flow lock through exchange/persistence so a state can succeed only once.
        // Failed exchanges leave it available for retry, matching TS; successful ones consume it.
        let mut pending = self.pending.lock().await;
        let flow = pending
            .get(state)
            .ok_or_else(|| upstream("Invalid or expired OAuth state"))?;
        if flow.created_at.elapsed() > FLOW_TTL {
            pending.remove(state);
            return Err(upstream("OAuth flow expired. Please try again."));
        }
        let spec = self.spec(flow.provider)?;
        let response = self
            .http
            .post(&spec.token_endpoint)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", spec.client_id),
                ("code", code),
                ("redirect_uri", &flow.redirect_uri),
                ("code_verifier", &flow.verifier),
            ])
            .send()
            .await
            .map_err(upstream)?;
        if !response.status().is_success() {
            return Err(upstream(format!(
                "Token exchange failed: {}",
                response.status().as_u16()
            )));
        }
        let data: Value = response.json().await.map_err(upstream)?;
        let tokens = endpoint_tokens(&data, None)?;
        self.store
            .upsert(&self.browseros_id, flow.provider.as_str(), &tokens)
            .await
            .map_err(upstream)?;
        pending.remove(state);
        if pending.is_empty() {
            self.callback.stop().await;
        }
        Ok(())
    }

    async fn start_device(self: &Arc<Self>, spec: &ProviderSpec) -> Result<StartFlow, OAuthError> {
        let mut params = vec![
            ("client_id", spec.client_id.to_owned()),
            ("scope", spec.scope.to_owned()),
        ];
        let verifier = (spec.provider == OAuthProvider::QwenCode).then(random_verifier);
        if let Some(verifier) = &verifier {
            params.push(("code_challenge", challenge(verifier)));
            params.push(("code_challenge_method", "S256".to_owned()));
        }
        let response = device_request(&self.http, spec, &spec.auth_endpoint, &params)
            .send()
            .await
            .map_err(upstream)?;
        if !is_json(&response) {
            return Err(upstream(
                "Authentication service temporarily unavailable. Please try again in a few minutes.",
            ));
        }
        if !response.status().is_success() {
            return Err(upstream(format!(
                "Failed to request device code: {}",
                response.status().as_u16()
            )));
        }
        let data: Value = response.json().await.map_err(upstream)?;
        if let Some(error) = data.get("error") {
            return Err(upstream(format!("Device code error: {}", js_text(error))));
        }
        let device_code = data
            .get("device_code")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| upstream("Invalid device code response"))?
            .to_owned();
        let user_code = data
            .get("user_code")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| upstream("Invalid device code response"))?
            .to_owned();
        let expires_in = data
            .get("expires_in")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let interval = data
            .get("interval")
            .and_then(Value::as_f64)
            .unwrap_or_default();
        let verification_uri = data
            .get("verification_uri_complete")
            .or_else(|| data.get("verification_uri"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();

        let manager = Arc::downgrade(self);
        let spec = spec.clone();
        let http = self.http.clone();
        let cancel = self.stopped.clone();
        // TS retries start independent pollers (its activeDeviceFlows set is never consulted).
        // Preserve that behavior, but cancel detached work when this manager shuts down.
        tokio::spawn(async move {
            tokio::select! {
                () = cancel.cancelled() => {}
                () = async {
                    if let Some(tokens) = poll_device(&http, &spec, device_code, interval, expires_in, verifier).await
                        && let Some(manager) = manager.upgrade()
                        && let Err(error) = manager.store.upsert(&manager.browseros_id, spec.provider.as_str(), &tokens).await {
                        tracing::warn!(%error, "Failed to persist device OAuth credentials");
                    }
                } => {}
            }
        });
        Ok(StartFlow::Device {
            user_code,
            verification_uri,
            expires_in,
        })
    }

    pub async fn store_token(
        &self,
        provider: OAuthProvider,
        input: TokenInput,
    ) -> Result<(), OAuthError> {
        // Extension-provided tokens intentionally do not contribute trusted account/email claims.
        self.store
            .upsert(
                &self.browseros_id,
                provider.as_str(),
                &OAuthTokens {
                    access_token: input.access_token,
                    refresh_token: input.refresh_token.unwrap_or_default(),
                    expires_at: input.expires_at.unwrap_or_default(),
                    email: None,
                    account_id: None,
                },
            )
            .await
            .map_err(upstream)
    }

    /// The route uses this fallible form so storage failures never turn into false success.
    pub async fn try_status(&self, provider: &str) -> Result<OAuthStatus, OAuthError> {
        let tokens = self
            .store
            .get(&self.browseros_id, provider)
            .await
            .map_err(upstream)?;
        Ok(OAuthStatus {
            authenticated: tokens.is_some(),
            email: tokens.and_then(|t| t.email),
            provider: provider.to_owned(),
        })
    }

    /// Unknown provider strings remain valid status lookups, as in TS.
    pub async fn status(&self, provider: &str) -> OAuthStatus {
        self.try_status(provider).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "OAuth status lookup failed");
            OAuthStatus {
                authenticated: false,
                email: None,
                provider: provider.to_owned(),
            }
        })
    }

    pub async fn try_logout(&self, provider: &str) -> Result<(), OAuthError> {
        self.store
            .delete(&self.browseros_id, provider)
            .await
            .map_err(upstream)
    }

    pub async fn logout(&self, provider: &str) {
        if let Err(error) = self.try_logout(provider).await {
            tracing::warn!(%error, "OAuth logout failed");
        }
    }

    /// Refresh at expiresAt - 300 seconds; Copilot always returns the stored access token.
    pub async fn credentials(
        &self,
        provider: OAuthProvider,
    ) -> Result<OAuthCredentials, OAuthError> {
        let spec = self.spec(provider)?;
        let missing = || {
            OAuthError::NotAuthenticated(format!(
                "Not authenticated with {}. Please login first.",
                spec.display_name
            ))
        };
        let mut refreshes = self.refreshes.lock().await;
        let refresh = if let Some((_, existing)) = refreshes.get(&provider) {
            existing.clone()
        } else {
            let tokens = self
                .store
                .get(&self.browseros_id, provider.as_str())
                .await
                .map_err(upstream)?
                .ok_or_else(missing)?;
            if provider == OAuthProvider::GitHubCopilot
                || tokens.expires_at == 0
                || now_ms() < tokens.expires_at.saturating_sub(EXPIRY_BUFFER_MS)
            {
                return Ok(credentials(tokens));
            }
            let id = Uuid::new_v4();
            let store = self.store.clone();
            let browseros_id = self.browseros_id.clone();
            let http = self.http.clone();
            let spec = spec.clone();
            let completed = self.refreshes.clone();
            // Start eagerly: dropping an HTTP/chat waiter must not cancel a rotating refresh
            // token exchange after the provider has already consumed the old refresh token.
            let task = tokio::spawn(async move {
                let result = execute_refresh(store, browseros_id, http, spec, tokens).await;
                // Cleanup belongs to the producer, not the request awaiting it. Otherwise an
                // aborted turn could leave a completed failure cached across the next login.
                let mut refreshes = completed.lock().await;
                if refreshes
                    .get(&provider)
                    .is_some_and(|(current, _)| *current == id)
                {
                    refreshes.remove(&provider);
                }
                result
            });
            let refresh = async move { task.await.map_err(upstream)? }
                .boxed()
                .shared();
            refreshes.insert(provider, (id, refresh.clone()));
            refresh
        };
        drop(refreshes);
        refresh.await.map(credentials)
    }

    pub async fn shutdown(&self) {
        self.stopped.cancel();
        self.callback.stop().await;
    }
}

impl Drop for TokenManager {
    fn drop(&mut self) {
        self.stopped.cancel();
    }
}

async fn execute_refresh(
    store: OAuthTokenStore,
    browseros_id: String,
    http: reqwest::Client,
    spec: ProviderSpec,
    tokens: OAuthTokens,
) -> RefreshResult {
    if tokens.refresh_token.is_empty() {
        store
            .delete(&browseros_id, spec.provider.as_str())
            .await
            .map_err(upstream)?;
        return Err(OAuthError::Expired(format!(
            "{} session expired (no refresh token). Please re-login.",
            spec.provider.as_str()
        )));
    }
    let response = http
        .post(&spec.token_endpoint)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", spec.client_id),
            ("refresh_token", &tokens.refresh_token),
        ])
        .send()
        .await
        .map_err(upstream)?;
    if !response.status().is_success() {
        let status = response.status();
        let rejection = response.json::<Value>().await.ok();
        let invalid_token = rejection
            .as_ref()
            .and_then(|data| data.get("error"))
            .and_then(Value::as_str)
            .is_some_and(|error| matches!(error, "invalid_grant" | "invalid_token"));
        // DECIDED deviation from TS: only an explicit credential revocation logs the user out.
        // Rate limits, outages, malformed error bodies and transport failures keep rotating
        // refresh tokens intact so a subsequent request can retry after the provider recovers.
        if matches!(status.as_u16(), 400 | 401) && invalid_token {
            store
                .delete(&browseros_id, spec.provider.as_str())
                .await
                .map_err(upstream)?;
            return Err(OAuthError::Expired(format!(
                "{} session expired. Please re-login.",
                spec.display_name
            )));
        }
        return Err(OAuthError::Upstream(format!(
            "{} token refresh failed: {status}",
            spec.display_name
        )));
    }
    let data: Value = response.json().await.map_err(upstream)?;
    let refreshed = endpoint_tokens(&data, Some(tokens))?;
    store
        .upsert(&browseros_id, spec.provider.as_str(), &refreshed)
        .await
        .map_err(upstream)?;
    Ok(refreshed)
}

fn device_request(
    http: &reqwest::Client,
    spec: &ProviderSpec,
    endpoint: &str,
    params: &[(&str, String)],
) -> reqwest::RequestBuilder {
    let request = http.post(endpoint).header("Accept", "application/json");
    if spec.provider == OAuthProvider::QwenCode {
        request.form(params)
    } else {
        request.json(
            &params
                .iter()
                .map(|(key, value)| (*key, value))
                .collect::<HashMap<_, _>>(),
        )
    }
}

async fn poll_device(
    http: &reqwest::Client,
    spec: &ProviderSpec,
    device_code: String,
    mut interval: f64,
    expires_in: u64,
    verifier: Option<String>,
) -> Option<OAuthTokens> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(expires_in);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs_f64((interval + 3.0).max(0.001))).await;
        let mut params = vec![
            ("client_id", spec.client_id.to_owned()),
            ("device_code", device_code.clone()),
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:device_code".to_owned(),
            ),
        ];
        if let Some(verifier) = &verifier {
            params.push(("code_verifier", verifier.clone()));
        }
        let result = async {
            let response = device_request(http, spec, &spec.token_endpoint, &params)
                .send()
                .await?;
            if !is_json(&response) {
                return Ok::<_, reqwest::Error>(None);
            }
            response.json::<Value>().await.map(Some)
        }
        .await;
        let data = match result {
            Ok(Some(data)) => data,
            Ok(None) => continue, // WAF HTML is transient during device polling.
            Err(error) => {
                tracing::warn!(%error, "OAuth device polling failed; retrying");
                continue;
            }
        };
        if let Some(access_token) = data
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            let expires = data
                .get("expires_in")
                .and_then(Value::as_f64)
                .unwrap_or_default();
            return Some(OAuthTokens {
                access_token: access_token.to_owned(),
                refresh_token: data
                    .get("refresh_token")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                expires_at: if expires == 0.0 {
                    0
                } else {
                    expires_at(expires)
                },
                email: None,
                account_id: None,
            });
        }
        match data.get("error").and_then(Value::as_str) {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval = data
                    .get("interval")
                    .and_then(Value::as_f64)
                    .unwrap_or(interval)
                    + 5.0;
            }
            _ => return None,
        }
    }
    None
}

/// Only provider HTTPS responses may contribute identity metadata. This parses claims without
/// verifying JWT signatures, matching TS; never call it on extension/caller-supplied tokens.
fn endpoint_tokens(data: &Value, previous: Option<OAuthTokens>) -> RefreshResult {
    let access_token = data
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| upstream("Invalid OAuth token response"))?
        .to_owned();
    let claims = access_token.split('.').collect::<Vec<_>>();
    let claims: Option<Value> = if claims.len() == 3 {
        URL_SAFE_NO_PAD
            .decode(claims[1].trim_end_matches('='))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    } else {
        None
    };
    let claim = |paths: &[&str]| -> Option<String> {
        paths.iter().find_map(|p| {
            claims
                .as_ref()?
                .pointer(p)
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
    };
    let account_id = claim(&[
        "/https:~1~1api.openai.com~1auth/chatgpt_account_id",
        "/chatgpt_account_id",
        "/account_id",
    ])
    .or_else(|| previous.as_ref().and_then(|t| t.account_id.clone()));
    let email = claim(&["/https:~1~1api.openai.com~1profile/email", "/email"])
        .or_else(|| previous.as_ref().and_then(|t| t.email.clone()));
    Ok(OAuthTokens {
        access_token,
        refresh_token: data
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| previous.map(|t| t.refresh_token))
            .unwrap_or_default(),
        expires_at: expires_at(
            data.get("expires_in")
                .and_then(Value::as_f64)
                .unwrap_or_default(),
        ),
        email,
        account_id,
    })
}

fn is_json(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|s| s.contains("application/json"))
}
fn credentials(tokens: OAuthTokens) -> OAuthCredentials {
    OAuthCredentials {
        access_token: tokens.access_token,
        account_id: tokens.account_id,
    }
}
fn random_verifier() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
pub(crate) fn expires_at(seconds: f64) -> i64 {
    now_ms().saturating_add((seconds * 1000.0) as i64)
}
fn upstream(error: impl std::fmt::Display) -> OAuthError {
    OAuthError::Upstream(error.to_string())
}
fn js_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}
