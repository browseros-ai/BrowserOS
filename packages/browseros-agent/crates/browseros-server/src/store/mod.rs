//! `store`: the existing `<state dir>/db/browseros.sqlite`, through SeaORM. Adopts the old
//! server's tables in place and never drops a table or a row, because the old server may be
//! started against the same file again (the browser falls back to its bundled server after
//! three failed health strikes). The entities, the migration and the column conventions are
//! private here.
//!
//! Four row sets hang off one `Store`, each with one writer in the rest of the server:
//!
//! ```text
//!   conversations()  ConversationRows   written only by `conversations`
//!   providers()      Providers          LLM provider rows (kind = 'llm'), through `http`
//!   agents()         Agents             coding-agent rows (kind = 'acp'), through `http`
//!   tokens()         Tokens             written only by `model::chatgpt`
//! ```
//!
//! The layering test enforces the two "written only by" rules. Secret columns are read only
//! here: a reader gets `has_*` flags, and an outbound model call gets `ProviderConfig`.

#[cfg(test)]
mod adoption_tests;
mod agents;
mod conversations;
mod entities;
mod migration;
mod providers;
mod tokens;

use std::path::{Path, PathBuf};
use std::time::Duration;

use sea_orm::sqlx::sqlite::{
    SqliteConnectOptions, SqliteError, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbErr, RuntimeErr, SqlxSqliteConnector, Statement,
};
use sea_orm_migration::MigratorTrait;

use migration::Migrator;

pub use agents::{AgentPatch, AgentRow, Agents, NewAgent};
pub use conversations::{ConversationRow, ConversationRows, ConversationSummary};
pub use providers::{
    Credentials, DraftError, ProviderConfig, ProviderDraft, ProviderEdit, ProviderKind,
    ProviderRow, Providers, RowKind,
};
pub use tokens::{TokenRow, Tokens};

/// A secret that cannot be logged or sent: no `Debug`, no `Serialize`.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The plaintext, for the one place that sends it outward: an outbound request header.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// How a save treats one nullable column: absent keeps the stored value, `null` clears it, a
/// value replaces it.
#[derive(Clone, PartialEq, Default)]
pub enum Edit<T> {
    #[default]
    Keep,
    Clear,
    Set(T),
}

/// How a save treats a credential: absent or blank keeps it, `null` clears it. The blank rule is
/// applied where the request is parsed, so `Set` never holds an empty secret.
pub type SecretEdit = Edit<Secret>;

/// The one handle on the database. Cheap to clone; every clone shares one connection, which is
/// what serialises writes.
#[derive(Clone)]
pub struct Store {
    db: DatabaseConnection,
}

/// Why a store call failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] DbErr),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The file is not the old server's schema 0014 after the legacy migrations ran, so this
    /// server refuses it rather than failing on the first query.
    #[error("the database is not the expected schema: {0} is missing")]
    Schema(String),
    /// A stored value this server cannot read (a transcript that is not a `UiMessage[]`, …).
    #[error("unreadable row: {0}")]
    Unreadable(String),
    /// A body the skeleton has not filled in yet.
    #[error("{0} is not implemented in this build of the server")]
    NotImplemented(&'static str),
}

impl StoreError {
    /// Another connection holds the database's write lock (SQLite `BUSY` after the 5 s busy
    /// timeout). At start-up this is worth waiting out; anything else is not.
    pub fn is_busy(&self) -> bool {
        matches!(self, StoreError::Db(error) if sqlite_code(error).is_some_and(|code| code & SQLITE_PRIMARY_RESULT_CODE_MASK == SQLITE_BUSY))
    }
}

/// The file under the state directory: the old server's path, so the same history.
pub const DATABASE_FILE: &str = "browseros.sqlite";

impl Store {
    /// Opens or creates `<state_dir>/db/browseros.sqlite`: WAL, foreign keys on, a 5 s busy
    /// timeout and one connection (claw-server's `Database::open`). Then, in order: the old
    /// server's missing Drizzle migrations are replayed (`migration.rs` says why), the result is
    /// checked to be the 0014 schema, and this server's own migrator runs. A file that is
    /// confirmed corrupt (`SQLITE_CORRUPT`, `SQLITE_NOTADB`) is renamed aside with its `-wal`
    /// and `-shm` (`browseros.sqlite.<n>.bak`, never over an earlier backup) and a fresh one is
    /// made; a lock or a newer schema never moves the file.
    pub async fn open(state_dir: &Path) -> Result<Self, StoreError> {
        let directory = state_dir.join("db");
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|source| StoreError::Io {
                path: directory.clone(),
                source,
            })?;
        let path = directory.join(DATABASE_FILE);
        match connect_and_migrate(&path).await {
            Err(StoreError::Db(error)) if is_corrupt(&error) => {
                tracing::error!(path = %path.display(), %error, "the database is corrupt; moving it aside and starting a fresh one");
                back_up(&path).await?;
                connect_and_migrate(&path).await
            }
            result => result,
        }
    }

    /// Closes the connection: the last step of shutdown, after every run wrote its row. Every
    /// clone shares the connection, so every clone is closed.
    pub async fn close(&self) {
        if let Err(error) = self.db.close_by_ref().await {
            tracing::warn!(%error, "closing the database");
        }
    }

    /// The connection, for the row sets in this module only.
    fn db(&self) -> &DatabaseConnection {
        &self.db
    }

    pub fn conversations(&self) -> ConversationRows<'_> {
        ConversationRows(self)
    }

    /// LLM provider rows, and the default pointer, which may point at a row of either kind.
    pub fn providers(&self) -> Providers<'_> {
        Providers(self)
    }

    /// Rows with kind = 'acp'.
    pub fn agents(&self) -> Agents<'_> {
        Agents(self)
    }

    pub fn tokens(&self) -> Tokens<'_> {
        Tokens(self)
    }
}

