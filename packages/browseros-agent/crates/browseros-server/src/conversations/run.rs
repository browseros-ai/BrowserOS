//! One server-owned turn: its frame log, the subscribers that read it, the sink a backend
//! writes into, and the task that drives the backend and writes the row.
//!
//! ```text
//!   run_task
//!      ├─ admission: write prefix
//!      ├─ run_turn(sink) ─▶ backend
//!      │                       └─ emit(event) ─▶ RunSink: apply event ─▶ notify subscribers
//!      │                                           └─ checkpoint: write prefix + sealed answer
//!      └─ the end: finish ─▶ notify ─▶ write the row ─▶ status = terminal ─▶ notify ─▶ ended
//! ```
//!
//! The run task is the only writer of the run's log and of its row: checkpoints are written
//! from inside the backend's `emit` call, which the task is awaiting, so every write of one run
//! happens in order, on one task, and none can happen after the task has ended.

use std::panic::AssertUnwindSafe;
use std::pin::{Pin, pin};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Instant;

use futures_util::FutureExt as _;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::{GRACE, Inner, RunStatus, TURN_BUDGET, lock};
use crate::store::{ConversationRow, Store};
use crate::turn::{
    ChatError, ChatErrorCode, Checkpoint, ConversationId, EventSink, FinishReason, Frame, RunId,
    Target, Transcript, TurnEnd, TurnEvent, TurnInput, UiMessage,
};

/// One server-owned turn. Written only by its own task, through the run's `EventSink`;
/// read by any number of subscribers, each with its own cursor into the log, so a slow
/// panel can never lose a frame and replay needs no second code path.
pub(super) struct Run {
    pub(super) id: RunId,
    pub(super) conversation: ConversationId,
    /// The stored transcript plus this turn's user message: the prefix of every write, and
    /// what `/state` serves while the run is live.
    pub(super) history: Arc<[UiMessage]>,
    /// std Mutex: held for microseconds, never across an await. The log and the wake-up
    /// change under it together, which is why "replay, then live" has no gap.
    pub(super) log: Mutex<RunLog>,
    pub(super) changed: Notify,
    /// Stop. Fired by `stop`, `release`, `delete` and `shutdown`; the backend sees it.
    pub(super) cancel: CancellationToken,
    /// Fired by the run's task as its very last step, after the row is written and the
    /// terminal status published. `release`, `delete` and `shutdown` wait on it, so nothing is
    /// written for this run once they go on.
    pub(super) ended: CancellationToken,
}

pub(super) struct RunLog {
    /// `None` once a finished run's frame log has been dropped (`KEEP_FINISHED`).
    transcript: Option<Transcript>,
    /// The frame count at the moment the log was dropped.
    dropped_frames: usize,
    pub(super) status: RunStatus,
    /// What `/state` serves: `history` while running, and with the sealed answer once ended,
    /// because the extension reads the stream only while the status is `running`.
    pub(super) messages: Arc<[UiMessage]>,
    pub(super) ended_at: Option<Instant>,
}

impl RunLog {
    pub(super) fn frame_count(&self) -> usize {
        self.transcript
            .as_ref()
            .map_or(self.dropped_frames, |transcript| transcript.frames(0).len())
    }

    pub(super) fn has_frames(&self) -> bool {
        self.transcript.is_some()
    }

    /// Frees the frame log; the run record (id, status, final messages) stays.
    pub(super) fn drop_frames(&mut self) {
        if let Some(transcript) = self.transcript.take() {
            self.dropped_frames = transcript.frames(0).len();
        }
    }
}

impl Run {
    pub(super) fn new(conversation: ConversationId, history: Arc<[UiMessage]>) -> Self {
        let assistant = crate::turn::MessageId(uuid::Uuid::new_v4().to_string());
        Self {
            id: uuid::Uuid::new_v4(),
            conversation,
            history: history.clone(),
            log: Mutex::new(RunLog {
                transcript: Some(Transcript::new(assistant)),
                dropped_frames: 0,
                status: RunStatus::Running,
                messages: history,
                ended_at: None,
            }),
            changed: Notify::new(),
            cancel: CancellationToken::new(),
            ended: CancellationToken::new(),
        }
    }

    pub(super) fn status(&self) -> RunStatus {
        lock(&self.log).status
    }

    pub(super) fn is_running(&self) -> bool {
        self.status() == RunStatus::Running
    }
}

/// A cursor into one run's log: frame 0 onward, then live, ending after the terminal frame.
/// Dropping it detaches; it never cancels the run.
pub struct Subscription {
    frames: BoxStream<'static, Frame>,
}

impl futures_util::Stream for Subscription {
    type Item = Frame;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Frame>> {
        self.get_mut().frames.poll_next_unpin(cx)
    }
}

/// What the cursor finds at its position.
enum Next {
    Frame(Frame),
    Wait,
    End,
}

/// A subscription at frame 0. The waiter is registered (`enable`) before the log is read, and
/// the writer wakes waiters after appending under the same lock, so a frame appended between
/// the read and the wait still wakes this cursor: no gap, and each frame exactly once.
pub(super) fn subscribe(run: Arc<Run>) -> Subscription {
    let frames = futures_util::stream::unfold((run, 0_usize), |(run, cursor)| async move {
        loop {
            let next = {
                let notified = run.changed.notified();
                let mut notified = pin!(notified);
                notified.as_mut().enable();
                match read(&run, cursor) {
                    Next::Wait => {
                        notified.await;
                        continue;
                    }
                    ready => ready,
                }
            };
            return match next {
                Next::Frame(frame) => Some((frame, (run, cursor + 1))),
                Next::Wait | Next::End => None,
            };
        }
    });
    Subscription {
        frames: frames.boxed(),
    }
}

