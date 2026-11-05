//! The one writer task, and the [`TraceSink`] the dispatcher talks to.
//!
//! The dispatcher's hooks are synchronous and must never wait on disk, so every event is a
//! channel send. The writer takes the first event, gathers more for up to `batch_window` (or
//! `batch_max` events), redacts them, and commits them in one transaction. After the commit it
//! answers any privacy commit or flush in the batch, then publishes the redacted events to
//! subscribers (the service API's SSE).
//!
//! Backpressure: if the disk stalls and the backlog passes `max_backlog`, operation events are
//! dropped and counted as the run's `trace_gaps` (which make its effects `unknown`). Session and
//! run lifecycle events, privacy commits and flushes are never dropped.
//!
//! Redaction needs one fact that only the end of an operation knows: whether its target was
//! sensitive. So the start row stores the input with both `secret` and `sensitive` paths
//! redacted. The raw input stays in this task's memory while the operation is open, and the end
//! rewrites the row with the precise redaction. A crash leaves the conservative copy.

use crate::trace_store::{
    self, OperationEndRow, OperationStartRow, RetentionPolicy, RunCode, RunEndRow, RunStartRow,
    Swept,
};
use crate::{
    Bounded, Delivery, EpochMillis, OperationEnded, OperationId, OperationStarted, Outcome,
    PrivacyRecord, PrivacyStore, RedactionPolicy, Redactor, RunEffects, RunEnded, RunId,
    RunStarted, RunStatus, SessionEnded, SessionId, SessionStarted, TargetSensitivity, TraceError,
    TraceStore, Truncation,
};
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot};

/// Tuning for the writer. The defaults suit one browser's worth of agents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriterConfig {
    /// Most events in one transaction.
    pub batch_max: usize,
    /// How long the writer gathers events after the first before committing.
    pub batch_window: Duration,
    /// Queued events past which operation events are dropped (and counted).
    pub max_backlog: usize,
    /// How long a finished run's exact result is kept to answer a repeat of its request id.
    pub replay_window: Duration,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            batch_max: 64,
            batch_window: Duration::from_millis(5),
            max_backlog: 8192,
            replay_window: Duration::from_secs(10 * 60),
        }
    }
}

/// A committed, redacted change, for live subscribers. Subscribers read details from the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceRecordEvent {
    /// A run was admitted.
    RunStarted {
        /// The run.
        run: RunId,
        /// Its session.
        session: SessionId,
        /// When.
        at: EpochMillis,
    },
    /// A run ended.
    RunEnded {
        /// The run.
        run: RunId,
        /// Its session.
        session: SessionId,
        /// How it ended.
        status: RunStatus,
        /// What it changed.
        effects: RunEffects,
        /// When.
        at: EpochMillis,
    },
    /// An operation was admitted.
    OperationStarted {
        /// The operation.
        id: OperationId,
        /// Its session.
        session: SessionId,
        /// When.
        at: EpochMillis,
    },
    /// An operation ended.
    OperationEnded {
        /// The operation.
        id: OperationId,
        /// Its session.
        session: SessionId,
        /// How it ended.
        outcome: Outcome,
        /// What happened to its side effect.
        delivery: Delivery,
        /// When.
        at: EpochMillis,
    },
    /// A session became private.
    SessionPrivate {
        /// The session.
        session: SessionId,
        /// When and why.
        record: PrivacyRecord,
    },
}

enum Event {
    SessionStarted(SessionStarted),
    SessionEnded(SessionEnded),
    RunStarted(RunStarted),
    RunEnded(RunEnded),
    OperationStarted(OperationStarted),
    OperationEnded(OperationEnded),
}

enum Command {
    Event(Box<Event>),
    PersistPrivate {
        session: SessionId,
        record: PrivacyRecord,
        reply: oneshot::Sender<Result<(), TraceError>>,
    },
    Sweep {
        policy: RetentionPolicy,
        now: EpochMillis,
        reply: oneshot::Sender<Result<Swept, TraceError>>,
    },
    Flush(oneshot::Sender<Result<(), TraceError>>),
}

/// Operation events dropped over the backlog cap, per run. Shared by every sink and the writer
/// itself, so a run's count survives the last sink being dropped while its end is still queued.
#[derive(Default)]
struct DroppedEvents(Mutex<HashMap<RunId, u64>>);

impl DroppedEvents {
    fn count(&self, run: RunId) {
        *self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(run)
            .or_default() += 1;
    }

