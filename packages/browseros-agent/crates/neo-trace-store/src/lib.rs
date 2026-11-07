//! Neo: Keeps neo-runtime's run traces (runs, browser calls, getters, pages, console lines) in one
//! SQLite file through SeaORM, and reads them back as typed views.
//!
//! ```text
//!  TraceStore::open ── db.rs: lock file, pragmas, corrupt-file recovery,
//!                      migration.rs on its own ledger, in one transaction
//!  sink ── ingest: per-run atomics + byte permits ──▶ writer: apply / commit / heads
//!  runs / run / calls / getters / session / sessions / knows
//!                   ── query.rs over a read pool of four (query_only) ──▶ views.rs
//! ```
//!
//! `TraceStore` is the crate's whole surface; SeaORM stays behind `pub(crate)` (`db.rs`).

use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use neo_runtime::{CallId, RunId, SessionKey, TraceSink};
use sea_orm::{DatabaseConnection, DbErr};
use serde::Serialize;
use tokio::runtime::Handle;
use tokio::sync::{Mutex as AsyncMutex, oneshot, watch};
use tokio::task::{self, JoinHandle};
use tokio::time::{self, Instant};
use tracing::warn;

mod apply;
mod db;
mod ingest;
mod live;
mod query;
mod retention;
#[cfg(test)]
mod tests;
mod views;
mod writer;

pub use views::{
    AfterCall, CallQuery, CallState, CallTrace, Detail, GetterOutcome, GetterTrace, LastCall,
    Listing, Loss, MAX_PAGE, OverCap, PageTrace, RunQuery, RunState, RunSummary, RunTrace,
    SessionHead, SessionQuery, SessionSummary, SessionTimeline, Settled, SettledBy, Timeline,
    TraceError, Unfinished,
};

/// Where the store keeps its file, and how much it keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreConfig {
    /// The database file, such as `<data_dir>/neo-traces.sqlite`; its lock file and SQLite's
    /// `-wal` and `-shm` files sit beside it.
    pub path: PathBuf,
    pub retention: Retention,
    /// Bytes of events waiting to be written before the ingest keeps only stubs: 32 MiB.
    pub queue_bytes: usize,
    /// Bytes held back, once the queue is full, for stubs and run lifecycle events: 4 MiB.
    pub reserve_bytes: usize,
}

impl StoreConfig {
    /// The defaults, at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            retention: Retention::default(),
            queue_bytes: 32 << 20,
            reserve_bytes: 4 << 20,
        }
    }
}

/// How long and how much the store keeps. A run leaves whole (with every row it owns) once it
/// started more than `max_age` ago, or, oldest first, while the file's used pages exceed
/// `max_bytes`. A run is never swept before it settles, unless it is a day past `max_age`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retention {
    pub max_age: Duration,
    pub max_bytes: u64,
}

impl Default for Retention {
    /// 7 days (claw-server's audit default) and 2 GiB.
    fn default() -> Self {
        Self {
            max_age: Duration::from_hours(7 * 24),
            max_bytes: 2 << 30,
        }
    }
}

/// How the store is doing, for a host's health surface. Counts are since `open`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreHealth {
    /// A newer build migrated the file: reads work, nothing is recorded.
    pub read_only: bool,
    pub closed: bool,
    /// Events taken from runs and not yet written.
    pub queued_events: u64,
    pub queued_bytes: u64,
    /// Queue entries and bytes still unflushed when the close budget expired. These remain
    /// visible after cancellation releases the in-memory queue; restart reconciliation marks
    /// persisted open runs incomplete.
    pub unflushed_events: u64,
    pub unflushed_bytes: u64,
    /// The writer's current batch has been failing since then; it keeps retrying.
    pub failing_since_ms: Option<u64>,
    pub last_error: Option<String>,
    /// Events kept only as stubs, and events not kept at all, across every run.
    pub events_stubbed: u64,
    pub events_dropped: u64,
    /// Runs whose last event could not be queued; the settle sweep closes them.
    pub settles_lost: u64,
    /// Events the writer gave up on after they failed alone, logged by kind and ids.
    pub writes_dropped: u64,
    /// neo-feeds' captures that did not fit the queue.
    pub artifacts_dropped: u64,
}

