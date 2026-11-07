//! The store's file: the lock that keeps it to one process, the pragmas, recovery of a corrupt
//! file, migrations on the store's own ledger, and the two kinds of connection.
//!
//! - **One write connection**, owned by the writer task (`writer.rs`): writes never contend with
//!   each other, so SQLite never has to upgrade a reader to a writer mid-transaction.
//! - **A read pool of four**, `query_only`: WAL lets reads run beside the writer, so a `/v1`
//!   poll never queues behind a batch. claw-server shares one connection for both
//!   (`cs/db/mod.rs:164-176`), which is how its slow reads end up blocking its tool calls.
//!
//! Every connection is opened inside `spawn_blocking`, so a paused tokio clock cannot expire
//! sqlx's acquire timeout while SQLite's worker thread answers (`cs/db/mod.rs:165-174`); every
//! pool opens all its connections up front for the same reason. Everything here is
//! `pub(crate)`: no SeaORM type leaves the crate.

use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::Duration;

use sea_orm::sqlx::Error as SqlxError;
use sea_orm::sqlx::sqlite::{
    SqliteAutoVacuum, SqliteConnectOptions, SqliteError, SqliteJournalMode, SqlitePoolOptions,
    SqliteSynchronous,
};
use sea_orm::{DatabaseConnection, DbErr, RuntimeErr, SqlxSqliteConnector};
use tokio::runtime::Handle;
use tokio::{fs, task};
use tracing::warn;

use crate::StoreError;

pub(crate) mod entities;
pub(crate) mod kinds;
pub(crate) mod migration;

use migration::{Migrator, Refusal, Schema};

/// The read pool's size.
const READERS: u32 = 4;

/// How long a connection waits on SQLite's lock before failing (claw-server's value).
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// SQLite's primary result codes, the low byte of an extended code.
pub(crate) const SQLITE_CORRUPT: i32 = 11;
pub(crate) const SQLITE_NOTADB: i32 = 26;

/// An open store file.
pub(crate) struct Files {
    /// `None` when a newer build migrated the file: this build reads it and never writes it.
    pub(crate) writer: Option<DatabaseConnection>,
    pub(crate) readers: DatabaseConnection,
    pub(crate) lock: LockFile,
}

/// Opens the store at `path`: takes the lock, connects the writer, recovers a corrupt file once
/// (`cs/db/mod.rs:96-148`), migrates, and connects the read pool.
pub(crate) async fn open(path: &Path) -> Result<Files, StoreError> {
    if let Some(parent) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .await
            .map_err(|source| StoreError::io(parent, source))?;
    }
    let lock = LockFile::take(path)?;
    let (writer, schema) = match connect_and_migrate(path).await {
        Err(Refusal::Db(error)) if sqlite_code(&error).is_some_and(is_corrupt) => {
            warn!(
                path = %path.display(),
                %error,
                "trace store file is corrupt or not a database; moving it aside and starting anew"
            );
            back_up(path).await?;
            connect_and_migrate(path).await
        }
        opened => opened,
    }
    .map_err(|refusal| match refusal {
        Refusal::Db(error) => StoreError::db(error),
        Refusal::Mismatch(reason) => StoreError::Schema {
            path: path.to_path_buf(),
            reason,
        },
    })?;
    let writer = match schema {
        Schema::Current => Some(writer),
        Schema::Ahead { unknown } => {
            warn!(
                path = %path.display(),
                unknown = unknown.join(", "),
                "a newer build migrated the trace store; this build opens it read-only"
            );
            writer.close().await.map_err(StoreError::db)?;
            None
        }
    };
    let readers = connect(reader_options(path), READERS)
        .await
        .map_err(StoreError::db)?;
    Ok(Files {
        writer,
        readers,
        lock,
    })
}

async fn connect_and_migrate(path: &Path) -> Result<(DatabaseConnection, Schema), Refusal> {
    let writer = connect_writer(path).await?;
    match migration::run_pending::<Migrator>(&writer, None).await {
        Ok(schema) => Ok((writer, schema)),
        Err(refusal) => {
            if let Err(error) = writer.close().await {
                warn!(%error, "closing the trace store after a failed open");
            }
            Err(refusal)
        }
    }
}

