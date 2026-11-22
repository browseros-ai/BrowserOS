//! LLM provider rows (`kind = 'llm'`) in the `providers` table, and the one list of provider
//! types.
//!
//! A row is read three ways, as three types, so a handler can never return a key: `ProviderRow`
//! for any reader (flags, no secrets), `ProviderEdit` for a save, and `ProviderConfig` for an
//! outbound model call only. `ProviderDraft` is the settings form's unsaved provider, for the
//! connection test. Coding-agent rows share the table (`kind = 'acp'`) and the single default
//! row; `Agents` owns their writes, and the default pointer may point at a row of either kind.

use std::collections::BTreeMap;

use sea_orm::sea_query::{Expr, OnConflict, SimpleExpr};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DbErr, DerivePartialModel, EntityTrait,
    FromQueryResult, QueryFilter, TransactionTrait,
};

use super::agents::parse_agent_kind;
use super::entities::providers::{ActiveModel, Column, Entity, Model};
use super::{Edit, Secret, SecretEdit, Store, StoreError};
use crate::turn::AgentKind;

pub struct Providers<'a>(pub(super) &'a Store);

pub(super) const LLM: &str = "llm";
pub(super) const ACP: &str = "acp";

/// The one list of provider types. `model` matches on it exhaustively, and the contract's
/// enum is derived from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    Anthropic,
    Openai,
    Google,
    Openrouter,
    Fireworks,
    Azure,
    Bedrock,
    OpenaiCompatible,
    Ollama,
    Lmstudio,
    Moonshot,
    ChatgptPro,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 12] = [
        ProviderKind::Anthropic,
        ProviderKind::Openai,
        ProviderKind::Google,
        ProviderKind::Openrouter,
        ProviderKind::Fireworks,
        ProviderKind::Azure,
        ProviderKind::Bedrock,
        ProviderKind::OpenaiCompatible,
        ProviderKind::Ollama,
        ProviderKind::Lmstudio,
        ProviderKind::Moonshot,
        ProviderKind::ChatgptPro,
    ];

    /// The `type` column's spelling, which is also the contract's.
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Openai => "openai",
            ProviderKind::Google => "google",
            ProviderKind::Openrouter => "openrouter",
            ProviderKind::Fireworks => "fireworks",
            ProviderKind::Azure => "azure",
            ProviderKind::Bedrock => "bedrock",
            ProviderKind::OpenaiCompatible => "openai-compatible",
            ProviderKind::Ollama => "ollama",
            ProviderKind::Lmstudio => "lmstudio",
            ProviderKind::Moonshot => "moonshot",
            ProviderKind::ChatgptPro => "chatgpt-pro",
        }
    }

    /// `None` for a type this server no longer runs (`github-copilot`, `qwen-code`): such rows
    /// stay in the table and are left out of every list.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

/// Which kind of chat target a listed row is, with its type. Rows of a type this server does
/// not run are never listed, so the type is always a known one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Llm(ProviderKind),
    Acp(AgentKind),
}

/// A row as any reader may see it: `has_*` flags, never a secret.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderRow {
    pub id: String,
    pub kind: RowKind,
    pub name: String,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
    pub is_default: bool,
    pub base_url: Option<String>,
    /// Sent with every model call; a value may hold the literal `{{conversationId}}`.
    pub headers: Option<Vec<(String, String)>>,
    pub supports_images: bool,
    pub context_window: Option<i64>,
    /// Stored and returned; no model call uses it (as today).
    pub temperature: f64,
    pub resource_name: Option<String>,
    pub region: Option<String>,
    pub working_directory: Option<String>,
    pub custom_config: Option<String>,
    pub has_api_key: bool,
    pub has_access_key_id: bool,
    pub has_secret_access_key: bool,
    pub has_session_token: bool,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
}

