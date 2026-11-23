//! How a run ended, as its metrics and its `neo.run` span say it.

/// How a finished run ended: the `neo.run.status` of `neo_runs_total` and
/// `neo_run_duration_seconds`, and the span's `neo.run.status`.
///
/// The finished half of `GET /v1/runs`' status vocabulary (`neo_trace::RunStatus`): `running`
/// is never a finished run, and `interrupted` is only ever written by the next process, for a
/// run this one never finished, so neither can reach a metric.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RunStatus {
    /// The script returned.
    Ok,
    /// The script threw, or Neo failed the run.
    Error,
    /// The caller went away or Neo stopped the run (MCP cancellation, shutdown).
    Cancelled,
    /// The run hit its wall-clock budget.
    Timeout,
}

impl RunStatus {
    /// The wire spelling, as `/v1/runs` serves it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
        }
    }
}