/// The one write connection, creating the file if it is missing.
pub(crate) async fn connect_writer(path: &Path) -> Result<DatabaseConnection, DbErr> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        // Takes only before the first table exists, so a file the store creates is incremental
        // from the start and retention's `incremental_vacuum` can hand pages back.
        .auto_vacuum(SqliteAutoVacuum::Incremental)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(BUSY_TIMEOUT);
    connect(options, 1).await
}

fn reader_options(path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .foreign_keys(true)
        .busy_timeout(BUSY_TIMEOUT)
        .pragma("query_only", "ON")
}

async fn connect(
    options: SqliteConnectOptions,
    connections: u32,
) -> Result<DatabaseConnection, DbErr> {
    let runtime = Handle::try_current().map_err(|error| {
        DbErr::Custom(format!("the trace store needs a tokio runtime: {error}"))
    })?;
    let pool = task::spawn_blocking(move || {
        runtime.block_on(async move {
            let pool = SqlitePoolOptions::new()
                .max_connections(connections)
                .min_connections(connections)
                .idle_timeout(None)
                .max_lifetime(None)
                .test_before_acquire(false)
                .connect_with(options)
                .await?;
            let mut opened = Vec::new();
            for _ in 0..connections {
                opened.push(pool.acquire().await?);
            }
            drop(opened);
            Ok::<_, SqlxError>(pool)
        })
    })
    .await
    .map_err(|error| DbErr::Custom(format!("SQLite connection task failed: {error}")))?
    .map_err(|error| DbErr::Conn(RuntimeErr::SqlxError(error)))?;
    Ok(SqlxSqliteConnector::from_sqlx_sqlite_pool(pool))
}

/// The primary SQLite result code behind `error`, if SQLite raised it.
pub(crate) fn sqlite_code(error: &DbErr) -> Option<i32> {
    let (DbErr::Conn(RuntimeErr::SqlxError(error))
    | DbErr::Exec(RuntimeErr::SqlxError(error))
    | DbErr::Query(RuntimeErr::SqlxError(error))) = error
    else {
        return None;
    };
    let database_error = error.as_database_error()?;
    database_error.try_downcast_ref::<SqliteError>()?;
    // An extended result code keeps the primary code in its low byte.
    database_error
        .code()
        .and_then(|code| code.parse::<i32>().ok())
        .map(|code| code & 0xff)
}

fn is_corrupt(code: i32) -> bool {
    matches!(code, SQLITE_CORRUPT | SQLITE_NOTADB)
}

/// Moves the file and its sidecars to `.bak*`, as claw-server does (`cs/db/mod.rs:264-287`).
async fn back_up(path: &Path) -> Result<(), StoreError> {
    for (from, to) in [("", ".bak"), ("-wal", ".bak-wal"), ("-shm", ".bak-shm")] {
        let source = sidecar(path, from);
        match fs::rename(&source, sidecar(path, to)).await {
            Err(error) if error.kind() != ErrorKind::NotFound => {
                return Err(StoreError::io(&source, error));
            }
            _ => {}
        }
    }
    Ok(())
}

/// `path` with `suffix` appended to its file name.
pub(crate) fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    name.into()
}

/// `<database>.lock`, held with an OS advisory lock for as long as the store is open. A second
/// store on the same file, in this process or another, fails to open instead of interleaving
/// writes and marking the first one's live runs `interrupted`. The lock belongs to the open file
/// handle, so a process that dies never leaves it held.
pub(crate) struct LockFile {
    _file: File,
}

impl LockFile {
    fn take(database: &Path) -> Result<Self, StoreError> {
        let path = sidecar(database, ".lock");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| StoreError::io(&path, source))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(StoreError::Locked {
                    path: database.to_path_buf(),
                    lock: path,
                });
            }
            Err(TryLockError::Error(source)) => return Err(StoreError::io(&path, source)),
        }
        // The holder's pid, for whoever finds the store locked; only the lock itself counts.
        if let Err(error) = file
            .set_len(0)
            .and_then(|()| writeln!(file, "{}", process::id()))
        {
            warn!(path = %path.display(), %error, "writing the trace store lock file's pid");
        }
        Ok(Self { _file: file })
    }
}

/// A stored integer as the `u64` it was written from; the store never writes a negative one.
pub(crate) fn unsigned(stored: i64) -> u64 {
    u64::try_from(stored).unwrap_or(0)
}

/// A `u64` as SQLite's `INTEGER`, saturating past `i64::MAX` (about 292,000 years of
/// microseconds).
pub(crate) fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
