use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "session_efficiency_stats")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub session_id: String,
    pub ended_at: i64,
    pub dispatch_count: i64,
    pub active_duration_ms: i64,
    pub tool_input_token_estimate: i64,
    pub tool_output_token_estimate: i64,
    pub screenshot_baseline_token_estimate: i64,
    pub efficiency_estimator_version: i64,
    pub computed_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
