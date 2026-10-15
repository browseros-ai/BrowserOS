//! Token lifecycle: start a flow (device code or PKCE redirect), store tokens, report status,
//! refresh at `expiresAt - 300 s` with one in-flight refresh per provider, delete on refresh
//! failure (`"<name> session expired. Please re-login."`).

use super::{CallbackServer, OAuthProvider};
use crate::db::OAuthTokenStore;
use std::sync::Arc;

pub enum StartFlow {
    Device {
        user_code: String,
        verification_uri: String,
        expires_in: u64,
    },
    /// `GET /oauth/{provider}/start` answers 302 to this URL.
    Redirect(url::Url),
}

/// `POST /oauth/{provider}/token` body.
#[derive(Debug, Clone)]
pub struct TokenInput {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
}

/// `{authenticated, email?, provider}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthStatus {
    pub authenticated: bool,
    pub email: Option<String>,
    pub provider: String,
}

#[derive(Debug, Clone)]
pub struct OAuthCredentials {
    pub access_token: String,
    pub account_id: Option<String>,
}

/// Texts are surfaced verbatim (e.g. `Not authenticated with ChatGPT. Please login first.`).
#[derive(Debug, thiserror::Error)]
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

pub struct TokenManager {
    store: OAuthTokenStore,
    browseros_id: String,
    http: reqwest::Client,
    callback: CallbackServer,
}

const STUB: &str = "OAuth is not implemented yet in server-rust";

impl TokenManager {
    pub fn new(store: OAuthTokenStore, browseros_id: String, http: reqwest::Client) -> Arc<Self> {
        Arc::new(Self {
            store,
            browseros_id,
            http,
            callback: CallbackServer::new(),
        })
    }

    pub async fn start(self: &Arc<Self>, provider: OAuthProvider) -> Result<StartFlow, OAuthError> {
        let _ = (provider, &self.http);
        Err(OAuthError::Unavailable(STUB.to_owned()))
    }

    pub async fn store_token(
        &self,
        provider: OAuthProvider,
        input: TokenInput,
    ) -> Result<(), OAuthError> {
        let _ = (provider, input, &self.store);
        Err(OAuthError::Unavailable(STUB.to_owned()))
    }

    /// Accepts any provider string (unknown ones are simply unauthenticated).
    pub async fn status(&self, provider: &str) -> OAuthStatus {
        OAuthStatus {
            authenticated: false,
            email: None,
            provider: provider.to_owned(),
        }
    }

    pub async fn logout(&self, provider: &str) {
        let _ = (provider, &self.browseros_id);
    }

    /// chatgpt-pro / qwen-code: refresh-if-expired (single-flight); github-copilot: stored token.
    pub async fn credentials(
        &self,
        provider: OAuthProvider,
    ) -> Result<OAuthCredentials, OAuthError> {
        let _ = provider;
        Err(OAuthError::Unavailable(STUB.to_owned()))
    }

    /// Stops the `:1455` listener.
    pub async fn shutdown(&self) {
        self.callback.stop().await;
    }
}
