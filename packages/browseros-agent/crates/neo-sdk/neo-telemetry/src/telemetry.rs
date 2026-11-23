//! The telemetry handle: the one type the server holds and calls.

use std::sync::Arc;
use std::time::Duration;

use crate::backend::Backend;
use crate::label::{CloseReason, ErrorLabel, TabOrigin};
use crate::pipeline::Pipeline;
#[cfg(feature = "test-support")]
use crate::probe::Probe;
use crate::{OpenTabsSource, RunFacts, RunSpan, TelemetryConfig, TelemetryError, Vocabulary};

/// Neo's telemetry, as the server sees it: the run hooks, the tab hooks, the Prometheus scrape
/// and the shutdown flush. Ops report through their run's [`crate::RunTelemetry`] instead, so
/// the door holds no handle. One is built at the top of startup and cloned to every component that
/// reports (the run service, the tab registry, the host API's scrape, the idle-tab janitor, the
/// host's private typing); clones share one pipeline. `Send + Sync`; a clone is a refcount bump.
///
/// [`Telemetry::noop`] records nothing at no cost: the hosted embedding and most tests run on
/// it. A hook never blocks and never fails its caller: a full export queue drops, it does not
/// wait.
#[derive(Clone, Debug)]
pub struct Telemetry {
    backend: Backend,
}

impl Telemetry {
    /// Telemetry off. Zero cost: a hook is one branch, with no allocation, lock, thread or I/O.
    #[must_use]
    pub const fn noop() -> Self {
        Self {
            backend: Backend::Noop,
        }
    }

    /// The live pipeline for `config`: the meter provider and its Prometheus reader (the scrape
    /// is always on), and the OTLP push when `config` turns it on, with a tracer provider when
    /// spans are pushed (its transport runs on a thread of its own, never on the caller's
    /// runtime). Starts no connection.
    pub fn new(config: &TelemetryConfig) -> Result<Self, TelemetryError> {
        Ok(Self {
            backend: Backend::Otel(Arc::new(Pipeline::start(config)?)),
        })
    }

