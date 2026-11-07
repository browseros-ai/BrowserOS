//! `trace_run_ids`: the host's own ids for a run (`zuzu.task` → `t_123`), keyed
//! `(run_id, key)`, with an index on `(key, value)` so finding runs by an id is one index range.
//! Every id is text, so a new kind of id is a new key, never a schema change.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "trace_run_ids")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub run_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    pub value: String,
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
