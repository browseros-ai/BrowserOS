//! `conversations`: owns every conversation. It is the single writer of the `conversations`
//! table and of run state; `http` (and one day the cron job) only call it. A run belongs to
//! the server, not to a request: subscribers come and go, and only `stop`, `release` and
//! `delete` cancel.
//!
//! ```text
//!   send ──▶ Run { log: Transcript + status } ◀── one task: runner.run_turn(sink) ── writes the row
//!              │                                   at ToolInput, ToolDone and the end
//!              ├──▶ Subscription (cursor 0..)  the POST /chat response
//!              └──▶ Subscription (cursor 0..)  a second panel, replaying then live
//! ```
//!
//! Replay and live are one code path: a subscriber is a cursor into the run's append-only log,
//! and the log and its wake-up change under one lock, so "replay, then live" has no gap.

use std::sync::Arc;

use tokio::sync::watch;

use crate::browser::Browser;
use crate::store::{ConversationSummary, Store, StoreError};
use crate::turn::{
    Attachment, ConversationId, Frame, MessageId, RunId, Target, Transcript, TurnContext,
    TurnRunner, UiMessage,
};

/// The run registry and the panel table. Cheap to clone.
#[derive(Clone)]
pub struct Conversations {
    inner: Arc<Inner>,
}

struct Inner {
    #[expect(dead_code, reason = "read once chunk `runs` writes the registry")]
    store: Store,
    #[expect(dead_code, reason = "read once chunk `runs` writes the registry")]
    runner: Arc<dyn TurnRunner>,
    /// Asked for the tabs a run's task opened after every `ToolDone` and before the terminal
    /// state is published, so a tab's `autoOpenId` reaches the panel table while the run is
    /// still `running` (the extension opens a panel only then).
    #[expect(dead_code, reason = "read once chunk `runs` writes the registry")]
    browser: Browser,
    panels: watch::Sender<Arc<PanelTable>>,
}

/// One server-owned turn. Written only by its own task, through the run's `EventSink`;
/// read by any number of subscribers, each with its own cursor into the log, so a slow
/// panel can never lose a frame and replay needs no second code path.
#[expect(dead_code, reason = "built once chunk `runs` writes the registry")]
struct Run {
    id: RunId,
    /// The stored transcript plus this turn's user message: what `/state` serves.
    history: Arc<[UiMessage]>,
    /// std Mutex: held for microseconds, never across an await. The log and the wake-up
    /// change under it together, which is why "replay, then live" has no gap.
    log: std::sync::Mutex<RunLog>,
    changed: tokio::sync::Notify,
    cancel: tokio_util::sync::CancellationToken,
}

#[expect(dead_code, reason = "built once chunk `runs` writes the registry")]
struct RunLog {
    transcript: Transcript,
    status: RunStatus,
    ended_at: Option<std::time::Instant>,
}

/// Where a run is. Every status but `Running` is terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    Completed,
    Aborted,
    Failed,
}

/// A turn as the domain sees it; `http` builds it from the request.
#[derive(Debug, Clone)]
pub struct NewTurn {
    pub conversation: ConversationId,
    /// The client's id for this user message. If the stored transcript already holds a
    /// message with this id, everything after it is dropped and it is answered again, so
    /// Retry and a repeated POST never store the question twice. `None`: the server makes one.
    pub user_message_id: Option<MessageId>,
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub target: Target,
    pub context: TurnContext,
    pub history: History,
    /// The tab whose side panel asked; it is assigned to the run in the panel table.
    pub panel_tab: Option<i64>,
}

/// Incognito turns are never written; their history comes from the request instead.
#[derive(Debug, Clone, PartialEq)]
pub enum History {
    Stored,
    Ephemeral(Vec<UiMessage>),
}

/// A turn is already running for this conversation (`409`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy;

/// What `GET /chat/{id}/stream?runId=` finds.
pub enum Attach {
    /// No run is known for the conversation (`204`).
    NoRun,
    /// A newer run replaced the one asked for (`409`).
    Changed,
    Run(Subscription),
}

/// A cursor into one run's log: frame 0 onward, then live, ending after the terminal frame.
/// Dropping it detaches; it never cancels the run.
pub struct Subscription {
    #[expect(dead_code, reason = "read once chunk `runs` writes the registry")]
    run: Option<Arc<Run>>,
    #[expect(dead_code, reason = "read once chunk `runs` writes the registry")]
    cursor: usize,
}

impl futures_util::Stream for Subscription {
    type Item = Frame;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Frame>> {
        std::task::Poll::Ready(None)
    }
}

/// The current run of a conversation, for a panel that did not start it.
#[derive(Debug, Clone)]
pub struct RunState {
    pub conversation: ConversationId,
    pub run: RunId,
    pub status: RunStatus,
    /// Running: the transcript up to and including this turn's user message (the stream
    /// supplies the answer). Finished: that plus the sealed answer, because the extension
    /// reads the stream only while `status` is `running`.
    pub messages: Arc<[UiMessage]>,
    pub frame_count: usize,
}

