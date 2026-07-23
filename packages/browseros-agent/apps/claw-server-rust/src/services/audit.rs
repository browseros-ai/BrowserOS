use crate::{
    db::{AuditLog, audit_log::RecordToolDispatchInput},
    error::{AppError, AppResult},
    ids::DispatchId,
    services::{cockpit::SessionVisualService, screenshots::ScreenshotService, sessions::Session},
};
use futures_util::future::BoxFuture;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
};
use tokio::sync::{Semaphore, mpsc, oneshot};
use tracing::warn;

pub const AUDIT_INGRESS_CAPACITY: usize = 64;
pub const AUDIT_PREVIEW_CONCURRENCY_LIMIT: usize = 2;

pub struct AuditEvent {
    pub input: RecordToolDispatchInput,
    pub preview_session: Option<Weak<Session>>,
}

impl AuditEvent {
    #[must_use]
    pub fn without_preview(input: RecordToolDispatchInput) -> Self {
        Self {
            input,
            preview_session: None,
        }
    }
}

trait AuditStore: Send + Sync {
    fn append(&self, input: RecordToolDispatchInput) -> BoxFuture<'_, AppResult<i64>>;
    fn mark_screenshot(&self, dispatch_id: i64) -> BoxFuture<'_, AppResult<()>>;
    fn refresh_task<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, AppResult<()>>;
}

impl AuditStore for AuditLog {
    fn append(&self, input: RecordToolDispatchInput) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(self.append_tool_dispatch(input))
    }

    fn mark_screenshot(&self, dispatch_id: i64) -> BoxFuture<'_, AppResult<()>> {
        Box::pin(self.mark_screenshot_without_projection(dispatch_id))
    }

    fn refresh_task<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(self.refresh_task(session_id))
    }
}

struct PreviewRequest {
    session_id: String,
    dispatch_id: DispatchId,
    row_id: i64,
    session: Weak<Session>,
}

trait PreviewBackend: Send + Sync {
    fn capture_and_write(&self, request: PreviewRequest) -> BoxFuture<'_, AppResult<bool>>;
}

struct ProductionPreviewBackend {
    visuals: Arc<SessionVisualService>,
    screenshots: Arc<ScreenshotService>,
}

impl PreviewBackend for ProductionPreviewBackend {
    fn capture_and_write(&self, request: PreviewRequest) -> BoxFuture<'_, AppResult<bool>> {
        Box::pin(async move {
            let Some(session) = request.session.upgrade() else {
                return Ok(false);
            };
            let Some(bytes) = self.visuals.capture_for_session(&session).await? else {
                return Ok(false);
            };
            self.screenshots
                .write(&request.session_id, request.row_id, &bytes)
                .await?;
            Ok(true)
        })
    }
}

enum AuditCommand {
    Event(AuditEvent),
    Flush {
        session_id: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
    Shutdown {
        reply: oneshot::Sender<AppResult<()>>,
    },
}

struct PreviewCompletion {
    session_id: String,
    dispatch_id: DispatchId,
    row_id: i64,
    result: AppResult<bool>,
}

#[derive(Default)]
struct SessionWork {
    preview_in_flight: bool,
    pending_preview: Option<PreviewRequest>,
    flush_waiters: Vec<oneshot::Sender<AppResult<()>>>,
    errors: Vec<String>,
}

/// Serializes audit persistence while preview captures run behind bounded per-session coalescing.
pub struct AuditWorker {
    sender: mpsc::Sender<AuditCommand>,
}

impl AuditWorker {
    #[must_use]
    pub fn new(
        audit_log: Arc<AuditLog>,
        screenshots: Arc<ScreenshotService>,
        visuals: Arc<SessionVisualService>,
    ) -> Arc<Self> {
        Self::start(
            audit_log,
            Arc::new(ProductionPreviewBackend {
                visuals,
                screenshots,
            }),
            AUDIT_INGRESS_CAPACITY,
            AUDIT_PREVIEW_CONCURRENCY_LIMIT,
        )
    }

    fn start(
        store: Arc<dyn AuditStore>,
        previews: Arc<dyn PreviewBackend>,
        ingress_capacity: usize,
        preview_concurrency: usize,
    ) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel(ingress_capacity);
        let (preview_sender, preview_receiver) = mpsc::channel(ingress_capacity);
        tokio::spawn(
            AuditActor {
                receiver,
                preview_sender,
                preview_receiver,
                store,
                previews,
                preview_limit: Arc::new(Semaphore::new(preview_concurrency)),
                sessions: HashMap::new(),
                shutdown_reply: None,
                command_channel_open: true,
            }
            .run(),
        );
        Arc::new(Self { sender })
    }

