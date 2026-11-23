//! One run's telemetry, from admission to its end: the `neo.run` span and the run metrics.

use std::fmt;
#[cfg(feature = "otel")]
use std::sync::Arc;

use crate::backend::Backend;
use crate::label::{ErrorLabel, RunStatus};
#[cfg(feature = "otel")]
use crate::pipeline::{Pipeline, PipelineRun};
use crate::run_telemetry::{RunLink, RunTelemetry};

/// What a run is when it starts ([`crate::Telemetry::start_run`]). Borrowed, so the no-op never
/// copies or formats anything.
#[derive(Clone, Copy)]
pub struct RunFacts<'a> {
    /// Neo's run id, as `GET /v1/runs/{id}` names it (the span's `neo.run.id`). Formatted only
    /// when a span is recorded.
    pub run_id: &'a dyn fmt::Display,
    /// The host's task (`_meta["com.browseros.neo/run"].taskId`), if the run has one.
    pub task_id: Option<&'a str>,
    /// The caller's trace, which the run's span joins; `None` starts a new trace.
    pub parent: Option<&'a crate::TraceContext>,
}

impl fmt::Debug for RunFacts<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunFacts")
            .field("run_id", &format_args!("{}", self.run_id))
            .field("task_id", &self.task_id)
            .field("parent", &self.parent)
            .finish()
    }
}

/// How a run ended ([`RunSpan::finish`]): the same end its trace record gets. A plain data bag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunVerdict {
    /// The run's status, as `/v1/runs` reports it.
    pub status: RunStatus,
    /// Its failure, exactly when its record carries an error.
    pub error: Option<ErrorLabel>,
}

/// One run's telemetry, owned by the run's own guard (the server's `RunGuard`, through its
/// `RunStart`) from the run's start to its settlement. For a counted run ([`crate::Telemetry::start_run`]) it holds the
/// run in `neo_run_active` while it lives, and ending it records the run's metrics and ends
/// its `neo.run` span, exactly once. An internal run's span ([`crate::Telemetry::internal_run`])
/// records nothing itself; its ops still count.
///
/// [`RunSpan::finish`] is the end the guard calls on both of its paths (its finish and its
/// drop), with the verdict the trace gets. Dropping it unfinished is only a backstop the guard
/// never relies on: in-flight still comes down, once, and the run is recorded as Neo's
/// `internal_error`.
#[must_use = "dropping it unfinished records the run as a crash; call finish"]
#[derive(Debug)]
pub struct RunSpan {
    ops: RunTelemetry,
    open: Option<OpenRun>,
}

/// Without `otel` this enum has no variant, so `RunSpan::open` is always `None`.
#[derive(Debug)]
enum OpenRun {
    #[cfg(feature = "otel")]
    Otel {
        pipeline: Arc<Pipeline>,
        run: PipelineRun,
    },
}

impl RunSpan {
    /// A run that records nothing, on a handle that records nothing.
    pub(crate) fn noop() -> Self {
        Self::internal(Backend::Noop)
    }

    /// Neo's own run on `backend`: no run metrics, no `neo.run` span; its ops still count.
    pub(crate) fn internal(backend: Backend) -> Self {
        Self {
            ops: RunTelemetry::new(backend, RunLink::Internal),
            open: None,
        }
    }

    #[cfg(feature = "otel")]
    pub(crate) fn otel(pipeline: Arc<Pipeline>, run: PipelineRun) -> Self {
        Self {
            ops: RunTelemetry::new(
                Backend::Otel(Arc::clone(&pipeline)),
                RunLink::Otel(run.ops()),
            ),
            open: Some(OpenRun::Otel { pipeline, run }),
        }
    }

    /// Whether this span records nothing (the no-op's), for the cost tests.
    #[cfg(test)]
    pub(crate) fn is_noop(&self) -> bool {
        self.open.is_none()
    }

    /// The run as its ops see it ([`RunTelemetry::op_settled`]). Kept with the run's state
    /// (the server's `RunCtx`), where the door's settle reaches it.
    #[must_use]
    pub fn ops(&self) -> RunTelemetry {
        self.ops.clone()
    }

    /// Ends the run with its verdict: in-flight comes down and the run's counter, histogram
    /// and span record `verdict`.
    pub fn finish(mut self, verdict: RunVerdict) {
        if let Some(open) = self.open.take() {
            end(open, Some(verdict));
        }
    }
}

impl Drop for RunSpan {
    fn drop(&mut self) {
        if let Some(open) = self.open.take() {
            end(open, None);
        }
    }
}

/// Shared by `finish` and `Drop`; `None` is a run that ended without a verdict.
#[cfg_attr(
    not(feature = "otel"),
    expect(
        unused_variables,
        clippy::needless_pass_by_value,
        reason = "without `otel` no run is open, so none ends"
    )
)]
fn end(open: OpenRun, verdict: Option<RunVerdict>) {
    match open {
        #[cfg(feature = "otel")]
        OpenRun::Otel { pipeline, run } => pipeline.run_ended(run, verdict),
    }
}
