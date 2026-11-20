//! The telemetry handle: the one type the server holds and calls.

use std::sync::Arc;
use std::time::Duration;

use crate::backend::Backend;
use crate::label::{CloseReason, ErrorLabel, TabOrigin};
use crate::pipeline::Pipeline;
#[cfg(feature = "test-support")]
use crate::recording::{Recorder, Recording, TelemetryCall};
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

    /// The live pipeline for `config`: the meter and tracer providers, the Prometheus reader,
    /// and the OTLP push when `config` turns it on (its transport runs on a thread of its own,
    /// never on the caller's runtime). Starts no connection.
    pub fn new(config: &TelemetryConfig) -> Result<Self, TelemetryError> {
        Ok(match Pipeline::start(config)? {
            Some(pipeline) => Self {
                backend: Backend::Otel(Arc::new(pipeline)),
            },
            None => Self::noop(),
        })
    }

    /// The test double: a handle that keeps every call, and the [`Recording`] that reads them
    /// back.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn recording() -> (Self, Recording) {
        let recorder = Arc::new(Recorder::new());
        (
            Self {
                backend: Backend::Recording(Arc::clone(&recorder)),
            },
            Recording::new(recorder),
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
            #[cfg(feature = "test-support")]
            Backend::Recording(recorder) => recorder.bind_vocabulary(vocabulary),
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
            #[cfg(feature = "test-support")]
            Backend::Recording(recorder) => {
                let run_id: Arc<str> = Arc::from(facts.run_id.to_string());
                recorder.record(TelemetryCall::RunStarted {
                    run_id: run_id.to_string(),
                    task_id: facts.task_id.map(str::to_owned),
                    parent: facts.parent.cloned(),
                });
                RunSpan::recorded(Arc::clone(recorder), run_id)
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
            #[cfg(feature = "test-support")]
            Backend::Recording(recorder) => recorder.record(TelemetryCall::RunRefused { error }),
        }
    }

    /// A tab opened (`neo_tabs_opened_total`).
    pub fn tab_opened(&self, origin: TabOrigin) {
        match &self.backend {
            Backend::Noop => {}
            Backend::Otel(pipeline) => pipeline.tab_opened(origin),
            #[cfg(feature = "test-support")]
            Backend::Recording(recorder) => recorder.record(TelemetryCall::TabOpened { origin }),
        }
    }

    /// A tab closed after `lifetime` open (`neo_tabs_closed_total`, `neo_tab_lifetime_seconds`).
    /// Called once per close, with the one reason that applies.
    pub fn tab_closed(&self, origin: TabOrigin, reason: CloseReason, lifetime: Duration) {
        match &self.backend {
            Backend::Noop => {}
            Backend::Otel(pipeline) => pipeline.tab_closed(origin, reason, lifetime),
            #[cfg(feature = "test-support")]
            Backend::Recording(recorder) => recorder.record(TelemetryCall::TabClosed {
                origin,
                reason,
                lifetime,
            }),
        }
    }

    /// A renderer crashed (`neo_renderer_crashes_total`), whether or not an op was using it.
    pub fn renderer_crashed(&self) {
        match &self.backend {
            Backend::Noop => {}
            Backend::Otel(pipeline) => pipeline.renderer_crashed(),
            #[cfg(feature = "test-support")]
            Backend::Recording(recorder) => recorder.record(TelemetryCall::RendererCrashed),
        }
    }

    /// Where `neo_tabs_open` reads the live counts at each collection. Set once, at startup; a
    /// later call replaces it. Until it is set the gauge reports nothing.
    pub fn set_open_tabs(&self, source: OpenTabsSource) {
        match &self.backend {
            Backend::Noop => {}
            Backend::Otel(pipeline) => pipeline.set_open_tabs(source),
            #[cfg(feature = "test-support")]
            Backend::Recording(recorder) => recorder.set_open_tabs(source),
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
            #[cfg(feature = "test-support")]
            Backend::Recording(recorder) => {
                recorder.record(TelemetryCall::Scraped);
                String::new()
            }
        }
    }

    /// Flushes pending spans and metrics and stops the exporters, returning within `budget`
    /// whether or not the collector answers. Call once, after in-flight runs drained; hooks
    /// after it record nothing that leaves the process. Never blocks a runtime worker.
    pub async fn shutdown(&self, budget: Duration) -> Result<(), TelemetryError> {
        match &self.backend {
            Backend::Noop => Ok(()),
            Backend::Otel(pipeline) => pipeline.shutdown(budget).await,
            #[cfg(feature = "test-support")]
            Backend::Recording(recorder) => {
                recorder.record(TelemetryCall::ShutDown { budget });
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Instant;
    use std::{fmt, thread};

    use super::*;
    use crate::label::{Delivery, ErrorCode, ErrorOwner, OpOutcome, RunStatus};
    use crate::{OpSettled, OpSite, OpenTabCounts, RunVerdict, TraceContext};

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
                .is_ok_and(|built| matches!(built.backend, Backend::Noop)),
            "until the pipeline is built, the real constructor hands back the no-op"
        );
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

    #[test]
    fn a_dropped_run_span_releases_in_flight_once() {
        let (telemetry, recording) = Telemetry::recording();
        let first = telemetry.start_run(run_facts(&"run-1"));
        let second = telemetry.start_run(run_facts(&"run-2"));
        assert_eq!(recording.runs_in_flight(), 2, "both runs are in flight");
        drop(first);
        assert_eq!(
            recording.runs_in_flight(),
            1,
            "a run whose guard died without a verdict still leaves in-flight"
        );
        second.finish(OK_RUN);
        assert_eq!(
            recording.runs_in_flight(),
            0,
            "a finished run leaves in-flight"
        );
        let ended = recording
            .calls()
            .into_iter()
            .filter(|call| {
                matches!(
                    call,
                    TelemetryCall::RunDropped { .. } | TelemetryCall::RunFinished { .. }
                )
            })
            .count();
        assert_eq!(ended, 2, "each run ends exactly once");
    }

    #[test]
    fn a_run_span_dropped_by_a_panic_releases_in_flight() {
        let (telemetry, recording) = Telemetry::recording();
        let panicked = thread::spawn(move || {
            let _run = telemetry.start_run(run_facts(&"run-1"));
            panic!("the run's task panicked");
        })
        .join();
        assert!(panicked.is_err(), "the thread panicked");
        assert_eq!(
            recording.runs_in_flight(),
            0,
            "unwinding drops the span, which releases in-flight"
        );
        assert_eq!(
            recording.calls().last(),
            Some(&TelemetryCall::RunDropped {
                run_id: "run-1".to_owned()
            }),
            "the run ends as dropped"
        );
    }

    #[test]
    fn an_internal_run_records_no_run_but_its_ops() {
        let (telemetry, recording) = Telemetry::recording();
        let sweep = telemetry.internal_run();
        sweep
            .ops()
            .op_settled(settled("page.close", &"run-1.1", Instant::now()));
        sweep.finish(OK_RUN);
        assert_eq!(
            recording.calls(),
            vec![TelemetryCall::OpSettled {
                run_id: None,
                op: "page.close",
                raw_name: None,
                op_label: Vocabulary::UNKNOWN,
                op_id: "run-1.1".to_owned(),
                site: None,
                tab_id: None,
                outcome: OpOutcome::Ok,
                delivery: Delivery::Done,
                error: None,
                url: None,
            }],
            "Neo's own runs leave neo_runs_* alone and their ops still count; with no vocabulary \
             bound yet the op labels as unknown"
        );
        assert_eq!(
            recording.runs_in_flight(),
            0,
            "an internal run is never in flight"
        );
    }

    #[test]
    fn the_recording_captures_run_and_op_calls_with_their_values() {
        let (telemetry, recording) = Telemetry::recording();
        telemetry.bind_vocabulary(vocabulary());
        let parent = TraceContext::from_w3c(PARENT, Some("k=v"));
        let run = telemetry.start_run(RunFacts {
            run_id: &"run-1",
            task_id: Some("task-a"),
            parent: parent.as_ref(),
        });
        let started = Instant::now();
        let ops = run.ops();
        ops.op_settled(OpSettled {
            site: Some(OpSite { line: 3, column: 9 }),
            tab_id: Some(42),
            url: Some("https://example.com/a?b=c"),
            ..settled("page.goto", &"run-1.1", started)
        });
        let failure = ErrorLabel {
            owner: ErrorOwner::Script,
            code: ErrorCode::new("method_unavailable"),
        };
        ops.op_settled(OpSettled {
            raw_name: Some("made.up"),
            outcome: OpOutcome::Error,
            delivery: Delivery::NotSent,
            error: Some(failure),
            ..settled("unknown", &"run-1.2", started)
        });
        let verdict = RunVerdict {
            status: RunStatus::Error,
            error: Some(ErrorLabel {
                owner: ErrorOwner::Script,
                code: ErrorCode::new("script_error"),
            }),
        };
        run.finish(verdict);
        telemetry.run_refused(failure);

        let mut calls = recording.calls();
        let Some(TelemetryCall::RunFinished { duration, .. }) = calls.get_mut(3) else {
            panic!("the fourth call is the run's end: {calls:?}");
        };
        *duration = Duration::ZERO;
        assert_eq!(
            calls,
            vec![
                TelemetryCall::RunStarted {
                    run_id: "run-1".to_owned(),
                    task_id: Some("task-a".to_owned()),
                    parent,
                },
                TelemetryCall::OpSettled {
                    run_id: Some("run-1".to_owned()),
                    op: "page.goto",
                    raw_name: None,
                    op_label: "page.goto",
                    op_id: "run-1.1".to_owned(),
                    site: Some(OpSite { line: 3, column: 9 }),
                    tab_id: Some(42),
                    outcome: OpOutcome::Ok,
                    delivery: Delivery::Done,
                    error: None,
                    url: Some("https://example.com/a?b=c".to_owned()),
                },
                TelemetryCall::OpSettled {
                    run_id: Some("run-1".to_owned()),
                    op: "unknown",
                    raw_name: Some("made.up".to_owned()),
                    op_label: Vocabulary::UNKNOWN,
                    op_id: "run-1.2".to_owned(),
                    site: None,
                    tab_id: None,
                    outcome: OpOutcome::Error,
                    delivery: Delivery::NotSent,
                    error: Some(failure),
                    url: None,
                },
                TelemetryCall::RunFinished {
                    run_id: "run-1".to_owned(),
                    verdict,
                    duration: Duration::ZERO,
                },
                TelemetryCall::RunRefused { error: failure },
            ],
            "every hook is kept in call order with the values it was given, ops under their run"
        );
    }

    #[tokio::test]
    async fn the_recording_captures_tab_scrape_and_shutdown_calls() -> Result<(), Box<dyn Error>> {
        let (telemetry, recording) = Telemetry::recording();
        assert_eq!(recording.open_tabs(), None, "no source until one is set");
        telemetry.tab_opened(TabOrigin::Agent);
        telemetry.tab_closed(
            TabOrigin::Agent,
            CloseReason::Janitor,
            Duration::from_secs(5),
        );
        telemetry.renderer_crashed();
        telemetry.set_open_tabs(Arc::new(|| OpenTabCounts {
            agent: 2,
            found: 1,
            neo: 1,
        }));
        assert_eq!(
            telemetry.render_prometheus(),
            "",
            "the double renders nothing"
        );
        telemetry.shutdown(Duration::from_secs(1)).await?;
        assert_eq!(
            recording.calls(),
            vec![
                TelemetryCall::TabOpened {
                    origin: TabOrigin::Agent
                },
                TelemetryCall::TabClosed {
                    origin: TabOrigin::Agent,
                    reason: CloseReason::Janitor,
                    lifetime: Duration::from_secs(5),
                },
                TelemetryCall::RendererCrashed,
                TelemetryCall::Scraped,
                TelemetryCall::ShutDown {
                    budget: Duration::from_secs(1)
                },
            ],
            "tab hooks, the scrape and the shutdown are kept with their values"
        );
        assert_eq!(
            recording.open_tabs(),
            Some(OpenTabCounts {
                agent: 2,
                found: 1,
                neo: 1
            }),
            "the double reads the registered source as a collection would"
        );
        Ok(())
    }
}
