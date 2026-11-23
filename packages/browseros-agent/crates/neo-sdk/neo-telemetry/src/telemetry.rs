//! The telemetry handle: the one type the server holds and calls.

#[cfg(feature = "otel")]
use std::sync::Arc;
use std::time::Duration;

use crate::backend::Backend;
use crate::label::{CloseReason, ErrorLabel, TabOrigin};
#[cfg(feature = "otel")]
use crate::pipeline::Pipeline;
#[cfg(all(feature = "otel", feature = "test-support"))]
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

#[cfg_attr(
    not(feature = "otel"),
    expect(
        unused_variables,
        clippy::needless_pass_by_value,
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "without `otel` every hook is the no-op's arm alone"
    )
)]
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
    ///
    /// Built without the `otel` feature it is the no-op, and a `config` that pushes over OTLP
    /// fails instead, so a set endpoint never goes unused in silence.
    pub fn new(config: &TelemetryConfig) -> Result<Self, TelemetryError> {
        #[cfg(feature = "otel")]
        let backend = Backend::Otel(Arc::new(Pipeline::start(config)?));
        #[cfg(not(feature = "otel"))]
        let backend = if config.otlp.traces.is_some() || config.otlp.metrics.is_some() {
            return Err(TelemetryError::Pipeline {
                stage: "start",
                reason: "an OTLP endpoint is set, but this build has no OTLP push (built without \
                         the `otel` feature)"
                    .to_owned(),
            });
        } else {
            Backend::Noop
        };
        Ok(Self { backend })
    }

    /// The live pipeline with its spans kept in memory instead of pushed, and the [`Probe`] that
    /// reads them back; its metrics read through [`Telemetry::render_prometheus`], as the host API
    /// reads them. For tests that assert on real output: the labels, counting rules and span
    /// attributes are the production ones, not a double's copy. `config.otlp` gives the resource
    /// and the sampler; nothing is pushed.
    #[cfg(all(feature = "otel", feature = "test-support"))]
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
    /// the door that runs it, so before any op or run can exist. The `neo.op.name` and `error.type`
    /// values intern against it; until it is bound, and for anything outside it, they read
    /// `unknown`. A call of its own, not a constructor argument, because the tab registry and
    /// the host API need the handle before the registry exists. The first call wins.
    pub fn bind_vocabulary(&self, vocabulary: Vocabulary) {
        match &self.backend {
            Backend::Noop => {}
            #[cfg(feature = "otel")]
            Backend::Otel(pipeline) => pipeline.bind_vocabulary(vocabulary),
        }
    }

    /// Starts a counted run (an agent's `run` call): `neo_run_active` goes up now and comes
    /// down exactly once, when the returned span is finished or dropped.
    pub fn start_run(&self, facts: RunFacts<'_>) -> RunSpan {
        match &self.backend {
            Backend::Noop => RunSpan::noop(),
            #[cfg(feature = "otel")]
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
    /// arguments): counted in `neo_runs_total{neo_run_status="error"}` and `neo_errors_total`, as
    /// `/v1/runs` records it; no span, no in-flight, no duration.
    pub fn run_refused(&self, error: ErrorLabel) {
        match &self.backend {
            Backend::Noop => {}
            #[cfg(feature = "otel")]
            Backend::Otel(pipeline) => pipeline.run_refused(error),
        }
    }

    /// A tab opened (`neo_tab_opened_total`).
    pub fn tab_opened(&self, origin: TabOrigin) {
        match &self.backend {
            Backend::Noop => {}
            #[cfg(feature = "otel")]
            Backend::Otel(pipeline) => pipeline.tab_opened(origin),
        }
    }

    /// A tab closed after `lifetime` open (`neo_tab_closed_total`, `neo_tab_duration_seconds`).
    /// Called once per close, with the one reason that applies.
    pub fn tab_closed(&self, origin: TabOrigin, reason: CloseReason, lifetime: Duration) {
        match &self.backend {
            Backend::Noop => {}
            #[cfg(feature = "otel")]
            Backend::Otel(pipeline) => pipeline.tab_closed(origin, reason, lifetime),
        }
    }

    /// A renderer crashed (`neo_renderer_crashes_total`), whether or not an op was using it.
    pub fn renderer_crashed(&self) {
        match &self.backend {
            Backend::Noop => {}
            #[cfg(feature = "otel")]
            Backend::Otel(pipeline) => pipeline.renderer_crashed(),
        }
    }

    /// Where `neo_tab_count` reads the live counts at each collection. Set once, at startup; a
    /// later call replaces it. Until it is set the gauge reports nothing.
    pub fn set_open_tabs(&self, source: OpenTabsSource) {
        match &self.backend {
            Backend::Noop => {}
            #[cfg(feature = "otel")]
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
            #[cfg(feature = "otel")]
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
            #[cfg(feature = "otel")]
            Backend::Otel(pipeline) => pipeline.shutdown(budget).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fmt;
    use std::sync::Arc;
    use std::time::Instant;

    use super::*;
    use crate::label::{
        CaptchaFamily, CaptchaProvider, CaptchaReason, CaptchaVerdict, Delivery, ErrorCode,
        ErrorOwner, HttpStatus, NetError, OpOutcome, RunStatus,
    };
    use crate::{
        CaptchaSolve, CaptchaSpend, Navigated, OpCounting, OpSettled, OpSite, OpenTabCounts,
        RunVerdict, TraceContext,
    };

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

    const NAVIGATED: Navigated = Navigated {
        status: HttpStatus::new(404),
        error: Some(NetError::Other),
    };

    const SOLVED: CaptchaSolve = CaptchaSolve {
        family: CaptchaFamily::Turnstile,
        verdict: CaptchaVerdict::Solved,
        reason: CaptchaReason::TokenAccepted,
        spend: Some(CaptchaSpend {
            provider: CaptchaProvider::TwoCaptcha,
            usd: 0.002,
        }),
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
        run.ops().navigated(NAVIGATED);
        run.ops().captcha_solved(SOLVED);
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
        Ok(())
    }

    #[cfg(feature = "otel")]
    #[test]
    fn the_real_constructor_always_builds_the_live_pipeline() {
        assert!(
            Telemetry::new(&TelemetryConfig::default())
                .is_ok_and(|built| matches!(built.backend, Backend::Otel(_))),
            "the scrape is always on"
        );
    }

    /// The build the Node add-on links: no OpenTelemetry SDK, so the real constructor is the
    /// no-op. Every hook still takes its call, the scrape still answers with a valid (empty)
    /// exposition, a bad variable still fails startup, and a configured push fails it too
    /// instead of going unused.
    #[cfg(not(feature = "otel"))]
    #[tokio::test]
    async fn without_otel_the_real_constructor_is_the_noop_and_refuses_a_push()
    -> Result<(), Box<dyn Error>> {
        let telemetry = Telemetry::new(&TelemetryConfig::from_env(&|_| None)?)?;
        assert!(matches!(telemetry.backend, Backend::Noop), "{telemetry:?}");
        telemetry.bind_vocabulary(vocabulary());
        telemetry.set_open_tabs(Arc::new(OpenTabCounts::default));
        let run = telemetry.start_run(run_facts(&"run-1"));
        assert!(
            run.is_noop() && run.ops().is_noop(),
            "nothing to record into"
        );
        run.ops()
            .op_settled(settled("page.goto", &"run-1.1", Instant::now()));
        run.ops().navigated(NAVIGATED);
        run.ops().captcha_solved(SOLVED);
        run.finish(OK_RUN);
        drop(telemetry.start_run(run_facts(&"run-2")));
        telemetry.tab_opened(TabOrigin::Agent);
        telemetry.tab_closed(TabOrigin::Agent, CloseReason::User, Duration::from_secs(1));
        telemetry.renderer_crashed();
        assert_eq!(
            telemetry.render_prometheus(),
            "",
            "GET /v1/metrics still answers: an empty exposition is valid text format"
        );
        telemetry.shutdown(Duration::from_secs(1)).await?;

        let endpoint = |name: &str| {
            (name == "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
                .then(|| "http://127.0.0.1:4318/v1/traces".to_owned())
        };
        let refused = Telemetry::new(&TelemetryConfig::from_env(&endpoint)?);
        assert!(
            matches!(
                &refused,
                Err(TelemetryError::Pipeline { stage: "start", reason }) if reason.contains("otel")
            ),
            "a push this build cannot make fails startup: {refused:?}"
        );
        let protocol =
            |name: &str| (name == "OTEL_EXPORTER_OTLP_PROTOCOL").then(|| "grpc".to_owned());
        let bad = TelemetryConfig::from_env(&protocol);
        assert!(
            matches!(
                bad,
                Err(TelemetryError::InvalidEnv {
                    name: "OTEL_EXPORTER_OTLP_PROTOCOL",
                    ..
                })
            ),
            "the variables are still read and checked: {bad:?}"
        );
        Ok(())
    }

    /// With no `OTEL_*` variable set, the live pipeline is the scrape alone: no tracer provider
    /// (so no span batch thread), no push reader and no transport (so no `neo-telemetry-io`
    /// thread and no socket), and no span is ever started.
    #[cfg(feature = "otel")]
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
