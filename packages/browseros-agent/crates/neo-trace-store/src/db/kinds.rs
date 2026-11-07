//! The closed sets the store keeps as text: a run's status and end, a call's state and delivery,
//! a getter's field and outcome. Each is a `DeriveActiveEnum` string enum, so no typo can reach a
//! row and every reader matches exhaustively. They stay inside the crate: the public views have
//! their own enums, so no SeaORM trait reaches the API.
//!
//! The stored strings are part of the file format: renaming one needs a migration, and the
//! migration fixtures (`db/migration.rs`) pin them.

use neo_runtime::{EndCause as RtEndCause, RunEnd};
use sea_orm::entity::prelude::*;

use crate::views;

/// `trace_runs.status`: how a run ended. NULL while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub(crate) enum RunStatus {
    #[sea_orm(string_value = "ok")]
    Ok,
    #[sea_orm(string_value = "error")]
    Error,
    #[sea_orm(string_value = "timeout")]
    Timeout,
    #[sea_orm(string_value = "cancelled")]
    Cancelled,
    /// The store's own verdict, never the runtime's: the run's process died (restart) or its end
    /// never reached the store (settle sweep). A real `RunEnded` replaces it.
    #[sea_orm(string_value = "interrupted")]
    Interrupted,
}

impl From<neo_runtime::RunStatus> for RunStatus {
    fn from(status: neo_runtime::RunStatus) -> Self {
        match status {
            neo_runtime::RunStatus::Ok => Self::Ok,
            neo_runtime::RunStatus::Error => Self::Error,
            neo_runtime::RunStatus::Timeout => Self::Timeout,
            neo_runtime::RunStatus::Cancelled => Self::Cancelled,
        }
    }
}

impl RunStatus {
    pub(crate) fn view(self) -> views::RunState {
        match self {
            Self::Ok => views::RunState::Ok,
            Self::Error => views::RunState::Error,
            Self::Timeout => views::RunState::Timeout,
            Self::Cancelled => views::RunState::Cancelled,
            Self::Interrupted => views::RunState::Interrupted,
        }
    }
}

/// `trace_runs.end_cause`: [`neo_runtime::EndCause`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub(crate) enum EndCause {
    #[sea_orm(string_value = "returned")]
    Returned,
    #[sea_orm(string_value = "threw")]
    Threw,
    #[sea_orm(string_value = "deadline")]
    Deadline,
    #[sea_orm(string_value = "cancelled")]
    Cancelled,
    #[sea_orm(string_value = "dropped")]
    Dropped,
    #[sea_orm(string_value = "tabUnavailable")]
    TabUnavailable,
    #[sea_orm(string_value = "unsupported")]
    Unsupported,
}

impl From<RtEndCause> for EndCause {
    fn from(cause: RtEndCause) -> Self {
        match cause {
            RtEndCause::Returned => Self::Returned,
            RtEndCause::Threw => Self::Threw,
            RtEndCause::Deadline => Self::Deadline,
            RtEndCause::Cancelled => Self::Cancelled,
            RtEndCause::Dropped => Self::Dropped,
            RtEndCause::TabUnavailable => Self::TabUnavailable,
            RtEndCause::Unsupported => Self::Unsupported,
        }
    }
}

impl From<EndCause> for RtEndCause {
    fn from(cause: EndCause) -> Self {
        match cause {
            EndCause::Returned => Self::Returned,
            EndCause::Threw => Self::Threw,
            EndCause::Deadline => Self::Deadline,
            EndCause::Cancelled => Self::Cancelled,
            EndCause::Dropped => Self::Dropped,
            EndCause::TabUnavailable => Self::TabUnavailable,
            EndCause::Unsupported => Self::Unsupported,
        }
    }
}

/// `trace_runs.settled_by`: who decided that nothing more of the run will come.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub(crate) enum SettledBy {
    /// The run's own `RunSettled`.
    #[sea_orm(string_value = "runtime")]
    Runtime,
    /// The settle sweep, after waiting for a `RunSettled` that never came. A late `RunSettled`
    /// still replaces it.
    #[sea_orm(string_value = "timeout")]
    Timeout,
    /// `open`, for a run a previous process left open.
    #[sea_orm(string_value = "restart")]
    Restart,
}

