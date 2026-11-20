//! A run as its ops see it: where every op of the run reports its settlement.

#[cfg(feature = "test-support")]
use std::sync::Arc;

use crate::OpSettled;
use crate::backend::Backend;
use crate::pipeline::PipelineParent;
#[cfg(feature = "test-support")]
use crate::recording::TelemetryCall;

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
    /// A counted run on the recording double, by its id.
    #[cfg(feature = "test-support")]
    Recorded(Arc<str>),
}

impl RunTelemetry {
    pub(crate) fn new(backend: Backend, link: RunLink) -> Self {
        Self { backend, link }
    }

    /// Records one settled op: `neo_operations_total`, `neo_operation_duration_seconds`,
    /// `neo_errors_total` when it failed, and its `neo.op` span, built now with `op.started` as
    /// its start. The door calls it exactly once per op, from its one settle, on every path
    /// (answered, refused, stopped, aborted, panicked).
    pub fn op_settled(&self, op: OpSettled<'_>) {
        match &self.backend {
            Backend::Noop => {}
            Backend::Otel(pipeline) => {
                let parent = match &self.link {
                    RunLink::Otel(parent) => Some(parent),
                    RunLink::Internal => None,
                    #[cfg(feature = "test-support")]
                    RunLink::Recorded(_) => None,
                };
                pipeline.op_settled(parent, &op);
            }
            #[cfg(feature = "test-support")]
            Backend::Recording(recorder) => recorder.record(TelemetryCall::OpSettled {
                run_id: match &self.link {
                    RunLink::Recorded(run_id) => Some(run_id.to_string()),
                    RunLink::Internal | RunLink::Otel(_) => None,
                },
                op: op.op,
                raw_name: op.raw_name.map(str::to_owned),
                op_label: recorder.vocabulary().op_label(op.op),
                op_id: op.op_id.to_string(),
                site: op.site,
                tab_id: op.tab_id,
                outcome: op.outcome,
                delivery: op.delivery,
                error: op.error,
                url: op.url.map(str::to_owned),
            }),
        }
    }

    /// The pipeline this records into, for the cost tests.
    #[cfg(test)]
    pub(crate) fn is_noop(&self) -> bool {
        matches!(self.backend, Backend::Noop)
    }
}
