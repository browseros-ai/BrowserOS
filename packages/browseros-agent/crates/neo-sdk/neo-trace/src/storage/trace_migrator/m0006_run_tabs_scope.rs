//! Record which tabs each run saw (`tabs.scope` with its task). NULL keeps runs recorded before
//! the column, which all saw every tab, and the wire leaves the field out for them.

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
                    .add_column(ColumnDef::new(runs::Column::TabsScope).string())
                    .to_owned(),
            )
            .await
    }
}
