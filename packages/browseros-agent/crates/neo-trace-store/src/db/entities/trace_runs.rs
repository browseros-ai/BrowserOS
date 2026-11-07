//! `trace_runs`: one row per run, keyed by its ULID, which sorts by start time, so the run id is
//! also the list order, the cursor and the retention cut. Written by any of the run's events: a
//! placeholder from the ULID's time if its `RunStarted` was shed, the end columns once, the
//! settle columns last.

use sea_orm::entity::prelude::*;

use crate::db::kinds::{Delivery, EndCause, RunStatus, SettledBy};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "trace_runs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub run_id: String,
    /// The host's label, or the run id for an unlabelled run (a one-run session).
    pub session_key: String,
    pub client: Option<String>,
    /// NULL only on a placeholder row, whose `RunStarted` never arrived.
    pub start_page: Option<String>,
    pub timeout_ms: Option<i64>,
    pub code_bytes: Option<i64>,
    pub script: Option<String>,
    pub script_truncated: bool,
    pub started_at_ms: i64,
    /// NULL while the run runs.
    pub status: Option<RunStatus>,
    pub end_cause: Option<EndCause>,
    pub error_name: Option<String>,
    pub error_message: Option<String>,
    pub value_json: Option<String>,
    pub value_truncated: bool,
    pub ended_at_ms: Option<i64>,
    pub duration_us: Option<i64>,
    pub calls_made: i64,
    /// Calls numbered past the row cap: counted, with the least safe delivery among them.
    pub calls_over_cap: i64,
    pub over_cap_delivery: Option<Delivery>,
    pub unreadable_calls: i64,
    pub getters_over_cap: i64,
    pub settled_at_ms: Option<i64>,
    pub settled_by: Option<SettledBy>,
    /// Bytes of call and getter detail kept so far, against the per-run budget.
    pub detail_bytes: i64,
    /// The ingest's loss counts, absolute, from `RunSettled`.
    pub events_stubbed: i64,
    pub events_dropped: i64,
    /// The run settled without its `RunSettled`, so its loss counts are unknown.
    pub incomplete: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::trace_sessions::Entity",
        from = "Column::SessionKey",
        to = "super::trace_sessions::Column::SessionKey"
    )]
    Session,
    #[sea_orm(has_many = "super::trace_calls::Entity")]
    Calls,
    #[sea_orm(has_many = "super::trace_getters::Entity")]
    Getters,
    #[sea_orm(has_many = "super::trace_pages::Entity")]
    Pages,
    #[sea_orm(has_many = "super::trace_logs::Entity")]
    Logs,
    #[sea_orm(has_many = "super::trace_run_ids::Entity")]
    Ids,
    #[sea_orm(has_many = "super::trace_call_artifacts::Entity")]
    Artifacts,
}

impl Related<super::trace_sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl Related<super::trace_calls::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Calls.def()
    }
}

impl Related<super::trace_getters::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Getters.def()
    }
}

impl Related<super::trace_pages::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Pages.def()
    }
}

impl Related<super::trace_logs::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Logs.def()
    }
}

impl Related<super::trace_run_ids::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Ids.def()
    }
}

impl Related<super::trace_call_artifacts::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Artifacts.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
