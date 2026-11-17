//! Which runs targeted each tab. The writer inserts the link in the operation batch,
//! and retention deletes links before runs; the relation adds no database foreign key.

use sea_orm::entity::prelude::*;

/// A persisted run/tab link, committed atomically with the operation targeting the tab.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "run_tabs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tab_id: i64,
    #[sea_orm(primary_key, auto_increment = false)]
    pub run_id: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::runs::Entity",
        from = "Column::RunId",
        to = "super::runs::Column::Id"
    )]
    Run,
}

impl ActiveModelBehavior for ActiveModel {}

impl Related<super::runs::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Run.def()
    }
}
