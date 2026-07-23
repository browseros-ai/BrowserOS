use crate::{
    AppState,
    error::{AppError, AppResult},
};
use std::{future::Future, time::Duration};
use tokio::{task::JoinHandle, time::timeout};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

const TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const SESSION_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Default)]
pub struct ShutdownHandle {
    token: CancellationToken,
}

impl ShutdownHandle {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(&self) {
        self.token.cancel();
    }

    pub async fn requested(&self) {
        self.token.cancelled().await;
    }

    fn child_token(&self) -> CancellationToken {
        self.token.child_token()
    }
}

struct BackgroundTask {
    name: &'static str,
    handle: JoinHandle<()>,
}

/** Owns the application's long-running tasks and its one ordered teardown sequence. */
pub struct AppRuntime {
    state: AppState,
    tasks: Vec<BackgroundTask>,
}

impl AppRuntime {
    #[must_use]
    pub fn start(state: AppState) -> Self {
        let shutdown = state.shutdown.clone();
        let tasks = vec![
            BackgroundTask {
                name: "browser reconnect loop",
                handle: state.browser.start(),
            },
            BackgroundTask {
                name: "session idle sweeper",
                handle: state
                    .sessions
                    .clone()
                    .spawn_idle_sweeper(shutdown.child_token()),
            },
            BackgroundTask {
                name: "recording retention sweeper",
                handle: state
                    .recordings
                    .clone()
                    .spawn_retention(state.config.replay_retention_days, shutdown.child_token()),
            },
            BackgroundTask {
                name: "session efficiency reconciliation",
                handle: tokio::spawn({
                    let session_efficiency = state.session_efficiency.clone();
                    let cancel = shutdown.child_token();
                    async move {
                        match session_efficiency.reconcile(cancel).await {
                            Ok(finalized) if finalized > 0 => {
                                info!(finalized, "reconciled session efficiency projections");
                            }
                            Ok(_) => {}
                            Err(error) => {
                                warn!(error = %error, "session efficiency reconciliation failed");
                            }
                        }
                    }
                }),
            },
        ];
        Self { state, tasks }
    }

    #[must_use]
    pub fn state(&self) -> AppState {
        self.state.clone()
    }

    pub fn spawn_task(
        &mut self,
        name: &'static str,
        task: impl Future<Output = ()> + Send + 'static,
    ) {
        self.tasks.push(BackgroundTask {
            name,
            handle: tokio::spawn(task),
        });
    }

    pub async fn shutdown(self) -> AppResult<()> {
        self.shutdown_with_session_timeout(SESSION_SHUTDOWN_TIMEOUT)
            .await
    }

    async fn shutdown_with_session_timeout(mut self, session_timeout: Duration) -> AppResult<()> {
        self.state.shutdown.request();
        let session_result = match timeout(session_timeout, self.state.sessions.shutdown()).await {
            Ok(result) => result,
            Err(_) => {
                warn!(
                    timeout_ms = session_timeout.as_millis(),
                    "session teardown exceeded shutdown timeout"
                );
                Err(AppError::Internal(format!(
                    "session teardown exceeded {} ms shutdown timeout",
                    session_timeout.as_millis()
                )))
            }
        };
        // Session teardown enqueues final ownership releases; wait for the FIFO barrier before
        // shutdown returns.
        self.state.session_tabs.drain_writes().await;
        self.state.recordings.close().await;
        self.state.browser.stop();
        self.join_tasks().await;
        self.state.session_efficiency.drain().await;
        self.state.analytics.shutdown().await;
        let drained = session_result?;
        info!(drained, "drained sessions during shutdown");
        Ok(())
    }

