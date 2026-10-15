//! Trait seams a host plugs into the shared MCP layer.
//!
//! Each trait marks a point where claw-server and a leaner host genuinely differ: claw
//! persists an audit timeline, a durable tab ledger, PostHog analytics and cockpit
//! activity, while a host without those surfaces passes the matching `noop` adapter.
//! Everything that decides MCP behaviour (sessions, ownership, grouping, notices) stays
//! in this crate and is identical for every host.

use crate::{error::HostResult, ids::DispatchId};
use browseros_core::BrowserSession;
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::sync::Arc;

/// Identity of a newly minted session, as the journal records it.
#[derive(Debug, Clone, Copy)]
pub struct SessionStart<'a> {
    pub session_id: &'a str,
    /// Conversation key (`{slug}-{fun-name}`); audit rows call it `agent_id`.
    pub convo_id: &'a str,
    pub slug: &'a str,
    pub agent_label: &'a str,
    pub client_name: &'a str,
    pub client_version: &'a str,
}

/// Durable record of session lifecycle, written by `Sessions` at mint and teardown.
///
/// Ordering is part of the contract: `record_session_start` runs before the session is
/// published, and an error aborts the mint so no live session exists without its start
/// row. Teardown releases tab claims and then records the end; only a successful end
/// fires the completion hook.
pub trait SessionJournal: Send + Sync {
    fn record_session_start<'a>(&'a self, start: SessionStart<'a>)
    -> BoxFuture<'a, HostResult<()>>;

    fn record_session_end<'a>(
        &'a self,
        session_id: &'a str,
        kind: &'a str,
        reason: Option<&'a str>,
    ) -> BoxFuture<'a, HostResult<()>>;

    /// Rewrites a dispatch the operator's Stop interrupted, after its call has drained.
    fn mark_dispatch_operator_cancelled<'a>(
        &'a self,
        dispatch_id: &'a DispatchId,
    ) -> BoxFuture<'a, HostResult<()>>;

    /// Queues release of every tab claim the session holds. Must not block teardown.
    fn release_session_tabs(&self, session_id: String);

    /// Stores the PII-scrubbed `name_session` summary for later search.
    fn set_task_summary<'a>(
        &'a self,
        session_id: &'a str,
        summary: &'a str,
    ) -> BoxFuture<'a, HostResult<()>>;
}

/// Product lifecycle facts the host layer reports. A host owns the wire catalog: claw
/// maps each onto its single-source PostHog event definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostEvent {
    /// `{ client_name }`
    SessionStarted,
    /// `{ task_category, client_name, task_summary? }`, at most once per session.
    SessionTaskDeclared,
    /// `{ client_name, tool_name, dispatch_count, total_duration_ms, max_duration_ms }`
    SessionToolUsage,
    /// `{ kind, client_name, dispatch_count, distinct_tool_count, max_concurrent_used_sessions }`
    SessionEnded,
}

/// Largest integer a JavaScript analytics consumer represents exactly. Usage counters
/// saturate here instead of wrapping or losing precision downstream.
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Fixed set of task kinds `name_session` accepts as `category`. Only these tokens are
/// meant to leave the machine; the free-form session name never does. Hosts that
/// normalize analytics coerce anything else to `other`.
pub const TASK_CATEGORY_VALUES: [&str; 11] = [
    "shopping",
    "research",
    "email-and-messaging",
    "form-filling",
    "data-extraction",
    "testing-and-qa",
    "dev-tools",
    "social-media",
    "finance-and-admin",
    "internal-tools",
    "other",
];

/// Receives host lifecycle events. Implementations own validation and delivery; the
/// host layer never depends on an analytics SDK.
pub trait HostAnalytics: Send + Sync {
    fn capture(&self, event: HostEvent, properties: Value);
}

/// Sees every browser session `BrowserService` creates (one per CDP epoch). claw's tab
/// registry seeds its tab-to-target map here for recording attribution and ledger
/// inheritance; a host without recordings has nothing to observe.
pub trait BrowserSessionObserver: Send + Sync {
    /// Attaches to a new epoch's session. Called once per epoch transition.
    fn observe_session(
        &self,
        session: Arc<BrowserSession>,
        epoch: u64,
    ) -> BoxFuture<'_, anyhow::Result<()>>;

    /// Whether the observer is fully seeded for `epoch`; false makes the monitor retry
    /// `reseed` on its next tick without re-attaching.
    fn is_ready(&self, epoch: u64) -> bool;

    fn reseed<'a>(
        &'a self,
        session: &'a BrowserSession,
        epoch: u64,
    ) -> BoxFuture<'a, anyhow::Result<()>>;
}
