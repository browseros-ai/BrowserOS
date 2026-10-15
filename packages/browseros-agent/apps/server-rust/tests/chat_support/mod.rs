//! Test adapters for the chat seams: a [`ScriptedEngine`] standing in for both `ChatEngine`
//! adapters and a [`NoBrowser`] standing in for the shared host's pages.

use futures_util::StreamExt as _;
use server_rust::chat::{
    BackgroundPages, ChatEngine, ChatRejection, Turn, TurnTarget, stopped_chunk,
};
use std::sync::{Mutex, PoisonError};
use ui_message_stream::{ChunkStream, UiChunk, UiMessage};
use uuid::Uuid;

/// What a [`ScriptedEngine`] saw of one turn.
#[derive(Debug, Clone)]
pub struct ScriptedTurn {
    pub conversation_id: Uuid,
    pub history: Vec<UiMessage>,
    pub acp_agent: Option<Uuid>,
}

/// Test adapter: replays a fixed chunk script per turn (ending with an `abort` chunk if the run
/// is cancelled mid-script, as real engines do) and records what it was given, so
/// `ConversationRuns`/`ChatService` can be tested with no model and no process.
pub struct ScriptedEngine {
    script: Vec<UiChunk>,
    rejection: Mutex<Option<ChatRejection>>,
    turns: Mutex<Vec<ScriptedTurn>>,
    forgotten: Mutex<Vec<Uuid>>,
}

impl ScriptedEngine {
    pub fn new(script: Vec<UiChunk>) -> Self {
        Self {
            script,
            rejection: Mutex::new(None),
            turns: Mutex::new(Vec::new()),
            forgotten: Mutex::new(Vec::new()),
        }
    }

    /// The next `start_turn` fails with `rejection` (once).
    pub fn reject_next(&self, rejection: ChatRejection) {
        *self
            .rejection
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(rejection);
    }

    /// Conversation ids of the turns started so far, in order.
    pub fn started_turns(&self) -> Vec<Uuid> {
        self.turns()
            .into_iter()
            .map(|turn| turn.conversation_id)
            .collect()
    }

    pub fn turns(&self) -> Vec<ScriptedTurn> {
        self.turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn forgotten(&self) -> Vec<Uuid> {
        self.forgotten
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl ChatEngine for ScriptedEngine {
    async fn start_turn(&self, turn: Turn) -> Result<ChunkStream, ChatRejection> {
        self.turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ScriptedTurn {
                conversation_id: turn.conversation_id,
                history: turn.history.clone(),
                acp_agent: match &turn.target {
                    TurnTarget::Acp(agent) => Some(agent.id),
                    TurnTarget::BrowserOs(_) => None,
                },
            });
        if let Some(rejection) = self
            .rejection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            return Err(rejection);
        }
        let cancel = turn.run.cancel().clone();
        let script = futures_util::stream::iter(self.script.clone());
        // Each chunk is yielded only while the run is live; a cancelled run gets one `abort`.
        let stream = futures_util::stream::unfold(
            (script, cancel, false),
            |(mut script, cancel, done)| async move {
                if done {
                    return None;
                }
                if cancel.is_cancelled() {
                    return Some((stopped_chunk(), (script, cancel, true)));
                }
                let chunk = script.next().await?;
                Some((chunk, (script, cancel, false)))
            },
        );
        Ok(Box::pin(stream))
    }

    async fn forget(&self, conversation_id: Uuid) -> bool {
        let mut forgotten = self
            .forgotten
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        forgotten.push(conversation_id);
        self.turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|turn| turn.conversation_id == conversation_id)
    }

    /// Conversations that ran a turn and were not forgotten since.
    fn session_count(&self) -> usize {
        let forgotten = self.forgotten();
        let mut live: Vec<Uuid> = self
            .started_turns()
            .into_iter()
            .filter(|id| !forgotten.contains(id))
            .collect();
        live.sort_unstable();
        live.dedup();
        live.len()
    }
}

/// A [`BackgroundPages`] with no browser behind it: every call fails, so scheduled runs fall
/// back to the request's browser context, as TS does when `newPage` fails.
pub struct NoBrowser;

#[async_trait::async_trait]
impl BackgroundPages for NoBrowser {
    async fn open_background(&self, url: &str) -> anyhow::Result<u32> {
        anyhow::bail!("browser is not connected; cannot open {url}")
    }

    async fn window_of(&self, page_id: u32) -> anyhow::Result<Option<i64>> {
        anyhow::bail!("browser is not connected; cannot look up page {page_id}")
    }

    async fn close(&self, page_id: u32) -> anyhow::Result<()> {
        anyhow::bail!("browser is not connected; cannot close page {page_id}")
    }
}
