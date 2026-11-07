//! Neo: `trace_getters`: one row per synchronous getter (`page.url()`, `page.isClosed()`,
//! `context.pages()`, a page's initializer), keyed `(run_id, getter_no)`. Getters have their own
//! numbers, so call numbers stay dense; a timeline orders the two by `offset_us`.

use sea_orm::entity::prelude::*;

use crate::db::kinds::{Detail, GetterOutcome};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "trace_getters")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub run_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub getter_no: i64,
    pub object: String,
    /// The runtime enum's serde name. A second active enum here would need updating for every
    /// getter and could drift from the bridge; query.rs decodes this back into the one enum.
    pub field: String,
    pub page_id: Option<String>,
    pub tab_id: Option<i64>,
    pub outcome: GetterOutcome,
    pub summary_json: Option<String>,
    pub error_name: Option<String>,
    pub error_message: Option<String>,
    pub offset_us: i64,
    pub duration_us: i64,
    pub detail: Detail,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::trace_runs::Entity",
        from = "Column::RunId",
        to = "super::trace_runs::Column::RunId"
    )]
    Run,
}

impl Related<super::trace_runs::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Run.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
