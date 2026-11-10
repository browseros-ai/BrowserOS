//! Neo: Private live call rows, keyed by the runtime's arrival order within a run.

use sea_orm::{
    ActiveModelBehavior, DeriveEntityModel, DerivePrimaryKey, DeriveRelation, EntityTrait,
    EnumIter, PrimaryKeyTrait, Related, RelationDef, RelationTrait,
};

use super::enums::{CallState, DeliveryState, UnfinishedReason};

/// A live or completed observer record; the composite key preserves per-run arrival order.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "calls")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub run_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub seq: i64,
    pub guid: String,
    pub method: String,
    pub title: Option<String>,
    pub params: String,
    pub line: Option<i64>,
    pub col: Option<i64>,
    pub started_at: i64,
    pub duration_ms: Option<i64>,
    pub result: Option<CallState>,
    pub error: Option<String>,
    pub unfinished_reason: Option<UnfinishedReason>,
    pub delivery: Option<DeliveryState>,
    pub log: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::runs::Entity",
        from = "Column::RunId",
        to = "super::runs::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Run,
}

impl Related<super::runs::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Run.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
