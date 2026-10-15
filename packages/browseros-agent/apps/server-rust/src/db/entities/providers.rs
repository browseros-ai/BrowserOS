//! `providers`: LLM providers (`kind = 'llm'`) and ACP agents (`kind = 'acp'`) in one table.
//! A CHECK requires `model_id` and `context_window` for `llm` rows; a partial unique index
//! (`providers_one_default`) admits one `is_default = 1` row.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "providers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub kind: String,
    #[sea_orm(column_name = "type")]
    pub provider_type: String,
    pub name: String,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<String>,
    pub is_default: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub base_url: Option<String>,
    /// JSON object text (`Record<string, string>`).
    pub headers: Option<String>,
    pub supports_images: bool,
    pub context_window: Option<i64>,
    pub temperature: f64,
    pub api_key: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    pub resource_name: Option<String>,
    pub region: Option<String>,
    pub reasoning_summary: Option<String>,
    pub working_directory: Option<String>,
    /// Raw `CustomAcpAgentConfig` JSON text.
    pub custom_config: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
