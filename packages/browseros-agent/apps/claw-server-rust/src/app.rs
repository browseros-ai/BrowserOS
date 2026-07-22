use crate::{
    analytics::{AnalyticsService, AnalyticsSink},
    api::http,
    config::Config,
    db::{AuditLog, DATABASE_FILENAME, Database, RecordingIndex, SessionTabLedger},
    error::AppResult,
    runtime::ShutdownHandle,
    services::{
        browser::{BrowserService, TabRegistry},
        cockpit::{CockpitQuery, SessionVisualService, TabActivityRecord, TabActivityService},
        harness::HarnessService,
        profiles::ProfileService,
        recordings::{RecordingIngestService, RecordingStore},
        replay::ReplayService,
        screenshots::ScreenshotService,
        sessions::Sessions,
    },
    storage::JsonStore,
};
use axum::{Router, middleware};
use std::{env, path::PathBuf, sync::Arc, time::Duration};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub audit_log: Arc<AuditLog>,
    pub session_tabs: Arc<SessionTabLedger>,
    pub recordings: Arc<RecordingStore>,
    pub recording_ingest: Arc<RecordingIngestService>,
    pub replay: Arc<ReplayService>,
    pub screenshots: Arc<ScreenshotService>,
    pub tab_activity: Arc<TabActivityService>,
    pub tab_registry: Arc<TabRegistry>,
    pub harness: Arc<HarnessService>,
    pub analytics: Arc<AnalyticsService>,
    pub profiles: Arc<ProfileService>,
    pub sessions: Arc<Sessions>,
    pub browser: Arc<BrowserService>,
    pub visuals: Arc<SessionVisualService>,
    pub cockpit: Arc<CockpitQuery>,
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
        let database = Database::open(config.browserclaw_dir.join(DATABASE_FILENAME)).await?;
        let audit_log = Arc::new(AuditLog::new(database.clone()));
        let session_tabs = Arc::new(SessionTabLedger::new(database.clone()));
        let recording_index = Arc::new(RecordingIndex::new(database));
        session_tabs.release_all_open().await?;
        let recordings = RecordingStore::new(
            config.browserclaw_dir.join("recordings"),
            recording_index.clone(),
            50,
            Duration::from_secs(30),
        );
        let replay = ReplayService::new(recordings.clone(), recording_index);
        let screenshots = Arc::new(ScreenshotService::new(
            config.browserclaw_dir.join("screenshots"),
            audit_log.clone(),
        ));
        let analytics = Arc::new(AnalyticsService::new(&config.browserclaw_dir).await?);
        let analytics_sink: Arc<dyn AnalyticsSink> = analytics.clone();
        let harness = Arc::new(HarnessService::new_with_analytics(
            config.browserclaw_dir.join("mcp-manager"),
            home_dir,
            analytics_sink.clone(),
        ));
        let profiles = Arc::new(ProfileService::new(store.clone()));
        let sessions = Sessions::new_with_analytics(
            audit_log.clone(),
            session_tabs.clone(),
            config.session_idle,
            config.session_retention,
            config.session_sweep_interval,
            analytics_sink,
        );
        let tab_registry = TabRegistry::new(session_tabs.clone());
        let browser =
            BrowserService::new(config.cdp_port, sessions.ownership(), tab_registry.clone());
        let recording_ingest =
            RecordingIngestService::new(recordings.clone(), browser.clone(), tab_registry.clone());
        let tab_activity = Arc::new(TabActivityService::default());
        let visuals = SessionVisualService::new(
            sessions.clone(),
            session_tabs.clone(),
            browser.clone(),
            tab_activity.clone(),
        );
        let cockpit = Arc::new(CockpitQuery::new(
            sessions.clone(),
            profiles.clone(),
            audit_log.clone(),
            session_tabs.clone(),
            browser.clone(),
            tab_activity.clone(),
        ));
        Ok(Self {
            config,
            audit_log,
            session_tabs,
            recordings,
            recording_ingest,
            replay,
            screenshots,
            tab_activity,
            tab_registry,
            harness,
            analytics,
            profiles,
            sessions,
            browser,
            visuals,
            cockpit,
            shutdown: ShutdownHandle::new(),
        })
    }

    pub async fn live_tab_activity(&self) -> Vec<TabActivityRecord> {
        let session = self.browser.session().await;
        self.tab_activity.snapshot(session.as_deref()).await
    }
}

pub fn build_router(state: AppState) -> Router {
    http::router(state.clone())
        .with_state(state)
        .layer(middleware::from_fn(http::request_context))
}

#[cfg(test)]
mod tests {
    use super::AppState;
    use crate::{config::Config, db::DATABASE_FILENAME};
    use std::{sync::Arc, time::Duration};
    use tempfile::tempdir;

    #[tokio::test]
    async fn new_with_home_uses_browserclaw_database_without_touching_old_file()
    -> anyhow::Result<()> {
        let dir = tempdir()?;
        let browserclaw_dir = dir.path().join("browserclaw");
        tokio::fs::create_dir_all(&browserclaw_dir).await?;
        let old_database = browserclaw_dir.join("audit.sqlite");
        let old_contents = b"old database stays untouched";
        tokio::fs::write(&old_database, old_contents).await?;
        let config = Arc::new(Config {
            server_port: 9200,
            cdp_port: 49337,
            proxy_port: None,
            resources_dir: dir.path().join("resources"),
            browserclaw_dir: browserclaw_dir.clone(),
            session_idle: Duration::from_secs(300),
            session_retention: Duration::from_secs(7_200),
            session_sweep_interval: Duration::from_secs(60),
            replay_retention_days: 7,
            dev_mode: false,
            auth_token: None,
        });

        let _state = AppState::new_with_home(config, dir.path().join("home")).await?;

        assert_eq!(DATABASE_FILENAME, "browserclaw.sqlite");
        assert!(browserclaw_dir.join(DATABASE_FILENAME).is_file());
        assert_eq!(tokio::fs::read(old_database).await?, old_contents);
        Ok(())
    }
}
