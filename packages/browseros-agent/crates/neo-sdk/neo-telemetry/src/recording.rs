//! The test double: a telemetry handle that keeps every call in memory.
//!
//! Built by [`crate::Telemetry::recording`] behind the `test-support` feature. The server's
//! tests hand its handle to the run service, the dispatcher or the tab registry, drive them,
//! and assert on [`Recording::calls`]: which hook fired, in what order, with which values.
//! It labels ops through the same [`Vocabulary`] the live pipeline uses (once the test hands it
//! one with [`crate::Telemetry::bind_vocabulary`]), so a test can check which label an op gets.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use crate::label::{CloseReason, Delivery, ErrorLabel, OpOutcome, TabOrigin};
use crate::{OpSite, OpenTabCounts, OpenTabsSource, RunVerdict, TraceContext, Vocabulary};

/// One facade call, as the recording double kept it. Owned copies of what the hook was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TelemetryCall {
    /// [`crate::Telemetry::start_run`].
    RunStarted {
        /// `RunFacts::run_id`, formatted.
        run_id: String,
        /// `RunFacts::task_id`.
        task_id: Option<String>,
        /// `RunFacts::parent`.
        parent: Option<TraceContext>,
    },
    /// [`crate::RunSpan::finish`].
    RunFinished {
        /// The run's id.
        run_id: String,
        /// What it was finished with.
        verdict: RunVerdict,
        /// How long the span was open; set it to zero before comparing whole calls.
        duration: Duration,
    },
    /// A [`crate::RunSpan`] dropped unfinished.
    RunDropped {
        /// The run's id.
        run_id: String,
    },
    /// [`crate::Telemetry::run_refused`].
    RunRefused {
        /// Why the call was refused.
        error: ErrorLabel,
    },
    /// [`crate::RunTelemetry::op_settled`].
    OpSettled {
        /// The op's run; `None` for an op of an internal run.
        run_id: Option<String>,
        /// `OpSettled::op`: the door's name for the method.
        op: &'static str,
        /// `OpSettled::raw_name`.
        raw_name: Option<String>,
        /// The `op` label the metrics would carry: `op` interned against the bound vocabulary,
        /// or `unknown`.
        op_label: &'static str,
        /// `OpSettled::op_id`, formatted.
        op_id: String,
        /// `OpSettled::site`.
        site: Option<OpSite>,
        /// `OpSettled::tab_id`.
        tab_id: Option<i64>,
        /// `OpSettled::outcome`.
        outcome: OpOutcome,
        /// `OpSettled::delivery`.
        delivery: Delivery,
        /// `OpSettled::error`.
        error: Option<ErrorLabel>,
        /// `OpSettled::url`, raw (the double applies no privacy dial).
        url: Option<String>,
    },
    /// [`crate::Telemetry::tab_opened`].
    TabOpened {
        /// Who opened it.
        origin: TabOrigin,
    },
    /// [`crate::Telemetry::tab_closed`].
    TabClosed {
        /// Who had opened it.
        origin: TabOrigin,
        /// Why it closed.
        reason: CloseReason,
        /// How long it was open.
        lifetime: Duration,
    },
    /// [`crate::Telemetry::renderer_crashed`].
    RendererCrashed,
    /// [`crate::Telemetry::render_prometheus`].
    Scraped,
    /// [`crate::Telemetry::shutdown`].
    ShutDown {
        /// The budget it was given.
        budget: Duration,
    },
}

/// The read side of a recording handle: what the handle's clones were called with so far.
/// Cheap to clone; every clone reads the same log.
#[derive(Clone, Debug)]
pub struct Recording {
    recorder: Arc<Recorder>,
}

impl Recording {
    pub(crate) fn new(recorder: Arc<Recorder>) -> Self {
        Self { recorder }
    }

    /// Every call so far, in the order they were made.
    #[must_use]
    pub fn calls(&self) -> Vec<TelemetryCall> {
        self.recorder.lock_calls().clone()
    }

    /// Runs started and not yet ended (finished or dropped): what `neo_runs_in_flight` would
    /// read now.
    #[must_use]
    pub fn runs_in_flight(&self) -> i64 {
        let mut in_flight = 0;
        for call in self.recorder.lock_calls().iter() {
            match call {
                TelemetryCall::RunStarted { .. } => in_flight += 1,
                TelemetryCall::RunFinished { .. } | TelemetryCall::RunDropped { .. } => {
                    in_flight -= 1;
                }
                _ => {}
            }
        }
        in_flight
    }

    /// The registered open-tabs source, read now as a scrape would; `None` before one is set.
    #[must_use]
    pub fn open_tabs(&self) -> Option<OpenTabCounts> {
        let source = self
            .recorder
            .open_tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        source.map(|source| source())
    }
}

/// The shared log behind a recording handle and its [`Recording`].
pub(crate) struct Recorder {
    vocabulary: OnceLock<Vocabulary>,
    calls: Mutex<Vec<TelemetryCall>>,
    open_tabs: Mutex<Option<OpenTabsSource>>,
}

impl fmt::Debug for Recorder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Recorder")
            .field("calls", &self.lock_calls().len())
            .finish_non_exhaustive()
    }
}

impl Recorder {
    pub(crate) fn new() -> Self {
        Self {
            vocabulary: OnceLock::new(),
            calls: Mutex::default(),
            open_tabs: Mutex::default(),
        }
    }

    /// The tables labels intern against; empty (everything `unknown`) until one is set.
    pub(crate) fn vocabulary(&self) -> &Vocabulary {
        static EMPTY: OnceLock<Vocabulary> = OnceLock::new();
        self.vocabulary
            .get()
            .unwrap_or_else(|| EMPTY.get_or_init(Vocabulary::default))
    }

    /// The first call wins, as on the live pipeline.
    pub(crate) fn bind_vocabulary(&self, vocabulary: Vocabulary) {
        let _ = self.vocabulary.set(vocabulary);
    }

    pub(crate) fn record(&self, call: TelemetryCall) {
        self.lock_calls().push(call);
    }

    pub(crate) fn set_open_tabs(&self, source: OpenTabsSource) {
        *self
            .open_tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(source);
    }

    // A test that panics mid-call must not hide the log from the next assertion.
    fn lock_calls(&self) -> MutexGuard<'_, Vec<TelemetryCall>> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
