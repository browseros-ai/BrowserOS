use sea_orm::entity::prelude::*;

/// A skill draft written from one finished task. `status` is `writing`, `ready` or `failed`;
/// `request_id` is what the decisions worker counts the draft by, reused when it is retried.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "skill_drafts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub session_id: String,
    pub status: String,
    pub request_id: String,
    pub draft_json: Option<String>,
    pub failure_json: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
