//! Opening the shared file: the order of steps and what each protects.
//!
//! Corrupt files are preserved before best-effort row salvage. A one-time VACUUM snapshot is
//! published by fsync + atomic rename before migration. Newer ledgers disable all writes; only
//! compatible files receive drizzle catch-up, the additive Rust ledger, and column validation.

use super::{DbError, MigrationOutcome, drizzle, migrator, recovery};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbErr, RuntimeErr, SqlxSqliteConnector, Statement,
    sqlx::{
        self, Connection,
        sqlite::{
            SqliteConnectOptions, SqliteConnection, SqliteError, SqliteJournalMode,
            SqlitePoolOptions,
        },
    },
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

const SQLITE_CORRUPT: i32 = 11;
const SQLITE_NOTADB: i32 = 26;
const SQLITE_PRIMARY_RESULT_CODE_MASK: i32 = 0xff;

/// Every column the stores read or write, per table. Extra columns (a newer server's additive
/// changes) are fine; a missing one means the file's schema is not one this build can serve.
const REQUIRED_COLUMNS: &[(&str, &[&str])] = &[
    (
        "providers",
        &[
            "id",
            "kind",
            "type",
            "name",
            "model_id",
            "reasoning_effort",
            "is_default",
            "created_at",
            "updated_at",
            "base_url",
            "headers",
            "supports_images",
            "context_window",
            "temperature",
            "api_key",
            "access_key_id",
            "secret_access_key",
            "session_token",
            "resource_name",
            "region",
            "reasoning_summary",
            "working_directory",
            "custom_config",
        ],
    ),
    (
        "conversations",
        &[
            "id",
            "messages",
            "last_user_message",
            "origin",
            "target_type",
            "agent_id",
            "last_messaged_at",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "oauth_tokens",
        &[
            "browseros_id",
            "provider",
            "access_token",
            "refresh_token",
            "expires_at",
            "email",
            "account_id",
            "updated_at",
        ],
    ),
    (
        "scheduled_jobs",
        &[
            "id",
            "name",
            "query",
            "schedule_type",
            "schedule_time",
            "schedule_interval",
            "enabled",
            "provider_id",
            "last_run_at",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "scheduled_job_runs",
        &[
            "id",
            "job_id",
            "status",
            "started_at",
            "completed_at",
            "result",
            "final_result",
            "execution_log",
            "tool_calls",
            "error",
            "created_at",
            "updated_at",
        ],
    ),
];

enum OpenFailure {
    Sqlite(DbErr),
    Other(DbError),
    Corrupt(String),
}

impl From<DbErr> for OpenFailure {
    fn from(error: DbErr) -> Self {
        Self::Sqlite(error)
    }
}

impl From<sqlx::Error> for OpenFailure {
    fn from(error: sqlx::Error) -> Self {
        Self::Sqlite(DbErr::Conn(RuntimeErr::SqlxError(error)))
    }
}

impl From<DbError> for OpenFailure {
    fn from(error: DbError) -> Self {
        Self::Other(error)
    }
}

pub(super) async fn open(path: &Path) -> Result<(DatabaseConnection, MigrationOutcome), DbError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|source| io_error("create the database directory", parent, source))?;
    }
    match open_once(path).await {
        Ok(opened) => Ok(opened),
        Err(failure) => {
            let corruption = match &failure {
                OpenFailure::Corrupt(reason) => Some(reason.clone()),
                OpenFailure::Sqlite(error) if is_recoverable(error) => Some(error.to_string()),
                _ => None,
            };
            let Some(reason) = corruption else {
                return Err(into_db_error(failure));
            };
            let aside = set_aside(path).await?;
            tracing::error!(%reason, moved_to = %aside.display(), "preserved corrupt SQLite database; attempting recovery");
            let (connection, _) = open_once(path).await.map_err(into_db_error)?;
            let salvaged = recovery::salvage(&aside, path).await;
            Ok((
                connection,
                MigrationOutcome::RecreatedAfterCorruption { aside, salvaged },
            ))
        }
    }
}

