//! Neo: One task owns migration, crash recovery, retention, and batched trace transactions.
//! Producers never await this task; failed transactions are diagnosed without touching outcomes.

use std::mem;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait,
    IntoActiveModel, PaginatorTrait, QueryFilter, SqlxSqliteConnector, TransactionTrait,
};
use sea_orm_migration::MigratorTrait;
use tokio::fs;
use tokio::sync::{mpsc::UnboundedReceiver, watch};
use tokio::time::{self, Instant, MissedTickBehavior};
use tracing::warn;

use super::enums::{DeliveryState, RunState};
use super::migration::Migrator;
use super::queue::{Budget, Message, Queued};
use super::{TraceError, calls, runs};

#[cfg(test)]
mod tests;

/// Neo: Amortize `SQLite` transactions without allowing a busy queue to monopolize the task.
const BATCH_COMMANDS: usize = 256;
/// Neo: The supplied persistence design retains seven days of runs and their child calls.
pub(super) const RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
/// Neo: The single writer performs the same age sweep hourly after the opening sweep.
const PRUNE_INTERVAL: Duration = Duration::from_hours(1);
/// Neo: Match the host database's bounded `SQLite` lock wait, independently of script deadlines.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) type Connection = Option<Result<DatabaseConnection, TraceError>>;

/// Owned write data crosses the collector/async boundary; no runtime hooks or database
/// objects cross back into the realm, and a failed handoff changes only persistence.
pub(super) enum Command {
    Start(runs::Model),
    Call {
        row: calls::Model,
        finished: bool,
    },
    Delivery {
        run_id: String,
        seq: i64,
        delivery: Option<DeliveryState>,
    },
    End {
        run_id: String,
        ended_at: i64,
        status: RunState,
        error: Option<String>,
        call_count: i64,
        calls_dropped: i64,
        dropped_delivery: Option<DeliveryState>,
        retained_calls: Option<Vec<i64>>,
    },
}

impl Command {
    pub(super) fn heap_bytes(&self) -> usize {
        match self {
            Self::Start(row) => {
                row.id.capacity()
                    + row.code.capacity()
                    + optional_bytes(row.page.as_ref())
                    + optional_bytes(row.error.as_ref())
            }
            Self::Call { row, .. } => {
                row.run_id.capacity()
                    + row.guid.capacity()
                    + row.method.capacity()
                    + row.params.capacity()
                    + optional_bytes(row.title.as_ref())
                    + optional_bytes(row.error.as_ref())
                    + optional_bytes(row.log.as_ref())
            }
            Self::Delivery { run_id, .. } => run_id.capacity(),
            Self::End {
                run_id,
                error,
                retained_calls,
                ..
            } => {
                run_id.capacity()
                    + optional_bytes(error.as_ref())
                    + retained_calls
                        .as_ref()
                        .map_or(0, |ids| ids.capacity() * mem::size_of::<i64>())
            }
        }
    }
}

fn optional_bytes(value: Option<&String>) -> usize {
    value.map_or(0, String::capacity)
}

pub(super) async fn run(
    path: &Path,
    mut receiver: UnboundedReceiver<Message>,
    budget: Arc<Budget>,
    ready: watch::Sender<Connection>,
) {
    let database = match open(path).await {
        Ok(database) => database,
        Err(error) => {
            budget.close();
            warn!(target: "neo_trace", path = %path.display(), %error, "trace store open failed; continuing with logs only");
            ready.send_replace(Some(Err(error)));
            return;
        }
    };
    ready.send_replace(Some(Ok(database.clone())));
    let mut prune = time::interval_at(Instant::now() + PRUNE_INTERVAL, PRUNE_INTERVAL);
    prune.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        let message = tokio::select! {
            message = receiver.recv() => message,
            _ = prune.tick() => {
                if let Err(error) = retain(&database, super::now_ms()).await {
                    warn!(target: "neo_trace", %error, "trace retention failed");
                }
                continue;
            }
        };
        let Some(message) = message else { break };
        let mut batch = Vec::with_capacity(BATCH_COMMANDS);
        receive(message, &mut receiver, &mut batch);
        while batch.len() < BATCH_COMMANDS {
            let Ok(message) = receiver.try_recv() else {
                break;
            };
            receive(message, &mut receiver, &mut batch);
        }
        if !batch.is_empty()
            && let Err(error) = write(&database, batch).await
        {
            warn!(target: "neo_trace", %error, "trace batch failed; run execution is unaffected");
        }
    }
    budget.close();
    if let Err(error) = database.close().await {
        warn!(target: "neo_trace", %error, "trace store close failed");
    }
}

fn receive(message: Message, receiver: &mut UnboundedReceiver<Message>, batch: &mut Vec<Queued>) {
    match message {
        Message::Write(queued) => batch.push(*queued),
        Message::Close => receiver.close(),
    }
}

