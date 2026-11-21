//! Coding-agent rows (`kind = 'acp'`) in the `providers` table. Only Claude and Codex rows are
//! listed; legacy `custom` agents stay in the table, unlisted.

use sea_orm::{
    ActiveValue, ColumnTrait, DbErr, DerivePartialModel, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, TransactionTrait,
};

use super::entities::providers::{ActiveModel, Column, Entity};
use super::providers::{ACP, now_ms, nullable_edit};
use super::{Edit, Store, StoreError};
use crate::turn::AgentKind;

pub struct Agents<'a>(pub(super) &'a Store);

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

/// The ACP view of the shared table omits fields that belong to model providers.
#[derive(DerivePartialModel, FromQueryResult)]
#[sea_orm(entity = "Entity")]
struct AgentRecord {
    id: String,
    name: String,
    #[sea_orm(from_col = "type")]
    agent_type: String,
    model_id: Option<String>,
    reasoning_effort: Option<String>,
    working_directory: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl TryFrom<AgentRecord> for AgentRow {
    type Error = StoreError;

    fn try_from(row: AgentRecord) -> Result<Self, StoreError> {
        Ok(Self {
            id: row.id,
            name: row.name,
            kind: parse_agent_kind(&row.agent_type)
                .ok_or_else(|| StoreError::Unreadable("unsupported coding agent type".into()))?,
            model_id: row.model_id,
            reasoning_effort: row.reasoning_effort,
            working_directory: row.working_directory,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

impl Agents<'_> {
    pub async fn list(&self) -> Result<Vec<AgentRow>, StoreError> {
        Entity::find()
            .filter(supported_agents())
            .order_by_desc(Column::UpdatedAt)
            .into_partial_model::<AgentRecord>()
            .all(self.0.db())
            .await?
            .into_iter()
            .map(AgentRow::try_from)
            .collect()
    }

    pub async fn get(&self, id: &str) -> Result<Option<AgentRow>, StoreError> {
        Entity::find_by_id(id)
            .filter(supported_agents())
            .into_partial_model::<AgentRecord>()
            .one(self.0.db())
            .await?
            .map(AgentRow::try_from)
            .transpose()
    }

    pub async fn create(&self, new: NewAgent) -> Result<AgentRow, StoreError> {
        let now = now_ms();
        let row = AgentRow {
            id: uuid::Uuid::new_v4().to_string(),
            name: new.name.trim().to_owned(),
            kind: new.kind,
            model_id: new.model_id.and_then(optional_text),
            reasoning_effort: new.reasoning_effort.and_then(optional_text),
            working_directory: new.working_directory.and_then(optional_text),
            created_at: now,
            updated_at: now,
        };
        let model = ActiveModel {
            id: ActiveValue::Set(row.id.clone()),
            kind: ActiveValue::Set(ACP.into()),
            r#type: ActiveValue::Set(agent_type(row.kind).into()),
            name: ActiveValue::Set(row.name.clone()),
            model_id: ActiveValue::Set(row.model_id.clone()),
            reasoning_effort: ActiveValue::Set(row.reasoning_effort.clone()),
            working_directory: ActiveValue::Set(row.working_directory.clone()),
            created_at: ActiveValue::Set(row.created_at),
            updated_at: ActiveValue::Set(row.updated_at),
            ..Default::default()
        };
        Entity::insert(model)
            .exec_without_returning(self.0.db())
            .await?;
        Ok(row)
    }

    pub async fn update(
        &self,
        id: &str,
        patch: AgentPatch,
    ) -> Result<Option<AgentRow>, StoreError> {
        let model = ActiveModel {
            name: patch.name.map_or(ActiveValue::NotSet, |name| {
                ActiveValue::Set(name.trim().to_owned())
            }),
            model_id: optional_edit(patch.model_id),
            reasoning_effort: optional_edit(patch.reasoning_effort),
            working_directory: optional_edit(patch.working_directory),
            updated_at: ActiveValue::Set(now_ms()),
            ..Default::default()
        };
        // Restrict the UPDATE itself to supported ACP rows. A patch cannot rewrite a provider
        // or disturb the shared default, even if the caller knows another kind's id.
        let tx = self.0.db().begin().await?;
        let result = Entity::update_many()
            .set(model)
            .filter(Column::Id.eq(id))
            .filter(supported_agents())
            .exec(&tx)
            .await?;
        if result.rows_affected == 0 {
            return Ok(None);
        }
        let row = Entity::find_by_id(id)
            .into_partial_model::<AgentRecord>()
            .one(&tx)
            .await?
            .ok_or_else(|| DbErr::RecordNotFound("updated agent".into()))?
            .try_into()?;
        tx.commit().await?;
        Ok(Some(row))
    }

    pub async fn delete(&self, id: &str) -> Result<bool, StoreError> {
        let result = Entity::delete_by_id(id)
            .filter(supported_agents())
            .exec(self.0.db())
            .await?;
        Ok(result.rows_affected > 0)
    }
}

fn supported_agents() -> sea_orm::Condition {
    sea_orm::Condition::all()
        .add(Column::Kind.eq(ACP))
        .add(Column::Type.is_in([agent_type(AgentKind::Claude), agent_type(AgentKind::Codex)]))
}

pub(super) fn parse_agent_kind(value: &str) -> Option<AgentKind> {
    [AgentKind::Claude, AgentKind::Codex]
        .into_iter()
        .find(|&kind| agent_type(kind) == value)
}

fn agent_type(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "claude",
        AgentKind::Codex => "codex",
    }
}

fn optional_text(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn optional_edit(edit: Edit<String>) -> ActiveValue<Option<String>> {
    match edit {
        Edit::Set(value) => ActiveValue::Set(optional_text(value)),
        other => nullable_edit(other),
    }
}
