//! `trace_calls`: one row per browser call or host method, keyed `(run_id, call_no)`. Created by
//! whichever of `BeforeCall` and `AfterCall` arrives first; ended once, guarded, so a replay or a
//! late duplicate cannot regress it.

use sea_orm::entity::prelude::*;

use crate::db::kinds::{CallState, Delivery, Detail, UnfinishedReason};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "trace_calls")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub run_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub call_no: i64,
    pub parent_no: Option<i64>,
    pub object: String,
    pub page_id: Option<String>,
    pub tab_id: Option<i64>,
    pub method: String,
    pub title: Option<String>,
    pub selector: Option<String>,
    pub params_json: Option<String>,
    pub params_truncated: bool,
    pub timeout_ms: Option<f64>,
    pub loc_line: Option<i64>,
    pub loc_column: Option<i64>,
    pub started_at_ms: i64,
    pub offset_us: i64,
    pub duration_us: Option<i64>,
    pub input_offset_us: Option<i64>,
    pub state: CallState,
    pub error_name: Option<String>,
    pub error_message: Option<String>,
    pub unfinished_reason: Option<UnfinishedReason>,
    pub delivery: Option<Delivery>,
    pub delivery_settled: bool,
    pub log_json: Option<String>,
    pub detail: Detail,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::trace_runs::Entity",
        from = "Column::RunId",
        to = "super::trace_runs::Column::RunId"
    )]
    Run,
}

impl Related<super::trace_runs::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Run.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
