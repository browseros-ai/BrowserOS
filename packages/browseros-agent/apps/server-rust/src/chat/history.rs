//! The single history per conversation, shared by both engines (TS kept two with subtly
//! different rules: `session.agent.messages` and `acpMessages`).
//!
//! A history has two forms. The *stored* form is the durable `UIMessage[]` JSON exactly as the
//! row holds it: rows carry shapes this build cannot decode (id-less imports, AI SDK v4
//! `tool-invocation` parts and `call` states, `data` messages), and every save writes them back
//! verbatim with only the new turn appended — never re-serialized through the typed form. The
//! *typed* form is its decodable subset, in order: what the engine reads and panels show. The
//! cache holds the stored form; the typed view is decoded from it for each turn.
//!
//! - Hydrate (first turn of a conversation/target in this process): the stored row is used iff it
//!   belongs to the request's target — `target_type = browseros` for the built-in agent, the same
//!   `agent_id` for an ACP agent — else the client's `previousConversation` seeds it. A row that
//!   cannot be read fails the turn (TS answers 500 there): seeding instead would let the turn's
//!   persist overwrite a history this process never saw.
//! - Afterwards the in-memory copy is authoritative until `DELETE /chat/{id}` forgets it.
//! - Persist: best-effort, never fails the turn; the caller decides *whether* (see
//!   `ChatService`), this module only writes.
//!
//! Owned by P8.

use super::request::PreviousMessage;
use crate::db::{AgentKind, DbResult, SaveRawConversation, TargetType};
use browseros_agent::prompt::Origin;
use serde::Deserialize as _;
use serde_json::Value;
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

/// A conversation's history as a turn starts from it, in both forms (see the module docs).
#[derive(Debug, Clone, Default)]
pub struct History {
    /// The durable form, verbatim.
    pub stored: Vec<Value>,
    /// The decodable subset of `stored`, in order.
    pub typed: Vec<UiMessage>,
}

impl History {
    /// Decodes message by message; returns how many stored messages had no typed form.
    fn from_stored(stored: Vec<Value>) -> (Self, usize) {
        let typed: Vec<UiMessage> = stored
            .iter()
            .filter_map(|message| UiMessage::deserialize(message).ok())
            .collect();
        let skipped = stored.len() - typed.len();
        (Self { stored, typed }, skipped)
    }

    fn from_typed(typed: Vec<UiMessage>) -> Self {
        Self {
            stored: to_stored(&typed),
            typed,
        }
    }

    /// The stored form with this turn's messages appended; the stored prefix is untouched.
    pub fn stored_with(&self, turn: &[UiMessage]) -> Vec<Value> {
        let mut stored = self.stored.clone();
        stored.extend(to_stored(turn));
        stored
    }
}

/// In-memory cache keyed by (conversation, target) in front of the `conversations` table. It
/// holds the stored form, so a turn served from cache appends to the row's verbatim messages too.
pub struct ConversationHistory {
    records: Arc<dyn ChatRecords>,
    cache: Mutex<HashMap<Uuid, HashMap<TargetKey, Vec<Value>>>>,
}

impl ConversationHistory {
    pub fn new(records: Arc<dyn ChatRecords>) -> Self {
        Self {
            records,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The history this turn continues. `hydrate_from_db` is false when the conversation is not
    /// persisted (the client then owns history and ships `previousConversation`). A stored
    /// message with no typed form is kept in `stored` and left out of `typed`, so one legacy
    /// entry never blocks chatting; a row that cannot be read at all is an error, never a silent
    /// fresh start.
    pub async fn load_or_seed(
        &self,
        conversation_id: Uuid,
        target: &TargetKey,
        hydrate_from_db: bool,
        previous: &[PreviousMessage],
    ) -> DbResult<History> {
        let cached = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&conversation_id)
            .and_then(|targets| targets.get(target))
            .cloned();
        if let Some(stored) = cached {
            return Ok(History::from_stored(stored).0);
        }
        if hydrate_from_db {
            let stored = self
                .records
                .conversation(conversation_id)
                .await
                .inspect_err(|error| {
                    tracing::warn!(%conversation_id, %error, "failed to load conversation history");
                })?;
            if let Some(stored) = stored
                && !stored.messages.is_empty()
                && belongs_to(&stored.summary, target)
            {
                let (history, skipped) = History::from_stored(stored.messages);
                if skipped > 0 {
                    tracing::warn!(
                        %conversation_id,
                        skipped,
                        "stored messages that are not UIMessages stay in the row but are not \
                         sent to the agent"
                    );
                }
                tracing::info!(
                    %conversation_id,
                    message_count = history.stored.len(),
                    "hydrated conversation history from database"
                );
                return Ok(history);
            }
        }
        Ok(History::from_typed(seed(previous)))
    }

    /// Makes `stored` the conversation's history for `target` in this process.
    pub fn replace(&self, conversation_id: Uuid, target: &TargetKey, stored: Vec<Value>) {
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(conversation_id)
            .or_default()
            .insert(target.clone(), stored);
    }

    /// Writes `stored` verbatim. Best-effort: logs, never fails the turn.
    pub async fn persist(
        &self,
        conversation_id: Uuid,
        target: &TargetKey,
        stored: &[Value],
        origin: Origin,
    ) {
        let (target_type, agent_id) = match target {
            TargetKey::BrowserOs => (TargetType::BrowserOs, None),
            // TS writes the agent's type (`claude|codex|custom`) as the row's target.
            TargetKey::Acp { kind, agent_id } => (
                match kind {
                    AgentKind::Claude => TargetType::Claude,
                    AgentKind::Codex => TargetType::Codex,
                    AgentKind::Custom => TargetType::Custom,
                },
                Some(agent_id.to_string()),
            ),
        };
        let save = SaveRawConversation {
            id: conversation_id,
            messages: stored.to_vec(),
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
        TargetKey::BrowserOs => summary.target_type == TargetType::BrowserOs,
        TargetKey::Acp { agent_id, .. } => {
            summary.agent_id.as_deref() == Some(agent_id.to_string().as_str())
        }
    }
}

/// Messages this process built, in the stored form. Serializing a `UiMessage` cannot fail in
/// practice; one that did would be logged and left out rather than abort the turn.
fn to_stored(messages: &[UiMessage]) -> Vec<Value> {
    messages
        .iter()
        .filter_map(|message| {
            serde_json::to_value(message)
                .inspect_err(|error| {
                    tracing::error!(message_id = %message.id, %error, "unserializable message");
                })
                .ok()
        })
        .collect()
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
