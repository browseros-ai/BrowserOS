//! ACP agents: `providers` rows of kind `acp` (`/agents*`, `/chat` ACP targets). `custom_config`
//! stays raw text here; `/agents` parses it, `/providers` returns it verbatim.

use super::{Database, DbError, DbResult, STUB};
use std::path::PathBuf;
use uuid::Uuid;

/// `AcpAgentType` of the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
    Codex,
    Custom,
}

impl AgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AcpAgentRow {
    pub id: Uuid,
    pub kind: AgentKind,
    pub name: String,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<String>,
    pub working_directory: Option<PathBuf>,
    pub custom_config: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateAgent {
    pub kind: AgentKind,
    pub name: String,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<String>,
    pub working_directory: Option<PathBuf>,
    pub custom_config: Option<String>,
}

/// Partial update; `Some(None)` clears a nullable column.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateAgent {
    pub name: Option<String>,
    pub model_id: Option<Option<String>>,
    pub reasoning_effort: Option<Option<String>>,
    pub working_directory: Option<Option<PathBuf>>,
    pub custom_config: Option<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct AgentStore {
    db: Database,
}

impl AgentStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn list(&self) -> DbResult<Vec<AcpAgentRow>> {
        let _ = &self.db;
        Err(DbError::Unavailable(STUB))
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<AcpAgentRow>> {
        let _ = id;
        Err(DbError::Unavailable(STUB))
    }

    pub async fn create(&self, input: CreateAgent) -> DbResult<AcpAgentRow> {
        let _ = input;
        Err(DbError::Unavailable(STUB))
    }

    pub async fn update(&self, id: Uuid, input: UpdateAgent) -> DbResult<Option<AcpAgentRow>> {
        let _ = (id, input);
        Err(DbError::Unavailable(STUB))
    }

    pub async fn delete(&self, id: Uuid) -> DbResult<bool> {
        let _ = id;
        Err(DbError::Unavailable(STUB))
    }
}
