//! How an operation ended, as its metrics and its span say it.

/// How an op ended: the `neo.op.outcome` of `neo_operations_total` and
/// `neo_operation_duration_seconds`, and the span's `neo.op.outcome`. Mirrors
/// `neo_trace::Outcome`, the `outcome` of an operation on `GET /v1/runs/{id}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OpOutcome {
    /// It returned a value.
    Ok,
    /// It failed with an error.
    Error,
    /// It was cut short: the run was cancelled, timed out, or ended while it ran.
    Cancelled,
}

impl OpOutcome {
    /// The wire spelling, as `/v1/runs/{id}` serves it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }
}
