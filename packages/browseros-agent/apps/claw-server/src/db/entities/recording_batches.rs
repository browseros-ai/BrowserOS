use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "recording_batches")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub document_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub batch_id: String,
    pub accepted_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
