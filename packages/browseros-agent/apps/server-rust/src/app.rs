//! Composition root: builds every long-lived service once at startup (`AppState::start`) and
//! mounts `http::ROUTES` (`build_router`). Construction order mirrors the process contract:
//! paths → DB → identity → metrics → stores → OAuth/Klavis (background) → `/mcp` host (CDP
//! reattach, non-fatal) → chat engines → chat service. Telemetry is process-wide: one PostHog
//! sink and one Sentry guard, created by `main` and shared with every service that reports.

use crate::{
    activity::ServerActivity,
    chat::DbChatRecords,
    chat::{AcpEngine, BrowserOsEngine, ChatDeps, ChatService, ConversationRuns},
    chat::{BrowserOsEngineDeps, RegistryLeases, klavis_state_name, production_models},
    config::Config,
    db::{self, Database},
    http,
    identity::{Identity, IdentityError},
    klavis::Klavis,
    mcp::{HostPages, HostTabs, McpConfig, McpServer, ToolLeases, connector_scope},
    mcp_manager::McpManagerService,
    oauth::TokenManager,
    paths::BrowserosPaths,
    telemetry::{
        metrics::{Metrics, MetricsConfig},
        sentry::SentryGuard,
    },
};
use axum::{Router, middleware, routing::MethodFilter};
use browseros_agent::llm::HttpStack;
use browseros_core::NativeCapabilities;
use browseros_mcp_host::HostError;
use std::{ops::Deref, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// Panel SSE keep-alive (TS parity).
const PANEL_HEARTBEAT: Duration = Duration::from_secs(5);

/// Shared, cheaply cloned handle to every service; the axum state.
#[derive(Clone)]
pub struct AppState(Arc<AppInner>);

/// One process lifetime owns these handles. HTTP requests only clone AppState;
/// shutdown belongs to the process owner rather than whichever request ends first.
pub struct AppInner {
    pub config: Config,
    pub origins: http::security::OriginPolicy,
    pub paths: BrowserosPaths,
    pub identity: Identity,
    pub activity: Arc<ServerActivity>,
    pub providers: db::ProviderStore,
    pub agents: db::AgentStore,
    pub conversations: db::ConversationStore,
    pub schedules: db::ScheduleStore,
    pub oauth: Arc<TokenManager>,
    pub klavis: Arc<Klavis>,
    pub mcp_manager: McpManagerService,
    pub leases: Arc<ToolLeases>,
    /// `/mcp`: the shared MCP host, which is also the process's browser (CDP) connection.
    pub mcp: Arc<McpServer>,
    pub chat: Arc<ChatService>,
    /// Concrete (not `dyn ChatEngine`): `/agents` needs `close_all_for_agent`.
    pub acp: Arc<AcpEngine>,
    pub http: HttpStack,
    /// PostHog sink (disabled without an install id or a build-time key); flushed by `stop`.
    pub metrics: Metrics,
    /// The process root's one Sentry client. Shared, never re-initialized here, so startup and
    /// request-error captures (`http::security::ingress`) go through the same redacting client
    /// the panic hook uses.
    pub sentry: Arc<SentryGuard>,
    pub shutdown: ShutdownSignal,
}

impl Deref for AppState {
    type Target = AppInner;

    fn deref(&self) -> &AppInner {
        &self.0
    }
}

/// Fired by `POST /system/shutdown`; `main` then stops serving and exits 0. Signals take a
/// different path (exit 3), so the two exits stay distinguishable to Chromium.
#[derive(Debug, Clone, Default)]
pub struct ShutdownSignal(CancellationToken);

impl ShutdownSignal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn trigger(&self) {
        self.0.cancel();
    }

    pub async fn triggered(&self) {
        self.0.cancelled().await;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error("failed to prepare server directories: {0}")]
    Paths(#[from] std::io::Error),
    #[error(transparent)]
    Identity(#[from] IdentityError),
    #[error(transparent)]
    Database(#[from] db::DbError),
    #[error("failed to build HTTP client: {0}")]
    HttpClient(#[from] reqwest::Error),
    #[error("failed to open the MCP host: {0}")]
    Mcp(#[from] HostError),
}

impl AppState {
    /// Composes the app without Sentry (tests and embedders). The process root uses
    /// [`AppState::start_with_sentry`] so its guard outlives config parsing and runtime shutdown.
    pub async fn start(config: Config) -> Result<Self, StartupError> {
        Self::start_with_sentry(config, Arc::new(SentryGuard::default())).await
    }

    pub async fn start_with_sentry(
        config: Config,
        sentry: Arc<SentryGuard>,
    ) -> Result<Self, StartupError> {
        let paths = BrowserosPaths::new(config.browseros_dir.clone());
        paths.prepare().await?;
        let activity = Arc::new(ServerActivity::new());
        let (db, _outcome) = Database::open(&paths).await?;
        let identity = Identity::load(&config, &paths).await?;
        // After identity: the sink needs the install id, and without one it stays disabled
        // (TS skips events when installation identity is unavailable).
        let metrics = Metrics::initialize(MetricsConfig {
            install_id: identity.install_id.clone(),
            browseros_version: config.instance.browseros_version.clone(),
            chromium_version: config.instance.chromium_version.clone(),
            server_version: Some(crate::VERSION.to_owned()),
            ..Default::default()
        })
        .await;
        let client = reqwest::Client::builder().build()?;
        let http = HttpStack::new();

        let oauth = TokenManager::new(
            db.oauth_tokens(),
            identity.browseros_id.clone(),
            client.clone(),
        );
        let klavis = Klavis::start(identity.browseros_id.clone(), client);
        let mcp_manager = McpManagerService::new(
            paths.mcp_manager_dir.clone(),
            config.env.mcp_public_url.clone(),
        );

        let runs = ConversationRuns::new(activity.clone(), PANEL_HEARTBEAT);
        let leases = ToolLeases::new(runs.clone());
        let connectors = klavis
            .tool_provider_with_scope(connector_scope(leases.runs().clone()), metrics.clone());
        let mcp = Arc::new(
            McpServer::open(
                McpConfig {
                    cdp_port: config.cdp_port,
                    data_root: paths.mcp_host_dir.clone(),
                    native: NativeCapabilities::from_browseros_version(
                        config.instance.browseros_version.as_deref(),
                    ),
                },
                leases.clone(),
                Some(connectors),
                metrics.clone(),
            )
            .await?,
        );
        mcp.start().await;
        let host = mcp.host().clone();
        let acp = Arc::new(AcpEngine::new(&config, &paths, leases.clone()));
        let klavis_for_key = klavis.clone();
        let browseros = Arc::new(BrowserOsEngine::new(BrowserOsEngineDeps {
            models: production_models(http.clone(), config.env.use_mock_llm),
            leases: Arc::new(RegistryLeases(leases.clone())),
            pages: Arc::new(HostPages(host.clone())),
            mcp_port: config.server_port,
            klavis_status: Arc::new(move || {
                Some(klavis_state_name(&klavis_for_key.proxy_state()).to_owned())
            }),
            connector_names: Klavis::catalog()
                .iter()
                .map(|connector| connector.name.to_owned())
                .collect(),
        }));
        let chat = ChatService::new(ChatDeps {
            runs: runs.clone(),
            records: Arc::new(DbChatRecords::new(&db)),
            browseros,
            acp: acp.clone(),
            oauth: oauth.clone(),
            tabs: Some(Arc::new(HostTabs(host.clone()))),
            use_mock_llm: config.env.use_mock_llm,
            metrics: metrics.clone(),
        });

        Ok(Self(Arc::new(AppInner {
            origins: http::security::OriginPolicy::new(&config.env),
            providers: db.providers(),
            agents: db.agents(),
            conversations: db.conversations(),
            schedules: db.schedules(),
            config,
            paths,
            identity,
            activity,
            oauth,
            klavis,
            mcp_manager,
            leases,
            mcp,
            chat,
            acp,
            http,
            metrics,
            sentry,
            shutdown: ShutdownSignal::new(),
        })))
    }

    /// The live CDP link, not the last one-second poll: health stays 200 while the
    /// host reattaches, and `cdpConnected` tells Chromium the truth meanwhile.
    pub async fn cdp_connected(&self) -> bool {
        self.mcp.cdp_connected().await
    }

    /// Bounded by the process owner's deadline; dropping HTTP clients never stops
    /// detached work, so service cleanup belongs to process shutdown instead.
    pub async fn stop(&self) {
        tokio::join!(
            self.oauth.shutdown(),
            self.klavis.stop(),
            self.acp.shutdown(),
            // Bounded by its own 2 s SDK timeout; queued events would otherwise be lost.
            self.metrics.shutdown(),
            self.mcp.stop()
        );
    }
}

/// Mounts exactly `http::ROUTES` (the contract test relies on this being the only registration
/// site) plus the plain-text 404 for unmatched paths and methods, all behind the one ingress
/// middleware (`http::security::ingress`).
pub fn build_router(state: AppState) -> Router {
    let mut router = Router::new();
    for route in http::ROUTES {
        match MethodFilter::try_from(route.method.clone()) {
            Ok(filter) => router = router.route(route.path, (route.mount)(filter)),
            Err(error) => tracing::error!(path = route.path, %error, "route method has no filter"),
        }
    }
    router
        .fallback(http::not_found)
        .method_not_allowed_fallback(http::not_found)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            http::security::ingress,
        ))
        .with_state(state)
}