/// A create-or-update of one LLM provider row. Required columns are plain values; a nullable
/// column is an `Edit` (absent keeps, `null` clears); a column with a default is an `Option`
/// (absent keeps on update, takes the default on insert). A save never moves the default row
/// and never rewrites `created_at` of an existing row.
///
/// A secret (`api_key`, `access_key_id`, `secret_access_key`, `session_token`) set to a
/// non-blank value is written. A kept or blank one keeps the stored value only while the
/// provider's address stays the same: a save that changes the type, or the base URL, resource
/// name or region that type sends its key to, clears every secret it does not bring anew.
#[derive(Clone)]
pub struct ProviderEdit {
    pub kind: ProviderKind,
    pub name: String,
    pub model_id: String,
    pub context_window: i64,
    pub base_url: Edit<String>,
    pub headers: Edit<Vec<(String, String)>>,
    pub supports_images: Option<bool>,
    pub temperature: Option<f64>,
    pub api_key: SecretEdit,
    pub access_key_id: SecretEdit,
    pub secret_access_key: SecretEdit,
    pub session_token: SecretEdit,
    pub resource_name: Edit<String>,
    pub region: Edit<String>,
    pub reasoning_effort: Edit<String>,
    pub reasoning_summary: Edit<String>,
    /// Honoured on insert only, so re-importing a provider keeps when it was first made.
    pub created_at: Option<i64>,
}

/// For outbound requests only. A different type from `ProviderRow`, so a handler cannot return a key.
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub model: String,
    pub base_url: Option<String>,
    pub headers: Vec<(String, String)>,
    pub context_window: usize,
    pub supports_images: bool,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
    pub resource_name: Option<String>,
    pub credentials: Credentials,
}

/// What a provider call authenticates with.
pub enum Credentials {
    None,
    ApiKey(Secret),
    Aws {
        access_key_id: Secret,
        secret_access_key: Secret,
        session_token: Option<Secret>,
        region: String,
    },
    /// The ChatGPT sign-in: tokens live in `oauth_tokens`, read by `model::chatgpt`.
    SignIn,
}

/// The settings projection asks SQLite for credential presence flags. Only the outbound
/// config query reads the underlying values from the database.
#[derive(DerivePartialModel, FromQueryResult)]
#[sea_orm(entity = "Entity")]
struct PublicProvider {
    id: String,
    kind: String,
    #[sea_orm(from_col = "type")]
    provider_type: String,
    name: String,
    model_id: Option<String>,
    reasoning_effort: Option<String>,
    reasoning_summary: Option<String>,
    is_default: bool,
    base_url: Option<String>,
    headers: Option<String>,
    supports_images: bool,
    context_window: Option<i64>,
    temperature: f64,
    resource_name: Option<String>,
    region: Option<String>,
    working_directory: Option<String>,
    custom_config: Option<String>,
    #[sea_orm(from_expr = "has_secret(Column::ApiKey)")]
    has_api_key: bool,
    #[sea_orm(from_expr = "has_secret(Column::AccessKeyId)")]
    has_access_key_id: bool,
    #[sea_orm(from_expr = "has_secret(Column::SecretAccessKey)")]
    has_secret_access_key: bool,
    #[sea_orm(from_expr = "has_secret(Column::SessionToken)")]
    has_session_token: bool,
    created_at: i64,
    updated_at: i64,
}

impl PublicProvider {
    fn into_row(self) -> Result<Option<ProviderRow>, StoreError> {
        let kind = match self.kind.as_str() {
            LLM => ProviderKind::parse(&self.provider_type).map(RowKind::Llm),
            ACP => parse_agent_kind(&self.provider_type).map(RowKind::Acp),
            _ => None,
        };
        let Some(kind) = kind else { return Ok(None) };
        let headers = decode_headers(self.headers.as_deref())?;
        Ok(Some(ProviderRow {
            id: self.id,
            kind,
            name: self.name,
            model_id: self.model_id,
            reasoning_effort: self.reasoning_effort,
            reasoning_summary: self.reasoning_summary,
            is_default: self.is_default,
            base_url: self.base_url,
            headers,
            supports_images: self.supports_images,
            context_window: self.context_window,
            temperature: self.temperature,
            resource_name: self.resource_name,
            region: self.region,
            working_directory: self.working_directory,
            custom_config: self.custom_config,
            has_api_key: self.has_api_key,
            has_access_key_id: self.has_access_key_id,
            has_secret_access_key: self.has_secret_access_key,
            has_session_token: self.has_session_token,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }))
    }
}

