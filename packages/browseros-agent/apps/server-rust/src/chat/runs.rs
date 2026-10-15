//! Port of `conversation-runs.ts`: one run record per conversation, registered before any await
//! (the 409 gate), a replay log plus live tail for `/chat/{id}/stream`, the "prepared" gate
//! `/state` waits on, run status, and the side-panel projection (`/chat/panels`).
//!
//! ```text
//! start ─ register (sync, 409) ─ spawn drive ──► prepare(RunHandle) ── Ok ─► pump ─► on_finish ─► finish
//!            │ seeded with the previous            │ (history published,          │  (persist,       │ final messages +
//!            │ record's messages                   │  engine prepared)            │   final history) │ status (one lock),
//!            ├ panels attached                     └ Err ──────────────────────────┼─────────────────► latches, panels
//!            └ the starter's cursor ─────────────────────────────────────────────┐ │ chunks appended
//!   subscribe ─► cursor over the one replay log (replay + tail, no gap) ◄────────┴─┘
//! ```
//!
//! Lifecycle rules a reader must not break:
//! - The run is driven by a spawned task, never by the HTTP request: a client disconnect (which
//!   drops the handler future) only drops a subscriber cursor. Only `stop`/`delete` cancel.
//! - `on_finish` (persistence) runs after the last chunk and before subscribers see the end, so a
//!   client that reloads right after `[DONE]` finds the saved conversation. Its final history is
//!   published by `finish` together with the terminal status: `/state` never pairs `running` with
//!   the finished assistant message (a resuming panel would replay the turn onto it twice).
//! - A record leaves the registry only through `delete_with`, and only after its run ended and
//!   the caller's cleanup ran; until then a POST for the conversation is 409.
//! - Lock order is `RunsState` → `RunInner`; never acquire the registry lock while holding a
//!   record's lock. Neither lock is held across an await.

use super::{PanelStream, Resume, engine::ChatRejection};
use crate::activity::ServerActivity;
use browseros_agent::llm::{ChatError, ChatErrorCode};
use futures_util::{FutureExt as _, StreamExt as _, future::BoxFuture};
use server_api::{
    ConversationPanelAssignment, ConversationPanelAssignments, ConversationRunStatus,
};
use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex, MutexGuard, PoisonError, Weak},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
use ui_message_stream::{ChunkStream, UiChunk, UiMessage, UiMessageAssembler};
use uuid::Uuid;

pub struct RunPresentation {
    pub panel_tab_ids: Vec<u32>,
    pub panels_visible: bool,
    /// Keep the replay log after the run ended and its last subscriber left. False for scheduled
    /// runs: their one viewer is the POST that started them, and each uses a fresh conversation
    /// id nobody DELETEs, so a retained log would live until restart.
    pub keep_replay: bool,
}

/// What an engine holds of its run: id, cancellation, and the hook that publishes the display
/// history (so `/state` shows the user message before the first chunk).
#[derive(Clone)]
pub struct RunHandle {
    run_id: Uuid,
    cancel: CancellationToken,
    /// The registry and conversation the handle publishes into; `None` for a detached handle.
    owner: Option<(Weak<ConversationRuns>, Uuid)>,
}

impl RunHandle {
    /// A handle bound to no registry (engine unit tests): `publish_messages` is a no-op.
    pub fn new(run_id: Uuid, cancel: CancellationToken) -> Self {
        Self {
            run_id,
            cancel,
            owner: None,
        }
    }

    pub fn run_id(&self) -> Uuid {
        self.run_id
    }

