//! The 11 foundational instruments, as one catalog (wire-contract.md, "Metrics").
//!
//! [`CATALOG`] is the one list of families: name, help, kind, unit and buckets. [`Instruments::new`]
//! builds every instrument from its entry, and the Prometheus renderer (`prometheus.rs`) walks the
//! same list for its `# HELP` / `# TYPE` lines and family order, so the pull door and the OTLP push
//! door cannot drift apart.
//!
//! Labels are closed by the record methods' signatures: each takes only the closed label enums
//! ([`crate::label`]) and the two vocabulary tokens [`OpLabel`] and [`CodeLabel`], which only a
//! lookup in the bound [`Vocabulary`] can mint (their fields are private to this module). No `&str`
//! from a caller reaches a `KeyValue` here, so no caller can add a series: never a URL, host, title,
//! tab, run or task id. Every label value is a `&'static str`, so recording allocates nothing.

use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use opentelemetry::metrics::{Counter, Histogram, Meter, UpDownCounter};
use opentelemetry::{Key, KeyValue};

use crate::label::{
    CloseReason, ErrorCode, ErrorLabel, ErrorOwner, OpOutcome, RunStatus, TabOrigin,
};
use crate::{OpenTabCounts, OpenTabsSource, Vocabulary};

/// How a family is typed in Prometheus text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A monotonic total; its name ends in `_total`.
    Counter,
    /// A value read as it is now (`neo_runs_in_flight`, `neo_tabs_open`).
    Gauge,
    /// A distribution in explicit buckets: `_bucket`, `_sum`, `_count`.
    Histogram,
}

impl Kind {
    /// The `# TYPE` spelling.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
        }
    }
}

/// One metric family as both doors name it: the Prometheus family and the OTLP metric share the
/// name. A plain data bag.
#[derive(Debug)]
pub(crate) struct MetricSpec {
    /// The family name, in both doors.
    pub(crate) name: &'static str,
    /// The `# HELP` text and the OTLP description.
    pub(crate) help: &'static str,
    /// How Prometheus types it.
    pub(crate) kind: Kind,
    /// The OTLP unit: `s` for the durations, empty otherwise (Prometheus reads the unit from the
    /// name's suffix instead).
    pub(crate) unit: &'static str,
    /// Bucket upper bounds in seconds, strictly increasing; empty for a non-histogram.
    pub(crate) buckets: &'static [f64],
}

/// Runs: a short script ends well under a second; a run's budget is 5 minutes unless the model
/// asks for more, and never above 15 (`Defaults::RUN_TIMEOUT`, `RUN_TIMEOUT_MAX`), so the last
/// bounds sit on those budgets.
const RUN_BUCKETS: &[f64] = &[
    0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0,
];
/// Ops: a local read takes milliseconds, a navigation seconds, a CAPTCHA solve up to 2 minutes.
const OP_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0,
];
/// Tabs: from a popup gone in a second to a tab kept open for a day.
const TAB_BUCKETS: &[f64] = &[
    1.0, 10.0, 30.0, 60.0, 300.0, 900.0, 1800.0, 3600.0, 7200.0, 14400.0, 43200.0, 86400.0,
];

