//! `providers` rows of kind `llm` (`GET/PUT/DELETE /providers*`, credential lookup for `/chat`,
//! `/test-provider`, `/refine-prompt`). Ordering: rowid order, no `ORDER BY` (drizzle parity).

use super::{Database, DbError, DbResult, STUB};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderRow {
    pub id: String,
    /// `LLM_PROVIDERS` id.
    pub provider_type: String,
    pub name: String,
    pub model_id: Option<String>,
    pub is_default: bool,
    pub base_url: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
    pub supports_images: Option<bool>,
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
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
}

/// `PUT /providers/{id}` body after validation; `None` leaves a column unchanged on update.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpsertProvider {
    pub provider_type: String,
    pub name: String,
    pub model_id: Option<String>,
    pub base_url: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
    pub supports_images: Option<bool>,
    pub context_window: Option<i64>,
    pub temperature: Option<f64>,
    pub api_key: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    pub resource_name: Option<String>,
    pub region: Option<String>,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderStore {
    db: Database,
}

impl ProviderStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn list(&self) -> DbResult<Vec<ProviderRow>> {
        let _ = &self.db;
        Err(DbError::Unavailable(STUB))
    }

    pub async fn get(&self, id: &str) -> DbResult<Option<ProviderRow>> {
        let _ = id;
        Err(DbError::Unavailable(STUB))
    }

    pub async fn default(&self) -> DbResult<Option<ProviderRow>> {
        Err(DbError::Unavailable(STUB))
    }

    pub async fn set_default(&self, id: &str) -> DbResult<Option<ProviderRow>> {
        let _ = id;
        Err(DbError::Unavailable(STUB))
    }

    pub async fn upsert_llm(&self, id: &str, input: UpsertProvider) -> DbResult<ProviderRow> {
        let _ = (id, input);
        Err(DbError::Unavailable(STUB))
    }

    pub async fn delete(&self, id: &str) -> DbResult<bool> {
        let _ = id;
        Err(DbError::Unavailable(STUB))
    }
}