impl Providers<'_> {
    /// Rows of both kinds. Legacy types stay in SQLite for rollback.
    pub async fn list(&self) -> Result<Vec<ProviderRow>, StoreError> {
        Entity::find()
            .into_partial_model::<PublicProvider>()
            .all(self.0.db())
            .await?
            .into_iter()
            .filter_map(|row| row.into_row().transpose())
            .collect()
    }

    /// A legacy default is treated as unselected by the extension.
    pub async fn default(&self) -> Result<Option<ProviderRow>, StoreError> {
        match Entity::find()
            .filter(Column::IsDefault.eq(true))
            .into_partial_model::<PublicProvider>()
            .one(self.0.db())
            .await?
        {
            Some(row) => row.into_row(),
            None => Ok(None),
        }
    }

    /// The lookup, clear, and set are one transaction across providers and coding agents.
    pub async fn set_default(&self, id: &str) -> Result<Option<ProviderRow>, StoreError> {
        let tx = self.0.db().begin().await?;
        let Some(target) = Entity::find_by_id(id)
            .into_partial_model::<PublicProvider>()
            .one(&tx)
            .await?
        else {
            return Ok(None);
        };
        let Some(mut target) = target.into_row()? else {
            return Ok(None);
        };
        if !target.is_default {
            // The partial unique index requires clearing the old selection before setting
            // the new one. Readers see the change only once both updates commit.
            Entity::update_many()
                .col_expr(Column::IsDefault, Expr::value(false))
                .filter(Column::IsDefault.eq(true))
                .exec(&tx)
                .await?;
            Entity::update_many()
                .col_expr(Column::IsDefault, Expr::value(true))
                .filter(Column::Id.eq(id))
                .exec(&tx)
                .await?;
            target.is_default = true;
        }
        tx.commit().await?;
        Ok(Some(target))
    }

    /// Upsert by the extension's id while preserving the row and its foreign key references.
    /// Repointing a saved provider clears the secrets the save does not bring (see
    /// `ProviderEdit`), so a stored key is never sent to an address the caller chose; the
    /// connection test (`ProviderDraft::resolve`) refuses the same thing.
    pub async fn save(&self, id: &str, edit: ProviderEdit) -> Result<ProviderRow, StoreError> {
        let now = now_ms();
        // The address is read in the upsert's transaction, so the row compared is the row
        // overwritten. An insert, or an id held by a coding agent (refused below), has no
        // address to compare.
        let tx = self.0.db().begin().await?;
        let stored = Entity::find_by_id(id)
            .filter(Column::Kind.eq(LLM))
            .into_partial_model::<StoredAddress>()
            .one(&tx)
            .await?;
        let moved = stored.is_some_and(|row| {
            let before = ProviderKind::parse(&row.provider_type).map(|kind| {
                address(
                    kind,
                    row.base_url.as_deref(),
                    row.resource_name.as_deref(),
                    row.region.as_deref(),
                )
            });
            let after = address(
                edit.kind,
                applied(&edit.base_url, row.base_url.as_deref()),
                applied(&edit.resource_name, row.resource_name.as_deref()),
                applied(&edit.region, row.region.as_deref()),
            );
            before != Some(after)
        });
        let headers = match edit.headers {
            Edit::Keep => Edit::Keep,
            Edit::Clear => Edit::Clear,
            Edit::Set(pairs) => {
                let object: BTreeMap<_, _> = pairs.into_iter().collect();
                let json = serde_json::to_string(&object)
                    .map_err(|_| StoreError::Unreadable("invalid provider headers".into()))?;
                Edit::Set(json)
            }
        };
        let model = ActiveModel {
            id: ActiveValue::Set(id.to_owned()),
            kind: ActiveValue::Set(LLM.into()),
            r#type: ActiveValue::Set(edit.kind.as_str().into()),
            name: ActiveValue::Set(edit.name),
            model_id: ActiveValue::Set(Some(edit.model_id)),
            context_window: ActiveValue::Set(Some(edit.context_window)),
            base_url: nullable_edit(edit.base_url),
            headers: nullable_edit(headers),
            supports_images: edit
                .supports_images
                .map_or(ActiveValue::NotSet, ActiveValue::Set),
            temperature: edit
                .temperature
                .map_or(ActiveValue::NotSet, ActiveValue::Set),
            api_key: credential_edit(edit.api_key, moved),
            access_key_id: credential_edit(edit.access_key_id, moved),
            secret_access_key: credential_edit(edit.secret_access_key, moved),
            session_token: credential_edit(edit.session_token, moved),
            resource_name: nullable_edit(edit.resource_name),
            region: nullable_edit(edit.region),
            reasoning_effort: nullable_edit(edit.reasoning_effort),
            reasoning_summary: nullable_edit(edit.reasoning_summary),
            created_at: ActiveValue::Set(edit.created_at.unwrap_or(now)),
            updated_at: ActiveValue::Set(now),
            ..Default::default()
        };
        // NotSet omits a field on insert and leaves it out of the conflict update.
        // created_at and is_default are intentionally excluded from that update.
        let updates = [
            Column::Type,
            Column::Name,
            Column::ModelId,
            Column::ContextWindow,
            Column::BaseUrl,
            Column::Headers,
            Column::SupportsImages,
            Column::Temperature,
            Column::ApiKey,
            Column::AccessKeyId,
            Column::SecretAccessKey,
            Column::SessionToken,
            Column::ResourceName,
            Column::Region,
            Column::ReasoningEffort,
            Column::ReasoningSummary,
            Column::UpdatedAt,
        ]
        .into_iter()
        .filter(|&column| !model.is_not_set(column));
        let conflict = OnConflict::column(Column::Id)
            .update_columns(updates)
            .action_cond_where(supported_llm())
            .to_owned();

        // Keep the upsert and the public readback on the same connection. RETURNING the
        // entire entity would select credentials and a second transaction could race a delete.
        let written = Entity::insert(model)
            .on_conflict(conflict)
            .exec_without_returning(&tx)
            .await?;
        if written == 0 {
            return Err(DbErr::RecordNotUpdated.into());
        }
        let row = Entity::find_by_id(id)
            .into_partial_model::<PublicProvider>()
            .one(&tx)
            .await?
            .ok_or_else(|| DbErr::RecordNotFound("saved provider".into()))?
            .into_row()?
            .ok_or_else(|| {
                StoreError::Unreadable("saved provider has an unsupported type".into())
            })?;
        tx.commit().await?;
        Ok(row)
    }

    /// Whether a save was refused because the id is owned by a different target.
    pub(crate) fn is_id_conflict(error: &StoreError) -> bool {
        matches!(error, StoreError::Db(DbErr::RecordNotUpdated))
    }

    /// Deleting an LLM provider lets the legacy foreign key clear its scheduled job references.
    pub async fn delete(&self, id: &str) -> Result<bool, StoreError> {
        let deleted = Entity::delete_by_id(id)
            .filter(supported_llm())
            .exec(self.0.db())
            .await?;
        Ok(deleted.rows_affected > 0)
    }

    /// Only outbound calls read the credential columns. An ACP or legacy target is not an LLM.
    pub async fn config(&self, id: Option<&str>) -> Result<Option<ProviderConfig>, StoreError> {
        let target = id.map_or_else(|| Column::IsDefault.eq(true), |id| Column::Id.eq(id));
        let row = Entity::find()
            .filter(supported_llm())
            .filter(target)
            .one(self.0.db())
            .await?;
        row.map(config_from_row).transpose()
    }
}

