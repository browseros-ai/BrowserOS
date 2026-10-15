//! The single display history per conversation, shared by both engines (TS kept two with subtly
//! different rules: `session.agent.messages` and `acpMessages`).
//!
//! - Hydrate (first turn of a conversation/target in this process): the stored row is used iff it
//!   belongs to the request's target — `target_type = browseros` for the built-in agent, the same
//!   `agent_id` for an ACP agent — else the client's `previousConversation` seeds it.
//! - Afterwards the in-memory copy is authoritative until `DELETE /chat/{id}` forgets it.
//! - Persist: best-effort, never fails the turn; the caller decides *whether* (see
//!   `ChatService`), this module only writes.
//!
//! Owned by P8.

use super::request::PreviousMessage;
use crate::db::{AgentKind, SaveConversation, TargetType};
use browseros_agent::prompt::Origin;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};
use ui_message_stream::{Role, TextPart, UiMessage, UiPart};
use uuid::Uuid;

use super::ChatRecords;

/// Whose history a cached copy is: the built-in agent's, or one ACP agent's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TargetKey {
    BrowserOs,
    Acp { kind: AgentKind, agent_id: Uuid },
}

/// In-memory cache keyed by (conversation, target) in front of the `conversations` table.
pub struct ConversationHistory {
    records: Arc<dyn ChatRecords>,
    cache: Mutex<HashMap<Uuid, HashMap<TargetKey, Vec<UiMessage>>>>,
}

impl ConversationHistory {
    pub fn new(records: Arc<dyn ChatRecords>) -> Self {
        Self {
            records,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The history this turn continues. `hydrate_from_db` is false when the conversation is not
    /// persisted (the client then owns history and ships `previousConversation`). A failed read
    /// is logged and falls back to `previous`, so a bad row never blocks chatting.
    pub async fn load_or_seed(
        &self,
        conversation_id: Uuid,
        target: &TargetKey,
        hydrate_from_db: bool,
        previous: &[PreviousMessage],
    ) -> Vec<UiMessage> {
        if let Some(cached) = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&conversation_id)
            .and_then(|targets| targets.get(target))
        {
            return cached.clone();
        }
        if hydrate_from_db {
            match self.records.conversation(conversation_id).await {
                Ok(Some(stored))
                    if !stored.messages.is_empty() && belongs_to(&stored.summary, target) =>
                {
                    tracing::info!(
                        %conversation_id,
                        message_count = stored.messages.len(),
                        "hydrated conversation history from database"
                    );
                    return stored.messages;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%conversation_id, %error, "failed to load conversation history")
                }
            }
        }
        seed(previous)
    }

    pub fn replace(&self, conversation_id: Uuid, target: &TargetKey, messages: Vec<UiMessage>) {
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(conversation_id)
            .or_default()
            .insert(target.clone(), messages);
    }

    /// Best-effort: logs, never fails the turn.
    pub async fn persist(
        &self,
        conversation_id: Uuid,
        target: &TargetKey,
        messages: &[UiMessage],
        origin: Origin,
    ) {
        let (target_type, agent_id) = match target {
            TargetKey::BrowserOs => (TargetType::BrowserOs, None),
            TargetKey::Acp { agent_id, .. } => (TargetType::Acp, Some(agent_id.to_string())),
        };
        let save = SaveConversation {
            id: conversation_id,
            messages: messages.to_vec(),
            origin: Some(
                match origin {
                    Origin::Sidepanel => "sidepanel",
                    Origin::Newtab => "newtab",
                }
                .to_owned(),
            ),
            target_type,
            agent_id,
        };
        if let Err(error) = self.records.save_conversation(save).await {
            tracing::warn!(%conversation_id, %error, "failed to persist conversation history");
        }
    }

    /// Drops every cached history of the conversation (all targets).
    pub fn forget(&self, conversation_id: Uuid) {
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&conversation_id);
    }
}

/// A stored row is reused only by the target that wrote it: a conversation id shared between the
/// built-in agent and an ACP agent (or two ACP agents) must not bleed history across them.
fn belongs_to(summary: &crate::db::ConversationSummary, target: &TargetKey) -> bool {
    match target {
        TargetKey::BrowserOs => summary.target_type == Some(TargetType::BrowserOs),
        TargetKey::Acp { agent_id, .. } => {
            summary.agent_id.as_deref() == Some(agent_id.to_string().as_str())
        }
    }
}

/// `previousConversation` as text-only UI messages with fresh ids; blank entries are skipped.
fn seed(previous: &[PreviousMessage]) -> Vec<UiMessage> {
    previous
        .iter()
        .filter(|message| !message.content.trim().is_empty())
        .map(|message| UiMessage {
            id: Uuid::new_v4().to_string(),
            role: match message.role {
                server_api::PreviousMessageRole::Assistant => Role::Assistant,
                server_api::PreviousMessageRole::User => Role::User,
            },
            metadata: None,
            parts: vec![UiPart::Text(TextPart {
                text: message.content.clone(),
                state: None,
                provider_metadata: None,
            })],
        })
        .collect()
}
