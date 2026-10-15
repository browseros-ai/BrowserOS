//! `scheduled_jobs`: `provider_id` references `providers.id` ON DELETE SET NULL.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "scheduled_jobs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub query: String,
    pub schedule_type: String,
    pub schedule_time: Option<String>,
    pub schedule_interval: Option<i64>,
    pub enabled: bool,
    pub provider_id: Option<String>,
    pub last_run_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
