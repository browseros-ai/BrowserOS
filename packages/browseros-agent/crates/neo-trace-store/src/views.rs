//! What the store hands back, and the queries that select it: plain, owned views of runs, calls,
//! getters and sessions, built from rows by `query.rs`. They reuse neo-runtime's public types
//! where those say exactly what is stored (`RunId`, `Delivery`, `PageRef`, …) and add their own
//! for what only the store knows (a call `lost`, a run `interrupted`, a stub). No SeaORM type
//! appears here.

use std::collections::BTreeMap;

use neo_runtime::{
    CallId, CallLocation, ConsoleLine, Delivery, EndCause, GetterField, IdKey, PageId, PageRef,
    RunId, SessionKey,
};
use serde::Serialize;
use serde_json::Value;

/// The most items one read returns; a larger `limit` reads as this.
pub const MAX_PAGE: u32 = 10_000;

/// One page of a list. `more` says whether items past the last one exist; the next page's query
/// starts after the last item (`RunQuery::before`, `CallQuery::after`, `SessionQuery::before`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Listing<T> {
    pub items: Vec<T>,
    pub more: bool,
}

/// Which runs [`TraceStore::runs`](crate::TraceStore::runs) lists, newest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunQuery {
    /// Only the runs of this session.
    pub session: Option<SessionKey>,
    /// Only the runs the host labelled with this id, such as `("zuzu.task", "t_123")`.
    pub id: Option<(IdKey, String)>,
    /// Only runs that started before this one: the last run of the previous page.
    pub before: Option<RunId>,
    pub limit: u32,
}

impl RunQuery {
    /// The `limit` most recent runs.
    #[must_use]
    pub fn recent(limit: u32) -> Self {
        Self {
            session: None,
            id: None,
            before: None,
            limit,
        }
    }
}

/// Which calls of one run [`TraceStore::calls`](crate::TraceStore::calls) lists, in call order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallQuery {
    /// Only calls numbered after this one: the last call of the previous page.
    pub after: Option<u64>,
    pub limit: u32,
}

/// Which sessions [`TraceStore::sessions`](crate::TraceStore::sessions) lists, most recently
/// active first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionQuery {
    /// Only sessions that sort after this one: the `(last_activity_ms, key)` of the last
    /// session of the previous page.
    pub before: Option<(u64, SessionKey)>,
    pub limit: u32,
}

impl SessionQuery {
    /// The `limit` most recently active sessions.
    #[must_use]
    pub fn recent(limit: u32) -> Self {
        Self {
            before: None,
            limit,
        }
    }
}

/// How much of a session's calls [`TraceStore::session`](crate::TraceStore::session) returns.
/// Every run is always there; only calls are bounded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timeline {
    All,
    /// The most recent `n` calls of the session, so a caller can bound a response (Zuzu reads at
    /// most 8 MiB).
    LastCalls(u32),
}

/// Where a run is in its life, as the store last heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RunState {
    Running,
    Ok,
    Error,
    Timeout,
    Cancelled,
    /// The store never learned how the run ended: its process died first, or its end was lost.
    Interrupted,
}

/// Who settled a run: decided that nothing more of it will come.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SettledBy {
    /// The run's own last event, with its exact loss counts.
    Runtime,
    /// The store gave up waiting for the run's last event; its loss counts are unknown.
    Timeout,
    /// The store found the run open when it opened: the process that ran it died.
    Restart,
}

/// When and by whom a run was settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settled {
    pub at_ms: u64,
    pub by: SettledBy,
}

/// What of a run's record the store could not keep. Nothing is lost silently: every event the
/// store could not keep whole is counted here or on [`OverCap`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Loss {
    /// Events kept as stubs: ids, timing, result kind, error name and delivery only.
    pub stubbed: u64,
    /// Events not kept at all.
    pub dropped: u64,
    /// The run settled without its last event, so `stubbed` and `dropped` are unknown.
    pub incomplete: bool,
}

/// A run's calls and getters past the store's row caps (50,000 calls and 10,000 getters by
/// number). They ran normally; only their rows were not kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverCap {
    pub calls: u64,
    /// The least safe delivery among the calls past the cap.
    pub delivery: Option<Delivery>,
    pub getters: u64,
}

/// An error a run or call ended with, by its JS `name`. `message` is `None` when the record was
/// kept as a stub.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TraceError {
    pub name: String,
    pub message: Option<String>,
}

/// A run as a list shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub id: RunId,
    /// The host's label, or the run's own id for a run the host did not label.
    pub session: SessionKey,
    pub client: Option<String>,
    /// The host's own ids for the run.
    pub ids: BTreeMap<IdKey, String>,
    /// The tab the run started from. `None` when the store never heard the run start (its
    /// `RunStarted` was shed) and made its row from a later event.
    pub start_page: Option<PageId>,
    pub timeout_ms: Option<u64>,
    pub code_bytes: Option<u64>,
    pub started_at_ms: u64,
    pub state: RunState,
    pub end_cause: Option<EndCause>,
    pub error: Option<TraceError>,
    pub ended_at_ms: Option<u64>,
    pub duration_us: Option<u64>,
    /// How many calls the run made, from its end; every one has a row or is counted.
    pub calls_made: u64,
    /// Messages the runtime could not read as a call.
    pub unreadable_calls: u64,
    pub over_cap: OverCap,
    pub settled: Option<Settled>,
    pub loss: Loss,
}

