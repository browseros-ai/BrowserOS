//! `providers` rows (`GET/PUT/DELETE /providers*`, credential lookup for `/chat`,
//! `/test-provider`, `/refine-prompt`). A port of `apps/server/src/lib/providers/provider-store.ts`.
//!
//! Rows here carry their credentials; the routes strip them (see [`ProviderRow::has_api_key`]
//! and friends for the public `has*` flags). Lists have no `ORDER BY`, so they come back in rowid
//! order exactly as drizzle's do.

use super::{
    Database, DbError, DbResult,
    decode::Columns,
    entities::providers::{ActiveModel, Column, Entity},
    js,
};
use sea_orm::{
    ActiveValue::Set,
    ColumnTrait, DbErr, EntityTrait, FromQueryResult, QueryFilter, QueryResult,
    sea_query::{Expr, OnConflict},
};
use std::{collections::BTreeMap, fmt};

/// One `providers` row, of either kind, credentials included.
#[derive(Clone, PartialEq)]
pub struct ProviderRow {
    pub id: String,
    /// `llm` or `acp`.
    pub kind: String,
    /// `LLM_PROVIDERS` id (llm) or `claude|codex|custom` (acp).
    pub provider_type: String,
    pub name: String,
    pub model_id: Option<String>,
    pub is_default: bool,
    pub base_url: Option<String>,
    /// `None` also when the stored text is not a JSON object of strings (logged).
    pub headers: Option<BTreeMap<String, String>>,
    pub supports_images: bool,
    pub context_window: Option<i64>,
    pub temperature: f64,
    pub api_key: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    pub resource_name: Option<String>,
    pub region: Option<String>,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
    /// ACP rows only.
    pub working_directory: Option<String>,
    /// ACP custom agents only: the raw JSON text, returned verbatim by `/providers`.
    pub custom_config: Option<String>,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
}

impl ProviderRow {
    /// The public row's `hasApiKey`: set and not `''` (blank reads as unset, matching the upsert).
    pub fn has_api_key(&self) -> bool {
        is_set(&self.api_key)
    }

    pub fn has_access_key_id(&self) -> bool {
        is_set(&self.access_key_id)
    }

    pub fn has_secret_access_key(&self) -> bool {
        is_set(&self.secret_access_key)
    }

    pub fn has_session_token(&self) -> bool {
        is_set(&self.session_token)
    }
}

fn is_set(value: &Option<String>) -> bool {
    value.as_deref().is_some_and(|value| !value.is_empty())
}

impl fmt::Debug for ProviderRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderRow")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("provider_type", &self.provider_type)
            .field("name", &self.name)
            .field("model_id", &self.model_id)
            .field("is_default", &self.is_default)
            .field("has_api_key", &self.has_api_key())
            .finish_non_exhaustive()
    }
}

impl FromQueryResult for ProviderRow {
    fn from_query_result(row: &QueryResult, prefix: &str) -> Result<Self, DbErr> {
        let columns = Columns::new(row, prefix);
        let id = columns.text("id")?;
        let headers = columns
            .opt_text("headers")?
            .and_then(|text| match serde_json::from_str(&text) {
                Ok(headers) => headers,
                Err(error) => {
                    tracing::warn!(provider_id = %id, %error, "ignoring unreadable provider headers");
                    None
                }
            });
        Ok(Self {
            kind: columns.text("kind")?,
            provider_type: columns.text("type")?,
            name: columns.text("name")?,
            model_id: columns.opt_text("model_id")?,
            is_default: columns.boolean("is_default")?,
            base_url: columns.opt_text("base_url")?,
            headers,
            supports_images: columns.boolean("supports_images")?,
            context_window: columns.opt_int("context_window")?,
            temperature: columns.real("temperature")?,
            api_key: columns.opt_text("api_key")?,
            access_key_id: columns.opt_text("access_key_id")?,
            secret_access_key: columns.opt_text("secret_access_key")?,
            session_token: columns.opt_text("session_token")?,
            resource_name: columns.opt_text("resource_name")?,
            region: columns.opt_text("region")?,
            reasoning_effort: columns.opt_text("reasoning_effort")?,
            reasoning_summary: columns.opt_text("reasoning_summary")?,
            working_directory: columns.opt_text("working_directory")?,
            custom_config: columns.opt_text("custom_config")?,
            created_at: columns.int("created_at")?,
            updated_at: columns.int("updated_at")?,
            id,
        })
    }
}

