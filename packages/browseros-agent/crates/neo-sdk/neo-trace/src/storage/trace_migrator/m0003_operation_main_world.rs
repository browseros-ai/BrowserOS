//! Add whether an operation sent script into a page's main world, without rewriting historical
//! operations. NULL means unknown: every row written before this migration, and an operation
//! whose end never arrived. An operation's end stores true or false.
//!
//! An INTEGER, as the store's other flags (`args_truncated`): SQLite has no boolean type.

use super::super::entities::operations;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(operations::Entity)
                    .add_column(ColumnDef::new(operations::Column::MainWorld).integer())
                    .to_owned(),
            )
            .await
    }
}
