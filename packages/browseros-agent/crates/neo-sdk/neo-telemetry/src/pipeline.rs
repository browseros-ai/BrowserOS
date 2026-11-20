//! The live OpenTelemetry pipeline behind a real [`crate::Telemetry`]: the providers, what each
//! run, op and tab event records, the scrape and the bounded shutdown.
//!
//! ```text
//!  Telemetry::new ─▶ Pipeline::start ─┬─ meter provider ─┬─ manual reader ─▶ prometheus.rs (scrape, always)
//!                                     │                  └─ Push::metrics (periodic OTLP, opt-in)
//!                                     ├─ tracer provider ── Push::spans (batch OTLP, opt-in), or the
//!                                     │                     in-memory capture (test-support); else no spans
//!                                     └─ Push::transport: the exporters' own thread and client
//!                                        resource: otlp::resource, sampler: otlp::sampler
//! ```
//!
//! `instruments.rs` holds the 11 instruments, `spans.rs` the two spans, `privacy.rs` the
//! site-detail dial, `prometheus.rs` the text renderer. The facade (`telemetry.rs`, `run_span.rs`,
//! `run_telemetry.rs`) owns each run's exactly-once end; this file decides what an end records. A
//! verdict of `None` is a run dropped unfinished: it records as `error` / `neo` /
//! `internal_error`, and in-flight comes down all the same.
//!
//! `neo_errors_total` counts each failure once, where it is attributed (wire-contract.md): every
//! counted run keeps the codes its ops reported ([`RunErrors`]), shared with its ops through
//! [`PipelineParent`]. That state may outlive the run, because ops aborted at the run's teardown
//! settle after it ended.
//!
//! Shutdown stops both providers on a thread of its own and awaits it for at most the budget: the
//! periodic reader's own shutdown ignores its timeout (5 s), and a `spawn_blocking` task would hold
//! the server's runtime past exit. A late thread is abandoned, never joined.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use opentelemetry::InstrumentationScope;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry::trace::{Span as _, SpanContext, TracerProvider as _};
use opentelemetry_otlp::MetricExporter;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::reader::MetricReader;
use opentelemetry_sdk::metrics::{
    InstrumentKind, ManualReader, PeriodicReader, Pipeline as ReaderPipeline, SdkMeterProvider,
    Temporality,
};
use opentelemetry_sdk::trace::{SdkTracerProvider, Span, SpanProcessor};
use tokio::sync::oneshot;
use tokio::time;

use crate::instruments::{CodeLabel, Instruments, OpLabel, OpenTabs};
use crate::label::{CloseReason, ErrorCode, ErrorLabel, ErrorOwner, RunStatus, TabOrigin};
use crate::otlp::{self, Push};
#[cfg(feature = "test-support")]
use crate::probe::Probe;
use crate::prometheus::Prometheus;
use crate::spans::Spans;
use crate::transport::Transport;
use crate::{
    OpSettled, OpenTabsSource, RunFacts, RunVerdict, TelemetryConfig, TelemetryError, Vocabulary,
};

/// What a run dropped unfinished records: Neo's own failure, as `run_span.rs` promises.
const DROPPED: RunVerdict = RunVerdict {
    status: RunStatus::Error,
    error: Some(ErrorLabel {
        owner: ErrorOwner::Neo,
        code: ErrorCode::new("internal_error"),
    }),
};

/// The providers, instruments, readers and push door one process records into.
#[derive(Debug)]
pub(crate) struct Pipeline {
    instruments: Instruments,
    /// The server's op names and error codes, bound once. Until then every `op` and
    /// `error_code` label is `unknown`.
    vocabulary: OnceLock<Vocabulary>,
    /// `None` when no span leaves the process (trace push off): then no span is ever started.
    spans: Option<Spans>,
    /// Where `neo_tabs_open` reads the live counts.
    open_tabs: Arc<OpenTabs>,
    /// The scrape's handle on the meter provider's manual reader.
    scrape: ScrapeReader,
    /// What shutdown stops. Taken by the first call, so a second one has nothing left to do.
    running: Mutex<Option<Providers>>,
}

