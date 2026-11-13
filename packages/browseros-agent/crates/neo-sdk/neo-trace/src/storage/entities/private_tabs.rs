//! Legacy table definition used only by the baseline migration. Runtime state no longer reads
//! or writes this table; keeping its DDL preserves compatibility with existing trace files.

use sea_orm::entity::prelude::*;

/// A legacy row retained in the schema for older trace files.
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
