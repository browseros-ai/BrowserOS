//! The `oauth_tokens` table: the ChatGPT sign-in's tokens, keyed by the install id from
//! `<state dir>/installation.json` and the provider name. Written only by `model::chatgpt` (the
//! layering test enforces it).

use super::{Secret, Store, StoreError};

/// Written only by `model::chatgpt`.
pub struct Tokens<'a>(
    #[expect(dead_code, reason = "read once chunk `settings` writes the queries")]
    pub(super)  &'a Store,
);

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
        _install_id: &str,
        _provider: &str,
    ) -> Result<Option<TokenRow>, StoreError> {
        Err(StoreError::NotImplemented("Tokens::get"))
    }

    pub async fn put(
        &self,
        _install_id: &str,
        _provider: &str,
        _row: TokenRow,
    ) -> Result<(), StoreError> {
        Err(StoreError::NotImplemented("Tokens::put"))
    }

    pub async fn delete(&self, _install_id: &str, _provider: &str) -> Result<(), StoreError> {
        Err(StoreError::NotImplemented("Tokens::delete"))
    }
}
