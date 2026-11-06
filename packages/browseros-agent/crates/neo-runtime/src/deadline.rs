//! When a run must stop. One `Deadline` per run is shared by every place that can wait: the
//! QuickJS interrupt handler, the timers, the synchronous getters and the `select!` statements
//! that bound each phase of a run, so they all agree on the same instant and cancellation.

use std::future;
use std::time::Duration;

use tokio::time;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

const FAR_FUTURE: Duration = Duration::from_hours(24 * 365 * 30);

/// Why a run stopped before its script finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    Timeout,
    Cancelled,
}

#[derive(Debug, Clone)]
pub(crate) struct Deadline {
    at: Instant,
    timeout: Duration,
    cancel: CancellationToken,
}

impl Deadline {
    pub(crate) fn new(timeout: Duration, cancel: CancellationToken) -> Self {
        let now = Instant::now();
        Self {
            // A timeout too large to add is no deadline at all; tokio uses the same horizon.
            at: now.checked_add(timeout).unwrap_or(now + FAR_FUTURE),
            timeout,
            cancel,
        }
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Cancellation wins over the clock, so a run cancelled after its deadline reports the
    /// caller's decision rather than a timeout.
    pub(crate) fn stop(&self) -> Option<Stop> {
        if self.cancel.is_cancelled() {
            Some(Stop::Cancelled)
        } else if Instant::now() >= self.at {
            Some(Stop::Timeout)
        } else {
            None
        }
    }

    /// Resolves when the run must stop.
    pub(crate) async fn reached(&self) -> Stop {
        tokio::select! {
            () = self.cancel.cancelled() => Stop::Cancelled,
            () = time::sleep_until(self.at) => Stop::Timeout,
        }
    }

    /// Backs `setTimeout`. A timer due at or after the deadline never fires: the run ends at
    /// the deadline, and the realm's `select!` on [`Self::reached`] is what ends it.
    pub(crate) async fn sleep(&self, delay: Duration) {
        match Instant::now().checked_add(delay) {
            Some(due) if due < self.at => time::sleep_until(due).await,
            _ => future::pending().await,
        }
    }
}
