//! Trait seams a host plugs into the shared MCP layer.
//!
//! Each trait marks a point where claw-server and a leaner host genuinely differ: claw
//! persists an audit timeline, a durable tab ledger, PostHog analytics and cockpit
//! activity, while a host without those surfaces passes the matching `noop` adapter.
//! Everything that decides MCP behaviour (sessions, ownership, grouping, notices) stays
//! in this crate and is identical for every host.

use crate::{
    ctx::HostCtx,
    error::HostResult,
    identity::ProfileView,
    ids::DispatchId,
    mcp::dispatch::{ToolCall, ToolEffectContext, ToolIdentity, ToolObserverContext},
    sessions::Session,
};
use browseros_core::{BrowserSession, TargetId, pages::PageInfo};
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;
use rmcp::model::{CallToolResult, Tool};
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

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

/// One agent touch of a tab, for a host's live activity view (claw's cockpit).
#[derive(Debug, Clone)]
pub struct TabActivity {
    pub target_id: TargetId,
    pub tab_id: i64,
    pub page_id: u32,
    pub session_id: String,
    /// Conversation key of the acting agent; claw's activity rows call it `agent_id`.
    pub convo_id: String,
    pub slug: String,
    pub tool_name: String,
}

/// A tab an agent opened, from the moment it was created.
#[derive(Debug, Clone)]
pub struct TabClaim {
    pub tab_id: i64,
    pub target_id: Option<String>,
    pub session_id: String,
    pub convo_id: String,
    pub claimed_at_ms: i64,
}

/// Host bookkeeping of which agent touched and opened which tab. Page ownership itself
/// (the label every agent sees) lives in `sessions::PageOwnership` and is not routed
/// through here; this seam only mirrors it into claw's cockpit activity view and its
/// durable tab ledger. Claims and releases are fire-and-forget queue writes.
pub trait TabJournal: Send + Sync {
    fn record_activity(&self, activity: TabActivity) -> BoxFuture<'_, ()>;

    /// Drops activity for a closed page incarnation (`page_id` + `target_id`).
    fn forget_incarnation<'a>(&'a self, page_id: u32, target_id: &'a str) -> BoxFuture<'a, ()>;

    fn claim_tab(&self, claim: TabClaim);

    fn release_tab(&self, tab_id: i64, session_id: String);
}

/// Saved agent profiles a client identity can resolve to. claw maps a client to a
/// cockpit-managed profile slug and label; with none, identities stay ephemeral.
pub trait ProfileDirectory: Send + Sync {
    fn profiles(&self) -> BoxFuture<'_, HostResult<Vec<ProfileView>>>;
}

/// A primitive a `run`/`evaluate` script executed through the captured view, handed to
/// `DispatchObserver::observe_inner_call`. It never passed through dispatch effects;
/// `call` is the parent script dispatch.
pub struct InnerCallContext<'a> {
    pub call: &'a ToolCall,
    pub identity: &'a ToolIdentity,
    /// Plain capability name (`read`, `wait`, …), routing prefix already stripped.
    pub tool_name: &'a str,
    pub page: Option<u32>,
    /// The page as it is after the primitive ran, when it is still live.
    pub live: Option<&'a PageInfo>,
    pub args: &'a Value,
    pub is_error: bool,
    /// The primitive ran inside a hot-loaded helper.
    pub from_helper: bool,
    pub duration_ms: i64,
    pub output_token_estimate: i64,
}

/// A finished host-local tool call (`name_session`, claw's `save_skill`, …), handed to
/// `DispatchObserver::observe_local_tool`.
pub struct LocalToolContext<'a> {
    pub session: &'a Arc<Session>,
    pub agent_label: &'a str,
    pub tool_name: &'a str,
    /// Arguments as they may be persisted (already scrubbed where the tool scrubs).
    pub args: &'a Value,
    pub result: &'a ToolResult,
    pub duration_ms: i64,
    pub dispatch_id: DispatchId,
}

/// Refuses a catalog dispatch before execution. Host guards run after the built-in ones
/// (navigate scheme, browser connected). Ownership must never become a guard.
pub trait DispatchGuard: Send + Sync {
    fn check<'a>(&'a self, call: &'a ToolCall) -> BoxFuture<'a, Option<ToolResult>>;
}

/// Rewrites or annotates a successful-or-failed result, in order, after execution and
/// before the wire. Host effects run after the built-in ones; a failing effect is logged
/// and the latest good result carries on.
pub trait DispatchEffect: Send + Sync {
    fn name(&self) -> &'static str;

    fn apply<'a>(
        &'a self,
        context: ToolEffectContext<'a>,
    ) -> BoxFuture<'a, anyhow::Result<Option<ToolResult>>>;
}

/// Read-only consumer of final dispatch outcomes (claw: audit timeline, run-failure
/// reports). Catalog dispatches arrive through `observe`, after effects, exactly once and
/// unchangeable; script primitives and local tools through the two defaulted methods.
pub trait DispatchObserver: Send + Sync {
    fn name(&self) -> &'static str;

    fn observe<'a>(&'a self, context: ToolObserverContext<'a>)
    -> BoxFuture<'a, anyhow::Result<()>>;

    fn observe_inner_call<'a>(&'a self, _call: InnerCallContext<'a>) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    fn observe_local_tool<'a>(
        &'a self,
        _dispatch: LocalToolContext<'a>,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

/// Everything a host-local tool sees for one call, already admitted to a live session.
pub struct LocalToolCall<'a> {
    pub host: &'a Arc<HostCtx>,
    pub session: &'a Arc<Session>,
    pub agent_label: &'a str,
    /// Arguments with the host-owned `session`/`agentName` fields already stripped.
    pub args: &'a Value,
    /// The MCP request's own token: fires when the client abandons this call.
    pub request_cancel: CancellationToken,
}

/// A tool served by the host next to the browser catalog. The crate registers
/// `name_session`; claw adds its skill and human-help tools. Local tools bypass the
/// catalog dispatch pipeline, so each one registers its own dispatch with the session
/// (`LocalToolCall::begin`) to honour Stop.
pub trait LocalTool: Send + Sync {
    /// The undecorated MCP definition; the host adds the `session`/`agentName` args.
    fn definition(&self) -> Tool;

    /// Whether `tools/list` offers it right now. An unlisted tool stays callable by name
    /// so a client holding a cached list gets the tool's own refusal.
    fn listed(&self) -> bool {
        true
    }

    fn call<'a>(&'a self, call: LocalToolCall<'a>) -> BoxFuture<'a, CallToolResult>;
}