    /// A successful submit acknowledges bounded queue admission, not durable persistence.
    pub async fn submit(&self, event: AuditEvent) -> AppResult<()> {
        self.sender
            .send(AuditCommand::Event(event))
            .await
            .map_err(|_| AppError::Internal("audit worker has shut down".to_string()))
    }

    pub async fn flush_session(&self, session_id: &str) -> AppResult<()> {
        let (reply, result) = oneshot::channel();
        self.sender
            .send(AuditCommand::Flush {
                session_id: session_id.to_string(),
                reply,
            })
            .await
            .map_err(|_| AppError::Internal("audit worker has shut down".to_string()))?;
        result
            .await
            .map_err(|_| AppError::Internal("audit worker stopped before flush".to_string()))?
    }

    pub async fn shutdown(&self) -> AppResult<()> {
        let (reply, result) = oneshot::channel();
        self.sender
            .send(AuditCommand::Shutdown { reply })
            .await
            .map_err(|_| AppError::Internal("audit worker has already shut down".to_string()))?;
        result
            .await
            .map_err(|_| AppError::Internal("audit worker stopped during shutdown".to_string()))?
    }
}

struct AuditActor {
    receiver: mpsc::Receiver<AuditCommand>,
    preview_sender: mpsc::Sender<PreviewCompletion>,
    preview_receiver: mpsc::Receiver<PreviewCompletion>,
    store: Arc<dyn AuditStore>,
    previews: Arc<dyn PreviewBackend>,
    preview_limit: Arc<Semaphore>,
    sessions: HashMap<String, SessionWork>,
    shutdown_reply: Option<oneshot::Sender<AppResult<()>>>,
    command_channel_open: bool,
}

enum ActorInput {
    Command(AuditCommand),
    CommandsClosed,
    Preview(PreviewCompletion),
}

impl AuditActor {
    async fn run(mut self) {
        loop {
            let input = tokio::select! {
                command = self.receiver.recv(), if self.command_channel_open => {
                    match command {
                        Some(command) => ActorInput::Command(command),
                        None => ActorInput::CommandsClosed,
                    }
                }
                preview = self.preview_receiver.recv(), if self.has_preview_in_flight() => {
                    match preview {
                        Some(preview) => ActorInput::Preview(preview),
                        None => break,
                    }
                }
            };

            let mut commands = Vec::new();
            let mut previews = Vec::new();
            match input {
                ActorInput::Command(command) => commands.push(command),
                ActorInput::CommandsClosed => self.command_channel_open = false,
                ActorInput::Preview(preview) => previews.push(preview),
            }
            while let Ok(command) = self.receiver.try_recv() {
                commands.push(command);
            }
            while let Ok(preview) = self.preview_receiver.try_recv() {
                previews.push(preview);
            }

            let mut dirty_sessions = HashSet::new();
            for command in commands {
                self.process_command(command, &mut dirty_sessions).await;
            }
            for preview in previews {
                self.process_preview(preview, &mut dirty_sessions).await;
            }
            self.refresh_dirty(dirty_sessions).await;
            self.settle_flushes();

            if !self.command_channel_open && !self.has_preview_in_flight() {
                self.finish_shutdown();
                return;
            }
        }
    }

    async fn process_command(
        &mut self,
        command: AuditCommand,
        dirty_sessions: &mut HashSet<String>,
    ) {
        match command {
            AuditCommand::Event(event) => {
                let session_id = event.input.session_id.clone();
                let dispatch_id = event.input.dispatch_id.clone();
                match self.store.append(event.input).await {
                    Ok(row_id) => {
                        dirty_sessions.insert(session_id.clone());
                        if let Some(session) = event.preview_session {
                            self.queue_preview(PreviewRequest {
                                session_id,
                                dispatch_id,
                                row_id,
                                session,
                            });
                        }
                    }
                    Err(error) => {
                        warn!(
                            error = %error,
                            dispatch_id = %dispatch_id,
                            "audit dispatch append failed"
                        );
                        self.sessions
                            .entry(session_id)
                            .or_default()
                            .errors
                            .push(error.to_string());
                    }
                }
            }
            AuditCommand::Flush { session_id, reply } => {
                self.sessions
                    .entry(session_id)
                    .or_default()
                    .flush_waiters
                    .push(reply);
            }
            AuditCommand::Shutdown { reply } => {
                if self.shutdown_reply.is_none() {
                    self.shutdown_reply = Some(reply);
                    self.receiver.close();
                } else {
                    let _ = reply.send(Err(AppError::Internal(
                        "audit worker shutdown already requested".to_string(),
                    )));
                }
            }
        }
    }

