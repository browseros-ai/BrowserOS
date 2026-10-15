//! `conversations`: one persisted display history (`messages`, JSON `UIMessage[]` text) per
//! conversation, tagged with the target that wrote it.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "conversations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub messages: String,
    pub last_user_message: Option<String>,
    pub origin: Option<String>,
    pub target_type: String,
    pub agent_id: Option<String>,
    pub last_messaged_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