    /// The run's count, taken once.
    fn take(&self, run: RunId) -> u64 {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&run)
            .unwrap_or(0)
    }
}

/// One command's work inside a batch, its reply already set aside.
enum Work {
    Event(Box<Event>),
    Private {
        session: SessionId,
        record: PrivacyRecord,
    },
    Sweep {
        policy: RetentionPolicy,
        now: EpochMillis,
    },
}

struct SinkInner {
    tx: mpsc::UnboundedSender<Command>,
    backlog: AtomicUsize,
    max_backlog: usize,
    dropped: Arc<DroppedEvents>,
    events: broadcast::Sender<TraceRecordEvent>,
}

/// The dispatcher's handle on the writer. Cheap to clone. Dropping the last one lets the writer
/// drain what is queued and stop.
#[derive(Clone)]
pub struct TraceSink {
    inner: Arc<SinkInner>,
}

impl TraceSink {
    /// A session started.
    pub fn session_started(&self, event: SessionStarted) {
        self.send(Event::SessionStarted(event));
    }

    /// A session ended.
    pub fn session_ended(&self, event: SessionEnded) {
        self.send(Event::SessionEnded(event));
    }

    /// A run was admitted.
    pub fn run_started(&self, event: RunStarted) {
        self.send(Event::RunStarted(event));
    }

    /// A run ended.
    pub fn run_ended(&self, event: RunEnded) {
        self.send(Event::RunEnded(event));
    }

    /// An operation was admitted. Dropped (and counted) when the writer is far behind.
    pub fn op_started(&self, event: OperationStarted) {
        if self.over_backlog(event.id.run) {
            return;
        }
        self.send(Event::OperationStarted(event));
    }

    /// An operation ended. Dropped (and counted) when the writer is far behind.
    pub fn op_ended(&self, event: OperationEnded) {
        if self.over_backlog(event.id.run) {
            return;
        }
        self.send(Event::OperationEnded(event));
    }

    /// Committed, redacted changes from now on.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<TraceRecordEvent> {
        self.inner.events.subscribe()
    }

    /// Waits until everything sent before this call is committed.
    ///
    /// # Errors
    /// When the writer has stopped, or a batch failed since the previous flush (`BatchLost`):
    /// then some records sent before this call were rolled back.
    pub async fn flush(&self) -> Result<(), TraceError> {
        let (reply, done) = oneshot::channel();
        self.command(Command::Flush(reply))?;
        done.await.map_err(|_| TraceError::Closed)?
    }

    /// Deletes records past `policy`'s age, in order with every other write.
    ///
    /// # Errors
    /// When the writer has stopped or the delete fails.
    pub async fn sweep(
        &self,
        policy: RetentionPolicy,
        now: EpochMillis,
    ) -> Result<Swept, TraceError> {
        let (reply, done) = oneshot::channel();
        self.command(Command::Sweep { policy, now, reply })?;
        done.await.map_err(|_| TraceError::Closed)?
    }

    fn over_backlog(&self, run: RunId) -> bool {
        if self.inner.backlog.load(Ordering::Relaxed) < self.inner.max_backlog {
            return false;
        }
        self.inner.dropped.count(run);
        true
    }

    fn send(&self, event: Event) {
        if self.command(Command::Event(Box::new(event))).is_err() {
            tracing::warn!("trace writer stopped; event lost");
        }
    }

    fn command(&self, command: Command) -> Result<(), TraceError> {
        self.inner.backlog.fetch_add(1, Ordering::Relaxed);
        self.inner.tx.send(command).map_err(|_| {
            self.inner.backlog.fetch_sub(1, Ordering::Relaxed);
            TraceError::Closed
        })
    }
}

impl PrivacyStore for TraceSink {
    fn persist_private(
        &self,
        session: SessionId,
        record: PrivacyRecord,
    ) -> BoxFuture<'_, Result<(), TraceError>> {
        Box::pin(async move {
            let (reply, done) = oneshot::channel();
            self.command(Command::PersistPrivate {
                session,
                record,
                reply,
            })?;
            done.await.map_err(|_| TraceError::Closed)?
        })
    }
}