    /// The live pipeline with its spans kept in memory instead of pushed, and the [`Probe`] that
    /// reads them back; its metrics read through [`Telemetry::render_prometheus`], as the host API
    /// reads them. For tests that assert on real output: the labels, counting rules and span
    /// attributes are the production ones, not a double's copy. `config.otlp` gives the resource
    /// and the sampler; nothing is pushed.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn in_memory(config: &TelemetryConfig) -> (Self, Probe) {
        let (pipeline, probe) = Pipeline::in_memory(config);
        (
            Self {
                backend: Backend::Otel(Arc::new(pipeline)),
            },
            probe,
        )
    }

    /// Binds Neo's own op names and error codes, from the server's tables (its method registry
    /// and its `ErrorCode` list), exactly once at startup: after the registry is built and before
    /// the door that runs it, so before any op or run can exist. The `op` and `error_code`
    /// labels intern against it; until it is bound, and for anything outside it, they read
    /// `unknown`. A call of its own, not a constructor argument, because the tab registry and
    /// the host API need the handle before the registry exists. The first call wins.
    pub fn bind_vocabulary(&self, vocabulary: Vocabulary) {
        match &self.backend {
            Backend::Noop => {}
            Backend::Otel(pipeline) => pipeline.bind_vocabulary(vocabulary),
        }
    }

    /// Starts a counted run (an agent's `run` call): `neo_runs_in_flight` goes up now and comes
    /// down exactly once, when the returned span is finished or dropped.
    pub fn start_run(&self, facts: RunFacts<'_>) -> RunSpan {
        match &self.backend {
            Backend::Noop => RunSpan::noop(),
            Backend::Otel(pipeline) => {
                RunSpan::otel(Arc::clone(pipeline), pipeline.run_started(&facts))
            }
        }
    }

    /// Starts one of Neo's own runs (the idle-tab janitor's sweep, the host's private typing):
    /// no run metric and no `neo.run` span, so `neo_runs_total` counts agents' runs only; its
    /// ops still count, and their spans are roots.
    pub fn internal_run(&self) -> RunSpan {
        RunSpan::internal(self.backend.clone())
    }

    /// A call refused before it became a run (browser unavailable, a duplicate `requestId`, bad
    /// arguments): counted in `neo_runs_total{status="error"}` and `neo_errors_total`, as
    /// `/v1/runs` records it; no span, no in-flight, no duration.
    pub fn run_refused(&self, error: ErrorLabel) {
        match &self.backend {
            Backend::Noop => {}
            Backend::Otel(pipeline) => pipeline.run_refused(error),
        }
    }

    /// A tab opened (`neo_tabs_opened_total`).
    pub fn tab_opened(&self, origin: TabOrigin) {
        match &self.backend {
            Backend::Noop => {}
            Backend::Otel(pipeline) => pipeline.tab_opened(origin),
        }
    }

    /// A tab closed after `lifetime` open (`neo_tabs_closed_total`, `neo_tab_lifetime_seconds`).
    /// Called once per close, with the one reason that applies.
    pub fn tab_closed(&self, origin: TabOrigin, reason: CloseReason, lifetime: Duration) {
        match &self.backend {
            Backend::Noop => {}
            Backend::Otel(pipeline) => pipeline.tab_closed(origin, reason, lifetime),
        }
    }

    /// A renderer crashed (`neo_renderer_crashes_total`), whether or not an op was using it.
    pub fn renderer_crashed(&self) {
        match &self.backend {
            Backend::Noop => {}
            Backend::Otel(pipeline) => pipeline.renderer_crashed(),
        }
    }

    /// Where `neo_tabs_open` reads the live counts at each collection. Set once, at startup; a
    /// later call replaces it. Until it is set the gauge reports nothing.
    pub fn set_open_tabs(&self, source: OpenTabsSource) {
        match &self.backend {
            Backend::Noop => {}
            Backend::Otel(pipeline) => pipeline.set_open_tabs(source),
        }
    }

    /// The metrics as Prometheus text exposition 0.0.4, for `GET /v1/metrics`: the same meter
    /// provider OTLP pushes from, read now. The no-op renders an empty exposition, which is
    /// valid text format. Never fails: a collection error is logged and renders what it could.
    #[must_use]
    pub fn render_prometheus(&self) -> String {
        match &self.backend {
            Backend::Noop => String::new(),
            Backend::Otel(pipeline) => pipeline.render_prometheus(),
        }
    }

    /// Flushes pending spans and metrics and stops the exporters, returning within `budget`
    /// whether or not the collector answers. Call once, after in-flight runs drained; hooks
    /// after it record nothing that leaves the process. Never blocks a runtime worker: the stop
    /// runs on a thread of its own and a tokio timer bounds the wait, so await it on a tokio
    /// runtime.
    pub async fn shutdown(&self, budget: Duration) -> Result<(), TelemetryError> {
        match &self.backend {
            Backend::Noop => Ok(()),
            Backend::Otel(pipeline) => pipeline.shutdown(budget).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fmt;
    use std::time::Instant;

    use super::*;
    use crate::label::{Delivery, ErrorCode, ErrorOwner, OpOutcome, RunStatus};
    use crate::{OpCounting, OpSettled, OpSite, OpenTabCounts, RunVerdict, TraceContext};

    const PARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    fn vocabulary() -> Vocabulary {
        Vocabulary::new(["page.goto", "page.close"], ["script_error", "no_match"])
    }

    fn run_facts(run_id: &dyn fmt::Display) -> RunFacts<'_> {
        RunFacts {
            run_id,
            task_id: None,
            parent: None,
        }
    }

    const OK_RUN: RunVerdict = RunVerdict {
        status: RunStatus::Ok,
        error: None,
    };

    fn settled<'a>(
        op: &'static str,
        op_id: &'a dyn fmt::Display,
        started: Instant,
    ) -> OpSettled<'a> {
        OpSettled {
            op,
            raw_name: None,
            op_id,
            site: None,
            tab_id: None,
            started,
            outcome: OpOutcome::Ok,
            delivery: Delivery::Done,
            counting: OpCounting::Counted,
            error: None,
            url: None,
        }
    }

    #[tokio::test]
    async fn the_noop_takes_every_call_and_does_nothing() -> Result<(), Box<dyn Error>> {
        let telemetry = Telemetry::noop();
        telemetry.bind_vocabulary(vocabulary());
        let parent = TraceContext::from_w3c(PARENT, None);
        let run = telemetry.clone().start_run(RunFacts {
            run_id: &"run-1",
            task_id: Some("task"),
            parent: parent.as_ref(),
        });
        run.ops().op_settled(OpSettled {
            op: "unknown",
            raw_name: Some("no.such.op"),
            op_id: &"run-1.1",
            site: Some(OpSite { line: 1, column: 2 }),
            tab_id: Some(7),
            started: Instant::now(),
            outcome: OpOutcome::Error,
            delivery: Delivery::NotSent,
            counting: OpCounting::Counted,
            error: Some(ErrorLabel {
                owner: ErrorOwner::Script,
                code: ErrorCode::new("method_unavailable"),
            }),
            url: Some("https://user:pw@example.com/?q=secret"),
        });
        run.finish(OK_RUN);
        drop(telemetry.start_run(run_facts(&"run-2")));
        let internal = telemetry.internal_run();
        internal
            .ops()
            .op_settled(settled("page.close", &"run-3.1", Instant::now()));
        internal.finish(OK_RUN);
        telemetry.run_refused(ErrorLabel {
            owner: ErrorOwner::Neo,
            code: ErrorCode::new("browser_unavailable"),
        });
        telemetry.tab_opened(TabOrigin::Agent);
        telemetry.tab_closed(TabOrigin::Found, CloseReason::User, Duration::from_secs(3));
        telemetry.renderer_crashed();
        telemetry.set_open_tabs(Arc::new(OpenTabCounts::default));
        assert_eq!(
            telemetry.render_prometheus(),
            "",
            "the no-op renders an empty exposition, which is valid text format"
        );
        telemetry.shutdown(Duration::ZERO).await?;
        assert!(
            Telemetry::new(&TelemetryConfig::default())
                .is_ok_and(|built| matches!(built.backend, Backend::Otel(_))),
            "the real constructor always builds the live pipeline: the scrape is always on"
        );
        Ok(())
    }

    /// With no `OTEL_*` variable set, the live pipeline is the scrape alone: no tracer provider
    /// (so no span batch thread), no push reader and no transport (so no `neo-telemetry-io`
    /// thread and no socket), and no span is ever started.
    #[test]
    fn with_nothing_configured_nothing_pushes_and_no_thread_starts() -> Result<(), Box<dyn Error>> {
        let telemetry = Telemetry::new(&TelemetryConfig::from_env(&|_| None)?)?;
        let Backend::Otel(pipeline) = &telemetry.backend else {
            return Err("the real constructor builds the live pipeline".into());
        };
        let shown = format!("{pipeline:?}");
        for part in ["spans: None", "tracers: None", "transport: None"] {
            assert!(shown.contains(part), "{part}: {shown}");
        }
        Ok(())
    }

    #[test]
    fn the_noop_run_span_holds_nothing() {
        let run = Telemetry::noop().start_run(run_facts(&"run-1"));
        assert!(
            run.is_noop(),
            "a no-op run span holds no state, so it allocated none"
        );
        assert!(run.ops().is_noop(), "its ops record nowhere");
    }
}
