//! The operation dispatcher: the one door every SDK operation walks through (skeleton D7).
//!
//! ```text
//!  script ─▶ bridge ─▶ Dispatcher::submit ─ op id, decode, op_started ─▶ task on the run's
//!    JoinSet ─▶ gates ─▶ target ─▶ rules ─▶ Method::call(&OpCtx) raced with the run's stop
//!    ─▶ reads checked again ─▶ page facts ─▶ OperationGuard::finish ─▶ reply over a oneshot
//!  a method ─▶ OpCtx::invoke ─▶ the same door, inline, as a child op
//! ```
//!
//! For every operation the door owns the id, both trace records, delivery (derived from the
//! write-once sent flag and how the run step ended, in one pure table), `retryable`, the mask,
//! page facts and the run's roll-up of what it changed. Methods report facts through
//! [`OpCtx`]; they never decide delivery and never reach another method except as a child op.
//!
//! Ops run as tokio tasks on their run's `JoinSet`, never inside the JS runtime: a method's
//! panic stays in its own task and still answers, ops progress while the script computes, and
//! no lock is held across an await, so overlapping calls never wait on each other.
//!
//! [`RunCtx`] lives here because every op belongs to one, and [`RunCtx::start`] records the
//! run, so an op can never run outside a recorded run. A stopped run's write-once
//! [`StopReason`] names the owner of every op it cut short.

mod dispatcher;
mod method;
mod op_ctx;
mod operation_error;
mod operation_guard;
mod page_target;
mod policy;
mod registry;
mod run_ctx;
#[cfg(test)]
mod tests;

pub(crate) use dispatcher::{Call, CallSite, Caller, Dispatcher, OpReply, PageFact, PendingReply};
pub(crate) use method::{Lifetime, Listing, Method, MethodSpec, Visibility};
pub(crate) use op_ctx::{OpCtx, PageChange};
pub(crate) use operation_error::{Answer, ErrorCode, LeafError, OpFailure, OperationError};
pub(crate) use page_target::PageTarget;
pub(crate) use policy::{Policy, PolicyCheck};
pub(crate) use registry::{Registry, RegistryBuilder, RegistryError};
pub(crate) use run_ctx::{OpLine, OpsReport, RunCtx, RunEnd, RunGuard, RunStart, StopReason};
