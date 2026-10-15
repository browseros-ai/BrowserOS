//! `scheduled_job_runs`: `job_id` references `scheduled_jobs.id` ON DELETE CASCADE; `tool_calls`
//! is JSON `ToolCallExecution[]` text.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "scheduled_job_runs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub job_id: String,
    pub status: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub result: Option<String>,
    pub final_result: Option<String>,
    pub execution_log: Option<String>,
    pub tool_calls: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
