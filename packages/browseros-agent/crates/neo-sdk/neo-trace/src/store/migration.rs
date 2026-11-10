//! Neo: A single baseline migration owns the trace schema independently of server state.

use sea_orm_migration::prelude::{DeriveIden, DeriveMigrationName};
use sea_orm_migration::sea_query::{ColumnDef, ForeignKey, ForeignKeyAction, Index, Table};
use sea_orm_migration::{DbErr, MigrationTrait, MigratorTrait, SchemaManager, async_trait};

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Migration)]
    }
}

#[derive(DeriveMigrationName)]
struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Runs::Table)
                    .col(ColumnDef::new(Runs::Id).text().not_null().primary_key())
                    .col(ColumnDef::new(Runs::StartedAt).big_integer().not_null())
                    .col(ColumnDef::new(Runs::EndedAt).big_integer())
                    .col(ColumnDef::new(Runs::Status).text().not_null())
                    .col(ColumnDef::new(Runs::Page).text())
                    .col(ColumnDef::new(Runs::Code).text().not_null())
                    .col(ColumnDef::new(Runs::Error).text())
                    .col(ColumnDef::new(Runs::CallCount).big_integer().not_null())
                    .col(ColumnDef::new(Runs::CallsDropped).big_integer().not_null())
                    .col(ColumnDef::new(Runs::DroppedDelivery).text())
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("runs_started_at")
                    .table(Runs::Table)
                    .col(Runs::StartedAt)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Calls::Table)
                    .col(ColumnDef::new(Calls::RunId).text().not_null())
                    .col(ColumnDef::new(Calls::Seq).big_integer().not_null())
                    .col(ColumnDef::new(Calls::Guid).text().not_null())
                    .col(ColumnDef::new(Calls::Method).text().not_null())
                    .col(ColumnDef::new(Calls::Title).text())
                    .col(ColumnDef::new(Calls::Params).text().not_null())
                    .col(ColumnDef::new(Calls::Line).big_integer())
                    .col(ColumnDef::new(Calls::Col).big_integer())
                    .col(ColumnDef::new(Calls::StartedAt).big_integer().not_null())
                    .col(ColumnDef::new(Calls::DurationMs).big_integer())
                    .col(ColumnDef::new(Calls::Result).text())
                    .col(ColumnDef::new(Calls::Error).text())
                    .col(ColumnDef::new(Calls::UnfinishedReason).text())
                    .col(ColumnDef::new(Calls::Delivery).text())
                    .col(ColumnDef::new(Calls::Log).text())
                    .primary_key(Index::create().col(Calls::RunId).col(Calls::Seq))
                    .foreign_key(
                        ForeignKey::create()
                            .from(Calls::Table, Calls::RunId)
                            .to(Runs::Table, Runs::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Calls::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Runs::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Runs {
    Table,
    Id,
    StartedAt,
    EndedAt,
    Status,
    Page,
    Code,
    Error,
    CallCount,
    CallsDropped,
    DroppedDelivery,
}

#[derive(DeriveIden)]
enum Calls {
    Table,
    RunId,
    Seq,
    Guid,
    Method,
    Title,
    Params,
    Line,
    Col,
    StartedAt,
    DurationMs,
    Result,
    Error,
    UnfinishedReason,
    Delivery,
    Log,
}