    async fn join_tasks(&mut self) {
        for mut task in self.tasks.drain(..) {
            match timeout(TASK_SHUTDOWN_TIMEOUT, &mut task.handle).await {
                Ok(Ok(())) => {}
                Ok(Err(join_error)) => {
                    error!(task = task.name, error = %join_error, "background task failed");
                }
                Err(_) => {
                    warn!(
                        task = task.name,
                        timeout_ms = TASK_SHUTDOWN_TIMEOUT.as_millis(),
                        "background task exceeded shutdown timeout"
                    );
                    task.handle.abort();
                    let _ = task.handle.await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppRuntime, ShutdownHandle};
    use crate::{
        AppState,
        analytics::{AnalyticsService, events},
        config::Config,
        db::{
            DATABASE_FILENAME, Database,
            audit_log::{DispatchResultSummary, RecordToolDispatchInput},
        },
        identity::{ClientIdentity, ConversationIdentity},
        ids::{DispatchId, SessionId},
        services::{
            session_efficiency::SessionEfficiencyService,
            sessions::{Session, Sessions},
        },
    };
    use axum::{Router, body::Bytes, routing::any};
    use serde_json::{Value, json};
    use std::{future::pending, sync::Arc, time::Duration};
    use tempfile::tempdir;
    use tokio::{
        net::TcpListener,
        sync::{Notify, mpsc},
        time::Instant,
    };

    fn efficiency_dispatch(session_id: &str, agent_id: &str) -> RecordToolDispatchInput {
        RecordToolDispatchInput {
            agent_id: agent_id.to_string(),
            slug: "agent".to_string(),
            agent_label: "Agent".to_string(),
            session_id: session_id.to_string(),
            tool_name: "navigate".to_string(),
            page_id: None,
            tab_id: None,
            target_id: None,
            url: None,
            title: None,
            raw_args: json!({}),
            duration_ms: 5,
            dispatch_id: DispatchId::new(),
            tool_input_token_estimate: 1,
            tool_output_token_estimate: 0,
            token_estimator_version: 1,
            result: DispatchResultSummary {
                is_error: false,
                cancelled: false,
                structured_content: json!({}),
                content: json!([]),
            },
        }
    }

    #[tokio::test]
    async fn repeated_requests_wake_every_shutdown_waiter() -> anyhow::Result<()> {
        let shutdown = ShutdownHandle::new();
        let first = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.requested().await }
        });
        let second = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.requested().await }
        });

        shutdown.request();
        shutdown.request();

        tokio::time::timeout(Duration::from_secs(1), first).await??;
        tokio::time::timeout(Duration::from_secs(1), second).await??;
        shutdown.requested().await;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_reconciles_eligible_ended_sessions_on_startup() -> anyhow::Result<()> {
        let root = tempdir()?;
        let config = Arc::new(Config {
            server_port: 9200,
            cdp_port: 49337,
            proxy_port: None,
            resources_dir: root.path().join("resources"),
            browserclaw_dir: root.path().to_path_buf(),
            session_idle: Duration::from_secs(300),
            session_retention: Duration::from_secs(7_200),
            session_sweep_interval: Duration::from_secs(60),
            replay_retention_days: 7,
            dev_mode: false,
            auth_token: None,
        });
        let state = AppState::new_with_home(config, root.path().join("home")).await?;
        state
            .audit_log
            .record_session_start(
                "reconcile-on-start",
                "agent",
                "agent",
                "Agent",
                "Codex",
                "1",
            )
            .await?;
        state
            .audit_log
            .record_tool_dispatch(efficiency_dispatch("reconcile-on-start", "agent"))
            .await?;
        state
            .audit_log
            .record_session_end("reconcile-on-start", "closed", None)
            .await?;
        let session_efficiency = state.session_efficiency.clone();

        let runtime = AppRuntime::start(state);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if session_efficiency.aggregate().await?.is_some() {
                    return Ok::<(), crate::error::AppError>(());
                }
                tokio::task::yield_now().await;
            }
        })
        .await??;
        runtime.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_drains_session_events_before_shutting_down_analytics() -> anyhow::Result<()> {
        let root = tempdir()?;
        let config = Arc::new(Config {
            server_port: 9200,
            cdp_port: 49337,
            proxy_port: None,
            resources_dir: root.path().join("resources"),
            browserclaw_dir: root.path().to_path_buf(),
            session_idle: Duration::from_secs(300),
            session_retention: Duration::from_secs(7_200),
            session_sweep_interval: Duration::from_secs(60),
            replay_retention_days: 7,
            dev_mode: false,
            auth_token: None,
        });
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let host = format!("http://{}", listener.local_addr()?);
        let (sender, mut requests) = mpsc::unbounded_channel();
        let endpoint = tokio::spawn(async move {
            let app = Router::new().fallback(any(move |body: Bytes| {
                let sender = sender.clone();
                async move {
                    if let Ok(value) = serde_json::from_slice::<Value>(&body) {
                        let _ = sender.send(value);
                    }
                    "ok"
                }
            }));
            let _ = axum::serve(listener, app).await;
        });

        let mut state = AppState::new_with_home(config.clone(), root.path().join("home")).await?;
        let analytics = Arc::new(
            AnalyticsService::new_for_test(&config.browserclaw_dir, Some("test-key"), host, true)
                .await?,
        );
        state.analytics = analytics.clone();
        let session_efficiency = Arc::new(SessionEfficiencyService::new_with_analytics(
            Database::open(config.browserclaw_dir.join(DATABASE_FILENAME)).await?,
            analytics.clone(),
        ));
        let sessions = Sessions::new_with_analytics(
            state.audit_log.clone(),
            state.session_tabs.clone(),
            config.session_idle,
            config.session_retention,
            config.session_sweep_interval,
            analytics.clone(),
        );
        sessions.set_completion_hook(Arc::new({
            let session_efficiency = session_efficiency.clone();
            move |session_id| {
                let _ = session_efficiency.queue_finalize(session_id);
            }
        }));
        state.session_efficiency = session_efficiency;
        state.sessions = sessions;
        let session = state
            .sessions
            .mint_with_id(
                SessionId::new("shutdown-session"),
                ClientIdentity::Ephemeral {
                    slug: "agent".to_string(),
                    label: "Agent".to_string(),
                },
                crate::identity::ClientInfo {
                    name: "Codex".to_string(),
                    version: "1".to_string(),
                    title: None,
                },
            )
            .await?;
        state
            .audit_log
            .record_tool_dispatch(efficiency_dispatch(
                session.id().as_str(),
                session.convo_id().as_str(),
            ))
            .await?;
        session
            .record_tool_usage("navigate", Duration::from_millis(5), 1)
            .await;

        AppRuntime::start(state).shutdown().await?;
        assert_eq!(analytics.shutdown_calls_for_testing(), 1);
        let mut captured = Vec::new();
        while let Ok(request) = requests.try_recv() {
            captured.extend(request["batch"].as_array().into_iter().flatten().cloned());
        }
        let ended = captured
            .iter()
            .find(|event| event["event"] == events::AGENT_SESSION_ENDED.name())
            .ok_or_else(|| anyhow::anyhow!("session-end event was not drained"))?;
        assert_eq!(
            ended["properties"],
            json!({
                "kind": "closed",
                "client_name": "codex",
                "dispatch_count": 1,
                "distinct_tool_count": 1,
                "max_concurrent_used_sessions": 1,
                "server_version": env!("CARGO_PKG_VERSION"),
                "os_platform": events::platform_token(),
                "$process_person_profile": false,
                "$geoip_disable": true,
                "$is_server": true,
            })
        );
        let efficiency = captured
            .iter()
            .find(|event| event["event"] == events::AGENT_SESSION_EFFICIENCY_COMPUTED.name())
            .ok_or_else(|| anyhow::anyhow!("session-efficiency event was not drained"))?;
        assert_eq!(
            efficiency["properties"],
            json!({
                "kind": "closed",
                "client_name": "codex",
                "dispatch_count": 1,
                "active_duration_ms": 5,
                "tool_input_token_estimate": 1,
                "tool_output_token_estimate": 0,
                "browserclaw_token_estimate": 1,
                "screenshot_baseline_token_estimate": 1_536,
                "screenshot_first_token_estimate": 1_537,
                "raw_token_savings_estimate": 1_536,
                "efficiency_estimator_version": 1,
                "screenshot_baseline_width": 1_920,
                "screenshot_baseline_height": 1_080,
                "screenshot_tokens_per_dispatch": 1_536,
                "server_version": env!("CARGO_PKG_VERSION"),
                "os_platform": events::platform_token(),
                "$process_person_profile": false,
                "$geoip_disable": true,
                "$is_server": true,
            })
        );
        endpoint.abort();
        Ok(())
    }

    #[tokio::test]
    async fn runtime_bounds_session_teardown_before_shutting_down_analytics() -> anyhow::Result<()>
    {
        let root = tempdir()?;
        let config = Arc::new(Config {
            server_port: 9200,
            cdp_port: 49337,
            proxy_port: None,
            resources_dir: root.path().join("resources"),
            browserclaw_dir: root.path().to_path_buf(),
            session_idle: Duration::from_secs(300),
            session_retention: Duration::from_secs(7_200),
            session_sweep_interval: Duration::from_secs(60),
            replay_retention_days: 7,
            dev_mode: false,
            auth_token: None,
        });
        let mut state = AppState::new_with_home(config.clone(), root.path().join("home")).await?;
        let analytics = Arc::new(
            AnalyticsService::new_for_test(
                &config.browserclaw_dir,
                None,
                "http://127.0.0.1:1".to_string(),
                true,
            )
            .await?,
        );
        let sessions = Sessions::new_with_analytics(
            state.audit_log.clone(),
            state.session_tabs.clone(),
            config.session_idle,
            config.session_retention,
            config.session_sweep_interval,
            analytics.clone(),
        );
        let hook_entered = Arc::new(Notify::new());
        sessions.set_retained_group_hook(Arc::new({
            let hook_entered = hook_entered.clone();
            move |_, _, _| {
                let hook_entered = hook_entered.clone();
                Box::pin(async move {
                    hook_entered.notify_one();
                    pending::<bool>().await
                })
            }
        }));
        let session = Session::new(
            SessionId::new("stuck-teardown"),
            ClientIdentity::Ephemeral {
                slug: "agent".to_string(),
                label: "Agent".to_string(),
            },
            ConversationIdentity::new("agent", "stuck-label".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );
        sessions.insert_for_testing(session.clone()).await;
        state.analytics = analytics.clone();
        state.sessions = sessions.clone();

        let remove = tokio::spawn({
            let sessions = sessions.clone();
            async move {
                sessions
                    .remove(session.id(), "closed", Some("transport closed"))
                    .await
            }
        });
        tokio::time::timeout(Duration::from_secs(1), hook_entered.notified()).await?;
        let result = AppRuntime {
            state,
            tasks: Vec::new(),
        }
        .shutdown_with_session_timeout(Duration::from_millis(50))
        .await;
        let Err(error) = result else {
            anyhow::bail!("stuck teardown unexpectedly completed");
        };
        remove.abort();
        assert!(error.to_string().contains("session teardown exceeded"));
        assert_eq!(analytics.shutdown_calls_for_testing(), 1);
        Ok(())
    }
}
