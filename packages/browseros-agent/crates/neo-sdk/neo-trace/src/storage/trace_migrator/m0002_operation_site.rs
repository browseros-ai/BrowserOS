//! Add the script location without rewriting historical operations. NULL means the operation
//! had no captured site, including all rows written before this migration.

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
                    .add_column(ColumnDef::new(operations::Column::Site).text())
                    .to_owned(),
            )
            .await
    }
}
