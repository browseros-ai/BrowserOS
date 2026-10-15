//! The chat conversation layer (layer 1 of the agent seam): hydrates the target, owns the run
//! registry (409, replay, panels), the one display history per conversation, chunk → UIMessage
//! assembly and persistence, then hands each turn to a [`ChatEngine`].
//!
//! ```text
//! POST /chat ─► ChatService::start ─► ConversationRuns::start (409 gate, before any await)
//!                                      └─ prepare: history + ChatEngine::start_turn(Turn)
//!                                           ├─ BrowserOsEngine (built-in loop)   ─┐ ChunkStream
//!                                           └─ AcpEngine (ACP agents)            ─┘   │
//!   replay buffer + live fan-out ◄── UiMessageAssembler ◄─────────────────────────────┘
//! ```
//!
//! Owned by P8 (`mod`, `runs`, `history`, `request`, `engine`), P14 (`browseros_engine`),
//! P16 (`acp_engine`). S0 ships the seam with stubs that reject every turn.

mod acp_engine;
mod browseros_engine;
pub mod engine;
pub mod history;
pub mod request;
pub mod runs;

pub use acp_engine::AcpEngine;
pub use browseros_engine::BrowserOsEngine;
pub use engine::{ChatEngine, ChatRejection, ResolvedLlm, ScriptedEngine, Turn, TurnTarget};
pub use history::{ConversationHistory, TargetKey};
pub use request::{ChatRequest, ChatTurnRequest, RequestTarget};
pub use runs::{ActiveRun, ConversationRuns, RunHandle, RunSnapshot, RunStatus};

use crate::{
    db::{AgentStore, ProviderStore},
    http::security::RequestTrust,
    oauth::TokenManager,
};
use std::{pin::Pin, sync::Arc};
use ui_message_stream::ChunkStream;
use uuid::Uuid;

/// Outcome of `GET /chat/{id}/stream`.
pub enum Resume {
    /// 204: no run record for this conversation.
    NoRun,
    /// 409: the caller's `runId` is not the current run.
    RunChanged,
    /// Replay of the run so far, then the live tail.
    Stream(ChunkStream),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteOutcome {
    pub deleted: bool,
    pub session_count: usize,
}

/// `GET /chat/panels` events: an assignments snapshot on every change, plus a heartbeat (5 s).
pub enum PanelEvent {
    Assignments(server_api::ConversationPanelAssignments),
    Heartbeat,
}

pub type PanelStream = Pin<Box<dyn futures_core::Stream<Item = PanelEvent> + Send + 'static>>;

/// What `ChatService` is built from. `browser` (the live `BrowserService`, for page-id
/// resolution and scheduled pages) arrives with the MCP host (H1/P15).
pub struct ChatDeps {
    pub runs: Arc<ConversationRuns>,
    pub history: ConversationHistory,
    pub browseros: Arc<dyn ChatEngine>,
    pub acp: Arc<dyn ChatEngine>,
    pub providers: ProviderStore,
    pub agents: AgentStore,
    pub oauth: Arc<TokenManager>,
    pub browseros_id: String,
}

pub struct ChatService {
    deps: ChatDeps,
}

impl ChatService {
    pub fn new(deps: ChatDeps) -> Arc<Self> {
        Arc::new(Self { deps })
    }

    /// Error order is TS's and is part of the contract: trust + provider-row hydration (403 / 400
    /// text) → run registration (409) → inside prepare: ACP agent lookup (404 / 400 mismatch) →
    /// credentials (OAuth refresh) and engine prepare (500 envelope, or 409 ACP busy). History is
    /// published before the engine is called.
    pub async fn start(
        &self,
        request: ChatRequest,
        trust: RequestTrust,
    ) -> Result<ChunkStream, ChatRejection> {
        let _ = (request, trust, &self.deps);
        Err(ChatRejection::Internal(runs::not_implemented()))
    }

    /// Waits on the prepared gate, so `/state` never shows a half-registered run.
    pub async fn state(&self, conversation_id: Uuid) -> Option<RunSnapshot> {
        self.deps.runs.prepared_snapshot(conversation_id).await
    }

    pub fn resume(&self, conversation_id: Uuid, run_id: Option<Uuid>) -> Resume {
        self.deps.runs.subscribe(conversation_id, run_id)
    }

    pub async fn stop(&self, conversation_id: Uuid) -> bool {
        self.deps.runs.stop(conversation_id).await
    }

    /// Stops and removes the run, then asks both engines to forget the conversation.
    pub async fn delete(&self, conversation_id: Uuid) -> DeleteOutcome {
        let _ = conversation_id;
        DeleteOutcome {
            deleted: false,
            session_count: 0,
        }
    }

    /// DELETE requires the trusted-app check only for ACP conversations.
    pub fn is_acp_conversation(&self, conversation_id: Uuid) -> bool {
        let _ = conversation_id;
        false
    }

    pub fn panel_assignments(&self) -> PanelStream {
        self.deps.runs.panel_assignments()
    }

    pub fn remove_panel_tab(&self, tab_id: u32) {
        self.deps.runs.remove_panel_tab(tab_id);
    }
}