/// Why a store call failed. Recording never fails a run: these come only from `open` and reads.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Another store, in this process or another, has the file open.
    #[error("another process has the trace store {path} open (it holds {lock})")]
    Locked { path: PathBuf, lock: PathBuf },
    #[error("trace store file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The file's migration ledger and this build disagree beyond repair.
    #[error("this build cannot open the trace store {path}: {reason}")]
    Schema { path: PathBuf, reason: String },
    #[error("trace store database: {source}")]
    Database {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    /// A stored value this build cannot read back.
    #[error("trace store row: {0}")]
    Row(String),
    #[error("the trace store is closed")]
    Closed,
    /// A newer build migrated the file, so this one does not write it.
    #[error("the trace store is read-only: a newer build migrated it")]
    ReadOnly,
}

impl StoreError {
    pub(crate) fn db(error: DbErr) -> Self {
        Self::Database {
            source: Box::new(error),
        }
    }

    pub(crate) fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// One trace store: its file, open for this process. Cheap to clone; every clone is the same
/// store.
#[derive(Clone)]
pub struct TraceStore {
    inner: Arc<Inner>,
}

struct Inner {
    readers: DatabaseConnection,
    read_only: bool,
    closed: AtomicBool,
    /// Held until `close`; the OS releases it if the process dies.
    lock: Mutex<Option<db::LockFile>>,
    queue: ingest::Queue,
    heads: Arc<live::Heads>,
    worker: Mutex<Option<JoinHandle<()>>>,
    shutdown_db: Option<DatabaseConnection>,
    close_gate: AsyncMutex<()>,
}

impl TraceStore {
    /// Opens (creating, migrating or recovering) the store at `config.path`.
    ///
    /// # Errors
    ///
    /// [`StoreError::Locked`] when another store has the file open, [`StoreError::Schema`]
    /// when its ledger has a gap or diverged from this build, and [`StoreError::Io`] or
    /// [`StoreError::Database`] when the file cannot be opened.
    pub async fn open(config: StoreConfig) -> Result<Self, StoreError> {
        let files = db::open(&config.path).await?;
        let read_only = files.writer.is_none();
        let (queue, rx) = ingest::Queue::new(&config);
        let heads = Arc::new(live::Heads::default());
        if let Some(writer) = &files.writer {
            apply::reconcile(writer, writer::now())
                .await
                .map_err(StoreError::db)?;
            while retention::step(writer, config.retention, writer::now())
                .await
                .map_err(StoreError::db)?
            {}
        }
        heads
            .initialize(&files.readers, writer::now())
            .await
            .map_err(StoreError::db)?;
        let shutdown_db = files.writer.clone();
        let worker = files.writer.map(|db| {
            task::spawn(
                writer::Writer {
                    db,
                    rx,
                    state: Arc::clone(&queue.state),
                    heads: Arc::clone(&heads),
                    retention: config.retention,
                }
                .run(),
            )
        });
        if read_only {
            queue.state.stopped.store(true, Ordering::Release);
        }
        Ok(Self {
            inner: Arc::new(Inner {
                readers: files.readers,
                read_only,
                closed: AtomicBool::new(false),
                lock: Mutex::new(Some(files.lock)),
                queue,
                heads,
                worker: Mutex::new(worker),
                shutdown_db,
                close_gate: AsyncMutex::new(()),
            }),
        })
    }

    /// A listener that only queues runtime facts. It is safe on a plain thread without tokio.
    #[must_use]
    pub fn sink(&self) -> Arc<dyn TraceSink> {
        Arc::new(self.inner.queue.clone())
    }

