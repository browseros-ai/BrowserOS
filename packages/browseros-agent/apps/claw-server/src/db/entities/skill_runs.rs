use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "skill_runs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub skill_name: String,
    pub session_id: String,
    pub run_number: i64,
    pub agent_id: String,
    pub tokens: Option<i64>,
    pub duration_ms: Option<i64>,
    pub tool_count: Option<i64>,
    pub clean: bool,
    pub errored_tool: Option<String>,
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
