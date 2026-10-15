//! Port of `conversation-runs.ts`: one run record per conversation, registered before any await
//! (the 409 gate), a replay log plus live tail for `/chat/{id}/stream`, the "prepared" gate
//! `/state` waits on, run status, and the side-panel projection (`/chat/panels`).
//!
//! ```text
//! start ─ register (sync, 409) ─ spawn drive ──► prepare(RunHandle) ── Ok ─► pump ─► on_finish ─► finish
//!            │                                    │ (history published,          │                  │
//!            └ panels attached                    │  engine prepared)            │ chunks appended  └ status, latches,
//!                                                 └ Err ──────────────────────────┼─────────────────► panels published
//!   subscribe ─► cursor over the one replay log (replay + tail, no gap) ◄─────────┘
//! ```
//!
//! Lifecycle rules a reader must not break:
//! - The run is driven by a spawned task, never by the HTTP request: a client disconnect (which
//!   drops the handler future) only drops a subscriber cursor. Only `stop`/`delete` cancel.
//! - `on_finish` (persistence) runs after the last chunk and before subscribers see the end, so a
//!   client that reloads right after `[DONE]` finds the saved conversation.
//! - Lock order is `RunsState` → `RunInner`; never acquire the registry lock while holding a
//!   record's lock. Neither lock is held across an await.
//!
//! Owned by P8.

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

/// How long a cancelled run's engine stream may keep draining (its `abort` chunk, tool
/// unwinding) before the registry drops it. Engines are required to end promptly on cancel; this
/// only bounds a misbehaving one, so `/stop` cannot hang on it.
const STOP_GRACE: Duration = Duration::from_secs(2);

pub struct RunPresentation {
    pub panel_tab_ids: Vec<u32>,
    pub panels_visible: bool,
    pub tab_group: Option<TabGroupPresentation>,
}

/// `"<target>/<first-5-words>"` plus the color identity of the tab group a run's pages join.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabGroupPresentation {
    pub title: String,
    pub color_key: String,
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

/// Persistence hook; runs before subscribers see the end of the stream, so a client that reloads
/// right after `[DONE]` always finds the saved conversation. Called exactly once per prepared run.
pub type OnFinish = Box<dyn FnOnce(FinishedTurn) -> BoxFuture<'static, ()> + Send>;

