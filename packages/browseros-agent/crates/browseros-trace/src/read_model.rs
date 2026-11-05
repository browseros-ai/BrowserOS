//! What [`TraceStore`](crate::TraceStore) reads back: records as stored (masked and bounded),
//! typed, ready for the service routes to map onto `claw-sdk-api` models.

use crate::{
    Delivery, Effect, EpochMillis, ErrorRecord, OperationId, OperationKind, Outcome, PageState,
    PrivacyRecord, RunEffects, RunId, RunStatus, SessionId,
};
use serde_json::Value;

/// One stored operation.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationRow {
    /// Its id.
    pub id: OperationId,
    /// The operation it ran inside.
    pub parent: Option<OperationId>,
    /// Its session.
    pub session: SessionId,
    /// The method name.
    pub name: String,
    /// Its place in the tree.
    pub kind: OperationKind,
    /// Read or write.
    pub effect: Effect,
    /// The page it targeted and how that page looked after it.
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
    /// When it was admitted.
    pub started_at: EpochMillis,
    /// When it marked itself sent.
    pub sent_at: Option<EpochMillis>,
    /// When it ended.
    pub ended_at: Option<EpochMillis>,
}

/// A run without its operations.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSummary {
    /// Its id.
    pub id: RunId,
    /// Its session.
    pub session: SessionId,
    /// How it ended, or `Running`.
    pub status: RunStatus,
    /// What it changed; `None` while it runs.
    pub effects: Option<RunEffects>,
    /// Its uncaught error.
    pub error: Option<ErrorRecord>,
    /// The host's own id for the request that started it.
    pub request_id: Option<String>,
    /// When it was admitted.
    pub started_at: EpochMillis,
    /// When it ended.
    pub ended_at: Option<EpochMillis>,
    /// How many operations it recorded.
    pub operation_count: u64,
    /// Operation events Neo dropped under disk pressure. Non-zero makes `effects` `Unknown`.
    pub trace_gaps: u64,
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

/// A session without its runs.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionSummary {
    /// Its id.
    pub id: SessionId,
    /// The MCP client's name.
    pub client: Option<String>,
    /// When it started.
    pub started_at: EpochMillis,
    /// When it ended, if it did (or when a restart found it open).
    pub ended_at: Option<EpochMillis>,
    /// Its latest run start or end.
    pub last_activity_at: EpochMillis,
    /// How many runs it has.
    pub run_count: u64,
    /// Its privacy, when it is private.
    pub privacy: Option<PrivacyRecord>,
}

/// A session with its runs, newest first.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionDetail {
    /// The session.
    pub summary: SessionSummary,
    /// Its runs, newest first.
    pub runs: Vec<RunSummary>,
}

/// A session with its runs and its latest operations, oldest first.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionTrace {
    /// The session.
    pub summary: SessionSummary,
    /// Its runs, oldest first.
    pub runs: Vec<RunSummary>,
    /// Its operations by start, oldest first, cut to the tail asked for.
    pub operations: Vec<OperationRow>,
    /// Whether the tail cut older operations.
    pub truncated: bool,
}

/// One page of a listing.
#[derive(Clone, Debug, PartialEq)]
pub struct Page<T> {
    /// The items on this page.
    pub items: Vec<T>,
    /// The cursor for the next page; `None` on the last one.
    pub next_cursor: Option<String>,
}

/// Whether a session has ended, as the store records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    /// No end recorded.
    Open,
    /// Ended.
    Ended,
}

/// Filters for listing runs. Every filter applies before paging.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunQuery {
    /// Only this session's runs.
    pub session: Option<SessionId>,
    /// Only runs started with this request id.
    pub request_id: Option<String>,
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

/// Filters for listing sessions, most recently active first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionQuery {
    /// Only open or only ended sessions.
    pub state: Option<SessionState>,
    /// Only sessions started at or after this time.
    pub since: Option<EpochMillis>,
    /// Only sessions started at or before this time.
    pub until: Option<EpochMillis>,
    /// The previous page's `next_cursor`.
    pub cursor: Option<String>,
    /// Page size, 1 to 100 (25 when zero).
    pub limit: u32,
}
