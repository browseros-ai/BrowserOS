//! `oauth_tokens` keyed by (browseros id, provider): ChatGPT, Copilot and Qwen credentials.

use super::{Database, DbError, DbResult, STUB};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Epoch milliseconds.
    pub expires_at: i64,
    pub email: Option<String>,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OAuthTokenStore {
    db: Database,
}

impl OAuthTokenStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn get(&self, browseros_id: &str, provider: &str) -> DbResult<Option<OAuthTokens>> {
        let _ = (&self.db, browseros_id, provider);
        Err(DbError::Unavailable(STUB))
    }

    pub async fn upsert(
        &self,
        browseros_id: &str,
        provider: &str,
        tokens: &OAuthTokens,
    ) -> DbResult<()> {
        let _ = (browseros_id, provider, tokens);
        Err(DbError::Unavailable(STUB))
    }

    pub async fn delete(&self, browseros_id: &str, provider: &str) -> DbResult<()> {
        let _ = (browseros_id, provider);
        Err(DbError::Unavailable(STUB))
    }
}
