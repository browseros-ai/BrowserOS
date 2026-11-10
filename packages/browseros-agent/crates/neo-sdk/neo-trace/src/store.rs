//! Neo: Durable traces sit behind the existing observer stream, never inside the runtime.
//! The handle only enqueues owned records. One background task opens `SQLite` and drains them;
//! readers may await the database, while producers and script deadlines never do.

mod calls;
mod enums;
mod migration;
mod queue;
mod runs;
mod writer;

#[cfg(test)]
mod tests;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use neo_runtime::{
    CallRecord, CallResult, CallStarted, Delivery, RunEnd, RunOutcome, RunRequest, RunStatus,
    StartPage,
};
use sea_orm::{ActiveEnum, ColumnTrait, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Serialize;
use thiserror::Error;
use tokio::runtime::Handle;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use tracing::{dispatcher, warn};

use crate::RunId;
use enums::{CallState, DeliveryState, RunState, UnfinishedReason};
use queue::Queue;
use writer::Command;

/// A persistence error is diagnostic only; no database type escapes the trace boundary.
#[derive(Debug, Clone, Error)]
pub enum TraceError {
    #[error("trace database: {0}")]
    Database(String),
    #[error("trace writer is unavailable")]
    Unavailable,
}

impl From<io::Error> for TraceError {
    fn from(error: io::Error) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<DbErr> for TraceError {
    fn from(error: DbErr) -> Self {
        Self::Database(error.to_string())
    }
}

/// A run's list view deliberately omits its raw script. Times are Unix milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sea_orm::FromQueryResult)]
pub struct RunRow {
    pub id: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub status: String,
    pub page: Option<String>,
    pub error: Option<String>,
    pub call_count: i64,
    pub calls_dropped: i64,
    pub dropped_delivery: Option<String>,
}

/// The persisted observer payload; nullable result/duration distinguishes a live call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sea_orm::FromQueryResult)]
pub struct CallRow {
    pub run_id: String,
    pub seq: i64,
    pub guid: String,
    pub method: String,
    pub title: Option<String>,
    pub params: String,
    pub line: Option<i64>,
    pub col: Option<i64>,
    pub started_at: i64,
    pub duration_ms: Option<i64>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub unfinished_reason: Option<String>,
    pub delivery: Option<String>,
    pub log: Option<String>,
}

/// A raw script and its calls in the runtime's arrival order, for tests or a later viewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunDetail {
    pub run: RunRow,
    pub code: String,
    pub calls: Vec<CallRow>,
}

/// The trace database and its one writer task. Clones share queue ownership and shutdown.
#[derive(Clone)]
pub struct TraceStore(Arc<Inner>);

struct Inner {
    queue: Queue,
    connection: watch::Receiver<writer::Connection>,
    writer: Mutex<Option<JoinHandle<()>>>,
}

impl TraceStore {
    /// Creates/migrates the file, recovers interrupted runs, and prunes expired runs.
    ///
    /// # Errors
    /// Returns an opening or migration error; callers can continue with log-only traces.
    pub async fn open(path: &Path) -> Result<Self, TraceError> {
        let store = Self::open_lazy(path.to_path_buf())?;
        store.connection().await?;
        Ok(store)
    }

    /// Starts opening on the writer, immediately returning a queue for the first run.
    /// The synchronous server owner uses this at lazy initialization: even a busy or broken
    /// database cannot consume a script's deadline, and its first run need not be discarded.
    ///
    /// # Errors
    /// Returns an error outside a Tokio runtime. Database failures are logged by the writer.
    pub fn open_lazy(path: PathBuf) -> Result<Self, TraceError> {
        let handle = Handle::try_current().map_err(|_| TraceError::Unavailable)?;
        let (queue, receiver) = Queue::new();
        let (ready, connection) = watch::channel(None);
        let budget = Arc::clone(&queue.budget);
        let writer_runtime = handle.clone();
        let subscriber = dispatcher::get_default(Clone::clone);
        // A synchronous realm getter can occupy the only async worker. Like the call
        // collector, this one writer polls off that worker so live rows still commit.
        // SQLite has its own worker; this future only polls channels, SQLx, and a timer.
        let writer = handle.spawn_blocking(move || {
            dispatcher::with_default(&subscriber, || {
                writer_runtime.block_on(writer::run(&path, receiver, budget, ready));
            });
        });
        Ok(Self(Arc::new(Inner {
            queue,
            connection,
            writer: Mutex::new(Some(writer)),
        })))
    }

    /// Commits accepted commands and stops the writer. Concurrent calls share the drain;
    /// cancelling one close future leaves its join handle available for the next caller.
    pub async fn close(&self) {
        let mut writer = self.0.writer.lock().await;
        if let Some(task) = writer.as_mut() {
            self.0.queue.close();
            if let Err(error) = task.await {
                warn!(target: "neo_trace", %error, "trace writer task failed");
            }
            writer.take();
        }
    }

    /// Newest runs first, without loading or returning their scripts.
    ///
    /// # Errors
    /// Returns a database opening/query error, including reads after close.
    pub async fn runs(&self, limit: u64) -> Result<Vec<RunRow>, TraceError> {
        let connection = self.connection().await?;
        runs::Entity::find()
            .select_only()
            .columns([
                runs::Column::Id,
                runs::Column::StartedAt,
                runs::Column::EndedAt,
                runs::Column::Status,
                runs::Column::Page,
                runs::Column::Error,
                runs::Column::CallCount,
                runs::Column::CallsDropped,
                runs::Column::DroppedDelivery,
            ])
            .order_by_desc(runs::Column::StartedAt)
            .order_by_desc(runs::Column::Id)
            .limit(limit)
            .into_model::<RunRow>()
            .all(&connection)
            .await
            .map_err(TraceError::from)
    }

    /// One run's script and calls in arrival order; a live call has a null result.
    ///
    /// # Errors
    /// Returns a database opening/query error, including reads after close.
    pub async fn run(&self, id: RunId) -> Result<Option<RunDetail>, TraceError> {
        let connection = self.connection().await?;
        let Some(row) = runs::Entity::find_by_id(id.to_string())
            .one(&connection)
            .await?
        else {
            return Ok(None);
        };
        let calls = calls::Entity::find()
            .filter(calls::Column::RunId.eq(id.to_string()))
            .order_by_asc(calls::Column::Seq)
            .into_model::<CallRow>()
            .all(&connection)
            .await?;
        Ok(Some(RunDetail {
            run: RunRow {
                id: row.id,
                started_at: row.started_at,
                ended_at: row.ended_at,
                status: row.status.to_value(),
                page: row.page,
                error: row.error,
                call_count: row.call_count,
                calls_dropped: row.calls_dropped,
                dropped_delivery: row.dropped_delivery.map(|value| value.to_value()),
            },
            code: row.code,
            calls,
        }))
    }

    async fn connection(&self) -> Result<sea_orm::DatabaseConnection, TraceError> {
        let mut ready = self.0.connection.clone();
        loop {
            if let Some(connection) = ready.borrow().clone() {
                return connection;
            }
            ready.changed().await.map_err(|_| TraceError::Unavailable)?;
        }
    }

    pub(crate) fn start_run(&self, id: RunId, request: &RunRequest) {
        let page = match &request.page {
            StartPage::Page(id) => Some(id.as_str().to_owned()),
            StartPage::NewTab => None,
        };
        self.0.queue.push(Command::Start(runs::Model {
            id: id.to_string(),
            started_at: now_ms(),
            ended_at: None,
            status: RunState::Running,
            page,
            code: request.code.clone(),
            error: None,
            call_count: 0,
            calls_dropped: 0,
            dropped_delivery: None,
        }));
    }

    pub(crate) fn start_call(&self, id: RunId, call: CallStarted) {
        self.0.queue.push(Command::Call {
            row: calls::Model {
                run_id: id.to_string(),
                seq: integer(call.call_id),
                guid: call.guid,
                method: call.method,
                title: call.title,
                params: call.params.to_string(),
                line: call.location.map(|location| i64::from(location.line)),
                col: call
                    .location
                    .and_then(|location| location.column.map(i64::from)),
                started_at: integer(call.started_at_ms),
                duration_ms: None,
                result: None,
                error: None,
                unfinished_reason: None,
                delivery: None,
                log: None,
            },
            finished: false,
        });
    }

    pub(crate) fn finish_call(&self, id: RunId, call: CallRecord) {
        let (result, error, unfinished_reason) = match call.result {
            CallResult::Ok => (CallState::Ok, None, None),
            CallResult::Error { name, message } => {
                (CallState::Failed, Some(format!("{name}: {message}")), None)
            }
            CallResult::Unfinished { reason } => (
                CallState::Unfinished,
                None,
                Some(match reason {
                    RunEnd::Settled => UnfinishedReason::Settled,
                    RunEnd::Timeout => UnfinishedReason::Timeout,
                    RunEnd::Cancelled => UnfinishedReason::Cancelled,
                }),
            ),
        };
        self.0.queue.push(Command::Call {
            row: calls::Model {
                run_id: id.to_string(),
                seq: integer(call.call_id),
                guid: call.guid,
                method: call.method,
                title: call.title,
                params: call.params.to_string(),
                line: call.location.map(|location| i64::from(location.line)),
                col: call
                    .location
                    .and_then(|location| location.column.map(i64::from)),
                started_at: integer(call.started_at_ms),
                duration_ms: Some(i64::try_from(call.duration.as_millis()).unwrap_or(i64::MAX)),
                result: Some(result),
                error,
                unfinished_reason,
                delivery: call.delivery.map(delivery),
                log: Some(call.log.join("\n")),
            },
            finished: true,
        });
    }

    pub(crate) fn update_delivery(&self, id: RunId, seq: u64, value: Option<Delivery>) {
        self.0.queue.push(Command::Delivery {
            run_id: id.to_string(),
            seq: integer(seq),
            delivery: value.map(delivery),
        });
    }

    pub(crate) fn finish_run(&self, id: RunId, outcome: &RunOutcome) {
        self.0.queue.push(Command::End {
            run_id: id.to_string(),
            ended_at: now_ms(),
            status: match outcome.status {
                RunStatus::Ok => RunState::Ok,
                RunStatus::Error => RunState::Failed,
                RunStatus::Timeout => RunState::Timeout,
                RunStatus::Cancelled => RunState::Cancelled,
            },
            error: outcome
                .error
                .as_ref()
                .map(|error| format!("{}: {}", error.name, error.message)),
            call_count: i64::try_from(outcome.calls.len()).unwrap_or(i64::MAX),
            calls_dropped: outcome
                .calls_dropped
                .map_or(0, |dropped| integer(dropped.count)),
            dropped_delivery: outcome
                .calls_dropped
                .and_then(|dropped| dropped.delivery.map(delivery)),
            // A Finished event can precede eviction when its completed log exceeds the
            // runtime's record budget. The outcome, rather than a second implementation
            // of those limits here, identifies exactly which live rows may remain.
            retained_calls: outcome.calls_dropped.map(|_| {
                outcome
                    .calls
                    .iter()
                    .map(|call| integer(call.call_id))
                    .collect()
            }),
        });
    }
}

fn delivery(value: Delivery) -> DeliveryState {
    match value {
        Delivery::NotSent => DeliveryState::NotSent,
        Delivery::Done => DeliveryState::Done,
        Delivery::Unknown => DeliveryState::Unknown,
    }
}

fn integer(value: u64) -> i64 {
    // SQLite integers are signed. A persistence-only conversion must never fail a run;
    // the runtime's bounded call ordinals remain well inside this representable range.
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}
