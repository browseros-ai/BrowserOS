//! Klavis/Strata connectors: the upstream client (`https://llm.browseros.com/klavis`, 30 s
//! timeouts), the 45-entry connector catalog, a background Strata connect with backoff, and the
//! connector tools exposed on `/mcp` through the host's `LocalToolProvider` (wired by P15).
//! Implemented by P12.

use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connector {
    pub name: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct OAuthUrls(pub std::collections::BTreeMap<String, String>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Integration {
    pub name: String,
    pub is_authenticated: bool,
}

#[derive(Debug, Clone)]
pub struct AddedServer {
    pub strata_server_url: String,
    pub oauth_url: Option<String>,
    pub api_key_url: Option<String>,
}

/// Feeds the built-in engine's `mcp_server_key` (a session rebuilds when it changes).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ProxyState {
    #[default]
    Disconnected,
    Connecting,
    Connected {
        fingerprint: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum KlavisError {
    #[error("{0}")]
    Upstream(String),
    #[error("{0}")]
    Unavailable(String),
}

const STUB: &str = "Klavis is not implemented yet in server-rust";

pub struct Klavis {
    browseros_id: String,
    http: reqwest::Client,
}

impl Klavis {
    /// Starts the background Strata connect. S0 stub: connects nothing.
    pub fn start(browseros_id: String, http: reqwest::Client) -> Arc<Self> {
        Arc::new(Self { browseros_id, http })
    }

    /// The connector catalog, verbatim from TS. S0 stub: empty.
    pub fn catalog() -> &'static [Connector] {
        &[]
    }

    pub async fn oauth_urls(&self) -> Result<OAuthUrls, KlavisError> {
        let _ = (&self.browseros_id, &self.http);
        Err(KlavisError::Unavailable(STUB.to_owned()))
    }

    pub async fn user_integrations(&self) -> Result<Vec<Integration>, KlavisError> {
        Err(KlavisError::Unavailable(STUB.to_owned()))
    }

    pub async fn add_server(&self, name: &str) -> Result<AddedServer, KlavisError> {
        let _ = name;
        Err(KlavisError::Unavailable(STUB.to_owned()))
    }

    pub async fn submit_api_key(
        &self,
        name: &str,
        api_key: &str,
        api_key_url: &url::Url,
    ) -> Result<(), KlavisError> {
        let _ = (name, api_key, api_key_url);
        Err(KlavisError::Unavailable(STUB.to_owned()))
    }

    pub async fn remove_server(&self, name: &str) -> Result<(), KlavisError> {
        let _ = name;
        Err(KlavisError::Unavailable(STUB.to_owned()))
    }

    pub fn proxy_state(&self) -> ProxyState {
        ProxyState::Disconnected
    }

    pub async fn stop(&self) {}
}
