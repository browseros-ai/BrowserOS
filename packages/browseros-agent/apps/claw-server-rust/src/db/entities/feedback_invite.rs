use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "feedback_invite")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub install_id: String,
    pub shown_at_ms: i64,
    pub outcome: String,
    pub settled_at_ms: Option<i64>,
    pub dismissed_at_ms: Option<i64>,
    pub round: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
