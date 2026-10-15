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
    /// Every known entry is applied and the ledger holds newer ones: written by a newer server.
    Ahead(Vec<String>),
}

/// Applies pending entries unless the file was migrated by a newer server, in which case
/// `Migrator::up` would abort on the unknown rows; refuses when the two have diverged.
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
        (false, 0) => Ok(LedgerOutcome::Ahead(unknown)),
        (false, _) => Err(DbErr::Custom(format!(
            "database schema is incompatible with this BrowserOS server: it has Rust migrations \
             this build does not know ({}) and this build has unapplied ones",
            unknown.join(", ")
        ))),
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
