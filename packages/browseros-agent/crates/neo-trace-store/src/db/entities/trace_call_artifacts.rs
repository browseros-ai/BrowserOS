//! `trace_call_artifacts` (migration 2): neo-feeds' after-call capture, keyed
//! `(run_id, call_no)` like the call it follows, so one retention owns both. The JPEG is NULL once
//! the session is withheld.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "trace_call_artifacts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub run_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub call_no: i64,
    pub tab_id: Option<i64>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub jpeg: Option<Vec<u8>>,
    pub captured_at_ms: i64,
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