fn read(run: &Run, cursor: usize) -> Next {
    let log = lock(&run.log);
    match &log.transcript {
        None => Next::End,
        Some(transcript) => match transcript.frames(cursor).first() {
            Some(frame) => Next::Frame(frame.clone()),
            None if log.status == RunStatus::Running => Next::Wait,
            None => Next::End,
        },
    }
}

/// Where a run's rows go. `Never` for an incognito turn, and for a turn whose stored
/// transcript could not be read: writing then would replace history this server never saw.
pub(super) enum Persist {
    Never,
    Row {
        target_type: &'static str,
        agent_id: Option<String>,
    },
}

impl Persist {
    /// Writes the whole row: the admitted transcript plus the sealed answer, if it has content.
    ///
    /// A failed write is logged and the turn goes on. A later checkpoint or the end can recover
    /// the missed write because each write includes the whole row.
    async fn write(&self, store: &Store, run: &Run, answer: Option<UiMessage>) {
        let Persist::Row {
            target_type,
            agent_id,
        } = self
        else {
            return;
        };
        let mut messages = run.history.to_vec();
        messages.extend(answer);
        let row = ConversationRow {
            id: run.conversation,
            messages,
            target_type: (*target_type).to_owned(),
            agent_id: agent_id.clone(),
        };
        if let Err(error) = store.conversations().save(&row).await {
            tracing::warn!(conversation = %run.conversation, %error, "could not write the conversation");
        }
    }
}

/// The `EventSink` a backend writes into. A checkpoint returns only after its write finished,
/// which is what makes "persisted before the tool runs" true without a backend knowing about
/// storage.
struct RunSink {
    inner: Arc<Inner>,
    run: Arc<Run>,
    persist: Persist,
}

#[async_trait::async_trait]
impl EventSink for RunSink {
    async fn emit(&mut self, event: TurnEvent) {
        let tool_done = matches!(event, TurnEvent::ToolDone { .. });
        let checkpoint = {
            let mut log = lock(&self.run.log);
            let Some(transcript) = log.transcript.as_mut() else {
                return;
            };
            match transcript.apply(event) {
                Checkpoint::Persist => Some(transcript.sealed()),
                Checkpoint::None => None,
            }
        };
        self.run.changed.notify_waiters();
        if let Some(answer) = checkpoint {
            self.persist
                .write(&self.inner.store, &self.run, answer)
                .await;
        }
        if tool_done {
            self.inner.assign_opened_tabs(&self.run);
        }
    }
}

/// Everything `send` hands the run's task.
pub(super) struct TurnJob {
    pub(super) target: Target,
    pub(super) input: TurnInput,
    pub(super) persist: Persist,
    /// The turn cannot run (its history could not be read): it ends with this, unrun.
    pub(super) refused: Option<ChatError>,
}

/// The run's task, the only writer of its log and of its row.
///
/// What is written, and when (a `Persist::Row` run only):
///   admission   prefix alone               before the backend starts
///   ToolInput   prefix + sealed answer      before the tool runs (own loop); as first seen (coding agent)
///   ToolDone    prefix + sealed answer      after it returns
///   the end     prefix + sealed answer      finish, stop or failure
pub(super) async fn run_task(inner: Arc<Inner>, run: Arc<Run>, job: TurnJob) {
    job.persist.write(&inner.store, &run, None).await;
    let TurnJob {
        target,
        input,
        persist,
        refused,
    } = job;
    let mut sink = RunSink {
        inner: inner.clone(),
        run: run.clone(),
        persist,
    };
    let end = match refused {
        Some(error) => TurnEnd::Failed(error),
        None => {
            // A backend that panics ends its turn with an error card instead of leaving the
            // conversation "running" forever.
            let turn = AssertUnwindSafe(inner.runner.run_turn(
                &target,
                &input,
                &mut sink,
                &run.cancel,
            ))
            .catch_unwind();
            tokio::select! {
                end = turn => end.unwrap_or_else(|_| TurnEnd::Failed(crashed())),
                // A backend that ignores Stop is dropped, which kills its child process and
                // closes its requests.
                () = grace_after(&run.cancel) => TurnEnd::Stopped,
                () = inner.abandon.cancelled() => TurnEnd::Stopped,
                () = tokio::time::sleep(TURN_BUDGET) => TurnEnd::Done(FinishReason::Length),
            }
        }
    };
    // The backend's future is gone here: nothing it held can emit into this run again.
    // Tabs it opened get their `autoOpenId` while the run still reads as `running`, because
    // the extension opens a panel only for a running run.
    inner.assign_opened_tabs(&run);
    let answer = {
        let mut log = lock(&run.log);
        log.transcript.as_mut().and_then(|transcript| {
            transcript.finish(&end);
            transcript.sealed()
        })
    };
    run.changed.notify_waiters();
    sink.persist.write(&inner.store, &run, answer.clone()).await;
    {
        let mut log = lock(&run.log);
        log.status = match end {
            TurnEnd::Done(_) => RunStatus::Completed,
            TurnEnd::Stopped => RunStatus::Aborted,
            TurnEnd::Failed(_) => RunStatus::Failed,
        };
        if let Some(answer) = answer {
            let mut messages = run.history.to_vec();
            messages.push(answer);
            log.messages = Arc::from(messages);
        }
        log.ended_at = Some(Instant::now());
    }
    run.changed.notify_waiters();
    if inner.has_panels(&run) {
        inner.publish_panels();
    }
    run.ended.cancel();
}

async fn grace_after(cancel: &CancellationToken) {
    cancel.cancelled().await;
    tokio::time::sleep(GRACE).await;
}

fn crashed() -> ChatError {
    ChatError::new(
        ChatErrorCode::Unknown,
        "Something went wrong",
        "The server hit an internal error while answering. Try again.",
    )
}