async fn open_once(path: &Path) -> Result<(DatabaseConnection, MigrationOutcome), OpenFailure> {
    let existed = tokio::fs::metadata(path)
        .await
        .is_ok_and(|metadata| metadata.len() > 0);
    if existed {
        back_up_once(path).await?;
    }

    let mut connection = SqliteConnection::connect_with(&connect_options(path)).await?;
    let check: Vec<String> = sea_orm::sqlx::query_scalar("PRAGMA quick_check")
        .fetch_all(&mut connection)
        .await?;
    if check != ["ok"] {
        connection.close().await?;
        return Err(OpenFailure::Corrupt(check.join("; ")));
    }
    let rust_unknown = migrator::unknown_sqlite_versions(&mut connection).await?;
    let caught_up = if rust_unknown.is_empty() {
        drizzle::catch_up(&mut connection).await
    } else {
        Ok(drizzle::CatchUp {
            applied: 0,
            unknown: rust_unknown,
        })
    };
    connection.close().await?;
    let caught_up = caught_up?;

    let pool = connect_pool(path, false).await?;
    let connection = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
    let settled = match if caught_up.unknown.is_empty() {
        migrator::migrate_forward_compatible(&connection).await
    } else {
        Ok(migrator::LedgerOutcome::Applied(0))
    } {
        Ok(ledger) => verify_columns(&connection).await.map(|()| ledger),
        Err(error) => Err(error),
    };
    let ledger = match settled {
        Ok(ledger) => ledger,
        Err(error) => {
            connection.close().await?;
            return Err(error.into());
        }
    };

    let mut unknown = caught_up.unknown;
    let rust_applied = match ledger {
        migrator::LedgerOutcome::Applied(applied) => applied,
        migrator::LedgerOutcome::Ahead(rust_unknown) => {
            unknown.extend(rust_unknown);
            0
        }
    };
    let outcome = if !unknown.is_empty() {
        MigrationOutcome::DatabaseAhead { unknown }
    } else if !existed {
        MigrationOutcome::Created
    } else if caught_up.applied == 0 && rust_applied == 0 {
        MigrationOutcome::UpToDate
    } else {
        MigrationOutcome::Migrated {
            drizzle_applied: caught_up.applied,
            rust_applied,
        }
    };
    if matches!(outcome, MigrationOutcome::DatabaseAhead { .. }) {
        connection.close().await?;
        let pool = connect_pool(path, true).await?;
        return Ok((SqlxSqliteConnector::from_sqlx_sqlite_pool(pool), outcome));
    }
    Ok((connection, outcome))
}

/// The TS server's pragmas (WAL, foreign keys on) plus a short per-attempt busy timeout.
/// Write admission bounds and backs off retries in write.rs. Nothing
/// else is set, so page size, auto-vacuum and synchronous stay what the file already has.
fn connect_options(path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(100))
}

async fn connect_pool(path: &Path, read_only: bool) -> Result<sqlx::SqlitePool, OpenFailure> {
    let options = if read_only {
        SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_millis(100))
    } else {
        connect_options(path)
    };
    // A blocking task keeps paused Tokio clocks (tests) from expiring sqlx's acquire timeout
    // before the SQLite worker answers (claw-server's workaround).
    let runtime = tokio::runtime::Handle::current();
    let pool = tokio::task::spawn_blocking(move || {
        runtime.block_on(
            SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options),
        )
    })
    .await
    .map_err(|error| DbErr::Custom(format!("SQLite connection task failed: {error}")))??;
    Ok(pool)
}

async fn verify_columns(connection: &DatabaseConnection) -> Result<(), DbErr> {
    for (table, required) in REQUIRED_COLUMNS {
        let present: Vec<String> = connection
            .query_all(Statement::from_string(
                connection.get_database_backend(),
                format!("SELECT name FROM pragma_table_info('{table}')"),
            ))
            .await?
            .iter()
            .map(|row| row.try_get::<String>("", "name"))
            .collect::<Result<_, _>>()?;
        let missing: Vec<&str> = required
            .iter()
            .copied()
            .filter(|column| !present.iter().any(|name| name == column))
            .collect();
        if !missing.is_empty() {
            return Err(DbErr::Custom(format!(
                "database schema is incompatible with this BrowserOS server: table {table} lacks {}",
                missing.join(", ")
            )));
        }
    }
    Ok(())
}