fn supported_llm() -> sea_orm::Condition {
    sea_orm::Condition::all()
        .add(Column::Kind.eq(LLM))
        .add(Column::Type.is_in(ProviderKind::ALL.map(ProviderKind::as_str)))
}

fn has_secret(column: Column) -> SimpleExpr {
    Expr::col(column)
        .is_not_null()
        .and(Expr::col(column).ne(""))
}

fn decode_headers(stored: Option<&str>) -> Result<Option<Vec<(String, String)>>, StoreError> {
    stored
        .map(|json| {
            serde_json::from_str::<BTreeMap<String, String>>(json)
                .map(|headers| headers.into_iter().collect())
                .map_err(|_| StoreError::Unreadable("invalid provider headers".into()))
        })
        .transpose()
}

fn config_from_row(row: Model) -> Result<ProviderConfig, StoreError> {
    let kind = ProviderKind::parse(&row.r#type)
        .ok_or_else(|| StoreError::Unreadable("unsupported provider type".into()))?;
    let credentials = match kind {
        ProviderKind::ChatgptPro => Credentials::SignIn,
        ProviderKind::Bedrock => match (
            read_secret(row.access_key_id),
            read_secret(row.secret_access_key),
            row.region.filter(|value| !value.is_empty()),
        ) {
            (Some(access_key_id), Some(secret_access_key), Some(region)) => Credentials::Aws {
                access_key_id,
                secret_access_key,
                session_token: read_secret(row.session_token),
                region,
            },
            _ => Credentials::None,
        },
        _ => read_secret(row.api_key).map_or(Credentials::None, Credentials::ApiKey),
    };
    Ok(ProviderConfig {
        id: row.id,
        kind,
        model: row
            .model_id
            .ok_or_else(|| StoreError::Unreadable("missing provider model".into()))?,
        base_url: row.base_url,
        headers: decode_headers(row.headers.as_deref())?.unwrap_or_default(),
        context_window: row
            .context_window
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| StoreError::Unreadable("invalid provider context window".into()))?,
        supports_images: row.supports_images,
        reasoning_effort: row.reasoning_effort,
        reasoning_summary: row.reasoning_summary,
        resource_name: row.resource_name,
        credentials,
    })
}

