//! Neo: The private parent entity; deleting an expired run cascades to its calls.

use sea_orm::{
    ActiveModelBehavior, DeriveEntityModel, DerivePrimaryKey, DeriveRelation, EntityTrait,
    EnumIter, PrimaryKeyTrait, Related, RelationDef, RelationTrait,
};

use super::enums::{DeliveryState, RunState};

/// A durable run header whose lifetime also owns its child call rows.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "runs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub status: RunState,
    pub page: Option<String>,
    pub code: String,
    pub error: Option<String>,
    pub call_count: i64,
    pub calls_dropped: i64,
    pub dropped_delivery: Option<DeliveryState>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::calls::Entity")]
    Calls,
}

impl Related<super::calls::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Calls.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
