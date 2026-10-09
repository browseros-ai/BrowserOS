use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_session_ends")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub created_at: i64,
    pub session_id: String,
    pub kind: String,
    pub reason: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
