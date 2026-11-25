//! One settled operation, as the door hands it to telemetry.

use std::fmt;
use std::time::Instant;

use crate::label::{Delivery, ErrorLabel, OpOutcome};

/// Where in the script an op was called (`run:LINE:COL`). A plain data bag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpSite {
    /// The line, from 1 (`code.line.number`).
    pub line: u32,
    /// The column, from 1 (`code.column.number`).
    pub column: u32,
}

/// Whether a settled op counts on the op metrics. Every op keeps its span either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpCounting {
    /// The script's op, or one Neo ran for it (a child op, the run's lazy page): counted on
    /// `neo_operations_total`, `neo_operation_duration_seconds` and, when it fails,
    /// `neo_errors_total`.
    Counted,
    /// Neo's own housekeeping, sent with no script call site: the frame-tree reads, the event
    /// pump and listener bookkeeping. Their number grows with open tabs rather than with the
    /// script's work, so counting them would dilute every op rate and op error rate. Span only,
    /// so a trace still shows them; a failure that ends the run is counted once, at the run.
    Internal,
}

/// One op, as the door settled it ([`crate::RunTelemetry::op_settled`]): the same decided values
/// its trace record gets. Borrowed, so the no-op never copies or formats anything.
#[derive(Clone, Copy)]
pub struct OpSettled<'a> {
    /// The door's name for the method it ran (`MethodSpec::name`): a registered name, or the
    /// door's own `unknown` for a name with no method. `'static`, so no text a script or page
    /// chose can become a label; the pipeline also interns it against [`crate::Vocabulary`].
    pub op: &'static str,
    /// The name the caller wrote, as the door's refusal shows it, only when it differs from `op`
    /// (an unregistered name). The door clips it, since a script can send any string; the
    /// span's `neo.op.name` is `raw_name.unwrap_or(op)` as given. No metric sees it.
    pub raw_name: Option<&'a str>,
    /// The op id `/v1/runs/{id}` uses (`{run}.{seq}`), which links the span to the op's heavy
    /// blobs and screenshots. Formatted only when a span is recorded.
    pub op_id: &'a dyn fmt::Display,
    /// Where the script called it, when known.
    pub site: Option<OpSite>,
    /// The tab it ended on, as the host API numbers tabs: known by now even for an op on the
    /// run's lazy default page.
    pub tab_id: Option<i64>,
    /// When the door opened it (`OperationGuard::start`): the span's start and the duration's.
    pub started: Instant,
    /// As the op's record on `/v1/runs/{id}` says.
    pub outcome: OpOutcome,
    /// As the op's record says; span only.
    pub delivery: Delivery,
    /// Whether the op metrics see it.
    pub counting: OpCounting,
    /// Its failure, exactly when its record carries an error; a retired background pull is
    /// `None`.
    pub error: Option<ErrorLabel>,
    /// The page's URL after the op, when the door holds it; `None` for the host's private typing
    /// (`OperationKind::Protected`). Raw: the span privacy dial (`CLAW_TELEMETRY_SITE_DETAIL`)
    /// decides what of it a span keeps; no metric ever sees it.
    pub url: Option<&'a str>,
}

impl fmt::Debug for OpSettled<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpSettled")
            .field("op", &self.op)
            .field("raw_name", &self.raw_name)
            .field("op_id", &format_args!("{}", self.op_id))
            .field("site", &self.site)
            .field("tab_id", &self.tab_id)
            .field("started", &self.started)
            .field("outcome", &self.outcome)
            .field("delivery", &self.delivery)
            .field("counting", &self.counting)
            .field("error", &self.error)
            .field("url", &self.url.map(|_| "<redacted>"))
            .finish()
    }
}
