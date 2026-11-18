use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "task_groups")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub group_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub origin: String,
    pub window_id: Option<String>,
    pub profile_id: Option<String>,
    pub created_at: i64,
    pub ended_at: Option<i64>,
    pub name: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
