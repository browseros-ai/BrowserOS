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
use std::sync::{Arc, Mutex};
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
    /// Display history; the last element is this turn's user message.
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

/// Pre-stream failures, each mapping to today's HTTP status and body dialect.
#[derive(Debug, thiserror::Error)]
pub enum ChatRejection {
    /// 403 `{"error":…}` from the conditional trusted-app check.
    #[error("forbidden")]
    Forbidden,
    /// 400 `{"error":"<text>"}` (provider hydration texts are verbatim TS).
    #[error("{0}")]
    BadRequest(String),
    /// 404: the ACP target's agent id has no row.
    #[error("Agent not found")]
    UnknownAgent,
    /// 400: the stored agent's type differs from the request's.
    #[error("Agent type mismatch")]
    AgentTypeMismatch,
    /// 409: a run is already active for this conversation (or the ACP session is busy).
    #[error("Conversation is busy")]
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
}

/// Test adapter: replays a fixed chunk script per turn and records what it was given, so
/// `ConversationRuns`/`ChatService` can be tested with no model and no process.
pub struct ScriptedEngine {
    script: Vec<UiChunk>,
    turns: Mutex<Vec<Uuid>>,
}

impl ScriptedEngine {
    pub fn new(script: Vec<UiChunk>) -> Self {
        Self {
            script,
            turns: Mutex::new(Vec::new()),
        }
    }

    /// Conversation ids of the turns started so far, in order.
    pub fn started_turns(&self) -> Vec<Uuid> {
        self.turns
            .lock()
            .map(|turns| turns.clone())
            .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl ChatEngine for ScriptedEngine {
    async fn start_turn(&self, turn: Turn) -> Result<ChunkStream, ChatRejection> {
        if let Ok(mut turns) = self.turns.lock() {
            turns.push(turn.conversation_id);
        }
        Ok(Box::pin(futures_util::stream::iter(self.script.clone())))
    }

    async fn forget(&self, conversation_id: Uuid) -> bool {
        let _ = conversation_id;
        false
    }
}
