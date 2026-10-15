//! Persistence over the SQLite file the TS server also uses (`<browserosDir>/db/browseros.sqlite`).
//!
//! Either server binary may run against this file on any launch (OTA swaps them independently),
//! so the Rust side never takes the file over. [`Database::open`] backs it up once, replays the TS
//! server's frozen drizzle migrations exactly as drizzle would (keeping `__drizzle_migrations`,
//! so a rolled-back TS server opens the file as a no-op), then runs the Rust-owned sea-orm ledger
//! (`seaql_migrations`, additive-only while the TS server can still run):
//!
//! ```text
//! open.rs ── set aside if corrupt ─ pre-rust backup ─► drizzle.rs catch-up (one tx, FKs off)
//!         ─► migrator.rs ledger ─► column check ─► one-connection pool ─► the five stores
//! ```
//!
//! The stores are concrete types over that single connection, which serializes writes the way
//! the TS per-store write queues did. They keep drizzle's encodings (booleans `0/1`, epoch-ms
//! integers, JSON as `JSON.stringify` text) and decode leniently (`decode.rs`), because every row
//! may have been written by the TS server.

mod agents;
mod conversations;
mod decode;
mod drizzle;
mod entities;
mod js;
mod migrator;
mod oauth_tokens;
mod open;
mod providers;
mod schedules;

pub use agents::{AcpAgentRow, AgentKind, AgentStore, CreateAgent, UpdateAgent};
pub use conversations::{
    ConversationStore, ConversationSummary, ImportConversation, SaveConversation,
    SaveRawConversation, StoredConversation, TargetType,
};
pub use oauth_tokens::{OAuthTokenStore, OAuthTokens};
pub use providers::{ProviderRow, ProviderStore, UpsertProvider};
pub use schedules::{
    RUNS_KEPT_PER_JOB, ScheduleStore, ScheduledJobRow, ScheduledRunRow, UpsertJob, UpsertRun,
};

use crate::paths::BrowserosPaths;
use sea_orm::{DatabaseConnection, DbErr};
use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Query(String),
    #[error("database migration failed: {0}")]
    Migration(String),
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
}

impl From<DbErr> for DbError {
    fn from(error: DbErr) -> Self {
        Self::Query(error.to_string())
    }
}

pub type DbResult<T> = Result<T, DbError>;

/// What `open` did to the file; logged at startup and reported by diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// The file did not exist: every drizzle migration and the Rust ledger were applied.
    Created,
    UpToDate,
    Migrated {
        drizzle_applied: usize,
        rust_applied: usize,
    },
    /// The file carries migrations this build does not know (drizzle hashes from a newer TS
    /// server, or Rust ledger entries from a newer Rust server). Nothing of ours was replayed over
    /// them; the server runs read-forward on what is, by the additive-only rule, a superset schema.
    DatabaseAhead {
        unknown: Vec<String>,
    },
}

/// The shared connection handle. Cloning shares the one connection.
#[derive(Clone)]
pub struct Database {
    connection: DatabaseConnection,
}

impl fmt::Debug for Database {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Database").finish_non_exhaustive()
    }
}

impl Database {
    /// Opens `paths.db_file`, creating it when absent, and brings it to the current schema
    /// (see the module docs for the order and why `__drizzle_migrations` is kept).
    pub async fn open(paths: &BrowserosPaths) -> Result<(Self, MigrationOutcome), DbError> {
        let (connection, outcome) = open::open(&paths.db_file).await?;
        match &outcome {
            MigrationOutcome::DatabaseAhead { unknown } => tracing::warn!(
                unknown = unknown.join(", "),
                "database was migrated by a newer BrowserOS server; starting read-forward"
            ),
            outcome => tracing::info!(?outcome, "database ready"),
        }
        Ok((Self { connection }, outcome))
    }

    /// Closes the connection; later store calls fail. For shutdown and tests that hand the file
    /// to another process.
    pub async fn close(&self) -> DbResult<()> {
        self.connection.clone().close().await?;
        Ok(())
    }

    pub(in crate::db) fn connection(&self) -> &DatabaseConnection {
        &self.connection
    }

    pub fn providers(&self) -> ProviderStore {
        ProviderStore::new(self.clone())
    }

    pub fn agents(&self) -> AgentStore {
        AgentStore::new(self.clone())
    }

    pub fn conversations(&self) -> ConversationStore {
        ConversationStore::new(self.clone())
    }

    pub fn schedules(&self) -> ScheduleStore {
        ScheduleStore::new(self.clone())
    }

    pub fn oauth_tokens(&self) -> OAuthTokenStore {
        OAuthTokenStore::new(self.clone())
    }
}