const RUNS_TOTAL: MetricSpec = MetricSpec {
    name: "neo_runs_total",
    help: "Runs that ended, and calls refused before they became a run, by status and by who \
           must fix the failure.",
    kind: Kind::Counter,
    unit: "",
    buckets: &[],
};
const RUNS_IN_FLIGHT: MetricSpec = MetricSpec {
    name: "neo_runs_in_flight",
    help: "Runs admitted and not yet ended.",
    kind: Kind::Gauge,
    unit: "",
    buckets: &[],
};
const RUN_DURATION: MetricSpec = MetricSpec {
    name: "neo_run_duration_seconds",
    help: "How long each run took, from admission to its end, by status.",
    kind: Kind::Histogram,
    unit: "s",
    buckets: RUN_BUCKETS,
};
const OPERATIONS_TOTAL: MetricSpec = MetricSpec {
    name: "neo_operations_total",
    help: "Operations settled, by op (unregistered names count as unknown), status and who must \
           fix the failure.",
    kind: Kind::Counter,
    unit: "",
    buckets: &[],
};
const OPERATION_DURATION: MetricSpec = MetricSpec {
    name: "neo_operation_duration_seconds",
    help: "How long each operation took, by op and status.",
    kind: Kind::Histogram,
    unit: "s",
    buckets: OP_BUCKETS,
};
const ERRORS_TOTAL: MetricSpec = MetricSpec {
    name: "neo_errors_total",
    help: "Failures by who must fix them and their code, each counted once where it is \
           attributed.",
    kind: Kind::Counter,
    unit: "",
    buckets: &[],
};
const TABS_OPEN: MetricSpec = MetricSpec {
    name: "neo_tabs_open",
    help: "Tabs open now, by who opened them.",
    kind: Kind::Gauge,
    unit: "",
    buckets: &[],
};
const TABS_OPENED_TOTAL: MetricSpec = MetricSpec {
    name: "neo_tabs_opened_total",
    help: "Tabs Neo registered, by who opened them.",
    kind: Kind::Counter,
    unit: "",
    buckets: &[],
};
const TABS_CLOSED_TOTAL: MetricSpec = MetricSpec {
    name: "neo_tabs_closed_total",
    help: "Tabs closed, by who opened them and why they closed.",
    kind: Kind::Counter,
    unit: "",
    buckets: &[],
};
const TAB_LIFETIME: MetricSpec = MetricSpec {
    name: "neo_tab_lifetime_seconds",
    help: "How long each closed tab was open, by who opened it and why it closed.",
    kind: Kind::Histogram,
    unit: "s",
    buckets: TAB_BUCKETS,
};
const RENDERER_CRASHES_TOTAL: MetricSpec = MetricSpec {
    name: "neo_renderer_crashes_total",
    help: "Renderer crashes, whether or not an op was using the tab.",
    kind: Kind::Counter,
    unit: "",
    buckets: &[],
};

/// Every family, in wire-contract order: the order of the Prometheus text.
pub(crate) const CATALOG: [&MetricSpec; 11] = [
    &RUNS_TOTAL,
    &RUNS_IN_FLIGHT,
    &RUN_DURATION,
    &OPERATIONS_TOTAL,
    &OPERATION_DURATION,
    &ERRORS_TOTAL,
    &TABS_OPEN,
    &TABS_OPENED_TOTAL,
    &TABS_CLOSED_TOTAL,
    &TAB_LIFETIME,
    &RENDERER_CRASHES_TOTAL,
];

const STATUS: Key = Key::from_static_str("status");
const ERROR_OWNER: Key = Key::from_static_str("error_owner");
const ERROR_CODE: Key = Key::from_static_str("error_code");
const OP: Key = Key::from_static_str("op");
const ORIGIN: Key = Key::from_static_str("origin");
const CLOSE_REASON: Key = Key::from_static_str("close_reason");

/// Every `origin` the open-tabs gauge reports, so each one shows a value, `0` included.
const ORIGINS: [TabOrigin; 3] = [TabOrigin::Agent, TabOrigin::Found, TabOrigin::Neo];

/// An `op` label value: a name from the bound [`Vocabulary`], or `unknown`. Minted only by
/// [`OpLabel::of`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OpLabel(&'static str);

impl OpLabel {
    /// The label of a call to `name`: the vocabulary's own copy of it, or `unknown` when the
    /// vocabulary is not bound yet or does not list it.
    pub(crate) fn of(vocabulary: Option<&Vocabulary>, name: &str) -> Self {
        Self(vocabulary.map_or(Vocabulary::UNKNOWN, |vocabulary| vocabulary.op_label(name)))
    }
}

/// An `error_code` label value: one of Neo's codes from the bound [`Vocabulary`], or `unknown`.
/// Minted only by [`CodeLabel::of`], so a code built at run time cannot open a series.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CodeLabel(&'static str);

impl CodeLabel {
    /// The label of `code`, looked up rather than trusted.
    pub(crate) fn of(vocabulary: Option<&Vocabulary>, code: ErrorCode) -> Self {
        Self(vocabulary.map_or(Vocabulary::UNKNOWN, |vocabulary| {
            vocabulary.error_code_label(code.as_str())
        }))
    }
}

