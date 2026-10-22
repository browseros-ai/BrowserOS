use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "skills")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub name: String,
    pub description: String,
    pub site: Option<String>,
    pub origin: String,
    pub source_session_id: Option<String>,
    pub version: i64,
    pub body_path: String,
    pub linked_agents_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
