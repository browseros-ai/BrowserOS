//! Schema history and the old-build refusal fence for trace.db.
//!
//! A higher `user_version`, any unknown applied migration, and session-era S1/S2 files are
//! refused. Fresh/S5 files and the additive S3/S4 shapes are adopted atomically. Every future
//! migration must bump USER_VERSION_FENCE in the same change: older Neo then replaces the file
//! instead of reading forward, losing live trace/request history. That rollback tradeoff is
//! deliberate; this build never reads forward or lowers the fence.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement, TransactionTrait};
use sea_orm_migration::prelude::{MigrationTrait, MigratorTrait, async_trait};

use crate::TraceError;

mod m0001_trace_baseline;

/// The ordered schema history, applied only inside prepare's owned SQLite transaction.
pub(crate) struct TraceMigrator;

const USER_VERSION_FENCE: i64 = 1;

#[async_trait::async_trait]
impl MigratorTrait for TraceMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m0001_trace_baseline::Migration)]
    }
}

/// Owns SQLite's adoption transaction: the library does not wrap SQLite migrations itself.
/// Dropping it on any refusal/error rolls back DDL, migration records and the version stamp.
pub(crate) async fn prepare(db: &DatabaseConnection) -> Result<(), TraceError> {
    // These file properties have no sea-query builder. WAL must be set outside the transaction,
    // after connecting without file-reading PRAGMAs (TraceStore::open's close-before-replace rule).
    db.execute_unprepared("PRAGMA journal_mode = WAL").await?;
    let row = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA user_version",
        ))
        .await?
        .ok_or_else(|| TraceError::Corrupt("missing trace schema version".into()))?;
    let version: i64 = row.try_get("", "user_version")?;
    if version > USER_VERSION_FENCE {
        return Err(TraceError::Corrupt(format!(
            "trace schema {version} is newer than this build's {USER_VERSION_FENCE}"
        )));
    }
    let txn = db.begin().await?;
    let known = TraceMigrator::migrations();
    let unknown: Vec<String> = TraceMigrator::get_migration_models(&txn)
        .await?
        .into_iter()
        .filter(|row| {
            !known
                .iter()
                .any(|migration| migration.name() == row.version)
        })
        .map(|row| row.version)
        .collect();
    if !unknown.is_empty() {
        return Err(TraceError::Corrupt(format!(
            "trace schema has migrations this build does not know: {}",
            unknown.join(", ")
        )));
    }
    TraceMigrator::up(&txn, None)
        .await
        .map_err(|error| match error {
            DbErr::Migration(message) if message == m0001_trace_baseline::SESSION_ERA => {
                TraceError::Corrupt(message)
            }
            other => TraceError::Store(other),
        })?;
    if version < USER_VERSION_FENCE {
        txn.execute_unprepared(&format!("PRAGMA user_version = {USER_VERSION_FENCE}"))
            .await?;
    }
    txn.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use sea_orm_migration::MigratorTrait;

    use super::{TraceMigrator, USER_VERSION_FENCE};

    #[test]
    fn the_fence_moves_with_every_migration() {
        assert_eq!(
            usize::try_from(USER_VERSION_FENCE).ok(),
            Some(TraceMigrator::migrations().len()),
            "every migration bumps the old-build fence so rollback always refuses newer files"
        );
    }
}
