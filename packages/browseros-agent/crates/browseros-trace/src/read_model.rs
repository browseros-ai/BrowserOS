//! What [`TraceStore`](crate::TraceStore) reads back: records as stored (masked and bounded),
//! typed, ready for the service routes to map onto `claw-sdk-api` models.
//!
//! A host finds its records by its own ids (`requestId`, `taskId`), by browser tab, and by
//! time.

use crate::{
    Delivery, Effect, EpochMillis, ErrorRecord, OperationId, OperationKind, Outcome, PageState,
    RequestId, RunEffects, RunId, RunStatus, TaskId,
};
use browseros_core::TabId;
use serde_json::Value;

/// One stored operation.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationRow {
    /// Its id.
    pub id: OperationId,
    /// The operation it ran inside.
    pub parent: Option<OperationId>,
    /// The tab it targeted, when it had one.
    pub tab: Option<TabId>,
    /// The method name: masked, clipped.
    pub name: String,
    /// Its place in the tree.
    pub kind: OperationKind,
    /// Read or write.
    pub effect: Effect,
    /// How its tab looked after it.
    pub page: PageState,
    /// Safe input: redacted, masked, bounded.
    pub args: Option<Value>,
    /// Whether `args` was cut.
    pub args_truncated: bool,
    /// Safe result: masked, bounded.
    pub result: Option<Value>,
    /// Whether `result` was cut.
    pub result_truncated: bool,
    /// How it ended; `None` while it runs.
    pub outcome: Option<Outcome>,
    /// What happened to its side effect; `None` while it runs.
    pub delivery: Option<Delivery>,
    /// Its error, when it failed.
    pub error: Option<ErrorRecord>,
    /// Its call-log lines, oldest first: masked, bounded. Empty while it runs, and when it noted
    /// nothing.
    pub notes: Vec<String>,
    /// When it was admitted.
    pub started_at: EpochMillis,
    /// When it marked itself sent.
    pub sent_at: Option<EpochMillis>,
    /// When it ended.
    pub ended_at: Option<EpochMillis>,
    /// What became of the screenshot taken after it; `None` when it has none.
    pub screenshot: Option<ShotState>,
}

/// What became of an operation's screenshot, as its row records it. An operation with none (no
/// job, or the job was dropped) has no state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotState {
    /// A JPEG is on disk.
    Stored,
    /// Its tab was private, so no pixels were kept.
    Withheld,
    /// The disk cap deleted it.
    Expired,
}

impl ShotState {
    /// The stored name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stored => "stored",
            Self::Withheld => "withheld",
            Self::Expired => "expired",
        }
    }

    /// Reads a stored name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Stored, Self::Withheld, Self::Expired]
            .into_iter()
            .find(|state| state.as_str() == name)
    }
}

/// One operation's screenshot, read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShotRead {
    /// The JPEG.
    Jpeg(Vec<u8>),
    /// Withheld because its tab was private.
    Withheld,
    /// Deleted by the disk cap, or missing from the disk.
    Expired,
    /// No screenshot: none was taken, it was dropped, or no operation has this id.
    None,
}

/// A run without its operations.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSummary {
    /// Its id.
    pub id: RunId,
    /// The host's task it belongs to.
    pub task_id: Option<TaskId>,
    /// The tabs its operations targeted, ascending.
    pub tabs: Vec<TabId>,
    /// How it ended, or `Running`.
    pub status: RunStatus,
    /// What it changed; `None` while it runs.
    pub effects: Option<RunEffects>,
    /// Its uncaught error.
    pub error: Option<ErrorRecord>,
    /// The host's own id for the request that started it.
    pub request_id: Option<RequestId>,
    /// When it was admitted.
    pub started_at: EpochMillis,
    /// When it ended.
    pub ended_at: Option<EpochMillis>,
    /// How many operations it recorded.
    pub operation_count: u64,
    /// Operation events Neo dropped under disk pressure. Non-zero makes `effects` `Unknown`.
    pub trace_gaps: u64,
    /// Ended, and every screenshot job it made was answered: a re-read returns the same data.
    pub complete: bool,
}

/// A run with its operations in admission order.
#[derive(Clone, Debug, PartialEq)]
pub struct RunDetail {
    /// The run.
    pub summary: RunSummary,
    /// The host data it was started with, masked.
    pub host_data: Option<Value>,
    /// Its code, masked; `None` when withheld.
    pub code: Option<String>,
    /// Whether `code` was cut.
    pub code_truncated: bool,
    /// Why the code is withheld (`sensitive_input`), when it is.
    pub code_withheld: Option<String>,
    /// Its return value, masked and bounded.
    pub result: Option<Value>,
    /// Whether `result` was cut.
    pub result_truncated: bool,
    /// Its operations, by `seq`.
    pub operations: Vec<OperationRow>,
}

/// The latest operations that match a [`TailQuery`], with the runs they belong to.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceTail {
    /// The runs of `operations`, oldest first.
    pub runs: Vec<RunSummary>,
    /// The matching operations by start, oldest first, cut to the tail asked for.
    pub operations: Vec<OperationRow>,
    /// Whether the tail cut older matching operations.
    pub truncated: bool,
}

/// The tab a task's latest operation acted on, and when, as
/// [`TraceStore::task_last_act`](crate::TraceStore::task_last_act) reads it. A follow view that
/// lost the task's live events re-reads it, and weighs `at` against the last front move it saw,
/// so an operation older than that move cannot undo it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskAct {
    /// The operation's target, or the tab of the page it ended on when it had none.
    pub tab: TabId,
    /// When the task acted on `tab`: the operation's start, or its end for a tab it ended on,
    /// which exists only from then (a `newPage`'s).
    pub at: EpochMillis,
}

/// One page of a listing.
#[derive(Clone, Debug, PartialEq)]
pub struct Page<T> {
    /// The items on this page.
    pub items: Vec<T>,
    /// The cursor for the next page; `None` on the last one.
    pub next_cursor: Option<String>,
}

/// Filters for listing runs, newest first. The filters combine with AND, and every one
/// applies before paging.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunQuery {
    /// Only runs started with this request id.
    pub request_id: Option<RequestId>,
    /// Only this task's runs.
    pub task_id: Option<TaskId>,
    /// Only runs with an operation that targeted this tab.
    pub tab: Option<TabId>,
    /// Only runs with this status.
    pub status: Option<RunStatus>,
    /// Only runs started at or after this time.
    pub since: Option<EpochMillis>,
    /// Only runs started at or before this time.
    pub until: Option<EpochMillis>,
    /// The previous page's `next_cursor`.
    pub cursor: Option<String>,
    /// Page size, 1 to 100 (25 when zero).
    pub limit: u32,
}

/// Which operations a [`TraceTail`] keeps. The filters combine with AND; with neither, the
/// tail covers every run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TailQuery {
    /// Only operations of this task's runs.
    pub task_id: Option<TaskId>,
    /// Only operations that targeted this tab.
    pub tab: Option<TabId>,
    /// How many of the latest operations to keep, 1 to 1000 (100 when zero).
    pub tail: u32,
}
