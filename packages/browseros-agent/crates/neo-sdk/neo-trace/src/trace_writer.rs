//! The one writer task, and the [`TraceSink`] the dispatcher talks to.
//!
//! The dispatcher's hooks are synchronous and must never wait on disk, so every event is a
//! channel send. The writer takes the first event, gathers more for up to `batch_window` (or
//! `batch_max` events), bounds their values, and commits them in one transaction. After the commit it
//! answers sweep and flush commands in the batch, then publishes the committed events to
//! subscribers (the service API's SSE).
//!
//! Backpressure: if the disk stalls and the backlog passes `max_backlog`, operation events are
//! dropped and counted as the run's `trace_gaps` (which make its effects `unknown`). Run
//! lifecycle events, sweeps and flushes are never dropped.
//!
//! Run and operation events name the run's task, and operation events the target tab, so the
//! SSE route filters by either without a store read. The writer learns a run's task at its start
//! and keeps it with the open run until the run's end, which is never dropped, so the map
//! cannot grow without bound in a live process.
//!
//! Input visibility follows the record lifecycle: an operation's arguments and notes appear
//! with its end, and a run's code and host data appear with the run's end. Start rows expose
//! identity and state only. Readers of the rows and SSE events rely on this deferral; writing
//! input earlier would also change what interrupted or dropped-end records retain.
//!
//! Raw input stays in this task's memory while its record is open, and is bounded only when
//! the end materializes it. An operation whose end never arrives keeps no stored input or
//! notes; a run with no end keeps no stored code or host data. An operation's notes become
//! final with its end, so readers of `operation.ended` see the complete bounded list.
//!
//! Screenshots: the writer decides and accounts, and the camera only takes pictures. This task
//! is the one that sees each operation's start (effect, tab) and end (outcome, error owner) in
//! order, so it applies the rule (`screenshot_rule`), makes a `ShotJob` per screenshot, and
//! counts each run's unanswered jobs. It also sees each answer in order with the run's end, so
//! the rule knows which of a run's jobs are still out when the run ends: a tab with one out
//! gets no run-end shot of its own. A batch's jobs go to the camera only after the batch
//! commits, so no job names an operation whose end is not on disk. Each answer comes back on
//! the command queue and is stored in a later batch, so no JPEG I/O happens in a transaction.
//! An answer comes off its run's count only when its batch commits, and a batch that rolls back
//! hands its answers to the next batch: a lost answer would hold its run incomplete for good,
//! and one counted twice would complete its run while a job is still out.
//! `runs.complete` has this one writer: it is set in the batch that ends a run with nothing
//! pending, or in the batch that stores its last answer, and `RunComplete` follows the commit.
//! Computed anywhere else, it would race the end row: a reader could see "ended, nothing
//! pending" before the run-end jobs existed.

use crate::screenshot_rule::{self, TouchedTabs};
use crate::storage::{
    OperationEndRow, OperationStartRow, RunCode, RunEndRow, RunStartRow, TraceBatch,
};
use crate::trace_store::{self, RetentionPolicy, Swept};
use crate::{
    Delivery, Effect, EpochMillis, OperationEnded, OperationId, OperationReached, OperationStarted,
    Outcome, Redactor, RunEffects, RunEnded, RunId, RunStarted, RunStatus, Shot, ShotAnswer,
    ShotJob, ShotQueue, TaskId, TraceError, TraceStore, Truncation,
};
use browseros_core::TabId;
use serde_json::Value;
use std::collections::HashMap;
use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot};

/// Tuning for the writer. [`Default`] is the fixed default claw-sdk runs with, sized for one
/// browser's worth of agents.
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

/// A committed change, for live subscribers. Subscribers read details from the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceRecordEvent {
    /// A run was admitted.
    RunStarted {
        /// The run.
        run: RunId,
        /// Its task.
        task: Option<TaskId>,
        /// When.
        at: EpochMillis,
    },
    /// A run ended.
    RunEnded {
        /// The run.
        run: RunId,
        /// Its task.
        task: Option<TaskId>,
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
        /// Its run's task.
        task: Option<TaskId>,
        /// The tab it targets.
        tab: Option<TabId>,
        /// Read or write, so a subscriber can follow a tab's in-flight writes.
        effect: Effect,
        /// When.
        at: EpochMillis,
    },
    /// An operation ended.
    OperationEnded {
        /// The operation.
        id: OperationId,
        /// Its run's task.
        task: Option<TaskId>,
        /// The tab it targeted.
        tab: Option<TabId>,
        /// How it ended.
        outcome: Outcome,
        /// What happened to its side effect.
        delivery: Delivery,
        /// When.
        at: EpochMillis,
    },
    /// A run is complete: it ended and every screenshot job it made was answered, so a re-read
    /// returns the same data. `GET /v1/runs/{id}?wait=` wakes on it. Not an SSE event.
    RunComplete {
        /// The run.
        run: RunId,
    },
}

