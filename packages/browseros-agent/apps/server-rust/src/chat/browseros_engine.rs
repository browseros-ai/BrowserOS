//! `BrowserOsEngine`: the built-in agent behind the seam. Caches one `browseros_agent::Agent` per
//! conversation, rebuilding it (with `[Context: …]` notices) when the MCP set, workspace or mode
//! changes; mints the conversation's tool lease for loopback `/mcp`; opens and closes the
//! scheduled run's `about:blank` page; builds the per-turn prompt copy. Implemented by P14.

use super::engine::{ChatEngine, ChatRejection, Turn};
use browseros_agent::llm::HttpStack;
use ui_message_stream::ChunkStream;
use uuid::Uuid;

pub struct BrowserOsEngine {
    http: HttpStack,
    /// `BROWSEROS_USE_MOCK_LLM=true` swaps every model for `ScriptedModel::mock()`.
    use_mock_llm: bool,
}

impl BrowserOsEngine {
    pub fn new(http: HttpStack, use_mock_llm: bool) -> Self {
        Self { http, use_mock_llm }
    }
}

#[async_trait::async_trait]
impl ChatEngine for BrowserOsEngine {
    async fn start_turn(&self, turn: Turn) -> Result<ChunkStream, ChatRejection> {
        let _ = (turn, &self.http, self.use_mock_llm);
        Err(ChatRejection::Internal(super::runs::not_implemented()))
    }

    async fn forget(&self, conversation_id: Uuid) -> bool {
        let _ = conversation_id;
        false
    }
}
