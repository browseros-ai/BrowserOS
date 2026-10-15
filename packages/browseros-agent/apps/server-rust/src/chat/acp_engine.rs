//! `AcpEngine`: a thin adapter from `Turn` to `acp_agents::AcpRuntime::stream` (P16, after
//! `acp-agents` exists). It mints/refreshes the conversation's ACP lease (re-minted only when
//! `{agentId, readOnly}` changes) and maps `RuntimeError::Busy` → 409, preparation failures → one
//! `error` chunk. S0 stub: every turn is a single `error` chunk, never a fake success.

use super::engine::{ChatEngine, ChatRejection, Turn};
use ui_message_stream::{ChunkStream, UiChunk};
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct AcpEngine {
    _private: (),
}

impl AcpEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// `/agents/{id}` update and delete close every live session of that agent. Returns how many
    /// sessions were closed.
    pub async fn close_all_for_agent(
        &self,
        agent_id: Uuid,
        discard_persistent_state: bool,
    ) -> usize {
        let _ = (agent_id, discard_persistent_state);
        0
    }

    pub async fn shutdown(&self) {}
}

#[async_trait::async_trait]
impl ChatEngine for AcpEngine {
    async fn start_turn(&self, turn: Turn) -> Result<ChunkStream, ChatRejection> {
        let _ = turn;
        Ok(Box::pin(futures_util::stream::iter([UiChunk::Error {
            error_text: "ACP not wired".to_owned(),
        }])))
    }

    async fn forget(&self, conversation_id: Uuid) -> bool {
        let _ = conversation_id;
        false
    }
}
