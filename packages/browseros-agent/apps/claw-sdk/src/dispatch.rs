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
//! Every record a run sends, the run's own and its ops', carries the values held since the run
//! began ([`HeldValues`]), so the trace masks it with at least what the run's reply is masked
//! with. A script's op replies are masked with them too on their way into its realm
//! ([`RunMask`]), so the script never receives a value it could carry out of Neo before its field
//! goes quiet. A run that lost one of them past the cap withholds what it produces instead
//! ([`Held`]).
//!
//! Ops run as tokio tasks on their run's `JoinSet`, never inside the JS runtime: a method's
//! panic stays in its own task and still answers, ops progress while the script computes, and
//! no lock is held across an await, so overlapping calls never wait on each other.
//!
//! [`RunCtx`] lives here because every op belongs to one, and [`RunCtx::start`] records the
//! run, so an op can never run outside a recorded run. A stopped run's write-once
//! [`StopReason`] names the owner of every op it cut short, but for a background pull: the
//! run's end is how a pull ends, so a stop retires it with no error.

mod dispatcher;
mod held_values;
mod method;
mod op_ctx;
mod operation_error;
mod operation_guard;
mod page_target;
mod policy;
mod refusal_brand;
mod registry;
mod run_ctx;
mod run_mask;
#[cfg(test)]
mod tests;

pub(crate) use dispatcher::{Call, CallSite, Caller, Dispatcher, OpReply, PageFact, PendingReply};
pub(crate) use held_values::{Held, HeldValues, HeldWatch, WITHHELD};
pub(crate) use method::{Lifetime, Listing, Method, MethodSpec, Output, Visibility};
pub(crate) use op_ctx::{OpCtx, PageChange};
pub(crate) use operation_error::{Answer, ErrorCode, LeafError, OpFailure, OperationError};
pub(crate) use page_target::PageTarget;
pub(crate) use policy::{Policy, PolicyCheck, ResultHold};
pub(crate) use registry::{Registry, RegistryBuilder, RegistryError};
pub(crate) use run_ctx::{OpLine, OpsReport, RunCtx, RunEnd, RunGuard, RunStart, StopReason};
pub(crate) use run_mask::{RunMask, TEXT_CHARS};
