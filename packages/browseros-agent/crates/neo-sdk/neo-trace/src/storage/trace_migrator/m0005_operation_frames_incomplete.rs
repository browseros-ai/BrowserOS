//! Add whether an operation read a tab's frame tree that was not whole, without rewriting
//! historical operations. NULL means unknown: every row written before this migration, and an
//! operation whose end never arrived. An operation's end stores true or false.
//!
//! A host reads it to find a run whose script was handed a short tree (neo-playwright
//! `frame_tree::frames`). An INTEGER, as the store's other flags (`main_world`): SQLite has no
//! boolean type.

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
                    .add_column(ColumnDef::new(operations::Column::FramesIncomplete).integer())
                    .to_owned(),
            )
            .await
    }
}
