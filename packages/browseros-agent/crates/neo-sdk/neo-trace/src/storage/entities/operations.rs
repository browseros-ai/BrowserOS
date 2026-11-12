//! One admitted operation, keyed by run and sequence. Inputs and results reach disk
//! only at the end, already bounded, so running rows expose identity and state only.

use sea_orm::entity::prelude::*;
use sea_orm::sea_query::SimpleExpr;

/// An operation's stored row, keyed by its admission sequence within the run.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "operations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub run_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub seq: i64,
    pub parent_id: Option<String>,
    pub name: String,
    pub site: Option<String>,
    pub kind: String,
    pub attempt: Option<i64>,
    pub effect: String,
    pub target_tab_id: Option<i64>,
    pub page_id: Option<i64>,
    pub tab_id: Option<i64>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub started_at: i64,
    pub sent_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub outcome: Option<String>,
    pub delivery: Option<String>,
    pub args: Option<String>,
    pub args_truncated: bool,
    pub result: Option<String>,
    pub result_truncated: bool,
    pub error: Option<String>,
    pub notes: Option<String>,
    pub screenshot: Option<String>,
    pub screenshot_bytes: Option<i64>,
    pub main_world: Option<bool>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::runs::Entity",
        from = "Column::RunId",
        to = "super::runs::Column::Id"
    )]
    Run,
}

impl ActiveModelBehavior for ActiveModel {}

impl Related<super::runs::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Run.def()
    }
}

/// Inline the stored-state literal so SQLite can prove the partial index's WHERE predicate.
/// A bound parameter would not imply `screenshot = 'stored'` during query planning.
pub(crate) fn stored_shot() -> SimpleExpr {
    Expr::col((Entity, Column::Screenshot)).eq(SimpleExpr::Constant("stored".into()))
}
