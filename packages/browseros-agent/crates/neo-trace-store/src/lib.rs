//! Keeps neo-runtime's run traces (runs, browser calls, getters, pages, console lines) in one
//! SQLite file through SeaORM, and reads them back as typed views.
//!
//! ```text
//!  TraceStore::open ── db.rs: lock file, pragmas, corrupt-file recovery,
//!                      migration.rs on its own ledger, in one transaction
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

use neo_runtime::{RunId, SessionKey};
use sea_orm::{DatabaseConnection, DbErr};
use serde::Serialize;
use tracing::warn;

mod db;
mod query;
#[cfg(test)]
mod tests;
mod views;

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
    /// Bytes held back, once the queue is full, for stubs and run lifecycle events: 4 MiB. A
    /// quarter of it only ever holds lifecycle events, so a run's end and settle outlast its
    /// calls' stubs.
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
        if let Some(writer) = files.writer
            && let Err(error) = writer.close().await
        {
            warn!(%error, "closing the trace store's write connection");
        }
        Ok(Self {
            inner: Arc::new(Inner {
                readers: files.readers,
                read_only,
                closed: AtomicBool::new(false),
                lock: Mutex::new(Some(files.lock)),
            }),
        })
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
        StoreHealth {
            read_only: self.inner.read_only,
            closed: self.inner.closed.load(Ordering::Acquire),
            ..StoreHealth::default()
        }
    }

    /// Closes the store: no more reads, the read pool closes and the lock is released, so the
    /// file can be opened again. Calling it again does nothing.
    pub async fn close(&self, _budget: Duration) -> StoreHealth {
        if !self.inner.closed.swap(true, Ordering::AcqRel) {
            if let Err(error) = self.inner.readers.clone().close().await {
                warn!(%error, "closing the trace store's read pool");
            }
            if let Ok(mut lock) = self.inner.lock.lock() {
                lock.take();
            }
        }
        self.health()
    }

    fn readers(&self) -> Result<&DatabaseConnection, StoreError> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(StoreError::Closed);
        }
        Ok(&self.inner.readers)
    }
}