/// One run with everything but its calls and getters, which are paged separately.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTrace {
    pub summary: RunSummary,
    /// The script, scrubbed of the run's secrets.
    pub script: Option<String>,
    pub script_truncated: bool,
    pub value: Option<Value>,
    pub value_truncated: bool,
    pub logs: Vec<ConsoleLine>,
    pub pages: Vec<PageTrace>,
}

/// A page a run touched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageTrace {
    pub page: PageRef,
    /// The run created the page, with call `by_call`.
    pub created: bool,
    pub by_call: Option<u64>,
    pub opened_offset_us: Option<u64>,
    /// When the run closed the page, if it did.
    pub closed_offset_us: Option<u64>,
}

/// Where a call is, as the store last heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CallState {
    Running,
    Ok,
    Error,
    /// The run ended under the call; [`CallTrace::unfinished`] says how.
    Unfinished,
    /// The call was still running when its run settled, so how it ended was lost.
    Lost,
}

/// Why a call never replied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Unfinished {
    /// The script returned or threw without awaiting it.
    Settled,
    Timeout,
    Cancelled,
    /// The process running it died.
    Interrupted,
}

/// How much of a call's or getter's record was kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Detail {
    Full,
    /// Ids, timing, result kind, error name and delivery only: the store had no room for the
    /// params, log and message, or the run had used up its detail budget (16 MiB).
    Stub,
}

/// One browser call or host method.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallTrace {
    /// `<run>@<n>`.
    pub id: CallId,
    /// The call that made this one, for a host method's nested calls.
    pub parent: Option<u64>,
    /// The object the call named, such as `frame@<targetId>`.
    pub object: String,
    pub page: Option<PageRef>,
    pub method: String,
    pub title: Option<String>,
    pub selector: Option<String>,
    /// The redacted params summary; `null` before the call ended or when kept as a stub.
    pub params: Value,
    pub params_truncated: bool,
    pub timeout_ms: Option<f64>,
    pub location: Option<CallLocation>,
    pub started_at_ms: u64,
    /// Since the run started.
    pub offset_us: u64,
    pub duration_us: Option<u64>,
    /// When the call wrote its first input event; from then its delivery may be unknown.
    pub input_offset_us: Option<u64>,
    pub state: CallState,
    pub error: Option<TraceError>,
    pub unfinished: Option<Unfinished>,
    /// Whether an input call's events reached the page; `None` for reads.
    pub delivery: Option<Delivery>,
    /// `delivery` is final: a late acknowledgement already moved it, or its run settled. A
    /// settled `Unknown` must never be repeated blindly.
    pub settled: bool,
    /// The call log, as Playwright's error messages fold it, scrubbed.
    pub log: Vec<String>,
    pub detail: Detail,
    /// What neo-feeds captured after the call.
    pub after: Option<AfterCall>,
}

/// neo-feeds' capture after a call ([`TraceStore::attach_after_call`](crate::TraceStore::attach_after_call)).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AfterCall {
    pub tab_id: Option<i64>,
    pub url: Option<String>,
    pub title: Option<String>,
    /// The viewport after the call, at most 64 KiB; `None` if there was none, it was larger, or
    /// its session is withheld. Not serialized.
    #[serde(skip)]
    pub jpeg: Option<Vec<u8>>,
    pub captured_at_ms: u64,
}

/// How a getter answered.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GetterOutcome {
    /// The value, summarised; `null` when kept as a stub.
    Ok {
        summary: Value,
    },
    Error(TraceError),
}

/// One synchronous getter (`page.url()`, `page.isClosed()`, `context.pages()`, a page's
/// initializer).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetterTrace {
    pub run: RunId,
    /// The getter's own number in the run, from 1.
    pub no: u64,
    pub object: String,
    pub field: GetterField,
    pub page: Option<PageRef>,
    pub outcome: GetterOutcome,
    pub offset_us: u64,
    /// How long the getter blocked the script.
    pub duration_us: u64,
    pub detail: Detail,
}

/// A session as a list shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub key: SessionKey,
    pub client: Option<String>,
    pub first_run: Option<RunId>,
    pub last_run: Option<RunId>,
    pub runs: u64,
    pub last_activity_ms: u64,
    /// neo-feeds withheld the session: no JPEG of it is kept.
    pub withheld: bool,
}

/// A session's runs and calls, oldest first: one root per run, then the calls.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTimeline {
    pub session: SessionSummary,
    pub runs: Vec<RunSummary>,
    pub calls: Vec<CallTrace>,
    /// Older calls were left out ([`Timeline::LastCalls`]).
    pub calls_truncated: bool,
}

/// A session's state in memory, moved by the writer only after the rows it describes are
/// readable. A `/v1` feed waits on it instead of re-reading the database, and caches a response
/// by its `version`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHead {
    /// Grows with every commit that changed the session; `0` until the store has read it.
    pub version: u64,
    pub runs: u64,
    pub live_runs: u64,
    /// Runs not yet settled; a session is complete only when this is `0`.
    pub unsettled_runs: u64,
    pub last_run: Option<RunId>,
    pub last_call: Option<LastCall>,
    pub changed_at_ms: u64,
}

/// The latest call of a session's latest run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastCall {
    pub id: CallId,
    pub state: CallState,
}