/// Where `neo_tabs_open` reads the live counts: the source the tab registry set, once set.
///
/// Read by the gauge's callback on whichever thread collects (a scrape's handler, the periodic
/// exporter's own thread), written by [`crate::Telemetry::set_open_tabs`].
#[derive(Default)]
pub(crate) struct OpenTabs(RwLock<Option<OpenTabsSource>>);

impl OpenTabs {
    /// Installs `source`, replacing any earlier one.
    pub(crate) fn set(&self, source: OpenTabsSource) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = Some(source);
    }

    /// The counts now, or `None` while no source is set: the gauge then reports nothing, rather
    /// than a zero that would read as "no tabs".
    pub(crate) fn read(&self) -> Option<OpenTabCounts> {
        // Taken out of the lock before it runs, so `set` never waits on a slow source. A poisoned
        // lock still holds a whole value (`set` only assigns), so it is read as is.
        let source = self
            .0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(Arc::clone);
        source.map(|source| source())
    }
}

impl fmt::Debug for OpenTabs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let set = self
            .0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        formatter
            .debug_struct("OpenTabs")
            .field("set", &set)
            .finish()
    }
}

/// The instruments one process records into, built from [`CATALOG`]. The pipeline holds one and
/// calls a record method per event; each method names its family's labels in its signature.
#[derive(Debug)]
pub(crate) struct Instruments {
    runs_total: Counter<u64>,
    runs_in_flight: UpDownCounter<i64>,
    run_duration: Histogram<f64>,
    operations_total: Counter<u64>,
    operation_duration: Histogram<f64>,
    errors_total: Counter<u64>,
    tabs_opened_total: Counter<u64>,
    tabs_closed_total: Counter<u64>,
    tab_lifetime: Histogram<f64>,
    renderer_crashes_total: Counter<u64>,
}

impl Instruments {
    /// Builds every instrument on `meter`, and registers `neo_tabs_open`'s callback over
    /// `open_tabs`. The two unlabelled series start at zero, so a scrape shows them from the
    /// start instead of after the first run or crash.
    pub(crate) fn new(meter: &Meter, open_tabs: Arc<OpenTabs>) -> Self {
        let instruments = Self {
            runs_total: counter(meter, &RUNS_TOTAL),
            runs_in_flight: meter
                .i64_up_down_counter(RUNS_IN_FLIGHT.name)
                .with_description(RUNS_IN_FLIGHT.help)
                .build(),
            run_duration: histogram(meter, &RUN_DURATION),
            operations_total: counter(meter, &OPERATIONS_TOTAL),
            operation_duration: histogram(meter, &OPERATION_DURATION),
            errors_total: counter(meter, &ERRORS_TOTAL),
            tabs_opened_total: counter(meter, &TABS_OPENED_TOTAL),
            tabs_closed_total: counter(meter, &TABS_CLOSED_TOTAL),
            tab_lifetime: histogram(meter, &TAB_LIFETIME),
            renderer_crashes_total: counter(meter, &RENDERER_CRASHES_TOTAL),
        };
        instruments.runs_in_flight.add(0, &[]);
        instruments.renderer_crashes_total.add(0, &[]);
        // The SDK keeps the callback for the meter provider's life; the returned handle has no
        // way to unregister it, so it is not kept. The callback is not wrapped in `catch_unwind`
        // (STYLE.md G16): the tab registry's source is a lock and a count that cannot panic.
        meter
            .u64_observable_gauge(TABS_OPEN.name)
            .with_description(TABS_OPEN.help)
            .with_callback(move |observer| {
                let Some(counts) = open_tabs.read() else {
                    return;
                };
                for origin in ORIGINS {
                    observer.observe(counts.of(origin), &[KeyValue::new(ORIGIN, origin.as_str())]);
                }
            })
            .build();
        instruments
    }

    /// A counted run was admitted: `neo_runs_in_flight` +1.
    pub(crate) fn run_started(&self) {
        self.runs_in_flight.add(1, &[]);
    }

