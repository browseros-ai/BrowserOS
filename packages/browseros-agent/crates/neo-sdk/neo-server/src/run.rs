//! The run lifecycle: one `run` call, from admission to its reply and its trace records.
//!
//! ```text
//!  RunService::run ─ RunHandle + this caller's Waiter
//!    RequestLedger::admit ─ Join (wait) │ Replay │ Duplicate │ Unavailable ─▶ that reply
//!     └ Fresh(ticket) ─▶ tokio::spawn(execute), the panic boundary
//!        view ─ page default (the task's own tab: TaskBook, or after a restart the trace's
//!          task_page_tab) ─ RunCtx::start
//!        every open tab (the init's `pages`) ─ run_script
//!          ‖ deadline, shutdown, orphaned (no waiter for the grace) ─▶ RunCtx::stop
//!        teardown: stop(ScriptReturned), drain ops, abort stragglers
//!        RunReply::build, then the run's snapshot texts committed as delta baselines or dropped
//!          (RunState::settle_snapshot_baselines) and its tips (snapshot_tips)
//!        RunGuard::finish (run_ended, the reply as replay)
//!          ─ ticket.finish (waiters, ledger)
//!    Waiter::reply ◀── the one reply, shared by every caller of the run
//! ```
//!
//! Who waits. The caller's future holds a `Waiter`, and dropping it is how the MCP layer says
//! the client left. A run with a `requestId` then keeps going for `Defaults::ORPHAN_GRACE`, so
//! the host's retry with the same id joins it instead of running it twice; a run without one
//! stops at once.
//!
//! Failure containment. Each run is its own task, so a panic ends that task only. Its cleanups
//! are guards that run while unwinding: `RunGuard` records `run_ended` as `neo /
//! internal_error`, and the `RunTicket` hands every waiter a crashed reply and keeps the
//! request id from running again. An MCP request always gets an answer.

mod request_ledger;
mod run_handle;
mod run_meta;
mod run_reply;
mod run_request;
mod run_service;
mod snapshot_tips;
#[cfg(test)]
mod tests;

pub(crate) use request_ledger::RequestLedger;
pub(crate) use run_handle::{RunHandle, Waiter};
pub(crate) use run_meta::{HostData, RUN_META_KEY, RunMeta};
pub(crate) use run_reply::{OpNote, ReplyError, RunReply};
pub(crate) use run_request::{RunRequest, RunTimeout};
pub(crate) use run_service::{RunService, RunServiceParts};
pub(crate) use snapshot_tips::ReplyTip;
