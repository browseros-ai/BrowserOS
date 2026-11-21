//! Coding-agent rows (`kind = 'acp'`) in the `providers` table. Only Claude and Codex rows are
//! listed; legacy `custom` agents stay in the table, unlisted.

use super::{Edit, Store, StoreError};
use crate::turn::AgentKind;

pub struct Agents<'a>(
    #[expect(dead_code, reason = "read once chunk `settings` writes the queries")]
    pub(super)  &'a Store,
);

/// A coding agent as every reader sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRow {
    /// A server-made UUID.
    pub id: String,
    pub name: String,
    pub kind: AgentKind,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<String>,
    pub working_directory: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A new agent; the server makes the id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAgent {
    pub name: String,
    pub kind: AgentKind,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<String>,
    pub working_directory: Option<String>,
}

/// An edit of an agent: an absent field keeps its value; a nullable one is cleared by `null`.
#[derive(Clone, Default)]
pub struct AgentPatch {
    pub name: Option<String>,
    pub model_id: Edit<String>,
    pub reasoning_effort: Edit<String>,
    pub working_directory: Edit<String>,
}

impl Agents<'_> {
    pub async fn list(&self) -> Result<Vec<AgentRow>, StoreError> {
        Err(StoreError::NotImplemented("Agents::list"))
    }

    pub async fn get(&self, _id: &str) -> Result<Option<AgentRow>, StoreError> {
        Err(StoreError::NotImplemented("Agents::get"))
    }

    pub async fn create(&self, _new: NewAgent) -> Result<AgentRow, StoreError> {
        Err(StoreError::NotImplemented("Agents::create"))
    }

    pub async fn update(
        &self,
        _id: &str,
        _patch: AgentPatch,
    ) -> Result<Option<AgentRow>, StoreError> {
        Err(StoreError::NotImplemented("Agents::update"))
    }

    pub async fn delete(&self, _id: &str) -> Result<bool, StoreError> {
        Err(StoreError::NotImplemented("Agents::delete"))
    }
}
