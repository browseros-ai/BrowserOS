//! Composition root: builds every long-lived service once at startup (`AppState::start`) and
//! mounts `http::ROUTES` (`build_router`). Construction order mirrors the process contract:
//! paths → identity → DB → stores → OAuth/Klavis (background) → chat engines → chat service.
//! The `browseros-mcp-host` `HostCtx` field and its construction lines are added by P15.
//! Owned by P3.

use crate::{
    activity::ServerActivity,
    chat::{
        AcpEngine, BrowserOsEngine, ChatDeps, ChatService, ConversationHistory, ConversationRuns,
    },
    config::Config,
    db::{self, Database},
    http,
    identity::{Identity, IdentityError},
    klavis::Klavis,
    mcp::ToolLeases,
    mcp_manager::McpManagerService,
    oauth::TokenManager,
    paths::BrowserosPaths,
};
use axum::{Router, routing::MethodFilter};
use browseros_agent::llm::HttpStack;
use std::{ops::Deref, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// Panel SSE keep-alive (TS parity).
const PANEL_HEARTBEAT: Duration = Duration::from_secs(5);

/// Shared, cheaply cloned handle to every service; the axum state.
#[derive(Clone)]
pub struct AppState(Arc<AppInner>);

pub struct AppInner {
    pub config: Config,
    pub paths: BrowserosPaths,
    pub identity: Identity,
    pub activity: Arc<ServerActivity>,
    pub db: Database,
    pub providers: db::ProviderStore,
    pub agents: db::AgentStore,
    pub conversations: db::ConversationStore,
    pub schedules: db::ScheduleStore,
    pub oauth: Arc<TokenManager>,
    pub klavis: Arc<Klavis>,
    pub mcp_manager: McpManagerService,
    pub leases: Arc<ToolLeases>,
    pub runs: Arc<ConversationRuns>,
    pub chat: Arc<ChatService>,
    /// Concrete (not `dyn ChatEngine`): `/agents` needs `close_all_for_agent`.
    pub acp: Arc<AcpEngine>,
    pub http: HttpStack,
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
    #[error(transparent)]
    Identity(#[from] IdentityError),
    #[error(transparent)]
    Database(#[from] db::DbError),
    #[error("failed to build HTTP client: {0}")]
    HttpClient(#[from] reqwest::Error),
}

impl AppState {
    pub async fn start(config: Config) -> Result<Self, StartupError> {
        let paths = BrowserosPaths::new(config.browseros_dir.clone());
        let identity = Identity::load(&config, &paths).await?;
        let activity = Arc::new(ServerActivity::new());
        let (db, _outcome) = Database::open(&paths).await?;
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
        let acp = Arc::new(AcpEngine::new());
        let browseros = Arc::new(BrowserOsEngine::new(http.clone(), config.env.use_mock_llm));
        let chat = ChatService::new(ChatDeps {
            runs: runs.clone(),
            history: ConversationHistory::new(db.conversations()),
            browseros,
            acp: acp.clone(),
            providers: db.providers(),
            agents: db.agents(),
            oauth: oauth.clone(),
            browseros_id: identity.browseros_id.clone(),
        });

        Ok(Self(Arc::new(AppInner {
            providers: db.providers(),
            agents: db.agents(),
            conversations: db.conversations(),
            schedules: db.schedules(),
            config,
            paths,
            identity,
            activity,
            db,
            oauth,
            klavis,
            mcp_manager,
            leases,
            runs,
            chat,
            acp,
            http,
            shutdown: ShutdownSignal::new(),
        })))
    }
}

/// Mounts exactly `http::ROUTES` (the contract test relies on this being the only registration
/// site) plus the plain-text 404 for unmatched paths and methods. P3 adds the middleware stack.
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
        .with_state(state)
}
