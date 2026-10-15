//! `conversations` rows: the one persisted display history per conversation, tagged with the
//! target that wrote it (`target_type`, `agent_id`) so hydration can tell whose history it is.

use super::{Database, DbError, DbResult, STUB};
use ui_message_stream::UiMessage;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetType {
    BrowserOs,
    Acp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationSummary {
    pub id: Uuid,
    pub last_user_message: Option<String>,
    pub origin: Option<String>,
    pub target_type: Option<TargetType>,
    pub agent_id: Option<String>,
    pub last_messaged_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredConversation {
    pub summary: ConversationSummary,
    pub messages: Vec<UiMessage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SaveConversation {
    pub id: Uuid,
    pub messages: Vec<UiMessage>,
    pub origin: Option<String>,
    pub target_type: TargetType,
    pub agent_id: Option<String>,
}

/// `PUT /conversations/{id}` (legacy import; messages are not validated, arrays included).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportConversation {
    pub messages: Vec<serde_json::Value>,
    pub origin: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConversationStore {
    db: Database,
}

impl ConversationStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn list(&self) -> DbResult<Vec<ConversationSummary>> {
        let _ = &self.db;
        Err(DbError::Unavailable(STUB))
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<StoredConversation>> {
        let _ = id;
        Err(DbError::Unavailable(STUB))
    }

    pub async fn save(&self, input: SaveConversation) -> DbResult<()> {
        let _ = input;
        Err(DbError::Unavailable(STUB))
    }

    /// `None` when a row already exists (the route answers with the existing summary flag).
    pub async fn import_if_absent(
        &self,
        id: Uuid,
        input: ImportConversation,
    ) -> DbResult<Option<ConversationSummary>> {
        let _ = (id, input);
        Err(DbError::Unavailable(STUB))
    }

    pub async fn delete(&self, id: Uuid) -> DbResult<bool> {
        let _ = id;
        Err(DbError::Unavailable(STUB))
    }
}