    pub fn cancel(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Replaces the run's snapshot messages. Returns false once the run has been replaced or
    /// removed (`updateMessages`), so a late publish never leaks into a newer turn.
    pub fn publish_messages(&self, messages: Vec<UiMessage>) -> bool {
        let Some((runs, conversation_id)) = &self.owner else {
            return false;
        };
        runs.upgrade()
            .is_some_and(|runs| runs.update_messages(*conversation_id, self.run_id, messages))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    Completed,
    Aborted,
    Failed,
}

impl RunStatus {
    pub fn as_wire(self) -> ConversationRunStatus {
        match self {
            Self::Running => ConversationRunStatus::Running,
            Self::Completed => ConversationRunStatus::Completed,
            Self::Aborted => ConversationRunStatus::Aborted,
            Self::Failed => ConversationRunStatus::Failed,
        }
    }
}

/// How a prepared turn ended, handed to its `on_finish`.
pub struct FinishedTurn {
    pub status: RunStatus,
    /// The turn's chunks folded into one assistant message (`None` before a `start` chunk).
    pub assistant: Option<UiMessage>,
}

/// Final display history and completion failure, returned after persistence and cleanup.
/// Keeping them together lets the registry publish the reply and failed status atomically,
/// even when the model succeeded but the durable save did not.
pub struct FinishOutcome {
    pub messages: Option<Vec<UiMessage>>,
    pub error: Option<ChatError>,
}

impl From<Option<Vec<UiMessage>>> for FinishOutcome {
    fn from(messages: Option<Vec<UiMessage>>) -> Self {
        Self {
            messages,
            error: None,
        }
    }
}

/// Persistence hook; runs once per prepared run before subscribers see the end. A failure
/// becomes an error chunk followed by a failed terminal state; final messages are still kept.
pub type OnFinish = Box<dyn FnOnce(FinishedTurn) -> BoxFuture<'static, FinishOutcome> + Send>;

pub struct Prepared {
    pub chunks: ChunkStream,
    pub on_finish: OnFinish,
}

/// A registered, prepared run. `chunks` is a subscriber cursor on this run's own record, opened at
/// registration: the POST that started a run streams exactly that run, from its first chunk, even
/// if the run ends and a newer turn (or a DELETE) replaces the conversation's record first.
pub struct StartedRun {
    pub run_id: Uuid,
    pub chunks: ChunkStream,
}

#[derive(Debug, Clone)]
pub struct RunSnapshot {
    pub conversation_id: Uuid,
    pub run_id: Uuid,
    pub status: RunStatus,
    pub messages: Vec<UiMessage>,
    pub chunk_count: usize,
}

/// The MCP run guard's view of a run (tool calls need an active run for the lease's conversation).
/// Pinned to one run: `associate_tabs` refuses once that run is replaced.
#[derive(Debug, Clone)]
pub struct ActiveRun {
    pub conversation_id: Uuid,
    pub run_id: Uuid,
    pub panels_visible: bool,
}

pub struct ConversationRuns {
    activity: Arc<ServerActivity>,
    heartbeat: Duration,
    state: Mutex<RunsState>,
}

#[derive(Default)]
struct RunsState {
    runs: HashMap<Uuid, Arc<RunRecord>>,
    /// tab id → the run its side panel shows. A `BTreeMap` so assignments come out sorted by tab.
    panels: BTreeMap<u32, Arc<RunRecord>>,
    /// tab id → creation event id; only tabs a run's tools created ask the panel to open.
    auto_open: HashMap<u32, String>,
    panel_subscribers: HashMap<u64, mpsc::UnboundedSender<ConversationPanelAssignments>>,
    next_panel_subscriber: u64,
}

struct RunRecord {
    conversation_id: Uuid,
    run_id: Uuid,
    panels_visible: bool,
    keep_replay: bool,
    cancel: CancellationToken,
    /// Latch released once `prepare` returned (or the run ended): what `/state` waits on.
    prepared: CancellationToken,
    /// Latch released when the run has fully ended, `on_finish` included.
    finished: CancellationToken,
    /// Bumped after every append and at the end; subscribers wait on it, then re-read the log.
    log_version: watch::Sender<u64>,
    inner: Mutex<RunInner>,
}

struct RunInner {
    status: RunStatus,
    messages: Vec<UiMessage>,
    /// The replay log: every chunk of the run, in order. Never truncated while the record lives,
    /// except that a `keep_replay: false` run drops it whole once it ended and has no subscriber.
    chunks: Vec<UiChunk>,
    /// Live cursors over `chunks`. Counted under this lock (not via the watch channel's receiver
    /// count) so two cursors detaching at once cannot both miss the release.
    subscribers: usize,
    saw_error_chunk: bool,
    ended: bool,
    deleting: bool,
    /// `prepare` has returned a stream (TS: `record.reader` is set).
    streaming: bool,
}

impl RunInner {
    /// Drops the replay log of an ended, unwatched `keep_replay: false` run.
    fn release_unwatched_log(&mut self, keep_replay: bool) {
        if !keep_replay && self.ended && self.subscribers == 0 {
            self.chunks = Vec::new();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl RunRecord {
    fn status(&self) -> RunStatus {
        lock(&self.inner).status
    }

    fn snapshot(&self) -> RunSnapshot {
        let inner = lock(&self.inner);
        RunSnapshot {
            conversation_id: self.conversation_id,
            run_id: self.run_id,
            status: inner.status,
            messages: inner.messages.clone(),
            chunk_count: inner.chunks.len(),
        }
    }
}

impl ConversationRuns {
    /// `heartbeat` = panel SSE keep-alive interval (5 s in production).
    pub fn new(activity: Arc<ServerActivity>, heartbeat: Duration) -> Arc<Self> {
        Arc::new(Self {
            activity,
            heartbeat,
            state: Mutex::new(RunsState::default()),
        })
    }

    /// Registers the run synchronously (409 if one is running or its conversation is being
    /// deleted), then runs `prepare` and the chunk pump on a spawned task and waits only for
    /// `prepare`. On success the run drains `Prepared::chunks` whether or not anyone subscribes.
    /// A rejected `prepare` leaves a `failed` (or `aborted`) record behind, as TS does.
    pub async fn start<F, Fut>(
        self: &Arc<Self>,
        conversation_id: Uuid,
        presentation: RunPresentation,
        prepare: F,
    ) -> Result<StartedRun, ChatRejection>
    where
        F: FnOnce(RunHandle) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Prepared, ChatRejection>> + Send + 'static,
    {
        let (record, own_cursor) = {
            let mut state = lock(&self.state);
            let previous_messages = match state.runs.get(&conversation_id) {
                Some(existing) => {
                    let inner = lock(&existing.inner);
                    if inner.status == RunStatus::Running || inner.deleting {
                        return Err(ChatRejection::Busy);
                    }
                    // The new record shows the conversation as it stands until this turn
                    // publishes its own history: a turn failing before that (unknown agent,
                    // credentials, history read) must not make other panels hydrate it empty.
                    inner.messages.clone()
                }
                None => Vec::new(),
            };
            let (log_version, _) = watch::channel(0);
            let record = Arc::new(RunRecord {
                conversation_id,
                run_id: Uuid::new_v4(),
                panels_visible: presentation.panels_visible,
                keep_replay: presentation.keep_replay,
                cancel: CancellationToken::new(),
                prepared: CancellationToken::new(),
                finished: CancellationToken::new(),
                log_version,
                inner: Mutex::new(RunInner {
                    status: RunStatus::Running,
                    messages: previous_messages,
                    chunks: Vec::new(),
                    subscribers: 0,
                    saw_error_chunk: false,
                    ended: false,
                    deleting: false,
                    streaming: false,
                }),
            });
            // Installed before any preparation await: this both rejects a concurrent POST and
            // gives loopback MCP calls an authoritative active run.
            state.runs.insert(conversation_id, record.clone());
            attach_initial_panels(&mut state, &record, &presentation.panel_tab_ids);
            // Opened before the run can produce (or end), so the starter never misses a chunk
            // and a `keep_replay: false` log cannot be released before the starter read it.
            let own_cursor = replay_and_tail(record.clone());
            (record, own_cursor)
        };
        // `/status` reports busy from registration until the run is fully unwound (preparation,
        // pump and `on_finish`); the guard moves into the driving task.
        let busy = self.activity.reserve();

        let handle = RunHandle {
            run_id: record.run_id,
            cancel: record.cancel.clone(),
            owner: Some((Arc::downgrade(self), conversation_id)),
        };
        let (reply, prepared) = oneshot::channel();
        let runs = self.clone();
        tokio::spawn(async move {
            let _busy = busy;
            let driven = AssertUnwindSafe(runs.drive(record.clone(), handle, prepare, reply))
                .catch_unwind()
                .await;
            if driven.is_err() {
                tracing::error!(%conversation_id, "chat run panicked; marking it failed");
                runs.finish(&record, RunStatus::Failed, None);
            }
        });
        match prepared.await {
            Ok(Ok(run_id)) => Ok(StartedRun {
                run_id,
                chunks: own_cursor,
            }),
            Ok(Err(rejection)) => Err(rejection),
            // The driving task died before answering (a panic in `prepare`).
            Err(_) => Err(ChatRejection::Internal(run_crashed())),
        }
    }

    async fn drive<F, Fut>(
        self: &Arc<Self>,
        record: Arc<RunRecord>,
        handle: RunHandle,
        prepare: F,
        reply: oneshot::Sender<Result<Uuid, ChatRejection>>,
    ) where
        F: FnOnce(RunHandle) -> Fut + Send,
        Fut: Future<Output = Result<Prepared, ChatRejection>> + Send,
    {
        match prepare(handle).await {
            Err(rejection) => {
                let status = if record.cancel.is_cancelled() {
                    RunStatus::Aborted
                } else {
                    RunStatus::Failed
                };
                self.finish(&record, status, None);
                let _ = reply.send(Err(rejection));
            }
            Ok(Prepared { chunks, on_finish }) => {
                lock(&record.inner).streaming = true;
                // Engines publish canonical display messages before returning, so `/state` may
                // hydrate from here on.
                record.prepared.cancel();
                let _ = reply.send(Ok(record.run_id));
                // `stop` cancels before it reads `streaming`, so either it saw `streaming` and waits
                // for the end, or this check sees its cancel: a stopped run is never pumped.
                if record.cancel.is_cancelled() {
                    // Stopped while preparing: the turn never runs.
                    drop(chunks);
                    let outcome = on_finish(FinishedTurn {
                        status: RunStatus::Aborted,
                        assistant: None,
                    })
                    .await;
                    self.complete(&record, RunStatus::Aborted, outcome);
                } else {
                    self.pump(&record, chunks, on_finish).await;
                }
            }
        }
    }

    /// Drains the engine's stream into the replay log and the assembler until it ends or the run
    /// is cancelled. A cancel ends the log at once: TS `stop` cancels the registry's reader in the
    /// same tick it aborts, so nothing the engine yields afterwards (its `abort` chunk, a racing
    /// delta) ever reaches a subscriber, and the wire ends with `[DONE]`. Dropping the stream is
    /// the engine's cancellation (HTTP bodies, tool tasks, leases, scheduled pages).
    async fn pump(&self, record: &RunRecord, mut chunks: ChunkStream, on_finish: OnFinish) {
        let mut assembler = UiMessageAssembler::new();
        loop {
            let next = tokio::select! {
                biased;
                () = record.cancel.cancelled() => None,
                next = chunks.next() => next,
            };
            let Some(chunk) = next else { break };
            assembler.push(&chunk);
            append(record, chunk);
        }
        // Dropping the stream releases the engine's resources (HTTP bodies, tool tasks).
        drop(chunks);
        let status = if record.cancel.is_cancelled() {
            RunStatus::Aborted
        } else if lock(&record.inner).saw_error_chunk {
            RunStatus::Failed
        } else {
            RunStatus::Completed
        };
        let outcome = on_finish(FinishedTurn {
            status,
            assistant: assembler.into_message(),
        })
        .await;
        self.complete(record, status, outcome);
    }

    /// A failed save is observable before the end latch, in both live streams and replay. It
    /// takes precedence over model completion or cancellation: the promised history was not saved.
    fn complete(&self, record: &RunRecord, status: RunStatus, outcome: FinishOutcome) {
        let status = if let Some(error) = outcome.error {
            append(
                record,
                UiChunk::Error {
                    error_text: browseros_agent::llm::chat_error_text(&error),
                },
            );
            RunStatus::Failed
        } else {
            status
        };
        self.finish(record, status, outcome.messages);
    }

    /// Ends the run once: final messages and status in one critical section, both latches,
    /// subscriber wake-up, panel status.
    fn finish(&self, record: &RunRecord, status: RunStatus, messages: Option<Vec<UiMessage>>) {
        {
            let mut inner = lock(&record.inner);
            if inner.ended {
                return;
            }
            inner.ended = true;
            inner.status = status;
            if let Some(messages) = messages {
                inner.messages = messages;
            }
            inner.release_unwatched_log(record.keep_replay);
        }
        // Setup failures never reach the prepared point, but panel hydration must still resolve
        // to the visible failed/aborted state instead of hanging forever.
        record.prepared.cancel();
        record.log_version.send_modify(|version| *version += 1);
        {
            let state = lock(&self.state);
            if state
                .panels
                .values()
                .any(|owner| std::ptr::eq(Arc::as_ptr(owner), record))
            {
                publish_panels(&state);
            }
        }
        record.finished.cancel();
    }

    /// `getPreparedSnapshot`: waits for preparation only, never for the whole turn, so a panel
    /// opening mid-preparation never sees a mapping before its conversation history.
    pub async fn prepared_snapshot(&self, conversation_id: Uuid) -> Option<RunSnapshot> {
        loop {
            let record = lock(&self.state).runs.get(&conversation_id).cloned()?;
            record.prepared.cancelled().await;
            let current = lock(&self.state)
                .runs
                .get(&conversation_id)
                .is_some_and(|current| Arc::ptr_eq(current, &record));
            if current {
                return Some(record.snapshot());
            }
        }
    }

    /// The current record's snapshot without waiting.
    pub fn snapshot(&self, conversation_id: Uuid) -> Option<RunSnapshot> {
        lock(&self.state)
            .runs
            .get(&conversation_id)
            .map(|record| record.snapshot())
    }

    /// Replay of the whole current run, then the live tail; a finished run replays and closes.
    /// `run_id` is compared as the string the client sent (TS compares strings), so anything
    /// but the canonical current id — an old run, a bad id, `""` — is `RunChanged`.
    pub fn subscribe(&self, conversation_id: Uuid, run_id: Option<&str>) -> Resume {
        let Some(record) = lock(&self.state).runs.get(&conversation_id).cloned() else {
            return Resume::NoRun;
        };
        if run_id.is_some_and(|run_id| run_id != record.run_id.to_string()) {
            return Resume::RunChanged;
        }
        Resume::Stream(replay_and_tail(record))
    }

    /// `updateMessages`: only the conversation's current run may publish.
    pub fn update_messages(
        &self,
        conversation_id: Uuid,
        run_id: Uuid,
        messages: Vec<UiMessage>,
    ) -> bool {
        let state = lock(&self.state);
        let Some(record) = state.runs.get(&conversation_id) else {
            return false;
        };
        if record.run_id != run_id {
            return false;
        }
        lock(&record.inner).messages = messages;
        true
    }

    /// Cancels a running run. While it is still preparing this returns at once (the preparer
    /// owns unwinding whatever it already built, and `drive` never pumps a cancelled run);
    /// otherwise it waits until the run has ended, `on_finish` included.
    pub async fn stop(&self, conversation_id: Uuid) -> bool {
        let Some(record) = lock(&self.state).runs.get(&conversation_id).cloned() else {
            return false;
        };
        if record.status() != RunStatus::Running {
            return false;
        }
        // Cancel before reading `streaming`: read first, and `drive` could flip it and start the
        // pump in between, leaving this call to answer `stopped` while the turn still streams.
        record.cancel.cancel();
        if lock(&record.inner).streaming {
            record.finished.cancelled().await;
        }
        true
    }

    /// [`Self::delete_with`] with nothing else to release.
    pub async fn delete(&self, conversation_id: Uuid) -> bool {
        self.delete_with(conversation_id, async {}).await.0
    }

    /// Retires the conversation's panels at once, blocks replacement turns, stops the run and
    /// awaits its end (including late preparation unwinding and `on_finish`), runs `cleanup` (the
    /// engines' and history's release), and only then removes the record: a POST racing the
    /// cleanup is 409, never a turn on a half-forgotten session. Returns whether this call
    /// removed the record, and `cleanup`'s output (`cleanup` runs even without a record).
    ///
    /// Dropping the future part-way still removes the record (see [`Retire`]), so an abandoned
    /// delete can leak what `cleanup` would have released but never leaves the conversation 409.
    pub async fn delete_with<T>(
        &self,
        conversation_id: Uuid,
        cleanup: impl Future<Output = T>,
    ) -> (bool, T) {
        let Some(record) = self.mark_deleting(conversation_id) else {
            return (false, cleanup.await);
        };
        let retire = Retire {
            runs: self,
            conversation_id,
            record: record.clone(),
        };
        // A no-op on an ended run; a run still preparing finishes once its preparer unwinds.
        record.cancel.cancel();
        record.finished.cancelled().await;
        let cleaned = cleanup.await;
        (retire.remove(), cleaned)
    }

    /// Blocks replacement turns and retires the record's panels, in one registry critical section.
    fn mark_deleting(&self, conversation_id: Uuid) -> Option<Arc<RunRecord>> {
        let mut state = lock(&self.state);
        let record = state.runs.get(&conversation_id).cloned()?;
        lock(&record.inner).deleting = true;
        let retired: Vec<u32> = state
            .panels
            .iter()
            .filter(|(_, owner)| Arc::ptr_eq(owner, &record))
            .map(|(tab_id, _)| *tab_id)
            .collect();
        for tab_id in &retired {
            state.panels.remove(tab_id);
            state.auto_open.remove(tab_id);
        }
        if !retired.is_empty() {
            publish_panels(&state);
        }
        Some(record)
    }

    /// The live, un-cancelled run of a conversation, if any.
    pub fn active_run(&self, conversation_id: Uuid) -> Option<ActiveRun> {
        let state = lock(&self.state);
        let record = state.runs.get(&conversation_id)?;
        if record.status() != RunStatus::Running || record.cancel.is_cancelled() {
            return None;
        }
        Some(ActiveRun {
            conversation_id,
            run_id: record.run_id,
            panels_visible: record.panels_visible,
        })
    }

    /// Side-panel effect: tabs a run's tools created join its panel set and ask to auto-open.
    /// Refused (false) once `run` is no longer the conversation's live run, for scheduled
    /// (panel-less) runs, and while deleting.
    pub fn associate_tabs(&self, run: &ActiveRun, tab_ids: &[u32]) -> bool {
        let mut state = lock(&self.state);
        let Some(record) = state.runs.get(&run.conversation_id).cloned() else {
            return false;
        };
        if record.run_id != run.run_id || !record.panels_visible || record.cancel.is_cancelled() {
            return false;
        }
        {
            let inner = lock(&record.inner);
            if inner.status != RunStatus::Running || inner.deleting {
                return false;
            }
        }
        let mut changed = false;
        for &tab_id in tab_ids {
            if state
                .panels
                .get(&tab_id)
                .is_some_and(|owner| Arc::ptr_eq(owner, &record))
            {
                continue;
            }
            state.panels.insert(tab_id, record.clone());
            state.auto_open.insert(tab_id, Uuid::new_v4().to_string());
            changed = true;
        }
        if changed {
            publish_panels(&state);
        }
        true
    }

    /// The complete assignment table now, then on every change, then every heartbeat (a full
    /// repeat, so a client can repair missed work). Dropping the stream unsubscribes it.
    pub fn panel_assignments(self: &Arc<Self>) -> PanelStream {
        let (sender, receiver) = mpsc::unbounded_channel();
        let id = {
            let mut state = lock(&self.state);
            let id = state.next_panel_subscriber;
            state.next_panel_subscriber += 1;
            let _ = sender.send(current_assignments(&state));
            state.panel_subscribers.insert(id, sender);
            id
        };
        let subscription = PanelSubscription {
            runs: Arc::downgrade(self),
            id,
        };
        let mut heartbeat =
            tokio::time::interval_at(tokio::time::Instant::now() + self.heartbeat, self.heartbeat);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        Box::pin(futures_util::stream::unfold(
            (receiver, heartbeat, subscription),
            |(mut receiver, mut heartbeat, subscription)| async move {
                let next = tokio::select! {
                    biased;
                    update = receiver.recv() => update,
                    _ = heartbeat.tick() => subscription
                        .runs
                        .upgrade()
                        .map(|runs| current_assignments(&lock(&runs.state))),
                };
                next.map(|assignments| (assignments, (receiver, heartbeat, subscription)))
            },
        ))
    }

    /// `DELETE /chat/panels/{tabId}`: the tab closed; forget its routing.
    pub fn remove_panel_tab(&self, tab_id: u32) {
        let mut state = lock(&self.state);
        state.auto_open.remove(&tab_id);
        if state.panels.remove(&tab_id).is_some() {
            publish_panels(&state);
        }
    }
}

/// Removes a deleting record from the registry, on [`Retire::remove`] or when dropped part-way.
/// Only the record it was made for: a newer record installed meanwhile is left alone.
struct Retire<'a> {
    runs: &'a ConversationRuns,
    conversation_id: Uuid,
    record: Arc<RunRecord>,
}

impl Retire<'_> {
    fn remove(&self) -> bool {
        let mut state = lock(&self.runs.state);
        let current = state
            .runs
            .get(&self.conversation_id)
            .is_some_and(|current| Arc::ptr_eq(current, &self.record));
        if current {
            state.runs.remove(&self.conversation_id);
        }
        current
    }
}

impl Drop for Retire<'_> {
    fn drop(&mut self) {
        self.remove();
    }
}

fn append(record: &RunRecord, chunk: UiChunk) {
    {
        let mut inner = lock(&record.inner);
        if matches!(chunk, UiChunk::Error { .. }) {
            inner.saw_error_chunk = true;
        }
        inner.chunks.push(chunk);
    }
    record.log_version.send_modify(|version| *version += 1);
}

/// A subscriber is a cursor into the record's one replay log: replay and live tail are the same
/// read loop, so there is no handoff gap and no per-viewer buffer. Dropping it detaches only it.
fn replay_and_tail(record: Arc<RunRecord>) -> ChunkStream {
    let cursor = Cursor::new(record);
    Box::pin(futures_util::stream::unfold(
        cursor,
        |mut cursor| async move {
            loop {
                // Mark the version seen *before* reading the log, so an append racing this read is
                // either visible now or wakes the `changed()` below.
                cursor.version.borrow_and_update();
                {
                    let inner = lock(&cursor.record.inner);
                    if let Some(chunk) = inner.chunks.get(cursor.position) {
                        let chunk = chunk.clone();
                        drop(inner);
                        cursor.position += 1;
                        return Some((chunk, cursor));
                    }
                    if inner.ended {
                        return None;
                    }
                }
                if cursor.version.changed().await.is_err() {
                    return None;
                }
            }
        },
    ))
}

/// One subscriber's place in a record's replay log. Registered in `RunInner::subscribers` for its
/// whole life; the last one to detach from an ended `keep_replay: false` run drops the log.
struct Cursor {
    record: Arc<RunRecord>,
    version: watch::Receiver<u64>,
    position: usize,
}

impl Cursor {
    fn new(record: Arc<RunRecord>) -> Self {
        let version = record.log_version.subscribe();
        lock(&record.inner).subscribers += 1;
        Self {
            record,
            version,
            position: 0,
        }
    }
}

impl Drop for Cursor {
    fn drop(&mut self) {
        let mut inner = lock(&self.record.inner);
        inner.subscribers -= 1;
        inner.release_unwatched_log(self.record.keep_replay);
    }
}

/// A later turn wakes every tab still mapped to this conversation; a tab claimed by another
/// conversation joins only if this turn explicitly asked for it (and loses its auto-open).
fn attach_initial_panels(state: &mut RunsState, record: &Arc<RunRecord>, requested: &[u32]) {
    if !record.panels_visible {
        return;
    }
    let mut tab_ids: Vec<u32> = requested.to_vec();
    tab_ids.extend(
        state
            .panels
            .iter()
            .filter(|(_, owner)| owner.conversation_id == record.conversation_id)
            .map(|(tab_id, _)| *tab_id),
    );
    tab_ids.sort_unstable();
    tab_ids.dedup();
    if tab_ids.is_empty() {
        return;
    }
    for tab_id in tab_ids {
        if state
            .panels
            .get(&tab_id)
            .is_none_or(|owner| owner.conversation_id != record.conversation_id)
        {
            state.auto_open.remove(&tab_id);
        }
        state.panels.insert(tab_id, record.clone());
    }
    publish_panels(state);
}

fn current_assignments(state: &RunsState) -> ConversationPanelAssignments {
    ConversationPanelAssignments::new(
        state
            .panels
            .iter()
            .map(|(tab_id, record)| {
                let mut assignment = ConversationPanelAssignment::new(
                    i64::from(*tab_id),
                    record.conversation_id.to_string(),
                    record.run_id.to_string(),
                    record.status().as_wire(),
                );
                assignment.auto_open_id = state.auto_open.get(tab_id).cloned();
                assignment
            })
            .collect(),
    )
}

fn publish_panels(state: &RunsState) {
    let assignments = current_assignments(state);
    for sender in state.panel_subscribers.values() {
        // A closed receiver is removed by its subscription's `Drop`.
        let _ = sender.send(assignments.clone());
    }
}

/// Owns one panel subscriber's registration; the heartbeat timer dies with the stream.
struct PanelSubscription {
    runs: Weak<ConversationRuns>,
    id: u64,
}

impl Drop for PanelSubscription {
    fn drop(&mut self) {
        if let Some(runs) = self.runs.upgrade() {
            lock(&runs.state).panel_subscribers.remove(&self.id);
        }
    }
}

/// The 500 a run gets when its driving task died without answering (a panic in `prepare`).
fn run_crashed() -> ChatError {
    ChatError {
        code: ChatErrorCode::Unknown,
        title: "Something went wrong".to_owned(),
        message: "The chat run stopped unexpectedly.".to_owned(),
        retryable: true,
        provider: None,
        status_code: None,
        docs_url: None,
        retry_after_seconds: None,
        details: None,
    }
}