    async fn process_preview(
        &mut self,
        preview: PreviewCompletion,
        dirty_sessions: &mut HashSet<String>,
    ) {
        let state = self.sessions.entry(preview.session_id.clone()).or_default();
        state.preview_in_flight = false;
        match preview.result {
            Ok(true) => match self.store.mark_screenshot(preview.row_id).await {
                Ok(()) => {
                    dirty_sessions.insert(preview.session_id.clone());
                }
                Err(error) => warn!(
                    error = %error,
                    dispatch_id = %preview.dispatch_id,
                    "audit screenshot marker failed"
                ),
            },
            Ok(false) => {}
            Err(error) => warn!(
                error = %error,
                dispatch_id = %preview.dispatch_id,
                "audit preview failed"
            ),
        }
        if let Some(next) = state.pending_preview.take() {
            self.start_preview(next);
        }
    }

    fn queue_preview(&mut self, request: PreviewRequest) {
        let state = self.sessions.entry(request.session_id.clone()).or_default();
        if state.preview_in_flight {
            state.pending_preview = Some(request);
        } else {
            self.start_preview(request);
        }
    }

    fn start_preview(&mut self, request: PreviewRequest) {
        self.sessions
            .entry(request.session_id.clone())
            .or_default()
            .preview_in_flight = true;
        let previews = self.previews.clone();
        let preview_limit = self.preview_limit.clone();
        let preview_sender = self.preview_sender.clone();
        tokio::spawn(async move {
            let session_id = request.session_id.clone();
            let dispatch_id = request.dispatch_id.clone();
            let row_id = request.row_id;
            let result = match preview_limit.acquire_owned().await {
                Ok(_permit) => previews.capture_and_write(request).await,
                Err(_) => Err(AppError::Internal(
                    "audit preview limiter has shut down".to_string(),
                )),
            };
            let _ = preview_sender
                .send(PreviewCompletion {
                    session_id,
                    dispatch_id,
                    row_id,
                    result,
                })
                .await;
        });
    }

    async fn refresh_dirty(&mut self, dirty_sessions: HashSet<String>) {
        for session_id in dirty_sessions {
            if let Err(error) = self.store.refresh_task(&session_id).await {
                warn!(
                    error = %error,
                    session_id,
                    "audit task projection failed"
                );
                self.sessions
                    .entry(session_id)
                    .or_default()
                    .errors
                    .push(error.to_string());
            }
        }
    }

    fn settle_flushes(&mut self) {
        for state in self.sessions.values_mut() {
            if state.preview_in_flight || state.pending_preview.is_some() {
                continue;
            }
            let waiters = std::mem::take(&mut state.flush_waiters);
            if waiters.is_empty() {
                continue;
            }
            let error = (!state.errors.is_empty()).then(|| state.errors.join("; "));
            state.errors.clear();
            for waiter in waiters {
                let result = error
                    .as_ref()
                    .map_or_else(|| Ok(()), |error| Err(AppError::Internal(error.clone())));
                let _ = waiter.send(result);
            }
        }
    }

    fn has_preview_in_flight(&self) -> bool {
        self.sessions.values().any(|state| state.preview_in_flight)
    }

