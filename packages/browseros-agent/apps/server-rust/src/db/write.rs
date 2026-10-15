//! Reserves SQLite's writer before any read, so a TS writer cannot invalidate our snapshot.
//! SeaORM 1.1 exposes only deferred SQLite transactions; the mode switch below retains its
//! pinned connection and rollback-on-drop guard while changing the transaction before use.

use super::{Database, DbError, DbResult};
use sea_orm::{ConnectionTrait, DatabaseTransaction, DbErr, RuntimeErr, TransactionTrait};
use std::time::Duration;

impl Database {
    pub(super) async fn begin_write(&self) -> DbResult<DatabaseTransaction> {
        self.ensure_writable()?;
        let database = self.clone();
        // Do not cancel between ROLLBACK and BEGIN: SQLx tracks one transaction, and its
        // rollback guard must always receive a live transaction. A detached task completes
        // that handoff even if its caller disappears; its returned guard then rolls back.
        tokio::spawn(async move {
            // Retain the instance lock as well as the pool while a canceled caller's task
            // finishes admission; another Rust process must not enter during this handoff.
            let transaction = database.connection.begin().await?;
            for attempt in 0..8 {
                match transaction
                    .execute_unprepared("ROLLBACK; BEGIN IMMEDIATE")
                    .await
                {
                    Ok(_) => return Ok(transaction),
                    Err(error) => {
                        // Restore SQLx's expected depth before retrying or returning the error.
                        if let Err(restore_error) =
                            transaction.execute_unprepared("BEGIN DEFERRED").await
                        {
                            // An I/O failure can prevent restoring SQLx's tracked depth. Do not
                            // return that connection to service with mismatched transaction state.
                            drop(transaction);
                            database.connection.clone().close().await?;
                            return Err(DbError::from(restore_error));
                        }
                        if !is_busy(&error) || attempt == 7 {
                            return Err(DbError::from(error));
                        }
                        tokio::time::sleep(Duration::from_millis(10 << attempt.min(5))).await;
                    }
                }
            }
            Err(DbError::Query("SQLite write retry exhausted".into()))
        })
        .await
        .map_err(|error| DbError::Query(format!("SQLite write admission task failed: {error}")))?
    }

    pub(super) fn ensure_writable(&self) -> DbResult<()> {
        if self.read_only {
            return Err(DbError::ReadOnly);
        }
        Ok(())
    }
}

fn is_busy(error: &DbErr) -> bool {
    let (DbErr::Conn(RuntimeErr::SqlxError(error))
    | DbErr::Exec(RuntimeErr::SqlxError(error))
    | DbErr::Query(RuntimeErr::SqlxError(error))) = error
    else {
        return false;
    };
    error
        .as_database_error()
        .and_then(|error| error.code())
        .and_then(|code| code.parse::<i32>().ok())
        .is_some_and(|code| matches!(code & 0xff, 5 | 6))
}