fn read_secret(value: Option<String>) -> Option<Secret> {
    value.filter(|value| !value.is_empty()).map(Secret::new)
}

/// The columns of a saved row that decide where its secrets go, read without the secrets.
#[derive(DerivePartialModel, FromQueryResult)]
#[sea_orm(entity = "Entity")]
struct StoredAddress {
    #[sea_orm(from_col = "type")]
    provider_type: String,
    base_url: Option<String>,
    resource_name: Option<String>,
    region: Option<String>,
}

/// Where a provider sends its secrets: its type and the fields that type builds its endpoint
/// from, a blank field counting as none (as `model` reads it). Every type is listed so a new
/// one must say where its key goes. A type without a base URL calls its own default endpoint,
/// so a change of type is a change of address.
#[derive(PartialEq, Eq)]
struct Address<'a>(ProviderKind, [Option<&'a str>; 2]);

fn address<'a>(
    kind: ProviderKind,
    base_url: Option<&'a str>,
    resource_name: Option<&'a str>,
    region: Option<&'a str>,
) -> Address<'a> {
    let given = |value: Option<&'a str>| value.filter(|value| !value.trim().is_empty());
    let fields = match kind {
        ProviderKind::Bedrock => [given(region), None],
        ProviderKind::Azure => [given(base_url), given(resource_name)],
        ProviderKind::ChatgptPro => [None, None],
        ProviderKind::Anthropic
        | ProviderKind::Openai
        | ProviderKind::Google
        | ProviderKind::Openrouter
        | ProviderKind::Fireworks
        | ProviderKind::OpenaiCompatible
        | ProviderKind::Ollama
        | ProviderKind::Lmstudio
        | ProviderKind::Moonshot => [given(base_url), None],
    };
    Address(kind, fields)
}

/// A column's value once `edit` is applied to the stored one.
fn applied<'a>(edit: &'a Edit<String>, stored: Option<&'a str>) -> Option<&'a str> {
    match edit {
        Edit::Keep => stored,
        Edit::Clear => None,
        Edit::Set(value) => Some(value),
    }
}

