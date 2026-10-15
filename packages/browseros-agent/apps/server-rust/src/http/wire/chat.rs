//! Chat-route response shapes the generated `server_api` models cannot express byte-faithfully:
//! the generator types counts as `f64` (serde writes `3.0`, JS writes `3`) and the run snapshot's
//! messages as the loose `ImportedMessage`. Field order is the TS objects' construction order.

use crate::chat::RunSnapshot;
use serde::Serialize;
use server_api::ConversationRunStatus;
use ui_message_stream::UiMessage;

/// `GET /chat/{conversationId}/state` body (`ConversationRunSnapshot`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSnapshotBody {
    pub conversation_id: String,
    pub run_id: String,
    pub status: ConversationRunStatus,
    pub messages: Vec<UiMessage>,
    pub chunk_count: usize,
}

impl From<RunSnapshot> for RunSnapshotBody {
    fn from(snapshot: RunSnapshot) -> Self {
        Self {
            conversation_id: snapshot.conversation_id.to_string(),
            run_id: snapshot.run_id.to_string(),
            status: snapshot.status.as_wire(),
            messages: snapshot.messages,
            chunk_count: snapshot.chunk_count,
        }
    }
}

/// `DELETE /chat/{conversationId}` success body.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteChatBody {
    pub success: bool,
    pub message: String,
    pub session_count: usize,
}
