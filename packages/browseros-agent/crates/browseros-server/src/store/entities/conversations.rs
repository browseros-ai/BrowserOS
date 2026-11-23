//! One conversation. `messages` is the display transcript, a JSON array of AI SDK `UIMessage`;
//! it is the only copy of the history and the only thing a reload reads. The model's context is
//! derived from it at the start of a turn.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "conversations")]
pub struct Model {
    /// The conversation UUID the extension made.
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub messages: String,
    /// The first 200 characters of the last user text, for the history list.
    pub last_user_message: Option<String>,
    pub origin: Option<String>,
    /// `browseros`, `claude` or `codex` (legacy `custom` rows stay readable).
    pub target_type: String,
    pub agent_id: Option<String>,
    /// Epoch milliseconds.
    pub last_messaged_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
