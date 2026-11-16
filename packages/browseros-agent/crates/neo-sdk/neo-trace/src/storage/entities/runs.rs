//! One run lifecycle, its bounded payloads and request replay. List reads use projections
//! so they never fetch code, results or replay bodies.

use sea_orm::entity::prelude::*;

/// A run's stored lifecycle and bounded payloads; domain parsing belongs to record_rows.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "runs")]
pub struct Model {
    pub values: Option<String>,
    pub tabs_scope: Option<String>,
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub task_id: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub status: String,
    pub effects: Option<String>,
    pub error: Option<String>,
    pub request_id: Option<String>,
    pub host_data: Option<String>,
    pub code: Option<String>,
    pub code_truncated: bool,
    pub code_withheld: Option<String>,
    pub result: Option<String>,
    pub result_truncated: bool,
    pub replay: Option<String>,
    pub replay_until: Option<i64>,
    pub operation_count: i64,
    pub trace_gaps: i64,
    pub complete: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
