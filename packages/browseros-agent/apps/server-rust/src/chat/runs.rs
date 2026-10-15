//! Port of `conversation-runs.ts`: one run record per conversation, registered before any await
//! (the 409 gate), a replay buffer plus live fan-out for `/chat/{id}/stream`, the "prepared" gate
//! `/state` waits on, run status, and the side-panel projection. Client disconnects never cancel
//! a run; only `stop`/`delete` do. Owned by P8; S0 stubs refuse every run.

use super::{PanelStream, Resume, engine::ChatRejection};
use crate::activity::ServerActivity;
use browseros_agent::llm::{ChatError, ChatErrorCode};
use futures_util::future::BoxFuture;
use std::{future::Future, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use ui_message_stream::{ChunkStream, UiMessage};
use uuid::Uuid;

pub struct RunPresentation {
    pub panel_tab_ids: Vec<u32>,
    pub panels_visible: bool,
    pub tab_group: Option<TabGroupPresentation>,
}

/// `"<target>/<first-5-words>"`.
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
}

impl RunHandle {
    pub fn new(run_id: Uuid, cancel: CancellationToken) -> Self {
        Self { run_id, cancel }
    }

    pub fn run_id(&self) -> Uuid {
        self.run_id
    }

    pub fn cancel(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Returns false once the run has been replaced or removed. S0 stub: never published.
    pub fn publish_messages(&self, messages: Vec<UiMessage>) -> bool {
        let _ = messages;
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    Completed,
    Aborted,
    Failed,
}

pub struct FinishedTurn {
    pub status: RunStatus,
    pub assistant: Option<UiMessage>,
}

/// Persistence hook; runs before subscribers see the end of the stream, so a client that reloads
/// right after `[DONE]` always finds the saved conversation.
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
#[derive(Debug, Clone)]
pub struct ActiveRun {
    pub conversation_id: Uuid,
    pub run_id: Uuid,
    pub panels_visible: bool,
}

pub struct ConversationRuns {
    activity: Arc<ServerActivity>,
    heartbeat: Duration,
}

impl ConversationRuns {
    /// `heartbeat` = panel SSE keep-alive interval (5 s in production).
    pub fn new(activity: Arc<ServerActivity>, heartbeat: Duration) -> Arc<Self> {
        Arc::new(Self {
            activity,
            heartbeat,
        })
    }

    /// Registers the run synchronously (409 if one is active), then awaits `prepare`. On success
    /// the run drains `Prepared::chunks` in the background whether or not anyone subscribes.
    pub async fn start<F, Fut>(
        self: &Arc<Self>,
        conversation_id: Uuid,
        presentation: RunPresentation,
        prepare: F,
    ) -> Result<Uuid, ChatRejection>
    where
        F: FnOnce(RunHandle) -> Fut + Send,
        Fut: Future<Output = Result<Prepared, ChatRejection>> + Send,
    {
        let _ = (
            conversation_id,
            presentation,
            prepare,
            &self.activity,
            self.heartbeat,
        );
        Err(ChatRejection::Internal(not_implemented()))
    }

    pub async fn prepared_snapshot(&self, conversation_id: Uuid) -> Option<RunSnapshot> {
        let _ = conversation_id;
        None
    }

    /// Replay of the whole run so far, then the live tail.
    pub fn subscribe(&self, conversation_id: Uuid, run_id: Option<Uuid>) -> Resume {
        let _ = (conversation_id, run_id);
        Resume::NoRun
    }

    pub async fn stop(&self, conversation_id: Uuid) -> bool {
        let _ = conversation_id;
        false
    }

    /// Stop, await the run's end (including `on_finish`), then remove the record.
    pub async fn delete(&self, conversation_id: Uuid) -> bool {
        let _ = conversation_id;
        false
    }

    pub fn active_run(&self, conversation_id: Uuid) -> Option<ActiveRun> {
        let _ = conversation_id;
        None
    }

    /// Side-panel effect: tabs a run's tools created join its panel set.
    pub fn associate_tabs(&self, run: &ActiveRun, tab_ids: &[u32]) -> bool {
        let _ = (run, tab_ids);
        false
    }

    pub fn panel_assignments(self: &Arc<Self>) -> PanelStream {
        Box::pin(futures_util::stream::empty())
    }

    pub fn remove_panel_tab(&self, tab_id: u32) {
        let _ = tab_id;
    }

    /// Feeds `/status` `can_update`.
    pub fn running_count(&self) -> usize {
        0
    }
}

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