/// A non-blank secret is written and `null` clears. Absent or blank keeps the stored secret,
/// unless the provider's address `moved`: then it is cleared, because keeping it would send a
/// saved key to an address the caller chose.
fn credential_edit(edit: SecretEdit, moved: bool) -> ActiveValue<Option<String>> {
    match edit {
        Edit::Set(value) if !value.expose().is_empty() => {
            ActiveValue::Set(Some(value.expose().to_owned()))
        }
        Edit::Clear => ActiveValue::Set(None),
        Edit::Keep | Edit::Set(_) if moved => ActiveValue::Set(None),
        Edit::Keep | Edit::Set(_) => ActiveValue::NotSet,
    }
}

pub(super) fn nullable_edit<T: Into<sea_orm::Value>>(edit: Edit<T>) -> ActiveValue<Option<T>>
where
    Option<T>: Into<sea_orm::Value>,
{
    match edit {
        Edit::Keep => ActiveValue::NotSet,
        Edit::Clear => ActiveValue::Set(None),
        Edit::Set(value) => ActiveValue::Set(Some(value)),
    }
}

pub(super) fn now_ms() -> i64 {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
}

/// A provider as typed into the settings form, for `POST /test-provider`.
#[derive(Clone)]
pub struct ProviderDraft {
    pub kind: ProviderKind,
    pub model: String,
    pub base_url: Option<String>,
    pub headers: Vec<(String, String)>,
    pub resource_name: Option<String>,
    pub region: Option<String>,
    pub api_key: Option<Secret>,
    pub access_key_id: Option<Secret>,
    pub secret_access_key: Option<Secret>,
    pub session_token: Option<Secret>,
}

/// Why a draft cannot be tested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftError {
    /// A credential is missing and the saved row named for borrowing is of another type.
    TypeMismatch,
    /// A credential this provider type needs is missing; the name is the form field's.
    MissingCredential(&'static str),
}

impl std::fmt::Display for DraftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DraftError::TypeMismatch => {
                f.write_str("The saved provider is of another type, so its key cannot be used.")
            }
            DraftError::MissingCredential(field) => write!(f, "{field} is required."),
        }
    }
}

/// The context window a test call is resolved with when no saved row gives one; a one-line
/// prompt never comes near it.
const DRAFT_CONTEXT_WINDOW: usize = 128_000;

