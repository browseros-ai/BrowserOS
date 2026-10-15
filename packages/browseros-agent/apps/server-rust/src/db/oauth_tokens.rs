//! `oauth_tokens` keyed by (browseros id, provider): ChatGPT, Copilot and Qwen credentials. A port
//! of `apps/server/src/lib/clients/oauth/token-store.ts`. The key is the installation id, so a
//! regenerated `installation.json` orphans (never deletes) the stored tokens.

use super::{
    Database, DbResult,
    decode::Columns,
    entities::oauth_tokens::{ActiveModel, Column, Entity},
    js,
};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DbErr, EntityTrait, FromQueryResult, QueryFilter, QueryResult,
    sea_query::OnConflict,
};
use std::fmt;

#[derive(Clone, PartialEq, Eq)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Epoch milliseconds.
    pub expires_at: i64,
    pub email: Option<String>,
    pub account_id: Option<String>,
}

impl fmt::Debug for OAuthTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthTokens")
            .field("expires_at", &self.expires_at)
            .field("email", &self.email)
            .finish_non_exhaustive()
    }
}

impl FromQueryResult for OAuthTokens {
    fn from_query_result(row: &QueryResult, prefix: &str) -> Result<Self, DbErr> {
        let columns = Columns::new(row, prefix);
        Ok(Self {
            access_token: columns.text("access_token")?,
            refresh_token: columns.text("refresh_token")?,
            expires_at: columns.int("expires_at")?,
            email: columns.opt_text("email")?,
            account_id: columns.opt_text("account_id")?,
        })
    }
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
        Ok(Entity::find()
            .filter(Column::BrowserosId.eq(browseros_id))
            .filter(Column::Provider.eq(provider))
            .into_model::<OAuthTokens>()
            .one(self.db.connection())
            .await?)
    }

    /// Insert or replace every column, stamping `updated_at`.
    pub async fn upsert(
        &self,
        browseros_id: &str,
        provider: &str,
        tokens: &OAuthTokens,
    ) -> DbResult<()> {
        let row = ActiveModel {
            browseros_id: Set(browseros_id.to_owned()),
            provider: Set(provider.to_owned()),
            access_token: Set(tokens.access_token.clone()),
            refresh_token: Set(tokens.refresh_token.clone()),
            expires_at: Set(tokens.expires_at),
            email: Set(tokens.email.clone()),
            account_id: Set(tokens.account_id.clone()),
            updated_at: Set(js::now_ms()),
        };
        Entity::insert(row)
            .on_conflict(
                OnConflict::columns([Column::BrowserosId, Column::Provider])
                    .update_columns([
                        Column::AccessToken,
                        Column::RefreshToken,
                        Column::ExpiresAt,
                        Column::Email,
                        Column::AccountId,
                        Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec_without_returning(self.db.connection())
            .await?;
        Ok(())
    }

    pub async fn delete(&self, browseros_id: &str, provider: &str) -> DbResult<()> {
        Entity::delete_many()
            .filter(Column::BrowserosId.eq(browseros_id))
            .filter(Column::Provider.eq(provider))
            .exec(self.db.connection())
            .await?;
        Ok(())
    }
}