    /// A counted run ended: `neo_runs_in_flight` -1, `neo_runs_total` and
    /// `neo_run_duration_seconds`.
    pub(crate) fn run_ended(&self, status: RunStatus, error: Option<ErrorLabel>, took: Duration) {
        self.runs_in_flight.add(-1, &[]);
        self.run_counted(status, error);
        self.run_duration.record(
            took.as_secs_f64(),
            &[KeyValue::new(STATUS, status.as_str())],
        );
    }

    /// A call refused before it became a run: `neo_runs_total{status="error"}` alone.
    pub(crate) fn run_refused(&self, error: ErrorLabel) {
        self.run_counted(RunStatus::Error, Some(error));
    }

    fn run_counted(&self, status: RunStatus, error: Option<ErrorLabel>) {
        self.runs_total.add(
            1,
            &[
                KeyValue::new(STATUS, status.as_str()),
                KeyValue::new(ERROR_OWNER, ErrorLabel::owner_label(error)),
            ],
        );
    }

    /// An op settled: `neo_operations_total` and `neo_operation_duration_seconds`. No
    /// `error_code` here: codes go on `neo_errors_total` alone, so ops never multiply by codes.
    pub(crate) fn op_settled(
        &self,
        op: OpLabel,
        outcome: OpOutcome,
        error: Option<ErrorLabel>,
        took: Duration,
    ) {
        let op = KeyValue::new(OP, op.0);
        let status = KeyValue::new(STATUS, outcome.as_str());
        self.operation_duration
            .record(took.as_secs_f64(), &[op.clone(), status.clone()]);
        self.operations_total.add(
            1,
            &[
                op,
                status,
                KeyValue::new(ERROR_OWNER, ErrorLabel::owner_label(error)),
            ],
        );
    }

    /// One failure, counted where it is attributed: `neo_errors_total`.
    pub(crate) fn error_counted(&self, owner: ErrorOwner, code: CodeLabel) {
        self.errors_total.add(
            1,
            &[
                KeyValue::new(ERROR_OWNER, owner.as_str()),
                KeyValue::new(ERROR_CODE, code.0),
            ],
        );
    }

    /// `neo_tabs_opened_total`.
    pub(crate) fn tab_opened(&self, origin: TabOrigin) {
        self.tabs_opened_total
            .add(1, &[KeyValue::new(ORIGIN, origin.as_str())]);
    }

    /// `neo_tabs_closed_total` and `neo_tab_lifetime_seconds`.
    pub(crate) fn tab_closed(&self, origin: TabOrigin, reason: CloseReason, lifetime: Duration) {
        let labels = [
            KeyValue::new(ORIGIN, origin.as_str()),
            KeyValue::new(CLOSE_REASON, reason.as_str()),
        ];
        self.tabs_closed_total.add(1, &labels);
        self.tab_lifetime.record(lifetime.as_secs_f64(), &labels);
    }

    /// `neo_renderer_crashes_total`.
    pub(crate) fn renderer_crashed(&self) {
        self.renderer_crashes_total.add(1, &[]);
    }
}

fn counter(meter: &Meter, spec: &MetricSpec) -> Counter<u64> {
    meter
        .u64_counter(spec.name)
        .with_description(spec.help)
        .build()
}