/// Which tab's side panel shows which run. Sent whole on every change and every 5 s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PanelTable {
    pub assignments: Vec<PanelAssignment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelAssignment {
    pub tab_id: i64,
    pub conversation: ConversationId,
    pub run: RunId,
    pub status: RunStatus,
    /// Set once for a tab the agent opened; tells the extension to open its panel there.
    pub auto_open_id: Option<String>,
}

/// A stored conversation with its transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversation {
    pub summary: ConversationSummary,
    pub messages: Vec<UiMessage>,
}

impl Conversations {
    /// Also starts one housekeeping task: the 5 s panel heartbeat and the eviction of
    /// finished runs. `browser` is asked for the tabs each run's task opened.
    pub fn new(store: Store, runner: Arc<dyn TurnRunner>, browser: Browser) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                runner,
                browser,
                panels: watch::channel(Arc::new(PanelTable::default())).0,
            }),
        }
    }

    /// Starts a turn: refuses if one is running, loads the stored transcript (only when the
    /// row's target matches), places the user message, registers the run, spawns its task,
    /// and returns a subscription at frame 0. Nothing is written yet.
    pub async fn send(&self, _turn: NewTurn) -> Result<Subscription, Busy> {
        Ok(Subscription {
            run: None,
            cursor: 0,
        })
    }

    pub fn state(&self, _id: ConversationId) -> Option<RunState> {
        None
    }

    /// `run: None` attaches to the conversation's current run (today's behaviour when the
    /// query is absent). `Some(id)` that is not the current run is `Attach::Changed`. A run
    /// whose frame log is gone is `Attach::NoRun`.
    pub fn attach(&self, _id: ConversationId, _run: Option<RunId>) -> Attach {
        Attach::NoRun
    }

    /// Fires the run's cancel token. Returns false when nothing was running.
    pub fn stop(&self, _id: ConversationId) -> bool {
        false
    }

    /// `DELETE /chat/{id}`: stop, wait for the task, drop the run and its panels. Keeps the row.
    pub async fn release(&self, _id: ConversationId) {}

    pub async fn list(&self) -> Result<Vec<ConversationSummary>, StoreError> {
        Err(StoreError::NotImplemented("Conversations::list"))
    }

    pub async fn get(&self, _id: ConversationId) -> Result<Option<Conversation>, StoreError> {
        Err(StoreError::NotImplemented("Conversations::get"))
    }

    /// `release`, then delete the row, then end the agent's session, in that order, so a
    /// late checkpoint cannot bring the row back.
    pub async fn delete(&self, _id: ConversationId) -> Result<bool, StoreError> {
        Err(StoreError::NotImplemented("Conversations::delete"))
    }

    pub fn panels(&self) -> watch::Receiver<Arc<PanelTable>> {
        self.inner.panels.subscribe()
    }

    pub fn forget_panel(&self, _tab_id: i64) {}

    /// `GET /status` reports `can_update: !busy()`.
    pub fn busy(&self) -> bool {
        false
    }

    /// Process shutdown: fire every run's cancel at once, drop a backend that has not returned
    /// by `deadline - 1 s`, write each sealed row, return by `deadline`. `app` passes 2.5 s of
    /// its 4 s total, because the launcher kills the process 5 s after SIGTERM.
    pub async fn shutdown(&self, _deadline: std::time::Duration) {}
}

/// The run's task, the only writer of its log and of its row.
///
///   let end = select! { end = runner.run_turn(..) => end,
///                       () = cancelled_then(GRACE) => TurnEnd::Stopped,   // a backend that ignores stop is dropped
///                       () = sleep(TURN_BUDGET)    => TurnEnd::Done(Length) };
///   log.transcript.finish(&end);  write the row;  status = terminal;  wake subscribers;  re-send the panel table
///
/// What is written, and when (History::Stored only):
///   ToolInput   prefix + sealed answer      before the tool runs
///   ToolDone    prefix + sealed answer      after it returns
///   the end     prefix + sealed answer      finish, stop or failure; skipped while the answer has no part
#[expect(dead_code, reason = "written by chunk `runs`")]
async fn run_task() {}

/// How long a backend may take to return after Stop before its future is dropped.
#[expect(dead_code, reason = "read once chunk `runs` writes the run task")]
const GRACE: std::time::Duration = std::time::Duration::from_secs(5);
/// The longest one turn may run.
#[expect(dead_code, reason = "read once chunk `runs` writes the run task")]
const TURN_BUDGET: std::time::Duration = std::time::Duration::from_secs(60 * 60);
/// How long a finished run's frame log stays replayable. The run record itself (id, status,
/// final messages) stays while a panel assignment points at it; a finished run with no panel
/// goes after this too.
#[expect(dead_code, reason = "read once chunk `runs` writes the run task")]
const KEEP_FINISHED: std::time::Duration = std::time::Duration::from_secs(600);