async fn connect_and_migrate(path: &Path) -> Result<Store, StoreError> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        // The old server opens with foreign keys on, and its migrations ran that way; the
        // replay must see the same connection.
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    // One connection: writes are serialised by the pool, and a transaction's BEGIN and COMMIT
    // always reach the same SQLite connection.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|error| DbErr::Conn(RuntimeErr::SqlxError(error)))?;
    let db = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
    match adopt(&db).await {
        Ok(()) => Ok(Store { db }),
        Err(error) => {
            let _ = db.close_by_ref().await;
            Err(error)
        }
    }
}

async fn adopt(db: &DatabaseConnection) -> Result<(), StoreError> {
    match migration::replay_drizzle(db).await? {
        migration::LegacySchema::Replayed(tags) => {
            tracing::info!(
                migrations = tags.join(", "),
                "brought the database to the old server's schema 0014"
            );
        }
        migration::LegacySchema::Current => {}
        migration::LegacySchema::Ahead { last_created_at } => {
            tracing::warn!(
                last_created_at,
                "the database holds old-server migrations newer than this build knows; reading forward"
            );
        }
    }
    if let Some(missing) = migration::missing_column(db).await? {
        return Err(StoreError::Schema(missing));
    }
    let known: Vec<String> = Migrator::migrations()
        .iter()
        .map(|migration| migration.name().to_owned())
        .collect();
    let applied = applied_migrations(db).await?;
    let unknown: Vec<&String> = applied
        .iter()
        .filter(|name| !known.contains(name))
        .collect();
    let pending = known.iter().any(|name| !applied.contains(name));
    match (unknown.is_empty(), pending) {
        (true, _) => Migrator::up(db, None).await?,
        // Written by a newer build of this server: its extra migrations are additive, so read
        // forward instead of letting `up` abort on rows it does not know.
        (false, false) => tracing::warn!(
            unknown = ?unknown,
            "the database was migrated by a newer server; reading forward"
        ),
        (false, true) => {
            return Err(StoreError::Schema(format!(
                "this build's migrations (the file has {unknown:?}, which this build does not know, \
                 and lacks some this build has)"
            )));
        }
    }
    Ok(())
}

/// SeaORM's applied migrations, or none on a file it never migrated.
async fn applied_migrations(db: &DatabaseConnection) -> Result<Vec<String>, DbErr> {
    let statement = Statement::from_string(
        db.get_database_backend(),
        "SELECT version FROM seaql_migrations ORDER BY version",
    );
    match db.query_all(statement).await {
        Ok(rows) => rows
            .iter()
            .map(|row| row.try_get::<String>("", "version"))
            .collect(),
        Err(error)
            if error
                .to_string()
                .contains("no such table: seaql_migrations") =>
        {
            Ok(Vec::new())
        }
        Err(error) => Err(error),
    }
}

const SQLITE_BUSY: i32 = 5;
const SQLITE_CORRUPT: i32 = 11;
const SQLITE_NOTADB: i32 = 26;
/// SQLite extended result codes keep the primary code in the low byte.
const SQLITE_PRIMARY_RESULT_CODE_MASK: i32 = 0xff;

fn sqlite_code(error: &DbErr) -> Option<i32> {
    let (DbErr::Conn(runtime) | DbErr::Exec(runtime) | DbErr::Query(runtime)) = error else {
        return None;
    };
    let RuntimeErr::SqlxError(error) = runtime else {
        return None;
    };
    let database_error = error.as_database_error()?;
    database_error.try_downcast_ref::<SqliteError>()?;
    database_error.code()?.parse().ok()
}

/// Only a confirmed corrupt or non-database file is moved aside.
fn is_corrupt(error: &DbErr) -> bool {
    sqlite_code(error).is_some_and(|code| {
        matches!(
            code & SQLITE_PRIMARY_RESULT_CODE_MASK,
            SQLITE_CORRUPT | SQLITE_NOTADB
        )
    })
}

/// Renames the file and its `-wal` and `-shm` aside as `<file>.<n>.bak`, `.<n>.bak-wal` and
/// `.<n>.bak-shm`, with `n` the first number no earlier backup uses: a second recovery must
/// never replace the copy the first one kept.
async fn back_up(path: &Path) -> Result<(), StoreError> {
    let taken = |n: u32| {
        ["", "-wal", "-shm"]
            .iter()
            .any(|suffix| with_suffix(path, &format!(".{n}.bak{suffix}")).exists())
    };
    let n = (1..).find(|n| !taken(*n)).unwrap_or(u32::MAX);
    for suffix in ["", "-wal", "-shm"] {
        let source = with_suffix(path, suffix);
        match tokio::fs::rename(&source, with_suffix(path, &format!(".{n}.bak{suffix}"))).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source_error) => {
                return Err(StoreError::Io {
                    path: source,
                    source: source_error,
                });
            }
        }
    }
    Ok(())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}
