//! Whether an op's side effect happened, as its span says it.

/// Whether an op's side effect happened, as far as Neo can know: the span's `neo.op.delivery`.
/// Mirrors `neo_trace::Delivery`, the `delivery` of an operation on `GET /v1/runs/{id}`. Never a
/// metric label (it would multiply every op series by three).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Delivery {
    /// No side effect was attempted.
    NotSent,
    /// Neo has a definite answer, success or failure.
    Done,
    /// Sent and never confirmed.
    Unknown,
}

impl Delivery {
    /// The wire spelling, as `/v1/runs/{id}` serves it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotSent => "not_sent",
            Self::Done => "done",
            Self::Unknown => "unknown",
        }
    }
}
