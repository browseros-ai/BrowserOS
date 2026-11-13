//! Mark records whose values the host chose not to retain. NULL preserves historical rows
//! and the default wire shape. Each record receives its marker only when it ends.

use super::super::entities::{operations, runs};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(runs::Entity)
                    .add_column(ColumnDef::new(runs::Column::Values).string())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(operations::Entity)
                    .add_column(ColumnDef::new(operations::Column::Values).string())
                    .to_owned(),
            )
            .await
    }
}