/// Starts the writer on `store`. It runs until every [`TraceSink`] clone is dropped.
#[must_use]
pub fn spawn_writer(store: &TraceStore, redactor: Redactor, config: WriterConfig) -> TraceSink {
    let (tx, rx) = mpsc::unbounded_channel();
    let dropped = Arc::new(DroppedEvents::default());
    let sink = TraceSink {
        inner: Arc::new(SinkInner {
            tx,
            backlog: AtomicUsize::new(0),
            max_backlog: config.max_backlog,
            dropped: Arc::clone(&dropped),
            events: broadcast::channel(1024).0,
        }),
    };
    let writer = Writer {
        store: store.clone(),
        redactor,
        config,
        runs: HashMap::new(),
        dropped,
        lost_since_flush: None,
        // A weak handle: the writer must not keep its own channel open.
        sink: Arc::downgrade(&sink.inner),
    };
    tokio::spawn(writer.run(rx));
    sink
}

/// What the writer remembers about a run while it is open.
struct OpenRun {
    session: SessionId,
    /// The masked, bounded code. It reaches disk only at the run's end, and only when no
    /// operation turned out sensitive, so a withheld literal is never written, not even to the
    /// WAL. `None` when the run's start was never seen.
    code: Option<Bounded<String>>,
    sensitive: bool,
    gaps: u64,
    operations: HashMap<u32, OpenOperation>,
}

struct OpenOperation {
    session: SessionId,
    args: Value,
    policy: Arc<RedactionPolicy>,
}

struct Writer {
    store: TraceStore,
    redactor: Redactor,
    config: WriterConfig,
    runs: HashMap<RunId, OpenRun>,
    dropped: Arc<DroppedEvents>,
    /// Why a batch failed since the last flush reply, so that flush reports the loss.
    lost_since_flush: Option<String>,
    sink: std::sync::Weak<SinkInner>,
}

