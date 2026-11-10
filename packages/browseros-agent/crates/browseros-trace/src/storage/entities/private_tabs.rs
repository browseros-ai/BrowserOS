//! Sticky privacy flags, independent of run retention. First-record-wins inserts
//! preserve the original private instant; only an ordered close command removes a flag.

use sea_orm::entity::prelude::*;

/// A durable private flag whose original timestamp survives repeated privacy commits.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "private_tabs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tab_id: i64,
    pub since: i64,
    pub reason: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