/// `PUT /providers/{id}` body after validation.
///
/// Fields mirror zod's three states: `None` = key absent (insert: column default or NULL;
/// update: column unchanged), `Some(None)` = JSON `null` (clears), `Some(Some(v))` = set. A
/// credential that is `Some(Some(""))` is inserted as `''` but leaves a stored credential
/// unchanged on update, because a never-filled form field submits `''` (TS parity).
#[derive(Clone, Default, PartialEq)]
pub struct UpsertProvider {
    pub provider_type: String,
    pub name: String,
    /// Required by the route schema; `None` is treated as absent.
    pub model_id: Option<String>,
    /// Required by the route schema; `None` is treated as absent.
    pub context_window: Option<i64>,
    pub supports_images: Option<bool>,
    pub temperature: Option<f64>,
    /// Only used when the row is inserted; an update never rewrites `created_at`.
    pub created_at: Option<i64>,
    pub base_url: Option<Option<String>>,
    pub headers: Option<Option<BTreeMap<String, String>>>,
    pub api_key: Option<Option<String>>,
    pub access_key_id: Option<Option<String>>,
    pub secret_access_key: Option<Option<String>>,
    pub session_token: Option<Option<String>>,
    pub resource_name: Option<Option<String>>,
    pub region: Option<Option<String>>,
    pub reasoning_effort: Option<Option<String>>,
    pub reasoning_summary: Option<Option<String>>,
}

