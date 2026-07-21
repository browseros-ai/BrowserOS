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
    use super::ShutdownHandle;
    use std::time::Duration;

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
}
