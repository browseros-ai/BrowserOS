//! The agent seam. `ChatService` (layer 1) hands every turn to a [`ChatEngine`] (layer 2):
//! `BrowserOsEngine` (built-in loop) or `AcpEngine` (external ACP agents), both emitting the same
//! `UiChunk` vocabulary. The two-phase shape mirrors TS `createStream(signal) => ReadableStream`:
//! `start_turn` resolves once the engine is prepared (errors become HTTP statuses), and the
//! returned stream is the turn itself (errors become `error` chunks). Owned by P8.

use super::{
    request::{ChatTurnRequest, ResolvedBrowserContext},
    runs::RunHandle,
};
use crate::db::AcpAgentRow;
use browseros_agent::llm::ChatError;
use futures_util::StreamExt as _;
use std::sync::{Arc, Mutex, PoisonError};
use ui_message_stream::{ChunkStream, UiChunk, UiMessage};
use uuid::Uuid;

/// Everything an engine needs for one turn. History is input only: engines never store it
/// (`ConversationHistory` owns the one display history per conversation).
pub struct Turn {
    pub conversation_id: Uuid,
    /// Run id + cancel token; owned by `ConversationRuns`. Cancelling it is the only way a turn
    /// stops (`/stop`, DELETE) — a client disconnect never cancels.
    pub run: RunHandle,
    pub target: TurnTarget,
    pub request: Arc<ChatTurnRequest>,
    /// Display history; the last element is this turn's user message (raw text — the engine
    /// builds the model-facing copy).
    pub history: Vec<UiMessage>,
    /// Page ids already resolved against the live browser.
    pub browser_context: Option<ResolvedBrowserContext>,
}

pub enum TurnTarget {
    BrowserOs(Box<ResolvedLlm>),
    Acp(AcpAgentRow),
}

/// Credentials and model facts for the built-in engine, resolved exactly as TS `config.ts` +
/// `chat-provider-config.ts` (stored row overrides inline fields; OAuth tokens refreshed).
#[derive(Debug, Clone)]
pub struct ResolvedLlm {
    pub config: browseros_agent::llm::LlmConfig,
    pub info: browseros_agent::ModelInfo,
    pub reasoning: browseros_agent::llm::ReasoningSettings,
}

/// Pre-stream failures, each mapping to today's HTTP status and body (`http::chat` renders them;
/// the `Display` texts are those bodies' messages).
#[derive(Debug, thiserror::Error)]
pub enum ChatRejection {
    /// 403 `{"error":"Forbidden"}` from the conditional trusted-app check.
    #[error("Forbidden")]
    Forbidden,
    /// 400 `{"error":"<text>"}` (provider hydration texts are verbatim TS).
    #[error("{0}")]
    BadRequest(String),
    /// 404 `{"error":"Unknown agent"}`: the ACP target's agent id has no row.
    #[error("Unknown agent")]
    UnknownAgent,
    /// 400 `{"error":"Agent type mismatch"}`: the stored agent's type differs from the request's.
    #[error("Agent type mismatch")]
    AgentTypeMismatch,
    /// 409 `{"error":"An agent turn is already running"}`: a run is active for this conversation
    /// (or the ACP session is busy).
    #[error("An agent turn is already running")]
    Busy,
    /// 500 chat envelope.
    #[error("{}", .0.message)]
    Internal(ChatError),
}

/// The agent seam. Adapters: `BrowserOsEngine`, `AcpEngine`, [`ScriptedEngine`] (tests).
#[async_trait::async_trait]
pub trait ChatEngine: Send + Sync {
    /// Phase 1 is awaited before `POST /chat` answers; errors map to HTTP. The stream is phase 2
    /// and must end promptly once `turn.run.cancel()` fires (with an `abort` chunk).
    async fn start_turn(&self, turn: Turn) -> Result<ChunkStream, ChatRejection>;
    /// `DELETE /chat/{id}`: drop sessions, leases, processes and scheduled pages held for this
    /// conversation. Returns whether anything was held.
    async fn forget(&self, conversation_id: Uuid) -> bool;
    /// Live sessions this engine holds; `DELETE /chat/{id}` reports the built-in engine's count
    /// as `sessionCount` (TS `sessionStore.count()`).
    fn session_count(&self) -> usize {
        0
    }
}

/// The `abort` chunk every engine ends a stopped turn with (`AbortError: Conversation stopped`).
pub fn stopped_chunk() -> UiChunk {
    UiChunk::Abort {
        reason: Some("AbortError: Conversation stopped".to_owned()),
    }
}

/// What a [`ScriptedEngine`] saw of one turn.
#[derive(Debug, Clone)]
pub struct ScriptedTurn {
    pub conversation_id: Uuid,
    pub run_id: Uuid,
    pub history: Vec<UiMessage>,
    pub acp_agent: Option<Uuid>,
    pub browser_context: Option<ResolvedBrowserContext>,
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
                run_id: turn.run.run_id(),
                history: turn.history.clone(),
                acp_agent: match &turn.target {
                    TurnTarget::Acp(agent) => Some(agent.id),
                    TurnTarget::BrowserOs(_) => None,
                },
                browser_context: turn.browser_context.clone(),
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
}
