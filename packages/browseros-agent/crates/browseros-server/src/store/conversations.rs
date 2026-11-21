//! The `conversations` table: one row per conversation, the display transcript as a JSON
//! `UIMessage[]`. Written only by `conversations`, one run per conversation, so one writer per
//! row (the layering test enforces it).
//!
//! Rows the old server wrote are read as they are and never rewritten on read. One fix is made
//! in memory: the old server stored `""` as every assistant message's id, and the panel needs
//! distinct ids, so an empty id reads as `legacy-<position>`, which is stable for a given row.

use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveValue, EntityTrait, QueryOrder};

use super::entities::conversations::{ActiveModel, Column, Entity};
use super::{Store, StoreError};
use crate::turn::{ConversationId, MessageId, Role, UiMessage, UiPart};

/// Written only by `conversations` (the layering test enforces it): one run per
/// conversation, so one writer per row.
pub struct ConversationRows<'a>(pub(super) &'a Store);

/// A whole stored conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationRow {
    pub id: ConversationId,
    pub messages: Vec<UiMessage>,
    /// `browseros`, `claude` or `codex` (legacy `custom` rows stay readable).
    pub target_type: String,
    pub agent_id: Option<String>,
}

/// A row for the history list: no transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationSummary {
    pub id: ConversationId,
    /// The first 200 characters of the last user text.
    pub last_user_message: Option<String>,
    pub origin: Option<String>,
    pub target_type: String,
    pub agent_id: Option<String>,
    /// Epoch milliseconds, like every time in the table.
    pub last_messaged_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// How much of the last user message the history list shows.
const LAST_USER_MESSAGE_CHARS: usize = 200;

impl ConversationRows<'_> {
    /// Every conversation, newest first. A row whose id is not a UUID cannot be addressed by
    /// any route, so it is skipped with a warning rather than failing the list.
    pub async fn list(&self) -> Result<Vec<ConversationSummary>, StoreError> {
        let rows = Entity::find()
            .order_by_desc(Column::LastMessagedAt)
            .all(self.0.db())
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| match uuid::Uuid::parse_str(&row.id) {
                Ok(id) => Some(ConversationSummary {
                    id,
                    last_user_message: row.last_user_message,
                    origin: row.origin,
                    target_type: row.target_type,
                    agent_id: row.agent_id,
                    last_messaged_at: row.last_messaged_at,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                }),
                Err(_) => {
                    tracing::warn!(
                        id = row.id,
                        "skipping a conversation whose id is not a UUID"
                    );
                    None
                }
            })
            .collect())
    }

    pub async fn load(&self, id: ConversationId) -> Result<Option<ConversationRow>, StoreError> {
        let Some(row) = Entity::find_by_id(id.to_string()).one(self.0.db()).await? else {
            return Ok(None);
        };
        let mut messages: Vec<UiMessage> = serde_json::from_str(&row.messages)
            .map_err(|error| StoreError::Unreadable(format!("conversation {id}: {error}")))?;
        for (position, message) in messages.iter_mut().enumerate() {
            if message.id.0.is_empty() {
                message.id = MessageId(format!("legacy-{position}"));
            }
        }
        Ok(Some(ConversationRow {
            id,
            messages,
            target_type: row.target_type,
            agent_id: row.agent_id,
        }))
    }

    /// Upserts the whole transcript and the derived columns. Idempotent. `created_at` and
    /// `origin` of an existing row are kept; INSERT … ON CONFLICT, never REPLACE.
    pub async fn save(&self, row: &ConversationRow) -> Result<(), StoreError> {
        let now = now_ms();
        let messages = serde_json::to_string(&row.messages)
            .map_err(|error| StoreError::Unreadable(format!("conversation {}: {error}", row.id)))?;
        let model = ActiveModel {
            id: ActiveValue::Set(row.id.to_string()),
            messages: ActiveValue::Set(messages),
            last_user_message: ActiveValue::Set(last_user_text(&row.messages)),
            origin: ActiveValue::Set(None),
            target_type: ActiveValue::Set(row.target_type.clone()),
            agent_id: ActiveValue::Set(row.agent_id.clone()),
            last_messaged_at: ActiveValue::Set(now),
            created_at: ActiveValue::Set(now),
            updated_at: ActiveValue::Set(now),
        };
        Entity::insert(model)
            .on_conflict(
                OnConflict::column(Column::Id)
                    .update_columns([
                        Column::Messages,
                        Column::LastUserMessage,
                        Column::TargetType,
                        Column::AgentId,
                        Column::LastMessagedAt,
                        Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec(self.0.db())
            .await?;
        Ok(())
    }

    /// Whether a row was deleted.
    pub async fn delete(&self, id: ConversationId) -> Result<bool, StoreError> {
        let result = Entity::delete_by_id(id.to_string())
            .exec(self.0.db())
            .await?;
        Ok(result.rows_affected > 0)
    }
}

/// The first 200 characters of the last user message's text, for the history list.
fn last_user_text(messages: &[UiMessage]) -> Option<String> {
    let message = messages
        .iter()
        .rev()
        .find(|message| message.role == Role::User)?;
    let text: String = message
        .parts
        .iter()
        .filter_map(|part| match part {
            UiPart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    let text: String = text.chars().take(LAST_USER_MESSAGE_CHARS).collect();
    (!text.is_empty()).then_some(text)
}

fn now_ms() -> i64 {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
}
