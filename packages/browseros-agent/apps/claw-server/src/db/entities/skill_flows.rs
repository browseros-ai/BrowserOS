use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "skill_flows")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub skill_name: String,
    pub status: String,
    pub status_reason: Option<String>,
    pub flow_version: i64,
    pub attempts: i64,
    pub site_failures: i64,
    pub effects: Option<String>,
    pub write_approved: bool,
    pub p50_ms: Option<i64>,
    pub source_session_id: Option<String>,
    pub verified_at: Option<i64>,
    pub repaired_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
