//! The single display history per conversation, shared by both engines (TS kept two with subtly
//! different rules). Hydrate rule: the stored row is used iff its `(target_type, agent_id)`
//! matches the request's target, else `previousConversation` seeds it. Persist rule: iff the
//! history ends in an assistant message after `filter_valid` (built-in also honours
//! scheduled/`persist`/`historyMode`). Owned by P8.

use super::request::PreviousMessage;
use crate::db::{AgentKind, ConversationStore};
use browseros_agent::prompt::Origin;
use ui_message_stream::UiMessage;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TargetKey {
    BrowserOs,
    Acp { kind: AgentKind, agent_id: Uuid },
}

/// In-memory cache keyed by (conversation, target) in front of `ConversationStore`.
pub struct ConversationHistory {
    store: ConversationStore,
}

impl ConversationHistory {
    pub fn new(store: ConversationStore) -> Self {
        Self { store }
    }

    pub async fn load_or_seed(
        &self,
        conversation_id: Uuid,
        target: &TargetKey,
        hydrate_from_db: bool,
        previous: &[PreviousMessage],
    ) -> Vec<UiMessage> {
        let _ = (
            &self.store,
            conversation_id,
            target,
            hydrate_from_db,
            previous,
        );
        Vec::new()
    }

    pub fn replace(&self, conversation_id: Uuid, target: &TargetKey, messages: Vec<UiMessage>) {
        let _ = (conversation_id, target, messages);
    }

    /// Best-effort: logs, never fails the turn.
    pub async fn persist(
        &self,
        conversation_id: Uuid,
        target: &TargetKey,
        messages: &[UiMessage],
        origin: Origin,
    ) {
        let _ = (conversation_id, target, messages, origin);
    }

    pub fn forget(&self, conversation_id: Uuid) {
        let _ = conversation_id;
    }
}
