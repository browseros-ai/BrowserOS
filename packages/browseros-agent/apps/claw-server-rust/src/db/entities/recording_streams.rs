use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "recording_streams")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub document_id: String,
    pub tab_id: i64,
    pub target_id: Option<String>,
    pub first_event_at: i64,
    pub last_event_at: i64,
    pub size_bytes: i64,
    pub event_count: i64,
    pub has_gap: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