async fn back_up_once(path: &Path) -> Result<(), OpenFailure> {
    let marker = with_suffix(path, ".pre-rust.bak");
    if tokio::fs::try_exists(&marker)
        .await
        .map_err(|source| io_error("check backup marker", &marker, source))?
    {
        return Ok(());
    }
    let temporary = with_suffix(path, &format!(".pre-rust-{}.tmp", uuid::Uuid::new_v4()));
    let cleanup = TemporaryFile(temporary.clone());
    // VACUUM reads a single SQLite snapshot, including committed WAL pages, while a TS
    // connection keeps writing/checkpointing. A raw copy of db + WAL has no such guarantee.
    let mut source = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(path)
            .busy_timeout(Duration::from_secs(5)),
    )
    .await?;
    let vacuum = sqlx::query("VACUUM INTO ?")
        .bind(temporary.to_string_lossy().as_ref())
        .execute(&mut source)
        .await;
    source.close().await?;
    vacuum?;
    let file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&temporary)
        .await
        .map_err(|source| io_error("open backup snapshot", &temporary, source))?;
    file.sync_all()
        .await
        .map_err(|source| io_error("fsync backup snapshot", &temporary, source))?;
    drop(file);
    tokio::fs::rename(&temporary, &marker)
        .await
        .map_err(|source| io_error("publish backup snapshot", &marker, source))?;
    // Sync the directory too: the rename is the durable completion marker after power loss.
    #[cfg(unix)]
    if let Some(parent) = marker.parent() {
        std::fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|source| io_error("fsync backup directory", parent, source))?;
    }
    drop(cleanup);
    tracing::info!(backup = %marker.display(), "backed up the database before its first Rust open");
    Ok(())
}

/// Unpublished snapshots never count as backups, including cancellation during VACUUM.
struct TemporaryFile(PathBuf);
impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Renames the unusable file (and its WAL/SHM) to `browseros.sqlite.corrupt-<ms>.bak*`.
async fn set_aside(path: &Path) -> Result<PathBuf, DbError> {
    let stamp = crate::js::now_ms();
    let aside = with_suffix(
        path,
        &format!(".corrupt-{stamp}-{}.bak", uuid::Uuid::new_v4()),
    );
    for suffix in ["", "-wal", "-shm"] {
        let source = with_suffix(path, suffix);
        match tokio::fs::rename(&source, with_suffix(&aside, suffix)).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("move the corrupt database aside", &source, error)),
        }
    }
    Ok(aside)
}

fn is_recoverable(error: &DbErr) -> bool {
    let (DbErr::Conn(RuntimeErr::SqlxError(error))
    | DbErr::Exec(RuntimeErr::SqlxError(error))
    | DbErr::Query(RuntimeErr::SqlxError(error))) = error
    else {
        return false;
    };
    let Some(database_error) = error.as_database_error() else {
        return false;
    };
    if database_error.try_downcast_ref::<SqliteError>().is_none() {
        return false;
    }
    database_error
        .code()
        .and_then(|code| code.parse::<i32>().ok())
        .is_some_and(|code| {
            // Extended result codes keep the primary code in the low byte.
            matches!(
                code & SQLITE_PRIMARY_RESULT_CODE_MASK,
                SQLITE_CORRUPT | SQLITE_NOTADB
            )
        })
}

fn into_db_error(failure: OpenFailure) -> DbError {
    match failure {
        OpenFailure::Sqlite(error) => DbError::Migration(error.to_string()),
        OpenFailure::Other(error) => error,
        OpenFailure::Corrupt(reason) => DbError::Migration(reason),
    }
}

fn io_error(action: &str, path: &Path, source: std::io::Error) -> DbError {
    DbError::Io {
        context: format!("failed to {action} at {}", path.display()),
        source,
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = OsString::from(path.as_os_str());
    value.push(suffix);
    value.into()
}