/// One counted run while it is open: when it started, its `neo.run` span, and the state its ops
/// share. Held inside the facade's `RunSpan`, which ends it exactly once.
#[derive(Debug)]
pub(crate) struct PipelineRun {
    started: Instant,
    /// `None` when spans are off. Boxed: an SDK span is a few hundred bytes, and the facade's
    /// `RunSpan` moves this around for the run's whole life.
    span: Option<Box<Span>>,
    shared: Arc<RunShared>,
}

/// A counted run's state its ops share ([`crate::RunTelemetry`]): its span context and its
/// error memo. It may outlive the run: ops aborted at the run's teardown settle after it ended.
#[derive(Clone, Debug)]
pub(crate) struct PipelineParent {
    shared: Arc<RunShared>,
}

#[derive(Debug)]
struct RunShared {
    /// The run's `neo.run` span context: its ops' parent. `None` when spans are off.
    span_context: Option<SpanContext>,
    errors: Mutex<RunErrors>,
}

/// One run's side of the "count each failure once" rule (wire-contract.md): which error codes
/// the run's own ops reported. Ops write it from any task; the run reads it once, at its end, and
/// seals it.
#[derive(Debug)]
enum RunErrors {
    /// The run is open: the distinct codes its ops failed with so far.
    Open { reported: Vec<ErrorCode> },
    /// The run ended, with this failure or none.
    Ended { error: Option<ErrorCode> },
}

impl RunErrors {
    /// An op of the run failed with `code`: whether `neo_errors_total` counts it. While the run
    /// is open, every op failure counts and its code is remembered. After the run ended (an op
    /// aborted at its teardown settles late) it counts unless it is the very failure the run
    /// ended with, which the run's end already accounted for.
    fn op_failed(&mut self, code: ErrorCode) -> bool {
        match self {
            Self::Open { reported } => {
                if !reported.contains(&code) {
                    reported.push(code);
                }
                true
            }
            Self::Ended { error } => *error != Some(code),
        }
    }

    /// The run ended with `error`: whether `neo_errors_total` counts the run's own failure. It
    /// does unless one of its ops already reported that code (a run that fails because an op
    /// failed is not counted twice). Seals the memo.
    fn run_ended(&mut self, error: Option<ErrorCode>) -> bool {
        let counted = match self {
            Self::Open { reported } => error.is_some_and(|code| !reported.contains(&code)),
            // `RunSpan` ends a run once; a second end would be a facade bug, and counts nothing.
            Self::Ended { .. } => false,
        };
        *self = Self::Ended { error };
        counted
    }
}

impl RunShared {
    fn lock_errors(&self) -> MutexGuard<'_, RunErrors> {
        // A poisoned lock still holds a whole value: nothing here panics mid-update.
        self.errors.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The scrape's reader: the meter provider owns one handle, [`Pipeline::render_prometheus`]
/// collects through the other.
#[derive(Clone, Debug)]
pub(crate) struct ScrapeReader(Arc<ManualReader>);

impl ScrapeReader {
    /// A manual reader with cumulative temporality, as Prometheus reads totals.
    pub(crate) fn new() -> Self {
        Self(Arc::new(
            ManualReader::builder()
                .with_temporality(Temporality::Cumulative)
                .build(),
        ))
    }
}

impl MetricReader for ScrapeReader {
    fn register_pipeline(&self, pipeline: Weak<ReaderPipeline>) {
        self.0.register_pipeline(pipeline);
    }

    fn collect(&self, metrics: &mut ResourceMetrics) -> OTelSdkResult {
        self.0.collect(metrics)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.0.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }

