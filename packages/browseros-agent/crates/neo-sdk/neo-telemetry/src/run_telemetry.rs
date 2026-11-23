//! A run as its ops see it: where every op of the run reports its settlement.

use crate::backend::Backend;
#[cfg(feature = "otel")]
use crate::pipeline::PipelineParent;
use crate::{CaptchaSolve, Navigated, OpSettled};

/// A run as its ops see it: the handle it records through and, for a counted run, the run's
/// span context and shared per-run state, so each op's span is a child of the run's
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
    #[cfg_attr(
        not(feature = "otel"),
        expect(dead_code, reason = "without `otel` every op settles into the no-op")
    )]
    link: RunLink,
}

/// Which run an op's span hangs under.
#[derive(Clone, Debug)]
pub(crate) enum RunLink {
    /// An internal run: no run span, no run metrics.
    Internal,
    /// A counted run on the live pipeline.
    #[cfg(feature = "otel")]
    Otel(PipelineParent),
}

impl RunTelemetry {
    pub(crate) fn new(backend: Backend, link: RunLink) -> Self {
        Self { backend, link }
    }

    /// Records one settled op: `neo_operations_total`, `neo_operation_duration_seconds`,
    /// `neo_errors_total` when it failed on its own (outcome `error`; a cancelled op's error is
    /// what stopped its run, which the run's end counts once), and its span, built now
    /// with `op.started` as its start; an [`crate::OpCounting::Internal`] op gets its span alone.
    /// The door calls it exactly once per op, from its one settle, on every path (answered,
    /// refused, stopped, aborted, panicked).
    #[cfg_attr(
        not(feature = "otel"),
        expect(
            unused_variables,
            reason = "without `otel` every op settles into the no-op"
        )
    )]
    pub fn op_settled(&self, op: OpSettled<'_>) {
        match &self.backend {
            Backend::Noop => {}
            #[cfg(feature = "otel")]
            Backend::Otel(pipeline) => {
                let parent = match &self.link {
                    RunLink::Otel(parent) => Some(parent),
                    RunLink::Internal => None,
                };
                pipeline.op_settled(parent, &op);
            }
        }
    }

    /// Records one navigation the browser was sent and that ended on its own:
    /// `neo_navigations_total` and, when the network refused it, `neo_navigation_errors_total`.
    /// The op that ran it settles as usual; this adds what its outcome cannot say, such as the
    /// 404 of a goto that settled `ok`. Recorded for an internal run's ops too.
    #[cfg_attr(
        not(feature = "otel"),
        expect(
            unused_variables,
            reason = "without `otel` every navigation records into the no-op"
        )
    )]
    pub fn navigated(&self, navigation: Navigated) {
        match &self.backend {
            Backend::Noop => {}
            #[cfg(feature = "otel")]
            Backend::Otel(pipeline) => pipeline.navigated(navigation),
        }
    }

    /// Records one CAPTCHA solve that returned an outcome: `neo_captcha_solves_total`,
    /// `neo_captcha_reasons_total` and its cost in `neo_captcha_spend_total`. Recorded for
    /// an internal run's ops too.
    #[cfg_attr(
        not(feature = "otel"),
        expect(
            unused_variables,
            reason = "without `otel` every solve records into the no-op"
        )
    )]
    pub fn captcha_solved(&self, solve: CaptchaSolve) {
        match &self.backend {
            Backend::Noop => {}
            #[cfg(feature = "otel")]
            Backend::Otel(pipeline) => pipeline.captcha_solved(solve),
        }
    }

    /// The pipeline this records into, for the cost tests.
    #[cfg(test)]
    pub(crate) fn is_noop(&self) -> bool {
        matches!(self.backend, Backend::Noop)
    }
}
