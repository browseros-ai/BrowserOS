//! What [`TraceStore`](crate::TraceStore) reads back: records as stored (bounded),
//! typed, ready for the service routes to map onto `neo-sdk-api` models.
//!
//! A host finds its records by its own ids (`requestId`, `taskId`), by browser tab, and by
//! time.

use crate::{
    Delivery, Effect, EpochMillis, ErrorRecord, OperationId, OperationKind, OperationSite, Outcome,
    PageState, RequestId, RunDownload, RunEffects, RunId, RunStatus, TaskId,
};
use browseros_core::TabId;
use serde_json::Value;

/// One stored operation.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationRow {
    /// `off` when trace.values omitted values at this operation's end; absent otherwise.
    pub values: Option<String>,
    /// The script call site; absent for old rows and work without a script frame.
    pub site: Option<OperationSite>,
    /// Its id.
    pub id: OperationId,
    /// The operation it ran inside.
    pub parent: Option<OperationId>,
    /// The tab it targeted, when it had one.
    pub tab: Option<TabId>,
    /// The method name: clipped.
    pub name: String,
    /// Its place in the tree.
    pub kind: OperationKind,
    /// Read or write.
    pub effect: Effect,
    /// How its tab looked after it.
    pub page: PageState,
    /// Input: bounded.
    pub args: Option<Value>,
    /// Whether `args` was cut.
    pub args_truncated: bool,
    /// Result: bounded.
    pub result: Option<Value>,
    /// Whether `result` was cut.
    pub result_truncated: bool,
    /// How it ended; `None` while it runs.
    pub outcome: Option<Outcome>,
    /// What happened to its side effect; `None` while it runs.
    pub delivery: Option<Delivery>,
    /// Its error, when it failed.
    pub error: Option<ErrorRecord>,
    /// Its call-log lines, oldest first: bounded. Empty while it runs, and when it noted
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
    /// Whether it sent script into a page's main world ([`OperationEnded::main_world`]).
    /// `None` while it runs, when its end never arrived, and for rows stored before the column.
    ///
    /// [`OperationEnded::main_world`]: crate::OperationEnded::main_world
    pub main_world: Option<bool>,
}

impl OperationRow {
    /// Whether Neo owns this subscription rather than an explicit script listener.
    /// Subscription names are shared with tab ownership and run-end screenshots; the script
    /// site distinguishes public listener calls from the prelude's background maintenance.
    /// Historical listener rows without a recorded site are indistinguishable from Neo's work.
    #[must_use]
    pub fn is_internal(&self) -> bool {
        crate::is_subscription(&self.name) && self.site.is_none()
    }
}

/// Which operations a host read includes, without changing what the trace records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OperationFilter {
    /// Every stored operation.
    #[default]
    All,
    /// Omit subscriptions without a recorded script site. Historical listeners may lack sites.
    Script,
}

impl OperationFilter {
    /// Whether a stored row belongs in this view.
    #[must_use]
    pub fn includes(self, operation: &OperationRow) -> bool {
        self == Self::All || !operation.is_internal()
    }
}

/// What became of an operation's screenshot, as its row records it. An operation with none (no
/// job, or the job was dropped) has no state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotState {
    /// A JPEG is on disk.
    Stored,
    /// No picture on purpose: the tab had taken a host's private typing (`Shot::Withheld`).
    /// Older writers recorded it too, for their own reasons.
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
    /// Withheld: the tab had taken a host's private typing, or a frame showed a private box's
    /// value (protected-fill ruling R45), or an older writer withheld it.
    Withheld,
    /// Deleted by the disk cap, or missing from the disk.
    Expired,
    /// No screenshot: none was taken, it was dropped, or no operation has this id.
    None,
}

/// A run without its operations.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSummary {
    /// `off` when trace.values omitted values at this run's end; absent otherwise.
    pub values: Option<String>,
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
    /// The bounded host data, present once the run ends.
    pub host_data: Option<Value>,
    /// Its bounded code; `None` while running or when unavailable.
    pub code: Option<String>,
    /// Whether `code` was cut.
    pub code_truncated: bool,
    /// Why code is absent: `unavailable`, or a legacy `sensitive_input` marker. The reader
    /// preserves older rows even though new runs no longer withhold code for sensitive fields.
    pub code_withheld: Option<String>,
    /// Its return value, bounded.
    pub result: Option<Value>,
    /// Whether `result` was cut.
    pub result_truncated: bool,
    /// Its operations, by `seq`.
    pub operations: Vec<OperationRow>,
    /// The downloads its `download` events handed the script, by index. Empty until the run
    /// ends: they are recorded beside the trace when it does.
    pub downloads: Vec<RunDownload>,
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

/// A task's own tabs as the trace can still tell them after a restart, and its built-in `page`
/// among them, as [`TraceStore::task_tabs`](crate::TraceStore::task_tabs) reads them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskTabs {
    /// The tabs the task surely opened: top-level opens in its runs that picked no task, in
    /// the order it opened them.
    pub opened: Vec<TabId>,
    /// Among `opened`, the one its own script last acted on, or one it opened after that.
    pub page: Option<TabId>,
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
    /// Filter before limiting, so the tail counts operations visible in the requested view.
    pub operations: OperationFilter,
}