    fn temporality(&self, kind: InstrumentKind) -> Temporality {
        self.0.temporality(kind)
    }
}

/// What shutdown stops, in its order: both providers, then the transport they export through.
#[derive(Debug)]
struct Providers {
    meters: SdkMeterProvider,
    tracers: Option<SdkTracerProvider>,
    transport: Option<Transport>,
}

impl Providers {
    /// Stops both providers, the span side on a second thread so a collector that holds the last
    /// span batch does not eat the metrics' share of `budget`, or the other way round. Each
    /// exports what it still holds.
    fn shut_down(self, budget: Duration) -> Result<(), TelemetryError> {
        // `transport` is dropped when this returns, after both shutdowns: the exporters send
        // their last batches through it.
        let Self {
            meters,
            tracers,
            transport: _transport,
        } = self;
        let (spans, metrics) = thread::scope(|scope| {
            let spans = tracers.map(|tracers| {
                thread::Builder::new()
                    .name("neo-telemetry-spans-stop".to_owned())
                    .spawn_scoped(scope, move || tracers.shutdown_with_timeout(budget))
            });
            let metrics = meters.shutdown_with_timeout(budget);
            let spans = match spans {
                None => Ok(()),
                Some(Ok(stopping)) => stopping.join().unwrap_or_else(|_| {
                    Err(OTelSdkError::InternalFailure(
                        "the span shutdown panicked".to_owned(),
                    ))
                }),
                Some(Err(error)) => Err(OTelSdkError::InternalFailure(format!(
                    "no thread for the span shutdown: {error}"
                ))),
            };
            (spans, metrics)
        });
        let failures: Vec<String> = [("spans", spans), ("metrics", metrics)]
            .into_iter()
            .filter_map(|(signal, outcome)| outcome.err().map(|error| format!("{signal}: {error}")))
            .collect();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(stopped(failures.join("; ")))
        }
    }
}

impl Pipeline {
    /// Builds the pipeline for `config`: always the meter provider and its scrape; the push door
    /// (`otlp::push`) adds a periodic metric reader and, with a traces endpoint, the tracer
    /// provider. Starts no connection and needs no runtime.
    pub(crate) fn start(config: &TelemetryConfig) -> Result<Self, TelemetryError> {
        let (spans, metrics, transport) = match otlp::push(&config.otlp)? {
            Some(Push {
                spans,
                metrics,
                transport,
            }) => (spans, metrics, Some(transport)),
            None => (None, None, None),
        };
        let tracers = spans.map(|processor| tracer_provider(config, processor));
        Ok(Self::build(config, tracers, metrics, transport))
    }

    /// The same pipeline with its spans kept in memory instead of pushed, and the probe that
    /// reads them back. `config.otlp` still gives the resource and the sampler.
    #[cfg(feature = "test-support")]
    pub(crate) fn in_memory(config: &TelemetryConfig) -> (Self, Probe) {
        let (capture, probe) = Probe::new();
        let tracers = tracer_provider(config, capture);
        let push: Option<PeriodicReader<MetricExporter>> = None;
        (Self::build(config, Some(tracers), push, None), probe)
    }

    /// The pipeline over its parts: the scrape always, `push` beside it when metrics are pushed,
    /// spans when a tracer provider is given.
    fn build<R: MetricReader>(
        config: &TelemetryConfig,
        tracers: Option<SdkTracerProvider>,
        push: Option<R>,
        transport: Option<Transport>,
    ) -> Self {
        let scrape = ScrapeReader::new();
        let mut meters = SdkMeterProvider::builder()
            .with_resource(otlp::resource(&config.otlp))
            .with_reader(scrape.clone());
        if let Some(push) = push {
            meters = meters.with_reader(push);
        }
        let meters = meters.build();
        let open_tabs = Arc::new(OpenTabs::default());
        let instruments =
            Instruments::new(&meters.meter_with_scope(scope()), Arc::clone(&open_tabs));
        let spans = tracers
            .as_ref()
            .map(|tracers| Spans::new(tracers.tracer_with_scope(scope()), config.site_detail));
        Self {
            instruments,
            vocabulary: OnceLock::new(),
            spans,
            open_tabs,
            scrape,
            running: Mutex::new(Some(Providers {
                meters,
                tracers,
                transport,
            })),
        }
    }