impl SettledBy {
    pub(crate) fn view(self) -> views::SettledBy {
        match self {
            Self::Runtime => views::SettledBy::Runtime,
            Self::Timeout => views::SettledBy::Timeout,
            Self::Restart => views::SettledBy::Restart,
        }
    }
}

/// `trace_calls.state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub(crate) enum CallState {
    /// Its `BeforeCall` arrived and its `AfterCall` has not.
    #[sea_orm(string_value = "running")]
    Running,
    #[sea_orm(string_value = "ok")]
    Ok,
    #[sea_orm(string_value = "error")]
    Error,
    /// The run ended under the call (`unfinished_reason` says how), or the process died.
    #[sea_orm(string_value = "unfinished")]
    Unfinished,
    /// The store's verdict: still `running` when its run settled, so its `AfterCall` was lost.
    /// A late `AfterCall` still replaces it.
    #[sea_orm(string_value = "lost")]
    Lost,
}

impl CallState {
    pub(crate) fn view(self) -> views::CallState {
        match self {
            Self::Running => views::CallState::Running,
            Self::Ok => views::CallState::Ok,
            Self::Error => views::CallState::Error,
            Self::Unfinished => views::CallState::Unfinished,
            Self::Lost => views::CallState::Lost,
        }
    }
}

/// `trace_calls.unfinished_reason`: [`RunEnd`], plus a process that died under the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub(crate) enum UnfinishedReason {
    #[sea_orm(string_value = "settled")]
    Settled,
    #[sea_orm(string_value = "timeout")]
    Timeout,
    #[sea_orm(string_value = "cancelled")]
    Cancelled,
    #[sea_orm(string_value = "interrupted")]
    Interrupted,
}

impl From<RunEnd> for UnfinishedReason {
    fn from(end: RunEnd) -> Self {
        match end {
            RunEnd::Settled => Self::Settled,
            RunEnd::Timeout => Self::Timeout,
            RunEnd::Cancelled => Self::Cancelled,
        }
    }
}

impl UnfinishedReason {
    pub(crate) fn view(self) -> views::Unfinished {
        match self {
            Self::Settled => views::Unfinished::Settled,
            Self::Timeout => views::Unfinished::Timeout,
            Self::Cancelled => views::Unfinished::Cancelled,
            Self::Interrupted => views::Unfinished::Interrupted,
        }
    }
}

/// `trace_calls.delivery`, `trace_runs.over_cap_delivery`: [`neo_runtime::Delivery`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub(crate) enum Delivery {
    #[sea_orm(string_value = "notSent")]
    NotSent,
    #[sea_orm(string_value = "done")]
    Done,
    #[sea_orm(string_value = "unknown")]
    Unknown,
}

impl From<neo_runtime::Delivery> for Delivery {
    fn from(delivery: neo_runtime::Delivery) -> Self {
        match delivery {
            neo_runtime::Delivery::NotSent => Self::NotSent,
            neo_runtime::Delivery::Done => Self::Done,
            neo_runtime::Delivery::Unknown => Self::Unknown,
        }
    }
}

impl From<Delivery> for neo_runtime::Delivery {
    fn from(delivery: Delivery) -> Self {
        match delivery {
            Delivery::NotSent => Self::NotSent,
            Delivery::Done => Self::Done,
            Delivery::Unknown => Self::Unknown,
        }
    }
}

/// `detail` of a call or getter row: whether its params, log, message or summary were kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub(crate) enum Detail {
    #[sea_orm(string_value = "full")]
    Full,
    /// Ids, timing, result kind, error name and delivery only: the ingest had no room for more,
    /// or the run used up its detail budget.
    #[sea_orm(string_value = "stub")]
    Stub,
}

impl Detail {
    pub(crate) fn view(self) -> views::Detail {
        match self {
            Self::Full => views::Detail::Full,
            Self::Stub => views::Detail::Stub,
        }
    }
}

/// `trace_getters.outcome`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub(crate) enum GetterOutcome {
    #[sea_orm(string_value = "ok")]
    Ok,
    #[sea_orm(string_value = "error")]
    Error,
}
