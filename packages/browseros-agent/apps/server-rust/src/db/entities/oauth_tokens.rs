//! `oauth_tokens`: subscription-provider credentials keyed by (install id, provider).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "oauth_tokens")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub browseros_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub provider: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub email: Option<String>,
    pub account_id: Option<String>,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