impl fmt::Debug for UpsertProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpsertProvider")
            .field("provider_type", &self.provider_type)
            .field("name", &self.name)
            .field("model_id", &self.model_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct ProviderStore {
    db: Database,
}

impl ProviderStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Every row of every kind, rowid order.
    pub async fn list(&self) -> DbResult<Vec<ProviderRow>> {
        Ok(Entity::find()
            .into_model::<ProviderRow>()
            .all(self.db.connection())
            .await?)
    }

    /// Only `kind = 'llm'` rows, rowid order (`/refine-prompt`'s "first llm row").
    pub async fn list_llm(&self) -> DbResult<Vec<ProviderRow>> {
        Ok(Entity::find()
            .filter(Column::Kind.eq("llm"))
            .into_model::<ProviderRow>()
            .all(self.db.connection())
            .await?)
    }

    pub async fn get(&self, id: &str) -> DbResult<Option<ProviderRow>> {
        Ok(Entity::find_by_id(id.to_owned())
            .into_model::<ProviderRow>()
            .one(self.db.connection())
            .await?)
    }

    /// The one row with `is_default = 1`, of any kind.
    pub async fn default(&self) -> DbResult<Option<ProviderRow>> {
        Ok(Entity::find()
            .filter(Column::IsDefault.eq(true))
            .into_model::<ProviderRow>()
            .one(self.db.connection())
            .await?)
    }

    /// Points the default at `id` (any kind) and returns the new default row; `None` when `id`
    /// is unknown, so a stale pointer cannot be stored. The old default is cleared first in the
    /// same transaction: the partial unique index admits one `is_default = 1` row at a time.
    pub async fn set_default(&self, id: &str) -> DbResult<Option<ProviderRow>> {
        let transaction = self.db.begin_write().await?;
        if Entity::find_by_id(id.to_owned())
            .into_model::<ProviderRow>()
            .one(&transaction)
            .await?
            .is_none()
        {
            return Ok(None);
        }
        Entity::update_many()
            .col_expr(Column::IsDefault, Expr::value(false))
            .filter(Column::IsDefault.eq(true))
            .filter(Column::Id.ne(id))
            .exec(&transaction)
            .await?;
        Entity::update_many()
            .col_expr(Column::IsDefault, Expr::value(true))
            .filter(Column::Id.eq(id))
            .exec(&transaction)
            .await?;
        let selected = Entity::find()
            .filter(Column::IsDefault.eq(true))
            .into_model::<ProviderRow>()
            .one(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(selected)
    }

    /// Insert-or-update by id, always as `kind = 'llm'`, returning the stored row with its
    /// credentials. `created_at` and `is_default` are never rewritten on conflict: re-importing a
    /// provider keeps its creation time, and a save does not move the selection.
    pub async fn upsert_llm(&self, id: &str, input: UpsertProvider) -> DbResult<ProviderRow> {
        let now = js::now_ms();
        let mut row = ActiveModel {
            id: Set(id.to_owned()),
            kind: Set("llm".to_owned()),
            provider_type: Set(input.provider_type),
            name: Set(input.name),
            created_at: Set(input.created_at.unwrap_or(now)),
            updated_at: Set(now),
            ..Default::default()
        };
        let mut updated = vec![
            Column::Kind,
            Column::ProviderType,
            Column::Name,
            Column::UpdatedAt,
        ];
        let mut field = |column: Column, present: bool| {
            if present {
                updated.push(column);
            }
        };
        if let Some(model_id) = input.model_id {
            row.model_id = Set(Some(model_id));
            field(Column::ModelId, true);
        }
        if let Some(context_window) = input.context_window {
            row.context_window = Set(Some(context_window));
            field(Column::ContextWindow, true);
        }
        if let Some(supports_images) = input.supports_images {
            row.supports_images = Set(supports_images);
            field(Column::SupportsImages, true);
        }
        if let Some(temperature) = input.temperature {
            row.temperature = Set(temperature);
            field(Column::Temperature, true);
        }
        if let Some(base_url) = input.base_url {
            row.base_url = Set(base_url);
            field(Column::BaseUrl, true);
        }
        if let Some(headers) = input.headers {
            let headers = headers
                .map(|headers| serde_json::to_string(&headers))
                .transpose()
                .map_err(|error| DbError::Query(format!("provider headers: {error}")))?;
            row.headers = Set(headers);
            field(Column::Headers, true);
        }
        for (column, value, slot) in [
            (Column::ApiKey, input.api_key, &mut row.api_key),
            (
                Column::AccessKeyId,
                input.access_key_id,
                &mut row.access_key_id,
            ),
            (
                Column::SecretAccessKey,
                input.secret_access_key,
                &mut row.secret_access_key,
            ),
            (
                Column::SessionToken,
                input.session_token,
                &mut row.session_token,
            ),
        ] {
            if let Some(value) = value {
                field(column, value.as_deref() != Some(""));
                *slot = Set(value);
            }
        }
        for (column, value, slot) in [
            (
                Column::ResourceName,
                input.resource_name,
                &mut row.resource_name,
            ),
            (Column::Region, input.region, &mut row.region),
            (
                Column::ReasoningEffort,
                input.reasoning_effort,
                &mut row.reasoning_effort,
            ),
            (
                Column::ReasoningSummary,
                input.reasoning_summary,
                &mut row.reasoning_summary,
            ),
        ] {
            if let Some(value) = value {
                *slot = Set(value);
                field(column, true);
            }
        }

        let transaction = self.db.begin_write().await?;
        Entity::insert(row)
            .on_conflict(
                OnConflict::column(Column::Id)
                    .update_columns(updated)
                    .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        let saved = Entity::find_by_id(id.to_owned())
            .into_model::<ProviderRow>()
            .one(&transaction)
            .await?
            .ok_or_else(|| DbError::Query(format!("provider {id} vanished after upsert")))?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Deletes a row of any kind (ACP agents included); jobs pointing at it get `provider_id`
    /// NULL through the foreign key.
    pub async fn delete(&self, id: &str) -> DbResult<bool> {
        let transaction = self.db.begin_write().await?;
        let deleted = Entity::delete_by_id(id.to_owned())
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(deleted.rows_affected > 0)
    }
}
