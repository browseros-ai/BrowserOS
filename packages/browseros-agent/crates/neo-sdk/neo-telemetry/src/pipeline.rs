//! The live OpenTelemetry pipeline behind a real [`crate::Telemetry`].
//!
//! Owned by the metrics-and-spans work, which fills in every body here. The facade
//! (`telemetry.rs`, `run_span.rs`, `run_telemetry.rs`) already routes each hook to these methods
//! and owns the run's exactly-once and drop semantics, so this file only records.
//!
//! ```text
//!  Telemetry::new ─▶ Pipeline::start ─┬─ meter provider ── manual reader ─▶ prometheus.rs (scrape, always)
//!                                     │                 └─ Push::metrics (periodic OTLP, opt-in)
//!                                     ├─ tracer provider ─ Push::spans (batch OTLP, opt-in)
//!                                     └─ Push::transport: the exporters' own thread and client
//!                                        resource: otlp::resource, sampler: otlp::sampler
//! ```
//!
//! `instruments.rs` holds the 11 instruments, `tracing_otel.rs` the two spans, `privacy.rs` the
//! site-detail dial, `prometheus.rs` the text renderer. A verdict of `None` is a guard dropped
//! unfinished: record it as `error` / `neo` / `internal_error`, and bring in-flight down all the
//! same. Shutdown arms the transport's deadline, runs both providers' shutdowns on a plain
//! `std::thread` and waits on it with `recv_timeout(budget)`, abandoning it if late: the
//! periodic reader's own shutdown ignores its timeout (5 s), and a `spawn_blocking` task would
//! hold the server's runtime past exit.

use std::time::Duration;

use crate::label::{CloseReason, ErrorLabel, TabOrigin};
use crate::{
    OpSettled, OpenTabsSource, RunFacts, RunVerdict, TelemetryConfig, TelemetryError, Vocabulary,
};

/// The providers, instruments, readers and push door one process records into.
#[derive(Debug)]
pub(crate) struct Pipeline {}

/// One counted run's state while it is open: its `neo.run` span and when it started.
#[derive(Debug)]
pub(crate) struct PipelineRun {}

/// A counted run's state its ops share ([`crate::RunTelemetry`]): its span context, and the
/// run's remembered op error codes for `neo_errors_total`'s de-dup rule (sealed at the run's
/// end; a later op error counts unless it equals the run's sealed error). It may outlive the
/// run: ops aborted at the run's teardown settle after it ended.
#[derive(Clone, Debug)]
pub(crate) struct PipelineParent {}

#[expect(
    clippy::unused_self,
    clippy::needless_pass_by_value,
    reason = "every body is a stub until the metrics and spans land"
)]
impl Pipeline {
    /// Builds the pipeline for `config`, or `None` for telemetry off.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "building the providers can fail; nothing is built yet"
    )]
    pub(crate) fn start(_config: &TelemetryConfig) -> Result<Option<Self>, TelemetryError> {
        Ok(None)
    }

    /// The server's tables, bound once before the first run: labels intern against them, and
    /// until then every op and error-code label is `unknown`.
    pub(crate) fn bind_vocabulary(&self, vocabulary: Vocabulary) {
        drop(vocabulary);
    }

    /// `neo_runs_in_flight` +1 and the `neo.run` span starts.
    pub(crate) fn run_started(&self, _facts: &RunFacts<'_>) -> PipelineRun {
        PipelineRun {}
    }

    /// `neo_runs_in_flight` -1, `neo_runs_total`, `neo_run_duration_seconds`, the span ends.
    pub(crate) fn run_ended(&self, run: PipelineRun, _verdict: Option<RunVerdict>) {
        let PipelineRun {} = run;
    }

    /// A call refused before it became a run: `neo_runs_total{status="error"}` and
    /// `neo_errors_total`; no span, no in-flight, no duration.
    pub(crate) fn run_refused(&self, _error: ErrorLabel) {}

    /// `neo_operations_total`, `neo_operation_duration_seconds`, `neo_errors_total` on a
    /// failure (subject to the de-dup rule), and the `neo.op` span built now from `op.started`,
    /// a child of `parent` (`None`: an op of an internal run, a root span).
    pub(crate) fn op_settled(&self, _parent: Option<&PipelineParent>, _op: &OpSettled<'_>) {}

    /// `neo_tabs_opened_total`.
    pub(crate) fn tab_opened(&self, _origin: TabOrigin) {}

    /// `neo_tabs_closed_total` and `neo_tab_lifetime_seconds`.
    pub(crate) fn tab_closed(&self, _origin: TabOrigin, _reason: CloseReason, _lifetime: Duration) {
    }

    /// `neo_renderer_crashes_total`.
    pub(crate) fn renderer_crashed(&self) {}

    /// The source `neo_tabs_open`'s callback reads.
    pub(crate) fn set_open_tabs(&self, source: OpenTabsSource) {
        drop(source);
    }

    /// Prometheus text from the manual reader (`prometheus.rs`).
    pub(crate) fn render_prometheus(&self) -> String {
        String::new()
    }

    /// Flushes and shuts both providers down within `budget` (see the module doc).
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "the flush awaits its helper thread once it exists"
    )]
    pub(crate) async fn shutdown(&self, _budget: Duration) -> Result<(), TelemetryError> {
        Ok(())
    }
}

impl PipelineRun {
    /// The state this run's ops share.
    #[expect(clippy::unused_self, reason = "a stub until spans land")]
    pub(crate) fn ops(&self) -> PipelineParent {
        PipelineParent {}
    }
}
