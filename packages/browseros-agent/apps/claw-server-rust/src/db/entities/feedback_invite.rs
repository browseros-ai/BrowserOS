use sea_orm::entity::prelude::*;

/// One row per installation that has been offered a feedback call, ever.
///
/// The install id is the primary key, which is the whole frequency rule: a second
/// invitation cannot be recorded, so it cannot be offered.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "feedback_invite")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub install_id: String,
    pub shown_at_ms: i64,
    pub outcome: String,
    pub settled_at_ms: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
