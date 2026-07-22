use crate::{AppState, error::AppResult};
use std::{future::Future, time::Duration};
use tokio::{task::JoinHandle, time::timeout};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

const TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

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

    pub async fn shutdown(mut self) -> AppResult<()> {
        self.state.shutdown.request();
        let session_result = self.state.sessions.shutdown().await;
        self.state.session_tabs.drain_writes().await;
        self.state.recordings.close().await;
        self.state.browser.stop();
        self.join_tasks().await;
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
        identity::{ClientIdentity, ConversationIdentity},
        ids::SessionId,
        services::sessions::{Session, Sessions},
    };
    use axum::{Router, body::Bytes, routing::any};
    use serde_json::Value;
    use std::{sync::Arc, time::Duration};
    use tempfile::tempdir;
    use tokio::{net::TcpListener, sync::mpsc, time::Instant};

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
        state.sessions = Sessions::new_with_analytics(
            state.audit_log.clone(),
            state.session_tabs.clone(),
            config.session_idle,
            config.session_retention,
            config.session_sweep_interval,
            analytics.clone(),
        );
        state
            .sessions
            .insert_for_testing(Session::new(
                SessionId::new("shutdown-session"),
                ClientIdentity::Ephemeral {
                    slug: "agent".to_string(),
                    label: "Agent".to_string(),
                },
                ConversationIdentity::new("agent", "shutdown-label".to_string()),
                Instant::now(),
            ))
            .await;

        AppRuntime::start(state).shutdown().await?;
        assert_eq!(analytics.shutdown_calls_for_testing(), 1);
        let request = tokio::time::timeout(Duration::from_secs(2), requests.recv())
            .await?
            .ok_or_else(|| anyhow::anyhow!("session-end event was not drained"))?;
        assert_eq!(
            request["batch"][0]["event"],
            events::AGENT_SESSION_ENDED.name()
        );
        endpoint.abort();
        Ok(())
    }
}
