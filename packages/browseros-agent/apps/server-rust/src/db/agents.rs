//! ACP agents: `providers` rows of kind `acp` (`/agents*`, `/chat` ACP targets). A port of
//! `apps/server/src/lib/agents/storage/acp-agent-store.ts`: generated ids, field-by-field
//! updates, names trimmed and blank optional text stored as NULL. `custom_config` stays raw text
//! here; `/agents` parses it, `/providers` returns it verbatim.

use super::{
    Database, DbError, DbResult,
    entities::providers::{ActiveModel, Column, Entity},
    js,
    providers::ProviderRow,
};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, TransactionTrait,
    sea_query::Expr,
};
use std::path::{Path, PathBuf};
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

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "custom" => Some(Self::Custom),
            _ => None,
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
    /// Raw `CustomAcpAgentConfig` JSON text.
    pub custom_config: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl AcpAgentRow {
    /// `None` (logged) for a row this build cannot represent: TS would list it with its raw id
    /// and type, but no route here can address it.
    fn from_provider(row: ProviderRow) -> Option<Self> {
        let (Ok(id), Some(kind)) = (
            Uuid::parse_str(&row.id),
            AgentKind::parse(&row.provider_type),
        ) else {
            tracing::warn!(
                agent_id = %row.id,
                agent_type = %row.provider_type,
                "skipping ACP agent row with an unrecognized id or type"
            );
            return None;
        };
        Some(Self {
            id,
            kind,
            name: row.name,
            model_id: row.model_id,
            reasoning_effort: row.reasoning_effort,
            working_directory: row.working_directory.map(PathBuf::from),
            custom_config: row.custom_config,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateAgent {
    pub kind: AgentKind,
    pub name: String,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<String>,
    pub working_directory: Option<PathBuf>,
    /// Serialized `CustomAcpAgentConfig`, stored verbatim.
    pub custom_config: Option<String>,
}

/// Partial update; `None` keeps a column, `Some(None)` clears a nullable one.
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

    /// Newest `updated_at` first.
    pub async fn list(&self) -> DbResult<Vec<AcpAgentRow>> {
        let rows = Entity::find()
            .filter(Column::Kind.eq("acp"))
            .order_by_desc(Column::UpdatedAt)
            .into_model::<ProviderRow>()
            .all(self.db.connection())
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(AcpAgentRow::from_provider)
            .collect())
    }

    pub async fn get(&self, id: Uuid) -> DbResult<Option<AcpAgentRow>> {
        Ok(self.find(id).await?.and_then(AcpAgentRow::from_provider))
    }

    pub async fn create(&self, input: CreateAgent) -> DbResult<AcpAgentRow> {
        let id = Uuid::new_v4();
        let now = js::now_ms();
        let row = ActiveModel {
            id: Set(id.to_string()),
            kind: Set("acp".to_owned()),
            provider_type: Set(input.kind.as_str().to_owned()),
            name: Set(js::trim(&input.name).to_owned()),
            model_id: Set(js::optional_text(input.model_id.as_deref())),
            reasoning_effort: Set(js::optional_text(input.reasoning_effort.as_deref())),
            working_directory: Set(js::optional_text(path_text(
                input.working_directory.as_deref(),
            ))),
            custom_config: Set(input.custom_config),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        Entity::insert(row)
            .exec_without_returning(self.db.connection())
            .await?;
        let saved = self
            .get(id)
            .await?
            .ok_or_else(|| DbError::Query(format!("agent {id} vanished after insert")))?;
        tracing::info!(agent_id = %id, agent_type = input.kind.as_str(), "ACP agent created");
        Ok(saved)
    }

    /// `None` when no ACP agent has this id.
    pub async fn update(&self, id: Uuid, input: UpdateAgent) -> DbResult<Option<AcpAgentRow>> {
        let transaction = self.db.connection().begin().await?;
        let exists = Entity::find_by_id(id.to_string())
            .filter(Column::Kind.eq("acp"))
            .into_model::<ProviderRow>()
            .one(&transaction)
            .await?
            .is_some();
        if !exists {
            return Ok(None);
        }
        let mut update = Entity::update_many()
            .col_expr(Column::UpdatedAt, Expr::value(js::now_ms()))
            .filter(Column::Kind.eq("acp"))
            .filter(Column::Id.eq(id.to_string()));
        if let Some(name) = input.name {
            update = update.col_expr(Column::Name, Expr::value(js::trim(&name).to_owned()));
        }
        for (column, value) in [
            (Column::ModelId, input.model_id),
            (Column::ReasoningEffort, input.reasoning_effort),
        ] {
            if let Some(value) = value {
                update = update.col_expr(column, Expr::value(js::optional_text(value.as_deref())));
            }
        }
        if let Some(value) = input.working_directory {
            update = update.col_expr(
                Column::WorkingDirectory,
                Expr::value(js::optional_text(path_text(value.as_deref()))),
            );
        }
        if let Some(value) = input.custom_config {
            update = update.col_expr(Column::CustomConfig, Expr::value(value));
        }
        update.exec(&transaction).await?;
        let saved = Entity::find_by_id(id.to_string())
            .into_model::<ProviderRow>()
            .one(&transaction)
            .await?;
        transaction.commit().await?;
        tracing::info!(agent_id = %id, "ACP agent updated");
        Ok(saved.and_then(AcpAgentRow::from_provider))
    }

    pub async fn delete(&self, id: Uuid) -> DbResult<bool> {
        let deleted = Entity::delete_many()
            .filter(Column::Kind.eq("acp"))
            .filter(Column::Id.eq(id.to_string()))
            .exec(self.db.connection())
            .await?;
        let deleted = deleted.rows_affected > 0;
        if deleted {
            tracing::info!(agent_id = %id, "ACP agent deleted");
        }
        Ok(deleted)
    }

    async fn find(&self, id: Uuid) -> DbResult<Option<ProviderRow>> {
        Ok(Entity::find_by_id(id.to_string())
            .filter(Column::Kind.eq("acp"))
            .into_model::<ProviderRow>()
            .one(self.db.connection())
            .await?)
    }
}

/// Paths arrive from JSON strings, so they are always valid UTF-8.
fn path_text(path: Option<&Path>) -> Option<&str> {
    path.and_then(Path::to_str)
}