    /// The server's tables, bound once before the first run: labels intern against them, and
    /// until then every op and error-code label is `unknown`. The first call wins.
    pub(crate) fn bind_vocabulary(&self, vocabulary: Vocabulary) {
        if self.vocabulary.set(vocabulary).is_err() {
            tracing::warn!("telemetry vocabulary bound twice; the first one stays");
        }
    }

    /// `neo_runs_in_flight` +1 and the `neo.run` span starts.
    pub(crate) fn run_started(&self, facts: &RunFacts<'_>) -> PipelineRun {
        self.instruments.run_started();
        let span = self
            .spans
            .as_ref()
            .map(|spans| Box::new(spans.run_started(facts)));
        let span_context = span.as_ref().map(|span| span.span_context().clone());
        PipelineRun {
            started: Instant::now(),
            span,
            shared: Arc::new(RunShared {
                span_context,
                errors: Mutex::new(RunErrors::Open {
                    reported: Vec::new(),
                }),
            }),
        }
    }

    /// `neo_runs_in_flight` -1, `neo_runs_total`, `neo_run_duration_seconds`,
    /// `neo_errors_total` for a failure none of its ops reported, and the span ends.
    pub(crate) fn run_ended(&self, run: PipelineRun, verdict: Option<RunVerdict>) {
        let PipelineRun {
            started,
            span,
            shared,
        } = run;
        let verdict = verdict.unwrap_or(DROPPED);
        self.instruments
            .run_ended(verdict.status, verdict.error, started.elapsed());
        let counted = shared
            .lock_errors()
            .run_ended(verdict.error.map(|error| error.code));
        if counted && let Some(error) = verdict.error {
            self.instruments
                .error_counted(error.owner, self.code_label(error.code));
        }
        if let Some(span) = span {
            Spans::run_ended(*span, &verdict);
        }
    }

    /// A call refused before it became a run: `neo_runs_total{status="error"}` and
    /// `neo_errors_total`; no span, no in-flight, no duration.
    pub(crate) fn run_refused(&self, error: ErrorLabel) {
        self.instruments.run_refused(error);
        self.instruments
            .error_counted(error.owner, self.code_label(error.code));
    }

    /// `neo_operations_total`, `neo_operation_duration_seconds`, `neo_errors_total` on a
    /// failure (subject to the run's once rule), and the `neo.op` span, built now from
    /// `op.started`: a child of `parent`'s span, or a root for an op of an internal run (`None`).
    pub(crate) fn op_settled(&self, parent: Option<&PipelineParent>, op: &OpSettled<'_>) {
        let took = op.started.elapsed();
        self.instruments.op_settled(
            OpLabel::of(self.vocabulary.get(), op.op),
            op.outcome,
            op.error,
            took,
        );
        if let Some(error) = op.error {
            // An internal run has no end that could count the failure again: it always counts.
            let counted =
                parent.is_none_or(|parent| parent.shared.lock_errors().op_failed(error.code));
            if counted {
                self.instruments
                    .error_counted(error.owner, self.code_label(error.code));
            }
        }
        if let Some(spans) = &self.spans {
            let run = parent.and_then(|parent| parent.shared.span_context.as_ref());
            spans.op_settled(run, op, took);
        }
    }

    /// `neo_tabs_opened_total`.
    pub(crate) fn tab_opened(&self, origin: TabOrigin) {
        self.instruments.tab_opened(origin);
    }

    /// `neo_tabs_closed_total` and `neo_tab_lifetime_seconds`.
    pub(crate) fn tab_closed(&self, origin: TabOrigin, reason: CloseReason, lifetime: Duration) {
        self.instruments.tab_closed(origin, reason, lifetime);
    }

    /// `neo_renderer_crashes_total`.
    pub(crate) fn renderer_crashed(&self) {
        self.instruments.renderer_crashed();
    }

    /// The source `neo_tabs_open`'s callback reads at each collection.
    pub(crate) fn set_open_tabs(&self, source: OpenTabsSource) {
        self.open_tabs.set(source);
    }

