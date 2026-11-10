//! Neo: Private database representations of the runtime's existing trace states.

use sea_orm::sea_query::StringLen;
use sea_orm::{DeriveActiveEnum, EnumIter};

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum RunState {
    #[sea_orm(string_value = "running")]
    Running,
    #[sea_orm(string_value = "ok")]
    Ok,
    #[sea_orm(string_value = "error")]
    Failed,
    #[sea_orm(string_value = "timeout")]
    Timeout,
    #[sea_orm(string_value = "cancelled")]
    Cancelled,
    #[sea_orm(string_value = "interrupted")]
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum CallState {
    #[sea_orm(string_value = "ok")]
    Ok,
    #[sea_orm(string_value = "error")]
    Failed,
    #[sea_orm(string_value = "unfinished")]
    Unfinished,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum UnfinishedReason {
    #[sea_orm(string_value = "settled")]
    Settled,
    #[sea_orm(string_value = "timeout")]
    Timeout,
    #[sea_orm(string_value = "cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum DeliveryState {
    #[sea_orm(string_value = "notSent")]
    NotSent,
    #[sea_orm(string_value = "done")]
    Done,
    #[sea_orm(string_value = "unknown")]
    Unknown,
}