fn histogram(meter: &Meter, spec: &MetricSpec) -> Histogram<f64> {
    meter
        .f64_histogram(spec.name)
        .with_description(spec.help)
        .with_unit(spec.unit)
        .with_boundaries(spec.buckets.to_vec())
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// wire-contract.md's table, row by row: name, kind, unit.
    const WIRE_CONTRACT: [(&str, Kind, &str); 11] = [
        ("neo_runs_total", Kind::Counter, ""),
        ("neo_runs_in_flight", Kind::Gauge, ""),
        ("neo_run_duration_seconds", Kind::Histogram, "s"),
        ("neo_operations_total", Kind::Counter, ""),
        ("neo_operation_duration_seconds", Kind::Histogram, "s"),
        ("neo_errors_total", Kind::Counter, ""),
        ("neo_tabs_open", Kind::Gauge, ""),
        ("neo_tabs_opened_total", Kind::Counter, ""),
        ("neo_tabs_closed_total", Kind::Counter, ""),
        ("neo_tab_lifetime_seconds", Kind::Histogram, "s"),
        ("neo_renderer_crashes_total", Kind::Counter, ""),
    ];

    #[test]
    fn the_catalog_is_the_wire_contract_in_its_order() {
        let catalog: Vec<(&str, Kind, &str)> = CATALOG
            .iter()
            .map(|spec| (spec.name, spec.kind, spec.unit))
            .collect();
        assert_eq!(
            catalog,
            WIRE_CONTRACT.to_vec(),
            "both doors take names, kinds and units from this list, so it must be the contract's"
        );
    }

    #[test]
    fn counters_end_in_total_once_and_only_histograms_have_buckets() {
        for spec in CATALOG {
            let totals = spec.name.matches("_total").count();
            match spec.kind {
                Kind::Counter => assert!(
                    totals == 1 && spec.name.ends_with("_total"),
                    "{}: a counter's name ends in `_total`, exactly once",
                    spec.name
                ),
                Kind::Gauge | Kind::Histogram => assert_eq!(
                    totals, 0,
                    "{}: only a counter's name carries `_total`",
                    spec.name
                ),
            }
            match spec.kind {
                Kind::Histogram => {
                    assert!(
                        spec.name.ends_with("_seconds") && spec.unit == "s",
                        "{}: a duration is in seconds, in its name and its unit",
                        spec.name
                    );
                    assert!(
                        !spec.buckets.is_empty()
                            && spec.buckets.windows(2).all(|pair| pair[0] < pair[1])
                            && spec
                                .buckets
                                .iter()
                                .all(|bound| bound.is_finite() && *bound > 0.0),
                        "{}: buckets are positive, finite and strictly increasing",
                        spec.name
                    );
                }
                Kind::Counter | Kind::Gauge => assert!(
                    spec.buckets.is_empty(),
                    "{}: only a histogram has buckets",
                    spec.name
                ),
            }
            assert!(
                spec.help.ends_with('.') && !spec.help.contains('\n'),
                "{}: help is one sentence on one line",
                spec.name
            );
        }
    }

    #[test]
    fn op_and_code_labels_come_only_from_the_vocabulary() {
        let vocabulary = Vocabulary::new(["page.goto"], ["no_match"]);
        assert_eq!(
            OpLabel::of(Some(&vocabulary), "page.goto"),
            OpLabel("page.goto"),
            "a registered op labels as itself"
        );
        assert_eq!(
            OpLabel::of(Some(&vocabulary), "page.gotoo"),
            OpLabel(Vocabulary::UNKNOWN),
            "anything else is unknown"
        );
        assert_eq!(
            OpLabel::of(None, "page.goto"),
            OpLabel(Vocabulary::UNKNOWN),
            "before the vocabulary is bound, every op is unknown"
        );
        assert_eq!(
            CodeLabel::of(Some(&vocabulary), ErrorCode::new("no_match")),
            CodeLabel("no_match"),
            "one of Neo's codes labels as itself"
        );
        let leaked: &'static str = String::leak("made_up_at_run_time".to_owned());
        assert_eq!(
            CodeLabel::of(Some(&vocabulary), ErrorCode::new(leaked)),
            CodeLabel(Vocabulary::UNKNOWN),
            "a code built at run time cannot open a series"
        );
    }

    #[test]
    fn the_open_tabs_slot_reads_nothing_until_set_then_the_latest_source() {
        let slot = OpenTabs::default();
        assert_eq!(slot.read(), None, "no source: nothing, not zero");
        slot.set(Arc::new(|| OpenTabCounts {
            agent: 1,
            found: 2,
            neo: 3,
        }));
        slot.set(Arc::new(|| OpenTabCounts {
            agent: 4,
            found: 0,
            neo: 1,
        }));
        assert_eq!(
            slot.read(),
            Some(OpenTabCounts {
                agent: 4,
                found: 0,
                neo: 1
            }),
            "a later source replaces the earlier one, and is read at each collection"
        );
    }
}
