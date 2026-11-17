//! Keeps the licence fresh: at start, then every four hours with some jitter so installs do not
//! refresh in step, and sooner with a backoff after a failure.

use super::{Billing, RefreshOutcome};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub const EVERY: Duration = Duration::from_secs(4 * 60 * 60);
pub const MAX_JITTER_SECS: u64 = 10 * 60;
pub const BACKOFF: [Duration; 4] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(15 * 60),
    Duration::from_secs(60 * 60),
];

pub fn spawn(billing: Billing, cancel: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut failures = 0;
        loop {
            let outcome = billing.scheduled_refresh().await;
            if outcome == RefreshOutcome::Failed {
                failures += 1;
            } else {
                failures = 0;
            }
            let wait = next_wait(failures, rand::random_range(0..=MAX_JITTER_SECS));
            tokio::select! {
                () = cancel.cancelled() => return,
                () = tokio::time::sleep(wait) => {}
            }
        }
    })
}

/// How long to wait after `failures` refreshes in a row have failed (none: the regular wait).
#[must_use]
pub fn next_wait(failures: usize, jitter_secs: u64) -> Duration {
    match failures {
        0 => EVERY + Duration::from_secs(jitter_secs.min(MAX_JITTER_SECS)),
        n => BACKOFF[(n - 1).min(BACKOFF.len() - 1)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_four_hours_plus_jitter_then_backs_off_after_failures() {
        assert_eq!(next_wait(0, 0), EVERY);
        assert_eq!(
            next_wait(0, 9_999),
            EVERY + Duration::from_secs(MAX_JITTER_SECS)
        );
        assert_eq!(next_wait(1, 0), Duration::from_secs(60));
        assert_eq!(next_wait(2, 0), Duration::from_secs(5 * 60));
        assert_eq!(next_wait(3, 0), Duration::from_secs(15 * 60));
        assert_eq!(next_wait(4, 0), Duration::from_secs(60 * 60));
        assert_eq!(next_wait(40, 0), Duration::from_secs(60 * 60));
    }
}