impl Writer {
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Command>) {
        while let Some(first) = rx.recv().await {
            let mut batch = vec![first];
            let deadline = tokio::time::Instant::now() + self.config.batch_window;
            while batch.len() < self.config.batch_max {
                match tokio::time::timeout_at(deadline, rx.recv()).await {
                    Ok(Some(command)) => batch.push(command),
                    Ok(None) | Err(_) => break,
                }
            }
            let count = batch.len();
            self.commit(batch).await;
            if let Some(sink) = self.sink.upgrade() {
                sink.backlog.fetch_sub(count, Ordering::Relaxed);
            }
        }
    }

    async fn commit(&mut self, batch: Vec<Command>) {
        // Every reply is taken out of the batch before the transaction runs. A command that
        // fails part-way through must not drop the replies of the commands after it: their
        // callers would see `Closed` instead of the batch's real outcome.
        let mut work = Vec::with_capacity(batch.len());
        let mut privacy_replies = Vec::new();
        let mut sweep_replies = Vec::new();
        let mut flushes = Vec::new();
        for command in batch {
            match command {
                Command::Event(event) => work.push(Work::Event(event)),
                Command::PersistPrivate {
                    session,
                    record,
                    reply,
                } => {
                    work.push(Work::Private { session, record });
                    privacy_replies.push(reply);
                }
                Command::Sweep { policy, now, reply } => {
                    work.push(Work::Sweep { policy, now });
                    sweep_replies.push(reply);
                }
                Command::Flush(reply) => flushes.push(reply),
            }
        }
        let mut published = Vec::new();
        let mut swept = Vec::new();
        let result = async {
            let mut tx = self.store.pool().begin().await?;
            for item in work {
                match item {
                    Work::Event(event) => {
                        if let Some(change) = self.apply(&mut tx, *event).await? {
                            published.push(change);
                        }
                    }
                    Work::Private { session, record } => {
                        trace_store::persist_private(&mut tx, session, record).await?;
                        published.push(TraceRecordEvent::SessionPrivate { session, record });
                    }
                    Work::Sweep { policy, now } => {
                        swept.push(trace_store::sweep(&mut tx, policy, now).await?);
                    }
                }
            }
            tx.commit().await?;
            Ok::<(), TraceError>(())
        }
        .await;
        match result {
            Ok(()) => {
                for reply in privacy_replies {
                    let _ = reply.send(Ok(()));
                }
                for (swept, reply) in swept.into_iter().zip(sweep_replies) {
                    let _ = reply.send(Ok(swept));
                }
                if let Some(sink) = self.sink.upgrade() {
                    for change in published {
                        // No subscriber is not an error.
                        let _ = sink.events.send(change);
                    }
                }
            }
            Err(error) => {
                tracing::error!(%error, "trace batch failed; its records are lost");
                for reply in privacy_replies {
                    let _ = reply.send(Err(TraceError::BatchLost(error.to_string())));
                }
                for reply in sweep_replies {
                    let _ = reply.send(Err(TraceError::BatchLost(error.to_string())));
                }
                for run in self.runs.values_mut() {
                    run.gaps += 1;
                }
                self.lost_since_flush = Some(error.to_string());
            }
        }
        if !flushes.is_empty() {
            let lost = self.lost_since_flush.take();
            for flush in flushes {
                let _ = flush.send(match &lost {
                    Some(reason) => Err(TraceError::BatchLost(reason.clone())),
                    None => Ok(()),
                });
            }
        }
    }

    async fn apply(
        &mut self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        event: Event,
    ) -> Result<Option<TraceRecordEvent>, TraceError> {
        match event {
            Event::SessionStarted(event) => {
                trace_store::touch_session(tx, event.id, event.client.as_deref(), event.at).await?;
                Ok(None)
            }
            Event::SessionEnded(event) => {
                trace_store::end_session(tx, event.id, event.at).await?;
                Ok(None)
            }
            Event::RunStarted(event) => {
                trace_store::touch_session(tx, event.session, None, event.at).await?;
                let code = self.redactor.code(&event.code);
                let host_data = event
                    .host_data
                    .as_ref()
                    .map(|data| self.redactor.host_data(data).value);
                trace_store::insert_run(
                    tx,
                    RunStartRow {
                        id: event.id,
                        session: event.session,
                        at: event.at,
                        request_id: event.request_id.as_deref(),
                        host_data: host_data.as_ref(),
                    },
                )
                .await?;
                self.runs.insert(
                    event.id,
                    OpenRun {
                        session: event.session,
                        code: Some(code),
                        sensitive: false,
                        gaps: 0,
                        operations: HashMap::new(),
                    },
                );
                Ok(Some(TraceRecordEvent::RunStarted {
                    run: event.id,
                    session: event.session,
                    at: event.at,
                }))
            }
            Event::OperationStarted(event) => self.operation_started(tx, event).await,
            Event::OperationEnded(event) => self.operation_ended(tx, event).await,
            Event::RunEnded(event) => self.run_ended(tx, event).await,
        }
    }

    async fn operation_started(
        &mut self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        event: OperationStarted,
    ) -> Result<Option<TraceRecordEvent>, TraceError> {
        if !self.runs.contains_key(&event.id.run) {
            // Its run's start was never seen (a failed batch): keep the operation reachable.
            trace_store::insert_run(
                tx,
                RunStartRow {
                    id: event.id.run,
                    session: event.session,
                    at: event.at,
                    request_id: None,
                    host_data: None,
                },
            )
            .await?;
        }
        let conservative =
            self.redactor
                .args(&event.args, &event.policy, TargetSensitivity::Sensitive);
        trace_store::insert_operation(
            tx,
            OperationStartRow {
                id: event.id,
                parent: event.parent,
                session: event.session,
                name: &event.name,
                kind: event.kind,
                effect: event.effect,
                page: event.page,
                at: event.at,
                args: &conservative.value,
                args_truncated: conservative.truncation == Truncation::Cut,
            },
        )
        .await?;
        let run = self.runs.entry(event.id.run).or_insert_with(|| OpenRun {
            session: event.session,
            code: None,
            sensitive: false,
            gaps: 0,
            operations: HashMap::new(),
        });
        run.operations.insert(
            event.id.seq,
            OpenOperation {
                session: event.session,
                args: event.args,
                policy: event.policy,
            },
        );
        Ok(Some(TraceRecordEvent::OperationStarted {
            id: event.id,
            session: event.session,
            at: event.at,
        }))
    }

    async fn operation_ended(
        &mut self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        event: OperationEnded,
    ) -> Result<Option<TraceRecordEvent>, TraceError> {
        let open = self.runs.get_mut(&event.id.run).and_then(|run| {
            if event.target == TargetSensitivity::Sensitive {
                run.sensitive = true;
            }
            run.operations.remove(&event.id.seq)
        });
        let precise = open
            .as_ref()
            .map(|open| self.redactor.args(&open.args, &open.policy, event.target));
        let result = event
            .result
            .as_ref()
            .map(|result| self.redactor.result(result));
        let error = event.error.map(|mut error| {
            error.message = self.redactor.text(&error.message);
            error
        });
        let page = event.page_after.map(|mut page| {
            page.url = page.url.map(|url| self.redactor.text(&url));
            page.title = page.title.map(|title| self.redactor.text(&title));
            page
        });
        let matched = trace_store::end_operation(
            tx,
            OperationEndRow {
                id: event.id,
                at: event.at,
                outcome: event.outcome,
                delivery: event.delivery,
                sent_at: event.sent_at,
                args: precise
                    .as_ref()
                    .map(|args| (&args.value, args.truncation == Truncation::Cut)),
                result: result
                    .as_ref()
                    .map(|result| (&result.value, result.truncation == Truncation::Cut)),
                error: error.as_ref(),
                page: page.as_ref(),
            },
        )
        .await?;
        if !matched {
            if let Some(run) = self.runs.get_mut(&event.id.run) {
                run.gaps += 1;
            }
            return Ok(None);
        }
        let session = open
            .map(|open| open.session)
            .or_else(|| self.runs.get(&event.id.run).map(|run| run.session));
        Ok(session.map(|session| TraceRecordEvent::OperationEnded {
            id: event.id,
            session,
            outcome: event.outcome,
            delivery: event.delivery,
            at: event.at,
        }))
    }

    async fn run_ended(
        &mut self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        event: RunEnded,
    ) -> Result<Option<TraceRecordEvent>, TraceError> {
        let open = self.runs.remove(&event.id);
        let gaps = self.dropped.take(event.id) + open.as_ref().map_or(0, |run| run.gaps);
        let result = event
            .result
            .as_ref()
            .map(|result| self.redactor.result(result));
        let error = event.error.map(|mut error| {
            error.message = self.redactor.text(&error.message);
            error
        });
        let code = match open.as_ref() {
            Some(run) if run.sensitive => RunCode::Withheld("sensitive_input"),
            Some(OpenRun {
                code: Some(code), ..
            }) => RunCode::Stored {
                text: &code.value,
                truncated: code.truncation == Truncation::Cut,
            },
            _ => RunCode::Withheld("unavailable"),
        };
        let replay_window =
            i64::try_from(self.config.replay_window.as_millis()).unwrap_or(i64::MAX);
        let effects = trace_store::end_run(
            tx,
            RunEndRow {
                id: event.id,
                at: event.at,
                status: event.status,
                error: error.as_ref(),
                result: result.as_ref().map(|result| &result.value),
                result_truncated: result
                    .as_ref()
                    .is_some_and(|result| result.truncation == Truncation::Cut),
                code,
                replay: event.replay.as_ref(),
                replay_until: event
                    .replay
                    .as_ref()
                    .map(|_| event.at.plus_millis(replay_window)),
                gaps,
            },
        )
        .await?;
        let session = match open {
            Some(run) => Some(run.session),
            None => sqlx::query_scalar::<_, String>("SELECT session_id FROM runs WHERE id = ?")
                .bind(event.id.to_string())
                .fetch_optional(&mut **tx)
                .await?
                .and_then(|text| text.parse().ok()),
        };
        if let Some(session) = session {
            trace_store::touch_session(tx, session, None, event.at).await?;
        }
        Ok(session.map(|session| TraceRecordEvent::RunEnded {
            run: event.id,
            session,
            status: event.status,
            effects,
            at: event.at,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{WriterConfig, spawn_writer};
    use crate::{
        EpochMillis, RedactionLimits, Redactor, RunId, RunStarted, SecretSet, SecretSetConfig,
        SessionId, TraceError, TraceStore,
    };
    use std::sync::Arc;

    #[tokio::test]
    async fn flush_reports_a_batch_that_was_rolled_back() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
        let store = TraceStore::open(&dir.path().join("trace.db"))
            .await
            .unwrap_or_else(|error| panic!("open: {error}"));
        let sink = spawn_writer(
            &store,
            Redactor::new(
                Arc::new(SecretSet::new(SecretSetConfig::default())),
                RedactionLimits::default(),
            ),
            WriterConfig::default(),
        );
        // Break the next batch: its run insert fails and the transaction rolls back.
        let dropped = sqlx::query("DROP TABLE runs").execute(store.pool()).await;
        assert!(dropped.is_ok());
        sink.run_started(RunStarted {
            id: RunId::new(),
            session: SessionId::new(),
            code: String::new(),
            request_id: None,
            host_data: None,
            at: EpochMillis::from_millis(1),
        });
        let first = sink.flush().await;
        assert!(matches!(first, Err(TraceError::BatchLost(_))), "{first:?}");
        assert!(sink.flush().await.is_ok(), "the loss is reported once");
    }
}
