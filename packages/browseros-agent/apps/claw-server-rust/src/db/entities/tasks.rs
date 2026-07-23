use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "tasks")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub session_id: String,
    pub agent_id: String,
    pub slug: String,
    pub agent_label: String,
    pub title: String,
    pub site: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_ms: i64,
    pub dispatch_count: i64,
    pub tool_sequence_json: String,
    pub status: String,
    pub error_count: i64,
    pub last_screenshot_dispatch_id: Option<i64>,
    pub cursor_id: i64,
    pub has_screenshots: bool,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
