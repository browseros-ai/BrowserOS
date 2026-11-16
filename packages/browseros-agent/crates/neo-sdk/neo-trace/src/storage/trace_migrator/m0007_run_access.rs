//! Retain host access at run start, even when a crash leaves no reply. NULL preserves ordinary
//! runs and older records without inventing a restriction during upgrade.

use super::super::entities::runs;
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
                    .add_column(ColumnDef::new(runs::Column::Access).string())
                    .to_owned(),
            )
            .await
    }
}
