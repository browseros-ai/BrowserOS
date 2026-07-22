use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "tool_dispatches")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub created_at: i64,
    pub agent_id: String,
    pub slug: String,
    pub agent_label: String,
    pub session_id: String,
    pub tool_name: String,
    pub page_id: Option<i64>,
    pub tab_id: Option<i64>,
    pub target_id: Option<String>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub args_json: Option<String>,
    pub result_meta: Option<String>,
    pub duration_ms: Option<i64>,
    pub tool_input_token_estimate: i64,
    pub tool_output_token_estimate: i64,
    pub token_estimator_version: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatch_id: Option<String>,
    pub has_screenshot: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
