//! The `oauth_tokens` table: the ChatGPT sign-in's tokens, keyed by the install id from
//! `<state dir>/installation.json` and the provider name. Written only by `model::chatgpt` (the
//! layering test enforces it).

use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveValue, EntityTrait};

use super::entities::oauth_tokens::{ActiveModel, Column, Entity};
use super::providers::now_ms;
use super::{Secret, Store, StoreError};

/// Written only by `model::chatgpt`.
pub struct Tokens<'a>(pub(super) &'a Store);

/// The tokens of one sign-in.
pub struct TokenRow {
    pub access: Secret,
    pub refresh: Secret,
    /// Epoch milliseconds; 0 means never.
    pub expires_at: i64,
    pub email: Option<String>,
    pub account_id: Option<String>,
}

impl Tokens<'_> {
    pub async fn get(
        &self,
        install_id: &str,
        provider: &str,
    ) -> Result<Option<TokenRow>, StoreError> {
        let row = Entity::find_by_id((install_id.to_owned(), provider.to_owned()))
            .one(self.0.db())
            .await?;
        Ok(row.map(|row| TokenRow {
            access: Secret::new(row.access_token),
            refresh: Secret::new(row.refresh_token),
            expires_at: row.expires_at,
            email: row.email,
            account_id: row.account_id,
        }))
    }

    /// Replace the token pair together, even on refresh. The install id and provider
    /// are the old server's composite key.
    pub async fn put(
        &self,
        install_id: &str,
        provider: &str,
        row: TokenRow,
    ) -> Result<(), StoreError> {
        let model = ActiveModel {
            browseros_id: ActiveValue::Set(install_id.to_owned()),
            provider: ActiveValue::Set(provider.to_owned()),
            access_token: ActiveValue::Set(row.access.expose().to_owned()),
            refresh_token: ActiveValue::Set(row.refresh.expose().to_owned()),
            expires_at: ActiveValue::Set(row.expires_at),
            email: ActiveValue::Set(row.email),
            account_id: ActiveValue::Set(row.account_id),
            updated_at: ActiveValue::Set(now_ms()),
        };
        Entity::insert(model)
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
            .exec_without_returning(self.0.db())
            .await?;
        Ok(())
    }

    pub async fn delete(&self, install_id: &str, provider: &str) -> Result<(), StoreError> {
        Entity::delete_by_id((install_id.to_owned(), provider.to_owned()))
            .exec(self.0.db())
            .await?;
        Ok(())
    }
}