    fn finish_shutdown(&mut self) {
        let errors = self
            .sessions
            .values()
            .flat_map(|state| state.errors.iter().cloned())
            .collect::<Vec<_>>();
        if let Some(reply) = self.shutdown_reply.take() {
            let result = if errors.is_empty() {
                Ok(())
            } else {
                Err(AppError::Internal(errors.join("; ")))
            };
            let _ = reply.send(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::audit_log::{bounded_args_json, result_meta};
    use std::sync::{
        Mutex as StdMutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    };
    use tokio::{
        sync::Notify,
        time::{Duration, timeout},
    };

    struct TestStore {
        append_permits: Semaphore,
        append_started: Notify,
        rows: StdMutex<Vec<(String, String)>>,
        refreshes: AtomicUsize,
        next_row_id: AtomicI64,
        failing_tool: Option<&'static str>,
    }

    impl TestStore {
        fn new(append_permits: usize, failing_tool: Option<&'static str>) -> Arc<Self> {
            Arc::new(Self {
                append_permits: Semaphore::new(append_permits),
                append_started: Notify::new(),
                rows: StdMutex::new(Vec::new()),
                refreshes: AtomicUsize::new(0),
                next_row_id: AtomicI64::new(1),
                failing_tool,
            })
        }

        fn tools(&self) -> Vec<String> {
            self.rows
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .map(|(_, tool)| tool.clone())
                .collect()
        }
    }

    impl AuditStore for TestStore {
        fn append(&self, input: RecordToolDispatchInput) -> BoxFuture<'_, AppResult<i64>> {
            Box::pin(async move {
                self.append_started.notify_waiters();
                let permit =
                    self.append_permits.acquire().await.map_err(|_| {
                        AppError::Internal("test append semaphore closed".to_string())
                    })?;
                permit.forget();
                if self.failing_tool == Some(input.tool_name.as_str()) {
                    return Err(AppError::Internal(format!("failed {}", input.tool_name)));
                }
                self.rows
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push((input.session_id, input.tool_name));
                Ok(self.next_row_id.fetch_add(1, Ordering::SeqCst))
            })
        }

        fn mark_screenshot(&self, _dispatch_id: i64) -> BoxFuture<'_, AppResult<()>> {
            Box::pin(async { Ok(()) })
        }

        fn refresh_task<'a>(&'a self, _session_id: &'a str) -> BoxFuture<'a, AppResult<()>> {
            Box::pin(async move {
                self.refreshes.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    struct NoPreview;

    impl PreviewBackend for NoPreview {
        fn capture_and_write(&self, _request: PreviewRequest) -> BoxFuture<'_, AppResult<bool>> {
            Box::pin(async { Ok(false) })
        }
    }

    fn event(session_id: &str, tool_name: &str) -> AuditEvent {
        AuditEvent::without_preview(RecordToolDispatchInput {
            agent_id: "agent-session".to_string(),
            slug: "agent".to_string(),
            agent_label: "Agent".to_string(),
            session_id: session_id.to_string(),
            tool_name: tool_name.to_string(),
            page_id: None,
            tab_id: None,
            target_id: None,
            url: None,
            title: None,
            args_json: bounded_args_json(&serde_json::json!({})),
            result_meta: result_meta(false, false, &serde_json::Value::Null, 0),
            duration_ms: 1,
            dispatch_id: DispatchId::new(),
            tool_input_token_estimate: 1,
            tool_output_token_estimate: 1,
            token_estimator_version: 1,
        })
    }

    #[tokio::test]
    async fn admission_returns_before_persistence_and_full_ingress_backpressures()
    -> anyhow::Result<()> {
        let store = TestStore::new(0, None);
        let worker = AuditWorker::start(store.clone(), Arc::new(NoPreview), 1, 1);
        let append_started = store.append_started.notified();

        worker.submit(event("s1", "one")).await?;
        append_started.await;
        worker.submit(event("s1", "two")).await?;
        let third_worker = worker.clone();
        let mut third =
            tokio::spawn(async move { third_worker.submit(event("s1", "three")).await });
        assert!(
            timeout(Duration::from_millis(50), &mut third)
                .await
                .is_err()
        );

        store.append_permits.add_permits(3);
        third.await??;
        worker.flush_session("s1").await?;
        assert_eq!(store.tools(), ["one", "two", "three"]);
        worker.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn accepted_order_survives_one_failed_event_and_the_worker_continues()
    -> anyhow::Result<()> {
        let store = TestStore::new(4, Some("fail"));
        let worker = AuditWorker::start(store.clone(), Arc::new(NoPreview), 4, 1);
        worker.submit(event("s1", "one")).await?;
        worker.submit(event("s1", "fail")).await?;
        worker.submit(event("s1", "three")).await?;

        let error = worker
            .flush_session("s1")
            .await
            .expect_err("failed append must complete the barrier with an error");
        assert!(error.to_string().contains("failed fail"));
        assert_eq!(store.tools(), ["one", "three"]);

        worker.submit(event("s1", "four")).await?;
        worker.flush_session("s1").await?;
        assert_eq!(store.tools(), ["one", "three", "four"]);
        worker.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn ready_events_refresh_one_projection_per_batch() -> anyhow::Result<()> {
        let store = TestStore::new(0, None);
        let worker = AuditWorker::start(store.clone(), Arc::new(NoPreview), 4, 1);
        let append_started = store.append_started.notified();
        worker.submit(event("s1", "one")).await?;
        append_started.await;
        worker.submit(event("s1", "two")).await?;
        worker.submit(event("s1", "three")).await?;
        store.append_permits.add_permits(3);

        worker.flush_session("s1").await?;
        assert_eq!(store.tools(), ["one", "two", "three"]);
        assert!(
            store.refreshes.load(Ordering::SeqCst) <= 2,
            "the first blocked event and one ready batch need at most two projections"
        );
        worker.shutdown().await?;
        Ok(())
    }
}
