use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "session_tabs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub session_id: String,
    pub agent_id: String,
    pub tab_id: i64,
    pub opened_target_id: Option<String>,
    pub claimed_at: i64,
    pub released_at: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
