//! Whether a host claim holds a tab, as the app's idle-tab janitor asks it.

use tokio::time::Instant;

/// A lock's, fit's, or host keep's claim on one tab: held now, or let go at an instant. The idle-tab
/// janitor counts a tab as in use while it is held, and restarts the tab's idle clock when the
/// last claim lets go. Ended claims are kept an hour, so a janitor that asks
/// once a minute sees every let-go instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabHold {
    /// A lock, fit, or host keep holds the tab.
    Now,
    /// The latest claim on the tab ended at this instant, on Tokio's clock.
    Until(Instant),
}

impl TabHold {
    /// The later of two claims: `Now` beats any instant.
    #[must_use]
    pub fn latest(self, other: Self) -> Self {
        match (self, other) {
            (Self::Now, _) | (_, Self::Now) => Self::Now,
            (Self::Until(left), Self::Until(right)) => Self::Until(left.max(right)),
        }
    }

    /// The later of two optional claims.
    #[must_use]
    pub fn latest_of(left: Option<Self>, right: Option<Self>) -> Option<Self> {
        match (left, right) {
            (Some(left), Some(right)) => Some(left.latest(right)),
            (one, None) | (None, one) => one,
        }
    }
}