    /// Prometheus text from one collection of the manual reader. A collection error (after
    /// shutdown, say) is logged, and what was collected is still rendered.
    pub(crate) fn render_prometheus(&self) -> String {
        let mut collected = ResourceMetrics::default();
        if let Err(error) = self.scrape.collect(&mut collected) {
            tracing::warn!(%error, "telemetry scrape could not collect the metrics");
        }
        Prometheus::render(&collected)
    }

    /// Flushes and stops both providers within `budget` (see the module doc). Returns by then
    /// whatever an exporter does: a late stop is reported as an error and left to finish in the
    /// background. A second call has nothing left to stop.
    pub(crate) async fn shutdown(&self, budget: Duration) -> Result<(), TelemetryError> {
        let Some(providers) = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return Ok(());
        };
        let (answer, answered) = oneshot::channel();
        thread::Builder::new()
            .name("neo-telemetry-stop".to_owned())
            .spawn(move || {
                // Nobody listens once the caller's budget ran out; the stop goes on regardless.
                let _ = answer.send(providers.shut_down(budget));
            })
            .map_err(|error| stopped(format!("no thread to stop the exporters on: {error}")))?;
        match time::timeout(budget, answered).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => Err(stopped(
                "the exporters' stop ended without an answer".to_owned(),
            )),
            Err(_) => Err(stopped(format!(
                "the exporters did not stop within {budget:?}; they finish in the background"
            ))),
        }
    }

    fn code_label(&self, code: ErrorCode) -> CodeLabel {
        CodeLabel::of(self.vocabulary.get(), code)
    }
}

impl PipelineRun {
    /// The state this run's ops share.
    pub(crate) fn ops(&self) -> PipelineParent {
        PipelineParent {
            shared: Arc::clone(&self.shared),
        }
    }
}

/// The tracer provider over `processor`: the resource and the sampler from `config.otlp`, both
/// set here so the SDK's own environment reads never decide them.
fn tracer_provider<P: SpanProcessor + 'static>(
    config: &TelemetryConfig,
    processor: P,
) -> SdkTracerProvider {
    SdkTracerProvider::builder()
        .with_resource(otlp::resource(&config.otlp))
        .with_sampler(otlp::sampler(&config.otlp))
        .with_span_processor(processor)
        .build()
}

/// The instrumentation scope both providers report: this crate, at its version.
fn scope() -> InstrumentationScope {
    InstrumentationScope::builder("neo-telemetry")
        .with_version(env!("CARGO_PKG_VERSION"))
        .build()
}

