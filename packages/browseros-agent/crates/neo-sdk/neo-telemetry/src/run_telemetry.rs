//! A run as its ops see it: where every op of the run reports its settlement.

use crate::OpSettled;
use crate::backend::Backend;
use crate::pipeline::PipelineParent;

/// A run as its ops see it: the handle it records through and, for a counted run, the run's
/// span context and shared per-run state, so each op's `neo.op` span is a child of the run's
/// `neo.run`. Taken from [`crate::RunSpan::ops`] when the run starts and kept with the run's
/// state, so the door's one settle reaches it for every op of the run, in any task: the door
/// holds no telemetry handle of its own. Cheap to clone, and holds no gauge, so it may outlive
/// its run: an op aborted at the run's teardown settles after the run ended and still finds it.
///
/// An internal run's ([`crate::Telemetry::internal_run`]) has no run span: its ops still count
/// on the op metrics, and their spans are roots.
#[derive(Clone, Debug)]
pub struct RunTelemetry {
    backend: Backend,
    link: RunLink,
}

/// Which run an op's span hangs under.
#[derive(Clone, Debug)]
pub(crate) enum RunLink {
    /// An internal run: no run span, no run metrics.
    Internal,
    /// A counted run on the live pipeline.
    Otel(PipelineParent),
}

impl RunTelemetry {
    pub(crate) fn new(backend: Backend, link: RunLink) -> Self {
        Self { backend, link }
    }

    /// Records one settled op: `neo_operations_total`, `neo_operation_duration_seconds`,
    /// `neo_errors_total` when it failed on its own (outcome `error`; a cancelled op's error is
    /// what stopped its run, which the run's end counts once), and its `neo.op` span, built now
    /// with `op.started` as its start; an [`crate::OpCounting::Internal`] op gets its span alone.
    /// The door calls it exactly once per op, from its one settle, on every path (answered,
    /// refused, stopped, aborted, panicked).
    pub fn op_settled(&self, op: OpSettled<'_>) {
        match &self.backend {
            Backend::Noop => {}
            Backend::Otel(pipeline) => {
                let parent = match &self.link {
                    RunLink::Otel(parent) => Some(parent),
                    RunLink::Internal => None,
                };
                pipeline.op_settled(parent, &op);
            }
        }
    }

    /// The pipeline this records into, for the cost tests.
    #[cfg(test)]
    pub(crate) fn is_noop(&self) -> bool {
        matches!(self.backend, Backend::Noop)
    }
}
