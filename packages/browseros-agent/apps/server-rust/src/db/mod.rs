//! Persistence over the SQLite file the TS server also uses (`<browserosDir>/db/browseros.sqlite`).
//! `Database::open` runs a drizzle-compatible catch-up over the frozen TS migrations (keeping
//! `__drizzle_migrations`, so a rolled-back TS server still opens the file) and then sea-orm's
//! migrator; the five stores are concrete types over one write connection. Implemented by P2;
//! S0 ships an inert handle whose stores return `DbError::Unavailable`.

mod agents;
mod conversations;
mod oauth_tokens;
mod providers;
mod schedules;

pub use agents::{AcpAgentRow, AgentKind, AgentStore, CreateAgent, UpdateAgent};
pub use conversations::{
    ConversationStore, ConversationSummary, ImportConversation, SaveConversation,
    StoredConversation, TargetType,
};
pub use oauth_tokens::{OAuthTokenStore, OAuthTokens};
pub use providers::{ProviderRow, ProviderStore, UpsertProvider};
pub use schedules::{
    RUNS_KEPT_PER_JOB, ScheduleStore, ScheduledJobRow, ScheduledRunRow, UpsertJob, UpsertRun,
};

use crate::paths::BrowserosPaths;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// The S0 stub's answer to every query, until P2 lands the real layer.
    #[error("database unavailable: {0}")]
    Unavailable(&'static str),
    #[error("database error: {0}")]
    Query(String),
    #[error("database migration failed: {0}")]
    Migration(String),
}

pub type DbResult<T> = Result<T, DbError>;

/// What `open` did to the file; logged at startup and reported by diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOutcome {
    Created,
    UpToDate,
    Migrated {
        drizzle_applied: usize,
        rust_applied: usize,
    },
}

/// The shared connection handle. Cloning shares the one write connection.
#[derive(Debug, Clone, Default)]
pub struct Database {
    _private: (),
}

impl Database {
    /// S0 stub: opens nothing, so the skeleton can boot and serve 501s.
    pub async fn open(paths: &BrowserosPaths) -> Result<(Self, MigrationOutcome), DbError> {
        let _ = paths;
        Ok((Self::default(), MigrationOutcome::UpToDate))
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

const STUB: &str = "the database layer is not implemented yet";
