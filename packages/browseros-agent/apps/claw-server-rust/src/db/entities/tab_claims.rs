use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "tab_claims")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub target_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub claimed_at: i64,
    pub released_at: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