enum Event {
    RunStarted(RunStarted),
    RunEnded(RunEnded),
    OperationStarted(OperationStarted),
    OperationNotes { id: OperationId, notes: Vec<String> },
    OperationEnded(OperationEnded),
}

enum Command {
    Event(Box<Event>),
    Sweep {
        policy: RetentionPolicy,
        now: EpochMillis,
        reply: oneshot::Sender<Result<Swept, TraceError>>,
    },
    Flush(oneshot::Sender<Result<(), TraceError>>),
    /// The camera's answer to one job. Never dropped: a lost answer would
    /// hold its run incomplete.
    Screenshot {
        op: OperationId,
        shot: Shot,
    },
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

    /// Whether any run that has not ended yet lost an operation event.
    fn any(&self) -> bool {
        !self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
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
    Sweep {
        policy: RetentionPolicy,
        now: EpochMillis,
    },
    Screenshot {
        op: OperationId,
        shot: Shot,
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

    /// Call-log lines for an open operation, oldest first; each call adds to the lines before
    /// it. The writer holds them with the operation and its [`op_ended`](Self::op_ended) stores
    /// them, so send them first: lines that arrive after the end are dropped, and an
    /// operation's notes never change once it has ended. Dropped too when the writer is far
    /// behind, and not counted as a gap: a note records no effect, so losing one cannot hide a
    /// write.
    pub fn op_notes(&self, id: OperationId, notes: Vec<String>) {
        if notes.is_empty() || self.inner.backlog.load(Ordering::Relaxed) >= self.inner.max_backlog
        {
            return;
        }
        self.send(Event::OperationNotes { id, notes });
    }

    /// An operation ended. Dropped (and counted) when the writer is far behind.
    pub fn op_ended(&self, event: OperationEnded) {
        if self.over_backlog(event.id.run) {
            return;
        }
        self.send(Event::OperationEnded(event));
    }

    /// The operation's script reached `tabs` besides its target, as a popup's main-world script
    /// reaches its opener through `window.opener`. A takeover lock's settle is to count the
    /// operation as a write in flight on each reached tab. Not recorded yet: the writer stores
    /// reaches from the next commit on.
    pub fn op_reached(&self, event: OperationReached) {
        let _ = event;
    }

    /// Committed changes from now on.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<TraceRecordEvent> {
        self.inner.events.subscribe()
    }

    /// Whether an operation event of a run that has not ended was dropped over the backlog cap.
    /// While it is true the store cannot rule out an operation it never heard of, so a reader
    /// that needs "nothing is in flight" (the takeover lock's settle) must not conclude it.
    /// A run's drops clear when its end is recorded; batches lost to a failed commit are
    /// reported by [`TraceSink::flush`] instead.
    #[must_use]
    pub fn lost_operation_events(&self) -> bool {
        self.inner.dropped.any()
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

    /// A screenshot job's answer, from [`ShotAnswer`]. It goes through the same queue and
    /// backlog count as every command, so the writer's count stays balanced.
    pub(crate) fn screenshot_answered(&self, op: OperationId, shot: Shot) {
        if self.command(Command::Screenshot { op, shot }).is_err() {
            tracing::warn!(%op, "trace writer stopped; screenshot answer lost");
        }
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

/// Starts the writer on `store`. It runs until every [`TraceSink`] clone, and every screenshot
/// job's answer, is dropped.
///
/// `screenshots` is the camera's queue (`screenshot_queue`). With `None` there is no camera: no
/// job is made, and a run is complete when it ends.
#[must_use]
pub fn spawn_writer(
    store: &TraceStore,
    redactor: Redactor,
    config: WriterConfig,
    screenshots: Option<ShotQueue>,
) -> TraceSink {
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
        shots: screenshots,
        finishing: HashMap::new(),
        planned: Vec::new(),
        answering: HashMap::new(),
        unstored: Vec::new(),
    };
    tokio::spawn(writer.run(rx));
    sink
}

/// What the writer remembers about a run while it is open.
struct OpenRun {
    /// Named by every event the run publishes until its end.
    task: Option<TaskId>,
    /// The code as given, held in memory until the run ends, when it is bounded.
    /// `None` when the run's start was never seen.
    code: Option<String>,
    /// The host's data as given, held in memory until the run ends, like the code.
    host_data: Option<Value>,
    gaps: u64,
    operations: HashMap<u32, OpenOperation>,
    /// Each tab's last operation and the jobs still out, for the run-end screenshots.
    tabs: TouchedTabs,
    /// Screenshot jobs made for the run and not answered yet.
    pending: u32,
}

impl OpenRun {
    /// A run the writer has seen no start of: no code and no host data to store.
    fn unseen(task: Option<TaskId>) -> Self {
        Self::new(task, None, None)
    }

    fn new(task: Option<TaskId>, code: Option<String>, host_data: Option<Value>) -> Self {
        Self {
            task,
            code,
            host_data,
            gaps: 0,
            operations: HashMap::new(),
            tabs: TouchedTabs::default(),
            pending: 0,
        }
    }
}

struct OpenOperation {
    tab: Option<TabId>,
    /// The operation opens a tab (`trace_store::OPENERS`), whose end names it.
    opens: bool,
    effect: Effect,
    args: Value,
    /// Raw, like `args`: bounded only when the end stores them.
    notes: Vec<String>,
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
    /// The camera's queue; `None` without a camera.
    shots: Option<ShotQueue>,
    /// Runs that ended with screenshot jobs unanswered, and how many. A run leaves once the
    /// batch storing its last answer commits.
    finishing: HashMap<RunId, u32>,
    /// The current batch's jobs. They reach the camera once the batch commits; a batch that
    /// rolls back drops them, and each answers `Dropped`.
    planned: Vec<ShotJob>,
    /// The current batch's answers, per run. They come off `finishing` and `pending` once the
    /// batch commits, never before: a batch that rolls back hands its answers to the next one,
    /// which counts them again.
    answering: HashMap<RunId, u32>,
    /// Answers that a rolled-back batch did not store, oldest first. The next batch stores them
    /// ahead of its own commands.
    unstored: Vec<(OperationId, Shot)>,
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
        // callers would see `Closed` instead of the batch's real outcome. Answers are set aside
        // the same way, so a rollback hands all of them on. The ones an earlier rollback handed
        // on go first: they were queued before this batch.
        let mut answers = mem::take(&mut self.unstored);
        let mut work = Vec::with_capacity(answers.len() + batch.len());
        work.extend(
            answers
                .iter()
                .map(|&(op, shot)| Work::Screenshot { op, shot }),
        );
        let mut sweep_replies = Vec::new();
        let mut flushes = Vec::new();
        for command in batch {
            match command {
                Command::Event(event) => work.push(Work::Event(event)),
                Command::Sweep { policy, now, reply } => {
                    work.push(Work::Sweep { policy, now });
                    sweep_replies.push(reply);
                }
                Command::Flush(reply) => flushes.push(reply),
                Command::Screenshot { op, shot } => {
                    work.push(Work::Screenshot { op, shot });
                    answers.push((op, shot));
                }
            }
        }
        let mut published = Vec::new();
        let mut swept = Vec::new();
        let result = async {
            let tx = TraceBatch::begin(self.store.db()).await?;
            for item in work {
                match item {
                    Work::Event(event) => self.apply(&tx, *event, &mut published).await?,
                    Work::Sweep { policy, now } => {
                        let files = self.store.files().clone();
                        let outputs = self.store.outputs().clone();
                        swept.push(tx.sweep(&files, &outputs, policy, now).await?);
                    }
                    Work::Screenshot { op, shot } => {
                        tx.store_screenshot(op, shot).await?;
                        // Only run-end shots read this, so it need not wait for the commit: an
                        // answer that a rollback hands on is taken off again, which is a no-op.
                        if let Some(run) = self.runs.get_mut(&op.run) {
                            run.tabs.answered(op.seq);
                        }
                        if self.answered(op.run) && tx.complete_run(op.run).await? {
                            published.push(TraceRecordEvent::RunComplete { run: op.run });
                        }
                    }
                }
            }
            tx.commit().await?;
            Ok::<(), TraceError>(())
        }
        .await;
        // Jobs leave only once their operations' rows are on disk. A rolled-back batch drops
        // its jobs, and each dropped answer reports `Dropped` through the queue, so its run
        // still completes.
        let planned = mem::take(&mut self.planned);
        let answering = mem::take(&mut self.answering);
        match result {
            Ok(()) => {
                self.settle(answering);
                if let Some(shots) = &self.shots {
                    for job in planned {
                        shots.send(job);
                    }
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
                drop(planned);
                for reply in sweep_replies {
                    let _ = reply.send(Err(TraceError::BatchLost(error.to_string())));
                }
                for run in self.runs.values_mut() {
                    run.gaps += 1;
                }
                self.unstored = answers;
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
        tx: &TraceBatch,
        event: Event,
        published: &mut Vec<TraceRecordEvent>,
    ) -> Result<(), TraceError> {
        let change = match event {
            Event::RunStarted(event) => {
                tx.insert_run(RunStartRow {
                    id: event.id,
                    task: event.task_id.as_ref(),
                    at: event.at,
                    request_id: event.request_id.as_ref(),
                })
                .await?;
                self.runs.insert(
                    event.id,
                    OpenRun::new(event.task_id.clone(), Some(event.code), event.host_data),
                );
                Some(TraceRecordEvent::RunStarted {
                    run: event.id,
                    task: event.task_id,
                    at: event.at,
                })
            }
            Event::OperationStarted(event) => self.operation_started(tx, event).await?,
            Event::OperationNotes { id, notes } => {
                if let Some(open) = self
                    .runs
                    .get_mut(&id.run)
                    .and_then(|run| run.operations.get_mut(&id.seq))
                {
                    open.notes.extend(notes);
                }
                None
            }
            Event::OperationEnded(event) => self.operation_ended(tx, event).await?,
            Event::RunEnded(event) => return self.run_ended(tx, event, published).await,
        };
        published.extend(change);
        Ok(())
    }

    async fn operation_started(
        &mut self,
        tx: &TraceBatch,
        event: OperationStarted,
    ) -> Result<Option<TraceRecordEvent>, TraceError> {
        if !self.runs.contains_key(&event.id.run) {
            // Its run's start was never seen (a failed batch): keep the operation reachable.
            tx.insert_run(RunStartRow {
                id: event.id.run,
                task: None,
                at: event.at,
                request_id: None,
            })
            .await?;
        }
        // Names identify running operations immediately; input still waits for the end.
        let name = self.redactor.text(&event.name);
        tx.insert_operation(OperationStartRow {
            site: event.site,
            id: event.id,
            parent: event.parent,
            name: &name,
            kind: event.kind,
            effect: event.effect,
            tab: event.tab,
            at: event.at,
        })
        .await?;
        let run = self
            .runs
            .entry(event.id.run)
            .or_insert_with(|| OpenRun::unseen(None));
        if let Some(tab) = event.tab {
            run.tabs.admitted(tab, event.id.seq);
        }
        run.operations.insert(
            event.id.seq,
            OpenOperation {
                tab: event.tab,
                opens: trace_store::OPENERS.contains(&event.name.as_str()),
                effect: event.effect,
                args: event.args,
                notes: Vec::new(),
            },
        );
        Ok(Some(TraceRecordEvent::OperationStarted {
            id: event.id,
            task: run.task.clone(),
            tab: event.tab,
            effect: event.effect,
            at: event.at,
        }))
    }

    async fn operation_ended(
        &mut self,
        tx: &TraceBatch,
        event: OperationEnded,
    ) -> Result<Option<TraceRecordEvent>, TraceError> {
        let mut open = self
            .runs
            .get_mut(&event.id.run)
            .and_then(|run| run.operations.remove(&event.id.seq));
        let redactor = self.redactor.clone();
        let precise = open.as_ref().map(|open| redactor.args(&open.args));
        let notes = open
            .as_mut()
            .map(|open| redactor.notes(mem::take(&mut open.notes)))
            .unwrap_or_default();
        let result = event.result.as_ref().map(|result| redactor.result(result));
        let error = event.error.map(|mut error| {
            error.message = redactor.text(&error.message);
            error
        });
        let page = event.page_after.map(|mut page| {
            page.url = page.url.map(|url| redactor.text(&url));
            page.title = page.title.map(|title| redactor.text(&title));
            page
        });
        let matched = tx
            .end_operation(OperationEndRow {
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
                notes: &notes,
                page: page.as_ref(),
                main_world: event.main_world,
            })
            .await?;
        if !matched {
            if let Some(run) = self.runs.get_mut(&event.id.run) {
                run.gaps += 1;
            }
            return Ok(None);
        }
        // The tab an opener opened is the run's: it had no target to link at the start.
        if let (true, Some(tab)) = (
            open.as_ref().is_some_and(|open| open.opens),
            page.as_ref().and_then(|page| page.tab_id),
        ) {
            tx.link_tab(event.id.run, tab).await?;
        }
        let shot_tab = open.as_ref().and_then(|open| {
            screenshot_rule::after_op(open.effect, open.tab, event.outcome, error.as_ref())
        });
        // Only while its run is open: a job for an ended run would hold it incomplete again. An
        // operation that starts after its run ended reopens the run here, so `finishing` is
        // checked too.
        if let Some(tab) = shot_tab
            && self.runs.contains_key(&event.id.run)
            && !self.finishing.contains_key(&event.id.run)
            && self.plan_shot(event.id, tab)
            && let Some(run) = self.runs.get_mut(&event.id.run)
        {
            run.pending += 1;
            run.tabs.shot(tab, event.id.seq);
        }
        Ok(Some(TraceRecordEvent::OperationEnded {
            id: event.id,
            task: self
                .runs
                .get(&event.id.run)
                .and_then(|run| run.task.clone()),
            tab: open.and_then(|open| open.tab),
            outcome: event.outcome,
            delivery: event.delivery,
            at: event.at,
        }))
    }

    async fn run_ended(
        &mut self,
        tx: &TraceBatch,
        event: RunEnded,
        published: &mut Vec<TraceRecordEvent>,
    ) -> Result<(), TraceError> {
        let open = self.runs.remove(&event.id);
        let gaps = self.dropped.take(event.id) + open.as_ref().map_or(0, |run| run.gaps);
        let mut pending = open.as_ref().map_or(0, |run| run.pending);
        for (tab, seq) in open
            .as_ref()
            .map(|run| run.tabs.run_end())
            .unwrap_or_default()
        {
            if self.plan_shot(OperationId::new(event.id, seq), tab) {
                pending += 1;
            }
        }
        // Answers earlier in this batch count toward the end, though they come off the run's
        // count only when the batch commits.
        let complete = pending <= self.answering.get(&event.id).copied().unwrap_or(0);
        let redactor = self.redactor.clone();
        let result = event.result.as_ref().map(|result| redactor.result(result));
        let error = event.error.map(|mut error| {
            error.message = redactor.text(&error.message);
            error
        });
        let bounded = match open.as_ref() {
            Some(run) => run.code.as_deref().map(|code| redactor.code(code)),
            _ => None,
        };
        let host_data = open.as_ref().and_then(|run| {
            let data = run.host_data.as_ref()?;
            Some(redactor.host_data(data).value)
        });
        let code = match bounded.as_ref() {
            Some(code) => RunCode::Stored {
                text: &code.value,
                truncated: code.truncation == Truncation::Cut,
            },
            _ => RunCode::Unavailable,
        };
        let replay_window =
            i64::try_from(self.config.replay_window.as_millis()).unwrap_or(i64::MAX);
        let effects = tx
            .end_run(RunEndRow {
                id: event.id,
                at: event.at,
                status: event.status,
                error: error.as_ref(),
                result: result.as_ref().map(|result| &result.value),
                result_truncated: result
                    .as_ref()
                    .is_some_and(|result| result.truncation == Truncation::Cut),
                code,
                host_data: host_data.as_ref(),
                replay: event.replay.as_ref(),
                replay_until: event
                    .replay
                    .as_ref()
                    .map(|_| event.at.plus_millis(replay_window)),
                gaps,
                complete,
            })
            .await?;
        if pending > 0 {
            self.finishing.insert(event.id, pending);
        }
        // A run the writer never saw start still names its stored task; a run with no row at
        // all publishes nothing.
        let task = match open {
            Some(run) => Some(run.task),
            None => tx.run_task(event.id).await?,
        };
        if let Some(task) = task {
            published.push(TraceRecordEvent::RunEnded {
                run: event.id,
                task,
                status: event.status,
                effects,
                at: event.at,
            });
            if complete {
                published.push(TraceRecordEvent::RunComplete { run: event.id });
            }
        }
        Ok(())
    }

    /// Makes a job for `op` on `tab`, which reaches the camera once this batch commits. False
    /// without a camera, or once no sink is left to carry the answer back (the writer is
    /// draining after its last sink was dropped).
    fn plan_shot(&mut self, op: OperationId, tab: TabId) -> bool {
        let (Some(_), Some(inner)) = (&self.shots, self.sink.upgrade()) else {
            return false;
        };
        self.planned.push(ShotJob {
            op,
            tab,
            path: self.store.files().path(op),
            answer: ShotAnswer::new(op, TraceSink { inner }),
        });
        true
    }

    /// Counts one answered job of `run` in this batch. True when it answers the last job of a
    /// run that has ended, which this batch then completes. An ended run's jobs are counted in
    /// `finishing` even when a late operation has reopened the run in `runs`.
    fn answered(&mut self, run: RunId) -> bool {
        let answered = self.answering.entry(run).or_default();
        *answered += 1;
        self.finishing
            .get(&run)
            .is_some_and(|pending| *pending <= *answered)
    }

    /// Takes a committed batch's answers off its runs' counts. A run left with no job out
    /// leaves `finishing`.
    fn settle(&mut self, answering: HashMap<RunId, u32>) {
        for (run, answered) in answering {
            if let Some(pending) = self.finishing.get_mut(&run) {
                *pending = pending.saturating_sub(answered);
                if *pending == 0 {
                    self.finishing.remove(&run);
                }
            } else if let Some(open) = self.runs.get_mut(&run) {
                open.pending = open.pending.saturating_sub(answered);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TraceRecordEvent, TraceSink, WriterConfig, spawn_writer};
    use crate::{
        Delivery, Effect, EpochMillis, OperationEnded, OperationId, OperationKind,
        OperationStarted, Outcome, RedactionLimits, Redactor, RetentionPolicy, RunEnded, RunId,
        RunStarted, RunStatus, Shot, ShotRead, TraceError, TraceStore, screenshot_queue,
    };
    use browseros_core::TabId;
    use std::time::Duration;

    async fn writer() -> (tempfile::TempDir, TraceStore, TraceSink) {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
        let store = TraceStore::open(&dir.path().join("trace.db"))
            .await
            .unwrap_or_else(|error| panic!("open: {error}"));
        let sink = spawn_writer(
            &store,
            Redactor::new(RedactionLimits::default()),
            WriterConfig::default(),
            None,
        );
        (dir, store, sink)
    }

    /// The invariant behind "swept with its runs": no tab link outlives its run, or `run_tabs`
    /// would grow for as long as the cell runs.
    #[tokio::test]
    async fn a_sweep_leaves_no_tab_link_without_its_run() {
        let (_dir, store, sink) = writer().await;
        let day = 24 * 60 * 60 * 1000;
        for (started, ended) in [(0, Some(10)), (8 * day, None)] {
            let run = RunId::new();
            sink.run_started(RunStarted {
                id: run,
                code: String::new(),
                request_id: None,
                task_id: None,
                host_data: None,
                at: EpochMillis::from_millis(started),
            });
            sink.op_started(OperationStarted {
                site: None,
                id: OperationId::new(run, 1),
                parent: None,
                name: "page.title".into(),
                kind: OperationKind::Builtin,
                effect: Effect::Read,
                tab: Some(TabId(3)),
                args: serde_json::Value::Null,
                at: EpochMillis::from_millis(started),
            });
            if let Some(ended) = ended {
                sink.run_ended(RunEnded {
                    id: run,
                    status: RunStatus::Ok,
                    error: None,
                    result: None,
                    replay: None,
                    at: EpochMillis::from_millis(ended),
                });
            }
        }
        let swept = sink
            .sweep(
                RetentionPolicy::default(),
                EpochMillis::from_millis(8 * day + 1),
            )
            .await;
        assert_eq!(swept.ok().map(|swept| swept.runs), Some(1));
        let (orphans, links) = crate::storage::test_faults::tab_link_counts(store.db())
            .await
            .unwrap_or_else(|error| panic!("tab links: {error}"));
        assert_eq!(orphans, 0);
        assert_eq!(links, 1, "the running run keeps its link");
    }

    /// A run's kept downloads live exactly as long as its record.
    #[tokio::test]
    async fn a_swept_runs_kept_downloads_go_with_it() {
        let (dir, store, sink) = writer().await;
        let day = 24 * 60 * 60 * 1000;
        let source = dir.path().join("statement.txt");
        tokio::fs::write(&source, b"balance")
            .await
            .unwrap_or_else(|error| panic!("source: {error}"));
        let (aged, recent) = (RunId::new(), RunId::new());
        for (run, started) in [(aged, 0), (recent, 8 * day)] {
            sink.run_started(RunStarted {
                id: run,
                code: String::new(),
                request_id: None,
                task_id: None,
                host_data: None,
                at: EpochMillis::from_millis(started),
            });
            sink.run_ended(RunEnded {
                id: run,
                status: RunStatus::Ok,
                error: None,
                result: None,
                replay: None,
                at: EpochMillis::from_millis(started + 10),
            });
            store
                .outputs()
                .keep_download(run, 0, &source)
                .await
                .unwrap_or_else(|error| panic!("keep: {error}"));
        }
        let swept = sink
            .sweep(
                RetentionPolicy::default(),
                EpochMillis::from_millis(8 * day + 1),
            )
            .await;
        assert_eq!(swept.ok().map(|swept| swept.runs), Some(1));
        for (run, kept) in [(aged, false), (recent, true)] {
            let found = store
                .outputs()
                .open_download(run, 0)
                .await
                .unwrap_or_else(|error| panic!("open: {error}"));
            assert_eq!(
                found.is_some(),
                kept,
                "only the recent run keeps its download"
            );
        }
    }

    /// Jobs leave only once their batch commits; a rolled-back batch's jobs answer `Dropped`,
    /// so the run still completes.
    #[tokio::test]
    async fn a_rolled_back_batch_sends_no_job_and_its_run_still_completes() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
        let store = TraceStore::open(&dir.path().join("trace.db"))
            .await
            .unwrap_or_else(|error| panic!("open: {error}"));
        let (queue, mut jobs) = screenshot_queue();
        let sink = spawn_writer(
            &store,
            Redactor::new(RedactionLimits::default()),
            WriterConfig::default(),
            Some(queue),
        );
        let mut events = sink.subscribe();
        let run = RunId::new();
        sink.run_started(RunStarted {
            id: run,
            code: String::new(),
            request_id: None,
            task_id: None,
            host_data: None,
            at: EpochMillis::from_millis(1),
        });
        assert!(sink.flush().await.is_ok());
        // Break the next batch: the click's tab link fails and the transaction rolls back.
        let dropped = crate::storage::test_faults::break_table(store.db(), "run_tabs").await;
        assert!(dropped.is_ok());
        let click = OperationId::new(run, 1);
        sink.op_started(OperationStarted {
            site: None,
            id: click,
            parent: None,
            name: "locator.click".into(),
            kind: OperationKind::Builtin,
            effect: Effect::Write,
            tab: Some(TabId(3)),
            args: serde_json::Value::Null,
            at: EpochMillis::from_millis(2),
        });
        sink.op_ended(OperationEnded {
            id: click,
            outcome: Outcome::Ok,
            delivery: Delivery::Done,
            sent_at: None,
            result: None,
            error: None,
            page_after: None,
            main_world: false,
            at: EpochMillis::from_millis(3),
        });
        let lost = sink.flush().await;
        assert!(matches!(lost, Err(TraceError::BatchLost(_))), "{lost:?}");
        assert!(
            jobs.try_recv().is_none(),
            "a job for a rolled-back operation reached the camera"
        );
        sink.run_ended(RunEnded {
            id: run,
            status: RunStatus::Ok,
            error: None,
            result: None,
            replay: None,
            at: EpochMillis::from_millis(4),
        });
        let complete = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match events.recv().await {
                    Ok(TraceRecordEvent::RunComplete { run: done }) if done == run => break true,
                    Ok(_) => {}
                    Err(_) => break false,
                }
            }
        })
        .await;
        assert_eq!(complete.ok(), Some(true));
        assert!(jobs.try_recv().is_none());
    }

    /// An answer whose batch rolls back is stored by the next batch and counted once: its run
    /// neither waits for it forever nor completes while its other job is still out.
    #[tokio::test]
    async fn an_answer_in_a_rolled_back_batch_is_stored_by_the_next_one()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
        let store = TraceStore::open(&dir.path().join("trace.db"))
            .await
            .unwrap_or_else(|error| panic!("open: {error}"));
        let (queue, mut jobs) = screenshot_queue();
        let sink = spawn_writer(
            &store,
            Redactor::new(RedactionLimits::default()),
            WriterConfig::default(),
            Some(queue),
        );
        let mut events = sink.subscribe();
        let run = RunId::new();
        sink.run_started(RunStarted {
            id: run,
            code: String::new(),
            request_id: None,
            task_id: None,
            host_data: None,
            at: EpochMillis::from_millis(1),
        });
        let (first, second) = (OperationId::new(run, 1), OperationId::new(run, 2));
        for click in [first, second] {
            sink.op_started(OperationStarted {
                site: None,
                id: click,
                parent: None,
                name: "locator.click".into(),
                kind: OperationKind::Builtin,
                effect: Effect::Write,
                tab: Some(TabId(3)),
                args: serde_json::Value::Null,
                at: EpochMillis::from_millis(2),
            });
            sink.op_ended(OperationEnded {
                id: click,
                outcome: Outcome::Ok,
                delivery: Delivery::Done,
                sent_at: None,
                result: None,
                error: None,
                page_after: None,
                main_world: false,
                at: EpochMillis::from_millis(3),
            });
        }
        sink.run_ended(RunEnded {
            id: run,
            status: RunStatus::Ok,
            error: None,
            result: None,
            replay: None,
            at: EpochMillis::from_millis(4),
        });
        assert!(sink.flush().await.is_ok());
        let (Some(first_job), Some(second_job)) = (jobs.try_recv(), jobs.try_recv()) else {
            panic!("a job per write expected");
        };
        let jpeg = b"\xff\xd8 retry \xff\xd9";
        for job in [&first_job, &second_job] {
            let parent = job.path.parent().ok_or("screenshot path without parent")?;
            tokio::fs::create_dir_all(parent).await?;
            tokio::fs::write(&job.path, jpeg).await?;
        }
        let complete = || {
            let store = store.clone();
            async move {
                store
                    .run_summary(run)
                    .await
                    .ok()
                    .flatten()
                    .map(|run| run.complete)
            }
        };
        // Fail an operation insert beside the answer, rolling back both in one transaction.
        // The next batch must retry the consumed answer without completing the other job.
        assert!(
            crate::storage::test_faults::reject_test_operation(store.db(), true)
                .await
                .is_ok()
        );
        first_job.answer.send(Shot::Taken {
            bytes_len: jpeg.len() as u64,
        });
        sink.op_started(OperationStarted {
            site: None,
            id: OperationId::new(RunId::new(), 1),
            parent: None,
            name: "test.fail".into(),
            kind: OperationKind::Builtin,
            effect: Effect::Read,
            tab: None,
            args: serde_json::Value::Null,
            at: EpochMillis::from_millis(5),
        });
        let lost = sink.flush().await;
        assert!(matches!(lost, Err(TraceError::BatchLost(_))), "{lost:?}");
        assert!(
            crate::storage::test_faults::reject_test_operation(store.db(), false)
                .await
                .is_ok()
        );

        let _ = sink.flush().await;
        assert_eq!(
            store.screenshot(first).await.ok(),
            Some(ShotRead::Jpeg(jpeg.to_vec())),
            "the next batch stored the answer"
        );
        assert_eq!(complete().await, Some(false), "the second job is still out");

        second_job.answer.send(Shot::Taken {
            bytes_len: jpeg.len() as u64,
        });
        let announced = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match events.recv().await {
                    Ok(TraceRecordEvent::RunComplete { run: done }) if done == run => break true,
                    Ok(_) => {}
                    Err(_) => break false,
                }
            }
        })
        .await;
        assert_eq!(announced.ok(), Some(true));
        assert_eq!(complete().await, Some(true));
        assert_eq!(
            store.screenshot(second).await.ok(),
            Some(ShotRead::Jpeg(jpeg.to_vec()))
        );
        Ok(())
    }

    #[tokio::test]
    async fn flush_reports_a_batch_that_was_rolled_back() {
        let (_dir, store, sink) = writer().await;
        // Break the next batch: its run insert fails and the transaction rolls back.
        let dropped = crate::storage::test_faults::break_table(store.db(), "runs").await;
        assert!(dropped.is_ok());
        sink.run_started(RunStarted {
            id: RunId::new(),
            code: String::new(),
            request_id: None,
            task_id: None,
            host_data: None,
            at: EpochMillis::from_millis(1),
        });
        let first = sink.flush().await;
        assert!(matches!(first, Err(TraceError::BatchLost(_))), "{first:?}");
        assert!(sink.flush().await.is_ok(), "the loss is reported once");
    }
}
