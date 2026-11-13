//! The operation dispatcher: the one door every SDK operation walks through (skeleton D7).
//!
//! ```text
//!  script ─▶ bridge ─▶ Dispatcher::submit ─ op id, decode, op_started ─▶ task on the run's
//!    JoinSet ─▶ gates ─▶ target ─▶ rules ─▶ Method::call(&OpCtx) raced with the run's stop
//!    ─▶ a read's value or a main-world script's answer checked again, as a read ─▶ page facts
//!    ─▶ OperationGuard::finish ─▶ reply over a oneshot
//!  a method ─▶ OpCtx::invoke ─▶ the same door, inline, as a child op
//! ```
//!
//! For every operation the door owns the id, both trace records, delivery (derived from the
//! write-once sent flag and how the run step ended, in one pure table), `retryable`, page facts
//! and the run's roll-up of what it changed. Methods report facts through [`OpCtx`]; they never
//! decide delivery and never reach another method except as a child op. What an op returns
//! reaches the script as the method produced it: the door masks nothing, and bounds only the
//! texts a script chooses that it puts in its own refusals ([`clip`]).
//!
//! Ops run as tokio tasks on their run's `JoinSet`, never inside the JS runtime: a method's
//! panic stays in its own task and still answers, ops progress while the script computes, and
//! overlapping method calls progress independently. Only the final cache observations share
//! a lock across their reads, so the reply serial orders complete fact batches.
//!
//! [`RunCtx`] lives here because every op belongs to one, and [`RunCtx::start`] records the
//! run, so an op can never run outside a recorded run. A stopped run's write-once
//! [`StopReason`] names the owner of every op it cut short, but for a background pull: the
//! run's end is how a pull ends, so a stop retires it with no error.

mod close_release;
mod dispatcher;
mod method;
mod observation;
mod op_abort;
mod op_ctx;
mod operation_error;
mod operation_guard;
mod page_target;
mod policy;
mod refusal_brand;
mod registry;
mod run_ctx;
mod run_tasks;
mod tabs_named;
#[cfg(test)]
mod tests;

pub(crate) use dispatcher::{
    Call, CallSite, Caller, Dispatcher, OpReply, PageFact, PageFacts, PendingReply,
};
pub(crate) use method::{Lifetime, Listing, Method, MethodSpec, PageScript, Visibility};
pub(crate) use observation::{
    ActedTarget, ApprovedClose, Claim, EventLog, FrameVersion, Observation, Observer, WorkerFact,
};
pub(crate) use op_abort::OpAbort;
pub(crate) use op_ctx::{OpCtx, PageChange};
pub(crate) use operation_error::{
    Answer, ErrorCode, LeafError, OpFailure, OperationError, TEXT_CHARS, clip,
};
pub(crate) use page_target::PageTarget;
pub(crate) use policy::{Policy, PolicyCheck};
pub(crate) use registry::{Registry, RegistryBuilder, RegistryError};
pub(crate) use run_ctx::{OpLine, OpsReport, RunCtx, RunEnd, RunGuard, RunStart, StopReason};

pub(crate) use run_tasks::{TabNote, TaskKind, TaskNote};
pub(crate) use tabs_named::tabs_named;
