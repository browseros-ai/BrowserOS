//! Klavis catalog/auth and the process-owned Strata connection. Metadata is cached
//! for an hour; live MCP sessions belong only to the background task and are closed
//! on cancellation. The shared host consumes tools through `LocalToolProvider`.

mod catalog;
mod client;
mod session;
mod tools;

use browseros_mcp_host::LocalToolProvider;
use client::{Client, Strata, auth_url};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
use tokio::{sync::Mutex as AsyncMutex, task::JoinHandle, time::Instant};
use tokio_util::sync::CancellationToken;
pub use tools::{ConnectorScope, ScopeResolver};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Connector {
    pub name: &'static str,
    pub description: &'static str,
}
#[derive(Debug, Clone, Default)]
pub struct OAuthUrls(pub BTreeMap<String, String>);
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Integration {
    pub name: String,
    pub is_authenticated: bool,
}
#[derive(Debug, Clone)]
pub struct AddedServer {
    pub strata_server_url: String,
    pub strata_id: String,
    pub added_servers: Vec<String>,
    pub oauth_url: Option<String>,
    pub api_key_url: Option<String>,
}
/// Feeds the built-in engine's MCP cache key when proxy availability changes.
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

/// Cached publication is synchronous for tools/list. Neither the MCP service nor
/// a request owns the background connection; dropping the process owner cancels it.
pub struct Klavis {
    client: Client,
    shared: Arc<Shared>,
    cancel: CancellationToken,
    task: Mutex<Option<JoinHandle<()>>>,
}

struct Shared {
    snapshot: RwLock<Snapshot>,
    cache: AsyncMutex<Option<(Strata, Instant)>>,
}
struct Snapshot {
    status: Value,
    proxy: ProxyState,
    session: Option<session::ProxySession>,
}

impl Klavis {
    pub fn start(browseros_id: String, http: reqwest::Client) -> Arc<Self> {
        Self::start_with_upstream(
            browseros_id,
            http,
            client::BASE_URL.into(),
            vec![5, 10, 20, 40, 60]
                .into_iter()
                .map(Duration::from_secs)
                .collect(),
        )
    }

    /// Explicit upstream/retry injection keeps acceptance entirely on loopback.
    pub fn start_with_upstream(
        browseros_id: String,
        http: reqwest::Client,
        base_url: String,
        retry_delays: Vec<Duration>,
    ) -> Arc<Self> {
        let enabled = !browseros_id.is_empty();
        let shared = Arc::new(Shared {
            snapshot: RwLock::new(Snapshot {
                status: if enabled {
                    json!({"state":"connecting"})
                } else {
                    json!({"state":"disabled","reason":"missing_browseros_id"})
                },
                proxy: if enabled {
                    ProxyState::Connecting
                } else {
                    ProxyState::Disconnected
                },
                session: None,
            }),
            cache: AsyncMutex::new(None),
        });
        let client = Client {
            id: browseros_id,
            http,
            base_url,
        };
        let cancel = CancellationToken::new();
        // The worker owns only shared state, not Arc<Klavis>; otherwise its owner
        // could never drop and cancel an abandoned startup/retry task.
        let task = enabled.then(|| {
            tokio::spawn(session::run(
                client.clone(),
                shared.clone(),
                cancel.clone(),
                retry_delays,
            ))
        });
        Arc::new(Self {
            client,
            shared,
            cancel,
            task: Mutex::new(task),
        })
    }

    pub fn catalog() -> &'static [Connector] {
        catalog::CATALOG
    }

    fn validate_name(&self, name: &str) -> Result<(), KlavisError> {
        self.client.require_id()?;
        if Self::catalog().iter().any(|s| s.name == name) {
            Ok(())
        } else {
            Err(KlavisError::Upstream(format!("Invalid server: {name}")))
        }
    }

    pub async fn oauth_urls(&self) -> Result<OAuthUrls, KlavisError> {
        let names: Vec<_> = Self::catalog().iter().map(|s| s.name).collect();
        let result = self.client.create(&names).await?;
        Ok(OAuthUrls(
            names
                .into_iter()
                .filter_map(|name| {
                    auth_url(&result.oauth_urls, name)
                        .filter(|url| !url.is_empty())
                        .map(|url| (name.into(), url))
                })
                .collect(),
        ))
    }
    pub async fn user_integrations(&self) -> Result<Vec<Integration>, KlavisError> {
        self.client.integrations().await
    }

    pub async fn add_server(&self, name: &str) -> Result<AddedServer, KlavisError> {
        self.validate_name(name)?;
        let result = self.client.create(&[name]).await?;
        *self.shared.cache.lock().await = None;
        Ok(AddedServer {
            oauth_url: auth_url(&result.oauth_urls, name),
            api_key_url: auth_url(&result.api_key_urls, name),
            strata_server_url: result.strata_server_url,
            strata_id: result.strata_id,
            added_servers: result.added_servers,
        })
    }
    pub async fn submit_api_key(
        &self,
        name: &str,
        api_key: &str,
        api_key_url: &url::Url,
    ) -> Result<(), KlavisError> {
        self.validate_name(name)?;
        self.client.submit_key(api_key_url, api_key).await?;
        *self.shared.cache.lock().await = None;
        Ok(())
    }
    pub async fn remove_server(&self, name: &str) -> Result<(), KlavisError> {
        self.validate_name(name)?;
        // TS creates the connector's Strata before deleting it, even if a cached
        // all-connectors instance exists. The upstream side effect is observable.
        let strata = self.client.create(&[name]).await?;
        self.client.remove(&strata.strata_id, name).await?;
        *self.shared.cache.lock().await = None;
        Ok(())
    }
    pub fn proxy_state(&self) -> ProxyState {
        self.shared
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .proxy
            .clone()
    }
    pub fn proxy_status(&self) -> Value {
        self.shared
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .status
            .clone()
    }

    pub fn tool_provider(self: &Arc<Self>) -> Arc<dyn LocalToolProvider> {
        self.tool_provider_with_scope(Arc::new(|caller| ConnectorScope {
            // Bound callers require P15's live-run resolver. Fail closed if the
            // host forgot to wire it; anonymous external MCP clients need no run.
            rejection: caller
                .filter(|c| c.session.is_some())
                .map(|_| "MCP tools require an active conversation run.".into()),
            ..Default::default()
        }))
    }
    /// P15 maps its opaque lease to selected connectors and a live-run refusal.
    /// The host never needs to expose its concrete lease type to this module.
    pub fn tool_provider_with_scope(
        self: &Arc<Self>,
        scope: ScopeResolver,
    ) -> Arc<dyn LocalToolProvider> {
        Arc::new(tools::Provider {
            klavis: self.clone(),
            scope,
        })
    }
    pub async fn stop(&self) {
        self.cancel.cancel();
        let task = self.task.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(task) = task {
            let _ = task.await;
        }
        let mut snapshot = self
            .shared
            .snapshot
            .write()
            .unwrap_or_else(|e| e.into_inner());
        snapshot.session = None;
        snapshot.proxy = ProxyState::Disconnected;
        if !self.client.id.is_empty() {
            snapshot.status = json!({"state":"stopped"});
        }
    }
}
impl Drop for Klavis {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