    /// Subscribe to a session's committed head; no database polling is required.
    #[must_use]
    pub fn watch(&self, key: &SessionKey) -> watch::Receiver<SessionHead> {
        self.inner.heads.watch(key)
    }

    /// A FIFO marker. Completion covers all entries admitted before it, including retries and
    /// the publication of their session heads. Hosts may bound the wait with their own timeout.
    pub async fn flushed(&self) {
        let (tx, rx) = oneshot::channel();
        if self.inner.queue.control(ingest::Command::Flush(tx)) {
            let _ = rx.await;
        }
    }

    /// Queue a feed's capture with the same byte budget and ordering as runtime events.
    pub fn attach_after_call(&self, call: CallId, after: AfterCall) {
        self.inner.queue.artifact(call, after);
    }

    /// Queue a privacy tombstone and delete already stored JPEGs. Captures arriving after the
    /// tombstone keep their metadata but never their JPEG. Await flushed for durable privacy.
    pub fn withhold(&self, key: &SessionKey) {
        self.inner
            .queue
            .control(ingest::Command::Withhold(key.clone()));
    }

    /// Erase a session and its owned rows through the writer's FIFO, in row-bounded chunks.
    ///
    /// # Errors
    /// Returns Closed/ReadOnly if writes are disabled, or a database error if erasure fails.
    pub async fn forget_session(&self, key: &SessionKey) -> Result<u64, StoreError> {
        if self.inner.read_only {
            return Err(StoreError::ReadOnly);
        }
        let (tx, rx) = oneshot::channel();
        if !self
            .inner
            .queue
            .control(ingest::Command::Forget(key.clone(), tx))
        {
            return Err(StoreError::Closed);
        }
        rx.await.map_err(|_| StoreError::Closed)?
    }

    /// The runs `query` selects, newest first.
    ///
    /// # Errors
    ///
    /// [`StoreError::Closed`] after [`close`](Self::close); otherwise a database or row error.
    pub async fn runs(&self, query: RunQuery) -> Result<Listing<RunSummary>, StoreError> {
        query::runs(self.readers()?, &query).await
    }

    /// One run with its logs and pages, or `None` if the store has no such run.
    ///
    /// # Errors
    ///
    /// [`StoreError::Closed`] after [`close`](Self::close); otherwise a database or row error.
    pub async fn run(&self, id: &RunId) -> Result<Option<RunTrace>, StoreError> {
        query::run(self.readers()?, *id).await
    }

    /// One page of a run's calls, in call order.
    ///
    /// # Errors
    ///
    /// [`StoreError::Closed`] after [`close`](Self::close); otherwise a database or row error.
    pub async fn calls(
        &self,
        run: &RunId,
        query: CallQuery,
    ) -> Result<Listing<CallTrace>, StoreError> {
        query::calls(self.readers()?, *run, query).await
    }

    /// A run's getters, in order (at most 10,000; the rest are counted on the run).
    ///
    /// # Errors
    ///
    /// [`StoreError::Closed`] after [`close`](Self::close); otherwise a database or row error.
    pub async fn getters(&self, run: &RunId) -> Result<Vec<GetterTrace>, StoreError> {
        query::getters(self.readers()?, *run).await
    }

    /// A session's runs and calls, or `None` if it has no runs.
    ///
    /// # Errors
    ///
    /// [`StoreError::Closed`] after [`close`](Self::close); otherwise a database or row error.
    pub async fn session(
        &self,
        key: &SessionKey,
        scope: Timeline,
    ) -> Result<Option<SessionTimeline>, StoreError> {
        query::session(self.readers()?, key, scope).await
    }

    /// The sessions `query` selects, most recently active first.
    ///
    /// # Errors
    ///
    /// [`StoreError::Closed`] after [`close`](Self::close); otherwise a database or row error.
    pub async fn sessions(
        &self,
        query: SessionQuery,
    ) -> Result<Listing<SessionSummary>, StoreError> {
        query::sessions(self.readers()?, &query).await
    }

