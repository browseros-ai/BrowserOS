//! Neo: `trace_sessions`: one row per session, kept by one upsert per run start and end (never
//! rebuilt from the runs), and removed with the session's last run.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "trace_sessions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub session_key: String,
    /// The first client a run of the session named.
    pub client: Option<String>,
    /// NULL until a run arrives, on a row `withhold` created first.
    pub first_run_id: Option<String>,
    pub last_run_id: Option<String>,
    pub runs: i64,
    pub last_activity_ms: i64,
    /// neo-feeds' privacy switch: the session's JPEGs are dropped and none is kept again.
    pub withheld: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::trace_runs::Entity")]
    Runs,
}

impl Related<super::trace_runs::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Runs.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