async fn open(path: &Path) -> Result<DatabaseConnection, TraceError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).await.map_err(TraceError::from)?;
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .pragma("auto_vacuum", "incremental")
        .busy_timeout(BUSY_TIMEOUT);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|error| TraceError::Database(error.to_string()))?;
    let database = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
    if let Err(error) = initialize(&database).await {
        let _closed = database.close().await;
        return Err(TraceError::from(error));
    }
    Ok(database)
}

async fn initialize(database: &DatabaseConnection) -> Result<(), DbErr> {
    // SeaORM does not wrap SQLite migrations in a transaction. Include all DDL and
    // its completion record here, so an interrupted first open can retry without
    // finding unrecorded schema objects. Recovery and retention follow migration
    // inside the same commit; no reader sees an incompletely initialized store.
    let transaction = database.begin().await?;
    Migrator::up(&transaction, None).await?;
    runs::Entity::update_many()
        .col_expr(runs::Column::Status, Expr::value(RunState::Interrupted))
        .col_expr(runs::Column::EndedAt, Expr::value(super::now_ms()))
        .filter(runs::Column::Status.eq(RunState::Running))
        .exec(&transaction)
        .await?;
    retain(&transaction, super::now_ms()).await?;
    transaction.commit().await
}

pub(super) async fn retain<C: sea_orm::ConnectionTrait>(
    database: &C,
    now: i64,
) -> Result<(), DbErr> {
    runs::Entity::delete_many()
        .filter(runs::Column::StartedAt.lt(now.saturating_sub(RETENTION_MS)))
        .exec(database)
        .await?;
    Ok(())
}

async fn write(database: &DatabaseConnection, batch: Vec<Queued>) -> Result<(), DbErr> {
    let transaction = database.begin().await?;
    for queued in batch {
        apply(&transaction, queued.command).await?;
    }
    transaction.commit().await
}

async fn apply(transaction: &DatabaseTransaction, command: Command) -> Result<(), DbErr> {
    match command {
        Command::Start(row) => {
            runs::Entity::insert(row.into_active_model())
                .exec(transaction)
                .await?;
        }
        Command::Call { row, finished } => {
            // Admission can drop Start independently of later calls after capacity
            // recovers. A missing parent is expected loss, not a storage failure:
            // do not let its foreign-key error roll back another run in this batch.
            // The single writer checks and inserts within the same transaction.
            if !runs::Entity::find_by_id(&row.run_id)
                .exists(transaction)
                .await?
            {
                return Ok(());
            }
            let mut conflict = OnConflict::columns([calls::Column::RunId, calls::Column::Seq]);
            if finished {
                conflict.update_columns([
                    calls::Column::Guid,
                    calls::Column::Method,
                    calls::Column::Title,
                    calls::Column::Params,
                    calls::Column::Line,
                    calls::Column::Col,
                    calls::Column::StartedAt,
                    calls::Column::DurationMs,
                    calls::Column::Result,
                    calls::Column::Error,
                    calls::Column::UnfinishedReason,
                    calls::Column::Delivery,
                    calls::Column::Log,
                ]);
            } else {
                conflict.do_nothing();
            }
            calls::Entity::insert(row.into_active_model())
                .on_conflict(conflict)
                .exec(transaction)
                .await?;
        }
        Command::Delivery {
            run_id,
            seq,
            delivery,
        } => {
            // A dropped call may still receive delivery. Zero matching rows is a
            // benign no-op; update-one would return RecordNotUpdated and poison
            // unrelated accepted writes. Real database errors still abort the batch.
            calls::Entity::update_many()
                .set(calls::ActiveModel {
                    delivery: Set(delivery),
                    ..Default::default()
                })
                .filter(calls::Column::RunId.eq(run_id))
                .filter(calls::Column::Seq.eq(seq))
                .exec(transaction)
                .await?;
        }
        Command::End {
            run_id,
            ended_at,
            status,
            error,
            call_count,
            calls_dropped,
            dropped_delivery,
            retained_calls,
        } => {
            if let Some(retained_calls) = retained_calls {
                let mut evicted =
                    calls::Entity::delete_many().filter(calls::Column::RunId.eq(&run_id));
                if !retained_calls.is_empty() {
                    evicted = evicted.filter(calls::Column::Seq.is_not_in(retained_calls));
                }
                // This shares the run-end transaction: a reader observing the terminal
                // status cannot still see a call the authoritative outcome discarded.
                evicted.exec(transaction).await?;
            }
            // End can outlive a dropped Start too. Keep its zero-row update benign
            // while retaining the shared transaction with authoritative call cleanup.
            runs::Entity::update_many()
                .set(runs::ActiveModel {
                    ended_at: Set(Some(ended_at)),
                    status: Set(status),
                    error: Set(error),
                    call_count: Set(call_count),
                    calls_dropped: Set(calls_dropped),
                    dropped_delivery: Set(dropped_delivery),
                    ..Default::default()
                })
                .filter(runs::Column::Id.eq(run_id))
                .exec(transaction)
                .await?;
        }
    }
    Ok(())
}
