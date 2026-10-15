//! The Rust-owned migration ledger (`seaql_migrations`), applied after the drizzle catch-up.
//!
//! While the TS server can still run against the same file, every entry here must be additive
//! (new tables, nullable columns) and must leave `__drizzle_migrations` alone; changes to the
//! drizzle-owned tables need a migration both runtimes understand. Tonight the ledger holds only
//! `m0001_adopt_drizzle_schema`, a no-op marker recording that this server adopted the file.
//! Version skew is tolerated the way claw-server's `migrate_forward_compatible` tolerates it.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement};
use sea_orm_migration::{MigrationName, MigrationTrait, MigratorTrait, SchemaManager};
use std::collections::HashSet;

pub(super) struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m0001_adopt_drizzle_schema::Migration)]
    }
}

mod m0001_adopt_drizzle_schema {
    use super::{DbErr, MigrationName, MigrationTrait, SchemaManager};

    /// Marker only: the drizzle catch-up already produced the schema, and replacing its
    /// bookkeeping would break a TS rollback.
    pub(super) struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m0001_adopt_drizzle_schema"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
            Ok(())
        }

        async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
            Ok(())
        }
    }
}

pub(super) enum LedgerOutcome {
    Applied(usize),
    /// The ledger holds entries from another build; do not replay anything over it.
    Ahead(Vec<String>),
}

/// Applies pending entries unless the file was migrated by a newer server, in which case
/// `Migrator::up` would abort on the unknown rows. Leave even diverged ledgers read-only.
pub(super) async fn migrate_forward_compatible(
    connection: &DatabaseConnection,
) -> Result<LedgerOutcome, DbErr> {
    let known: Vec<String> = Migrator::migrations()
        .iter()
        .map(|migration| migration.name().to_owned())
        .collect();
    let applied = applied_versions(connection).await?;
    let applied_set: HashSet<&str> = applied.iter().map(String::as_str).collect();
    let pending = known
        .iter()
        .filter(|name| !applied_set.contains(name.as_str()))
        .count();
    let unknown: Vec<String> = applied
        .iter()
        .filter(|name| !known.contains(name))
        .cloned()
        .collect();

    match (unknown.is_empty(), pending) {
        (true, pending) => {
            Migrator::up(connection, None).await?;
            Ok(LedgerOutcome::Applied(pending))
        }
        (false, _) => Ok(LedgerOutcome::Ahead(unknown)),
    }
}

async fn applied_versions(connection: &DatabaseConnection) -> Result<Vec<String>, DbErr> {
    let exists = connection
        .query_one(Statement::from_string(
            connection.get_database_backend(),
            "SELECT 1 AS present FROM sqlite_master WHERE type = 'table' AND name = 'seaql_migrations'",
        ))
        .await?
        .is_some();
    if !exists {
        return Ok(Vec::new());
    }
    connection
        .query_all(Statement::from_string(
            connection.get_database_backend(),
            "SELECT version FROM seaql_migrations ORDER BY version",
        ))
        .await?
        .iter()
        .map(|row| row.try_get::<String>("", "version"))
        .collect()
}

/// Probe before drizzle catch-up too: a future Rust ledger may coexist with older TS tables.
pub(super) async fn unknown_sqlite_versions(
    connection: &mut sea_orm::sqlx::SqliteConnection,
) -> Result<Vec<String>, sea_orm::sqlx::Error> {
    let exists: i64 = sea_orm::sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='seaql_migrations'",
    )
    .fetch_one(&mut *connection)
    .await?;
    if exists == 0 {
        return Ok(Vec::new());
    }
    let versions: Vec<String> =
        sea_orm::sqlx::query_scalar("SELECT version FROM seaql_migrations ORDER BY version")
            .fetch_all(&mut *connection)
            .await?;
    let known = Migrator::migrations()
        .into_iter()
        .map(|migration| migration.name().to_owned())
        .collect::<Vec<_>>();
    Ok(versions
        .into_iter()
        .filter(|version| !known.contains(version))
        .collect())
}
