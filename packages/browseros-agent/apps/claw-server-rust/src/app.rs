use crate::{
    agents::AgentService,
    browser::BrowserService,
    capture::{
        audit::AuditService, recordings::RecordingStore,
        replays::ReplayService as ReplayReadService, screencast::ScreencastService,
        screenshots::ScreenshotService,
    },
    config::Config,
    error::AppResult,
    harness::HarnessService,
    routes,
    runtime::ShutdownHandle,
    sessions::Sessions,
    storage::JsonStore,
    tabs::{
        activity::{TabActivityRecord, TabActivityService},
        targets::TabTargetMap,
    },
    telemetry::TelemetryService,
};
use axum::{Router, middleware};
use std::{env, path::PathBuf, sync::Arc, time::Duration};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub audit: Arc<AuditService>,
    pub recordings: Arc<RecordingStore>,
    pub replays: Arc<ReplayReadService>,
    pub screenshots: Arc<ScreenshotService>,
    pub tab_activity: Arc<TabActivityService>,
    pub tab_targets: Arc<TabTargetMap>,
    pub harness: Arc<HarnessService>,
    pub telemetry: Arc<TelemetryService>,
    pub agents: Arc<AgentService>,
    pub sessions: Arc<Sessions>,
    pub browser: Arc<BrowserService>,
    pub screencast: Arc<ScreencastService>,
    pub shutdown: ShutdownHandle,
}

impl AppState {
    pub async fn new(config: Arc<Config>) -> AppResult<Self> {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| config.browserclaw_dir.clone());
        Self::new_with_home(config, home).await
    }

    pub async fn new_with_home(config: Arc<Config>, home_dir: PathBuf) -> AppResult<Self> {
        tokio::fs::create_dir_all(&config.browserclaw_dir).await?;
        let store = JsonStore::new(config.browserclaw_dir.clone());
        let audit =
            Arc::new(AuditService::open(config.browserclaw_dir.join("audit.sqlite")).await?);
        audit.release_all_open_claims().await?;
        let recordings = RecordingStore::new(
            config.browserclaw_dir.join("recordings"),
            audit.clone(),
            50,
            Duration::from_secs(30),
        );
        let replays = ReplayReadService::new(recordings.clone(), audit.clone());
        let screenshots = Arc::new(ScreenshotService::new(
            config.browserclaw_dir.join("screenshots"),
        ));
        let harness = Arc::new(HarnessService::new(
            config.browserclaw_dir.join("mcp-manager"),
            home_dir,
        ));
        let telemetry = Arc::new(TelemetryService::new(&config.browserclaw_dir));
        let agents = Arc::new(AgentService::new(store.clone()));
        let sessions = Sessions::new(
            audit.clone(),
            config.session_idle,
            config.session_retention,
            config.session_sweep_interval,
        );
        let tab_targets = TabTargetMap::new(audit.clone());
        let browser =
            BrowserService::new(config.cdp_port, sessions.ownership(), tab_targets.clone());
        let tab_activity = Arc::new(TabActivityService::default());
        Ok(Self {
            config,
            audit,
            recordings,
            replays,
            screenshots,
            tab_activity,
            tab_targets,
            harness,
            telemetry,
            agents,
            sessions,
            browser,
            screencast: ScreencastService::new(50),
            shutdown: ShutdownHandle::new(),
        })
    }

    pub async fn live_tab_activity(&self) -> Vec<TabActivityRecord> {
        let session = self.browser.session().await;
        self.tab_activity.snapshot(session.as_deref()).await
    }
}

pub fn build_router(state: AppState) -> Router {
    routes::router(state.clone())
        .with_state(state)
        .layer(middleware::from_fn(routes::request_context))
}