fn stopped(reason: String) -> TelemetryError {
    TelemetryError::Pipeline {
        stage: "shutdown",
        reason,
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::future;
    use std::sync::mpsc;

    use opentelemetry::Context;
    use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
    use opentelemetry_sdk::trace::SpanData;

    use super::*;

    const NO_MATCH: ErrorCode = ErrorCode::new("no_match");
    const TIMEOUT: ErrorCode = ErrorCode::new("run_timeout");

    fn open() -> RunErrors {
        RunErrors::Open {
            reported: Vec::new(),
        }
    }

    #[test]
    fn every_op_failure_counts_while_its_run_is_open() {
        let mut errors = open();
        assert!(errors.op_failed(NO_MATCH), "the first failure counts");
        assert!(
            errors.op_failed(NO_MATCH),
            "a second op failing the same way is a second failure"
        );
        assert!(errors.op_failed(TIMEOUT), "and so is a different one");
    }

    #[test]
    fn a_run_failing_with_a_code_its_ops_reported_is_not_counted_again() {
        let mut errors = open();
        assert!(errors.op_failed(NO_MATCH), "the op's failure counts");
        assert!(
            !errors.run_ended(Some(NO_MATCH)),
            "the run failed because its op did: the same failure, already counted"
        );
    }

    #[test]
    fn a_run_failing_on_its_own_counts_once() {
        let mut errors = open();
        assert!(errors.op_failed(NO_MATCH), "the op's failure counts");
        assert!(
            errors.run_ended(Some(TIMEOUT)),
            "a timeout none of its ops reported is the run's own failure"
        );
        assert!(
            !open().run_ended(None),
            "a run that did not fail counts no error"
        );
    }

    #[test]
    fn a_late_op_failure_counts_unless_it_is_the_runs_own() {
        let mut errors = open();
        assert!(errors.run_ended(Some(TIMEOUT)), "the run's timeout counts");
        assert!(
            !errors.op_failed(TIMEOUT),
            "an op aborted by that timeout settles late with the same failure: not again"
        );
        assert!(
            errors.op_failed(NO_MATCH),
            "a late op failing differently is a failure of its own"
        );
        let mut ok = open();
        assert!(!ok.run_ended(None), "an ok run counts nothing");
        assert!(
            ok.op_failed(NO_MATCH),
            "an op that fails after an ok run ended still counts"
        );
    }

    /// A span exporter whose shutdown never finishes on its own, whatever its timeout says: a
    /// collector that holds the last batch. Released when the test drops its sender.
    #[derive(Debug)]
    struct StuckSpans(Mutex<mpsc::Receiver<()>>);

    impl SpanProcessor for StuckSpans {
        fn on_start(&self, _span: &mut Span, _cx: &Context) {}

        fn on_end(&self, _span: SpanData) {}

        fn force_flush(&self) -> OTelSdkResult {
            Ok(())
        }

        fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
            let _ = self.0.lock().unwrap_or_else(PoisonError::into_inner).recv();
            Ok(())
        }
    }

    /// A metric exporter whose export never returns: the periodic reader's final export at
    /// shutdown hangs, and the reader then waits its own hard-coded 5 s, ignoring its timeout.
    #[derive(Debug)]
    struct HungCollector;

    impl PushMetricExporter for HungCollector {
        async fn export(&self, _metrics: &ResourceMetrics) -> OTelSdkResult {
            future::pending().await
        }

        fn force_flush(&self) -> OTelSdkResult {
            Ok(())
        }

        fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
            Ok(())
        }

        fn temporality(&self) -> Temporality {
            Temporality::Cumulative
        }
    }

    #[tokio::test]
    async fn shutdown_returns_within_its_budget_when_both_exporters_hang()
    -> Result<(), Box<dyn Error>> {
        let (release, stuck) = mpsc::channel::<()>();
        let config = TelemetryConfig::default();
        let pipeline = Pipeline::build(
            &config,
            Some(tracer_provider(&config, StuckSpans(Mutex::new(stuck)))),
            Some(
                PeriodicReader::builder(HungCollector)
                    .with_interval(Duration::from_secs(3600))
                    .build(),
            ),
            None,
        );
        let budget = Duration::from_millis(300);
        let started = Instant::now();
        let outcome = pipeline.shutdown(budget).await;
        let took = started.elapsed();
        assert!(
            matches!(
                outcome,
                Err(TelemetryError::Pipeline {
                    stage: "shutdown",
                    ..
                })
            ),
            "a stop that outlives its budget is reported, not waited for: {outcome:?}"
        );
        assert!(
            took < budget + Duration::from_millis(500),
            "shutdown took {took:?}: it must return by its {budget:?} budget, not the reader's 5 s"
        );
        let again = Instant::now();
        pipeline.shutdown(budget).await?;
        assert!(
            again.elapsed() < Duration::from_millis(100),
            "a second shutdown has nothing left to stop"
        );
        drop(release);
        Ok(())
    }

    #[tokio::test]
    async fn shutdown_of_a_quiet_pipeline_is_prompt_and_clean() -> Result<(), Box<dyn Error>> {
        let pipeline = Pipeline::start(&TelemetryConfig::default())?;
        let started = Instant::now();
        pipeline.shutdown(Duration::from_secs(1)).await?;
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "with nothing to push, stopping is immediate"
        );
        Ok(())
    }
}
