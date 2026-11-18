//! Schema history and the old-build refusal fence for trace.db.
//!
//! A higher `user_version`, any unknown applied migration, and session-era S1/S2 files are
//! refused. Fresh/S5 files and the additive S3/S4 shapes are adopted atomically. Every future
//! migration must bump USER_VERSION_FENCE in the same change: older Neo then replaces the file
//! instead of reading forward, losing live trace/request history. That rollback tradeoff is
//! deliberate; this build never reads forward or lowers the fence.
//!
//! The file's other pragmas live here too: the log checkpoint that follows a retention sweep.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement, TransactionTrait};
use sea_orm_migration::prelude::{MigrationTrait, MigratorTrait, async_trait};

use crate::TraceError;

mod m0001_trace_baseline;
mod m0002_operation_site;
mod m0003_operation_main_world;
mod m0004_trace_values;
mod m0005_operation_frames_incomplete;
mod m0006_run_tabs_scope;
mod m0007_run_access;

/// The ordered schema history, applied only inside prepare's owned SQLite transaction.
pub(crate) struct TraceMigrator;

const USER_VERSION_FENCE: i64 = 7;

#[async_trait::async_trait]
impl MigratorTrait for TraceMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m0001_trace_baseline::Migration),
            Box::new(m0002_operation_site::Migration),
            Box::new(m0003_operation_main_world::Migration),
            Box::new(m0004_trace_values::Migration),
            Box::new(m0005_operation_frames_incomplete::Migration),
            Box::new(m0006_run_tabs_scope::Migration),
            Box::new(m0007_run_access::Migration),
        ]
    }
}

/// Copies every committed page into `trace.db` and empties its write-ahead log. Secure delete
/// (set on every connection in `TraceStore::open`) zeroes what a retention sweep freed, but under
/// WAL the zeroed pages go to the log first, and older copies of the deleted rows stay in the file
/// and the log until a checkpoint overwrites them; this is that checkpoint, so a swept run leaves
/// no readable bytes behind. Best effort: it waits up to the busy timeout for readers on an older
/// snapshot, and what it cannot finish waits for the next sweep that deletes something.
pub(crate) async fn truncate_log(db: &DatabaseConnection) {
    let checkpoint = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA wal_checkpoint(TRUNCATE)",
        ))
        .await;
    match checkpoint {
        Ok(Some(row)) if row.try_get::<i64>("", "busy").is_ok_and(|busy| busy == 0) => {}
        Ok(_) => tracing::debug!("a reader held the trace log; it is emptied after a later sweep"),
        Err(error) => {
            tracing::warn!(%error, "the trace log could not be checkpointed after a sweep")
        }
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
    use std::error::Error;

    use sea_orm::{ConnectionTrait, Database, EntityTrait};
    use sea_orm_migration::MigratorTrait;

    use super::{TraceMigrator, USER_VERSION_FENCE};
    use crate::storage::entities::runs;

    #[test]
    fn the_fence_moves_with_every_migration() {
        assert_eq!(
            usize::try_from(USER_VERSION_FENCE).ok(),
            Some(TraceMigrator::migrations().len()),
            "every migration bumps the old-build fence so rollback always refuses newer files"
        );
    }

    #[tokio::test]
    async fn upgrading_schema_six_keeps_ordinary_access_absent() -> Result<(), Box<dyn Error>> {
        let db = Database::connect("sqlite::memory:").await?;
        TraceMigrator::up(&db, Some(6)).await?;
        db.execute_unprepared(
            "INSERT INTO runs(id, started_at, status) VALUES ('old-run', 1, 'ok')",
        )
        .await?;
        super::prepare(&db).await?;
        let row = runs::Entity::find_by_id("old-run")
            .one(&db)
            .await?
            .ok_or("old row survives")?;
        assert_eq!(row.access, None, "NULL keeps the pre-access run ordinary");
        assert_eq!(row.started_at, 1);
        assert_eq!(row.status, "ok");
        Ok(())
    }
}
