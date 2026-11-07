//! Neo: `trace_pages`: one row per page a run touched (its start page, a page it listed, one it
//! created), keyed `(run_id, page_id)`, with when the run got it and, if it closed it, when.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "trace_pages")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub run_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub page_id: String,
    pub tab_id: Option<i64>,
    pub created: bool,
    pub by_call: Option<i64>,
    /// NULL when the run's `PageOpened` was shed and only its `PageClosed` arrived.
    pub opened_offset_us: Option<i64>,
    pub closed_offset_us: Option<i64>,
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