impl ProviderDraft {
    /// Fills a blank credential from `saved`. A saved row of another provider type lends
    /// nothing (an openrouter key is never sent as an azure one). Whenever any credential is
    /// borrowed, the saved row's base URL, resource name and region replace the typed ones: a
    /// saved secret is never sent to an address the caller chose. Pure.
    pub fn resolve(self, saved: Option<ProviderConfig>) -> Result<ProviderConfig, DraftError> {
        let mismatched = saved.as_ref().is_some_and(|saved| saved.kind != self.kind);
        let lender = saved.as_ref().filter(|saved| saved.kind == self.kind);
        let (saved_key, saved_aws) = match lender.map(|saved| &saved.credentials) {
            Some(Credentials::ApiKey(key)) => (Some(key.clone()), None),
            Some(Credentials::Aws {
                access_key_id,
                secret_access_key,
                session_token,
                region,
            }) => (
                None,
                Some((
                    access_key_id.clone(),
                    secret_access_key.clone(),
                    session_token.clone(),
                    region.clone(),
                )),
            ),
            _ => (None, None),
        };
        let mut borrowed = false;
        let mut take = |typed: Option<Secret>, stored: Option<Secret>| match typed {
            Some(secret) => Some(secret),
            None => {
                borrowed |= stored.is_some();
                stored
            }
        };
        let api_key = take(self.api_key, saved_key);
        let access_key_id = take(
            self.access_key_id,
            saved_aws.as_ref().map(|aws| aws.0.clone()),
        );
        let secret_access_key = take(
            self.secret_access_key,
            saved_aws.as_ref().map(|aws| aws.1.clone()),
        );
        let session_token = take(
            self.session_token,
            saved_aws.as_ref().and_then(|aws| aws.2.clone()),
        );

        let (base_url, resource_name, region) = match lender {
            Some(saved) if borrowed => (
                saved.base_url.clone(),
                saved.resource_name.clone(),
                saved_aws.as_ref().map(|aws| aws.3.clone()),
            ),
            _ => (self.base_url, self.resource_name, self.region),
        };
        let missing = |field: &'static str| {
            if mismatched {
                DraftError::TypeMismatch
            } else {
                DraftError::MissingCredential(field)
            }
        };
        let credentials = match self.kind {
            ProviderKind::ChatgptPro => Credentials::SignIn,
            ProviderKind::Bedrock => Credentials::Aws {
                access_key_id: access_key_id.ok_or_else(|| missing("accessKeyId"))?,
                secret_access_key: secret_access_key.ok_or_else(|| missing("secretAccessKey"))?,
                session_token,
                region: region.ok_or(DraftError::MissingCredential("region"))?,
            },
            ProviderKind::OpenaiCompatible | ProviderKind::Ollama | ProviderKind::Lmstudio => {
                api_key.map_or(Credentials::None, Credentials::ApiKey)
            }
            ProviderKind::Anthropic
            | ProviderKind::Openai
            | ProviderKind::Google
            | ProviderKind::Openrouter
            | ProviderKind::Fireworks
            | ProviderKind::Azure
            | ProviderKind::Moonshot => {
                Credentials::ApiKey(api_key.ok_or_else(|| missing("apiKey"))?)
            }
        };
        Ok(ProviderConfig {
            id: lender.map_or_else(|| "draft".to_owned(), |saved| saved.id.clone()),
            kind: self.kind,
            model: self.model,
            base_url,
            headers: self.headers,
            context_window: lender.map_or(DRAFT_CONTEXT_WINDOW, |saved| saved.context_window),
            supports_images: lender.is_some_and(|saved| saved.supports_images),
            reasoning_effort: lender.and_then(|saved| saved.reasoning_effort.clone()),
            reasoning_summary: lender.and_then(|saved| saved.reasoning_summary.clone()),
            resource_name,
            credentials,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(kind: ProviderKind) -> ProviderDraft {
        ProviderDraft {
            kind,
            model: "m".into(),
            base_url: Some("https://typed.example".into()),
            headers: Vec::new(),
            resource_name: None,
            region: None,
            api_key: None,
            access_key_id: None,
            secret_access_key: None,
            session_token: None,
        }
    }

    fn saved(kind: ProviderKind, credentials: Credentials) -> ProviderConfig {
        ProviderConfig {
            id: "saved".into(),
            kind,
            model: "m".into(),
            base_url: Some("https://saved.example".into()),
            headers: Vec::new(),
            context_window: 8_000,
            supports_images: true,
            reasoning_effort: None,
            reasoning_summary: None,
            resource_name: None,
            credentials,
        }
    }

    #[test]
    fn a_borrowed_key_goes_only_to_the_saved_address() -> Result<(), DraftError> {
        let config = draft(ProviderKind::Openrouter).resolve(Some(saved(
            ProviderKind::Openrouter,
            Credentials::ApiKey(Secret::new("sk-saved")),
        )))?;
        assert_eq!(config.base_url.as_deref(), Some("https://saved.example"));
        assert!(
            matches!(config.credentials, Credentials::ApiKey(key) if key.expose() == "sk-saved")
        );
        Ok(())
    }

    #[test]
    fn a_typed_key_keeps_the_typed_address() -> Result<(), DraftError> {
        let mut typed = draft(ProviderKind::Openrouter);
        typed.api_key = Some(Secret::new("sk-typed"));
        let config = typed.resolve(Some(saved(
            ProviderKind::Openrouter,
            Credentials::ApiKey(Secret::new("sk-saved")),
        )))?;
        assert_eq!(config.base_url.as_deref(), Some("https://typed.example"));
        Ok(())
    }

    #[test]
    fn a_row_of_another_type_lends_nothing() {
        let result = draft(ProviderKind::Azure).resolve(Some(saved(
            ProviderKind::Openrouter,
            Credentials::ApiKey(Secret::new("sk-saved")),
        )));
        assert_eq!(result.err(), Some(DraftError::TypeMismatch));
        assert_eq!(
            draft(ProviderKind::Azure).resolve(None).err(),
            Some(DraftError::MissingCredential("apiKey"))
        );
    }
}