pub struct Prepared {
    pub chunks: ChunkStream,
    pub on_finish: OnFinish,
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
    pub tab_group: Option<TabGroupPresentation>,
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
    tab_group: Option<TabGroupPresentation>,
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
    /// The replay log: every chunk of the run, in order. Never truncated while the record lives.
    chunks: Vec<UiChunk>,
    saw_error_chunk: bool,
    ended: bool,
    deleting: bool,
    /// `prepare` has returned a stream (TS: `record.reader` is set).
    streaming: bool,
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
    ) -> Result<Uuid, ChatRejection>
    where
        F: FnOnce(RunHandle) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Prepared, ChatRejection>> + Send + 'static,
    {
        let record = {
            let mut state = lock(&self.state);
            let previous_group = match state.runs.get(&conversation_id) {
                Some(existing) => {
                    let inner = lock(&existing.inner);
                    if inner.status == RunStatus::Running || inner.deleting {
                        return Err(ChatRejection::Busy);
                    }
                    existing.tab_group.clone()
                }
                None => None,
            };
            let (log_version, _) = watch::channel(0);
            let record = Arc::new(RunRecord {
                conversation_id,
                run_id: Uuid::new_v4(),
                panels_visible: presentation.panels_visible,
                // Keep the first useful presentation across turns: a group first created on a
                // later turn still describes the conversation, not only its latest prompt.
                tab_group: previous_group.or(presentation.tab_group),
                cancel: CancellationToken::new(),
                prepared: CancellationToken::new(),
                finished: CancellationToken::new(),
                log_version,
                inner: Mutex::new(RunInner {
                    status: RunStatus::Running,
                    messages: Vec::new(),
                    chunks: Vec::new(),
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
            record
        };
        // `/status` reports busy from registration until the run is fully unwound (preparation,
        // pump and `on_finish`); the guard moves into the driving task.
        let busy = self.activity.run_started_owned();

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
                runs.finish(&record, RunStatus::Failed);
            }
        });
        match prepared.await {
            Ok(outcome) => outcome,
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
                self.finish(&record, status);
                let _ = reply.send(Err(rejection));
            }
            Ok(Prepared { chunks, on_finish }) => {
                lock(&record.inner).streaming = true;
                // Engines publish canonical display messages before returning, so `/state` may
                // hydrate from here on.
                record.prepared.cancel();
                let _ = reply.send(Ok(record.run_id));
                if record.cancel.is_cancelled() {
                    // Stopped while preparing: the turn never runs.
                    drop(chunks);
                    on_finish(FinishedTurn {
                        status: RunStatus::Aborted,
                        assistant: None,
                    })
                    .await;
                    self.finish(&record, RunStatus::Aborted);
                } else {
                    self.pump(&record, chunks, on_finish).await;
                }
            }
        }
    }

    /// Drains the engine's stream into the replay log and the assembler. After a cancel the
    /// stream may drain for [`STOP_GRACE`] (its `abort` chunk) before it is dropped.
    async fn pump(&self, record: &RunRecord, mut chunks: ChunkStream, on_finish: OnFinish) {
        let mut assembler = UiMessageAssembler::new();
        let mut grace_deadline = None;
        loop {
            let next = if record.cancel.is_cancelled() {
                let deadline =
                    *grace_deadline.get_or_insert_with(|| tokio::time::Instant::now() + STOP_GRACE);
                match tokio::time::timeout_at(deadline, chunks.next()).await {
                    Ok(next) => next,
                    Err(_) => {
                        tracing::warn!(
                            conversation_id = %record.conversation_id,
                            "engine kept streaming after stop; dropping its stream"
                        );
                        None
                    }
                }
            } else {
                tokio::select! {
                    biased;
                    next = chunks.next() => next,
                    () = record.cancel.cancelled() => continue,
                }
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
        on_finish(FinishedTurn {
            status,
            assistant: assembler.into_message(),
        })
        .await;
        self.finish(record, status);
    }

    /// Ends the run once: status, both latches, subscriber wake-up, panel status.
    fn finish(&self, record: &RunRecord, status: RunStatus) {
        {
            let mut inner = lock(&record.inner);
            if inner.ended {
                return;
            }
            inner.ended = true;
            inner.status = status;
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
    /// owns unwinding whatever it already built); otherwise it waits until the run has ended,
    /// `on_finish` included.
    pub async fn stop(&self, conversation_id: Uuid) -> bool {
        let Some(record) = lock(&self.state).runs.get(&conversation_id).cloned() else {
            return false;
        };
        let streaming = {
            let inner = lock(&record.inner);
            if inner.status != RunStatus::Running {
                return false;
            }
            inner.streaming
        };
        record.cancel.cancel();
        if streaming {
            record.finished.cancelled().await;
        }
        true
    }

    /// Retires the conversation's panels at once, blocks replacement turns, stops the run, awaits
    /// its end (including late preparation unwinding and `on_finish`), then removes the record.
    pub async fn delete(&self, conversation_id: Uuid) -> bool {
        let record = {
            let mut state = lock(&self.state);
            let Some(record) = state.runs.get(&conversation_id).cloned() else {
                return false;
            };
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
            record
        };
        if record.status() == RunStatus::Running {
            self.stop(conversation_id).await;
        }
        record.finished.cancelled().await;
        let mut state = lock(&self.state);
        if !state
            .runs
            .get(&conversation_id)
            .is_some_and(|current| Arc::ptr_eq(current, &record))
        {
            return false;
        }
        state.runs.remove(&conversation_id);
        true
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
            tab_group: record.tab_group.clone(),
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

    /// Runs still in progress; feeds `/status` `can_update` together with `ServerActivity`.
    pub fn running_count(&self) -> usize {
        lock(&self.state)
            .runs
            .values()
            .filter(|record| record.status() == RunStatus::Running)
            .count()
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
    let version = record.log_version.subscribe();
    Box::pin(futures_util::stream::unfold(
        (record, version, 0_usize),
        |(record, mut version, cursor)| async move {
            loop {
                // Mark the version seen *before* reading the log, so an append racing this read
                // is either visible now or wakes the `changed()` below.
                version.borrow_and_update();
                {
                    let inner = lock(&record.inner);
                    if let Some(chunk) = inner.chunks.get(cursor) {
                        let chunk = chunk.clone();
                        drop(inner);
                        return Some((chunk, (record, version, cursor + 1)));
                    }
                    if inner.ended {
                        return None;
                    }
                }
                if version.changed().await.is_err() {
                    return None;
                }
            }
        },
    ))
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

/// The 500 an engine stub answers until its piece lands (kept for `browseros_engine`'s S0 stub).
pub(crate) fn not_implemented() -> ChatError {
    ChatError {
        code: ChatErrorCode::Unknown,
        title: "Something went wrong".to_owned(),
        message: "Chat is not implemented yet in server-rust".to_owned(),
        retryable: false,
        provider: None,
        status_code: Some(501),
        docs_url: None,
        retry_after_seconds: None,
        details: None,
    }
}
