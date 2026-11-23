//! Why a tab closed, as a label.

/// Why a tab closed: the `neo.tab.close_reason` of `neo_tab_closed_total` and
/// `neo_tab_duration_seconds`. Exactly one per close, so no close is counted twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CloseReason {
    /// A script closed it (`page.close`).
    Script,
    /// The idle-tab janitor closed it.
    Janitor,
    /// Someone outside Neo closed it: the owner, the browser UI, the page itself.
    User,
    /// Its renderer crashed.
    Crash,
    /// Neo was shutting down.
    Shutdown,
}

impl CloseReason {
    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Janitor => "janitor",
            Self::User => "user",
            Self::Crash => "crash",
            Self::Shutdown => "shutdown",
        }
    }
}