    /// Whether the store has a run of this session.
    ///
    /// # Errors
    ///
    /// [`StoreError::Closed`] after [`close`](Self::close); otherwise a database error.
    pub async fn knows(&self, key: &SessionKey) -> Result<bool, StoreError> {
        query::knows(self.readers()?, key).await
    }

    #[must_use]
    pub fn health(&self) -> StoreHealth {
        let mut health = self.inner.queue.state.health(self.inner.read_only);
        health.closed = self.inner.closed.load(Ordering::Acquire);
        health.last_error = health
            .failing_since_ms
            .map(|_| "database write failed; retrying".to_string());
        health
    }

    /// Seal producer admission, then drain within budget. On timeout the task is cancelled;
    /// pool cleanup retains the OS lock until SQLite has finished any in-flight operation.
    pub async fn close(&self, budget: Duration) -> StoreHealth {
        let deadline = Instant::now() + budget;
        let Ok(_close) = time::timeout_at(deadline, self.inner.close_gate.lock()).await else {
            return self.health();
        };
        if self.inner.closed.swap(true, Ordering::AcqRel) {
            return self.health();
        }
        self.inner
            .queue
            .state
            .stopped
            .store(true, Ordering::Release);
        let mut worker = self
            .inner
            .worker
            .lock()
            .ok()
            .and_then(|mut worker| worker.take());
        let drained = time::timeout_at(deadline, async {
            while self.inner.queue.state.producers.load(Ordering::Acquire) != 0 {
                task::yield_now().await;
            }
            self.inner.queue.send(ingest::Command::Stop, None);
            if let Some(worker) = &mut worker {
                let _ = worker.await;
            }
        })
        .await
        .is_ok();
        if !drained {
            self.inner.queue.state.unflushed_events.store(
                self.inner.queue.state.queued.load(Ordering::Acquire),
                Ordering::Release,
            );
            self.inner.queue.state.unflushed_bytes.store(
                self.inner.queue.state.bytes.load(Ordering::Acquire),
                Ordering::Release,
            );
            if let Some(worker) = &worker {
                worker.abort();
            }
        }
        let lock = self.inner.lock.lock().ok().and_then(|mut lock| lock.take());
        let writer = self.inner.shutdown_db.clone();
        let readers = self.inner.readers.clone();
        // Cleanup can outlive a timed-out close; it owns the lock until both pools close.
        let mut cleanup = task::spawn(async move {
            if !drained && let Some(worker) = worker {
                let _ = worker.await;
            }
            if let Some(writer) = writer
                && let Err(error) = writer.close().await
            {
                warn!(%error, "closing trace write pool");
            }
            if let Err(error) = readers.close().await {
                warn!(%error, "closing trace read pool");
            }
            drop(lock);
        });
        let _ = time::timeout_at(deadline, &mut cleanup).await;
        self.health()
    }

    fn readers(&self) -> Result<&DatabaseConnection, StoreError> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(StoreError::Closed);
        }
        Ok(&self.inner.readers)
    }
}

// A host that drops its last store without close must not leave an immortal retrying writer.
// Sinks may still exist, so seal admission before cancelling the task.
impl Drop for Inner {
    fn drop(&mut self) {
        self.queue.state.stopped.store(true, Ordering::Release);
        let worker = self.worker.get_mut().ok().and_then(Option::take);
        if let Some(worker) = &worker {
            worker.abort();
        }
        let lock = self.lock.get_mut().ok().and_then(Option::take);
        let writer = self.shutdown_db.clone();
        let readers = self.readers.clone();
        if let Ok(runtime) = Handle::try_current() {
            // SQLx can still have a SQLite operation running on its blocking worker after its
            // caller is cancelled. Cleanup owns the lock until both pools have truly closed.
            runtime.spawn(async move {
                if let Some(worker) = worker {
                    let _ = worker.await;
                }
                if let Some(writer) = writer {
                    let _ = writer.close().await;
                }
                let _ = readers.close().await;
                drop(lock);
            });
        }
    }
}
