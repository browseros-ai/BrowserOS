//! Whose behaviour must change to fix a failure, as a label.

/// Who must change to fix a failure: the `neo.error.owner` of `neo_runs_total`,
/// `neo_operations_total` and `neo_errors_total`, and the spans' `neo.error.owner`. Mirrors
/// `neo_trace::ErrorOwner`. A run or op that did not fail carries [`ErrorOwner::NONE`] on its
/// metrics (and no attribute on its span): there is no `None` variant, so a failure always has
/// a real owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorOwner {
    /// What the page did.
    Website,
    /// The agent's own code.
    Script,
    /// Neo itself, the browser or the OS.
    Neo,
    /// The host's rules, services or owner.
    Host,
}

impl ErrorOwner {
    /// The `neo.error.owner` of a run or op that did not fail.
    pub const NONE: &'static str = "none";

    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Website => "website",
            Self::Script => "script",
            Self::Neo => "neo",
            Self::Host => "host",
        }
    }
}
