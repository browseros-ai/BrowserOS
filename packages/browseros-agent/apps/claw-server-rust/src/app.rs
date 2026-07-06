use crate::{
    config::Config,
    domain::SessionRegistry,
    error::AppResult,
    routes,
    services::{
        agents::AgentService, audit::AuditService, browser::BrowserService,
        harness::HarnessService, replay::ReplayService, screencast::ScreencastService,
        screenshots::ScreenshotService, tab_activity::TabActivityService,
    },
    storage::JsonStore,
};
use axum::{Router, middleware};
use std::{env, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{Mutex, oneshot};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub audit: Arc<AuditService>,
    pub replay: Arc<ReplayService>,
    pub screenshots: Arc<ScreenshotService>,
    pub tab_activity: Arc<TabActivityService>,
    pub harness: Arc<HarnessService>,
    pub agents: Arc<AgentService>,
    pub sessions: Arc<SessionRegistry>,
    pub browser: Arc<BrowserService>,
    pub screencast: Arc<ScreencastService>,
    pub shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl AppState {
    pub async fn new(
        config: Arc<Config>,
        shutdown_tx: Option<oneshot::Sender<()>>,
    ) -> AppResult<Self> {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| config.browserclaw_dir.clone());
        Self::new_with_home(config, shutdown_tx, home).await
    }

    pub async fn new_with_home(
        config: Arc<Config>,
        shutdown_tx: Option<oneshot::Sender<()>>,
        home_dir: PathBuf,
    ) -> AppResult<Self> {
        tokio::fs::create_dir_all(&config.claw_dir).await?;
        let store = JsonStore::new(config.claw_dir.clone());
        let audit = Arc::new(AuditService::open(config.claw_dir.join("audit.sqlite")).await?);
        let replay = Arc::new(ReplayService::new(
            config.claw_dir.join("replays"),
            50,
            Duration::from_secs(30),
        ));
        let screenshots = Arc::new(ScreenshotService::new(config.claw_dir.join("screenshots")));
        let harness = Arc::new(HarnessService::new(
            config.claw_dir.join("mcp-manager"),
            home_dir,
        ));
        let agents = Arc::new(AgentService::new(store.clone(), config.public_mcp_url()));
        let sessions = SessionRegistry::new(
            audit.clone(),
            replay.clone(),
            config.session_idle,
            config.session_sweep_interval,
        );
        let browser = BrowserService::new(config.cdp_port);
        let tab_activity = Arc::new(TabActivityService::default());
        Ok(Self {
            config,
            audit,
            replay,
            screenshots,
            tab_activity,
            harness,
            agents,
            sessions,
            browser,
            screencast: ScreencastService::new(50),
            shutdown: Arc::new(Mutex::new(shutdown_tx)),
        })
    }
}

pub fn build_router(state: AppState) -> Router {
    routes::router()
        .with_state(state)
        .layer(middleware::from_fn(routes::request_context))
}
