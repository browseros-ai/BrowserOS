//! One chat target: an LLM provider (`kind = "llm"`) or a coding agent (`kind = "acp"`). The
//! four credential columns are plaintext today and are never serialised outward: only `store`
//! reads them, to make an outbound call.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "providers")]
pub struct Model {
    /// Client-made for llm rows, a server UUID for acp rows.
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// `llm` or `acp`.
    pub kind: String,
    /// A provider type or agent type; unknown values are kept, not listed.
    pub r#type: String,
    pub name: String,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<String>,
    /// At most one row, by the partial unique index `providers_one_default`.
    pub is_default: bool,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
    pub base_url: Option<String>,
    /// A JSON object of header names to values.
    pub headers: Option<String>,
    pub supports_images: bool,
    pub context_window: Option<i64>,
    /// Stored and returned; no model call uses it (as today).
    pub temperature: f64,
    pub api_key: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    pub resource_name: Option<String>,
    pub region: Option<String>,
    pub reasoning_summary: Option<String>,
    /// acp rows.
    pub working_directory: Option<String>,
    /// Only on legacy `custom` agents, which are not listed.
    pub custom_config: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
