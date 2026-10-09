use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "recording_payloads")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub document_id: String,
    pub events_ndjson: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
