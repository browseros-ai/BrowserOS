//! Playwright's two retry schedules, as data (server/frames.ts and server/dom.ts at v1.64.1).
//!
//! An action retries at two levels. The outer loop re-resolves the locator, so a re-rendered
//! element is found again; the inner loop retries the same pinned element while it scrolls,
//! settles or stops being covered. Matching upstream's waits keeps Neo's timing and call logs
//! Playwright-shaped (decisions.md). The waits run on Rust's clock through `OpScope::sleep`,
//! never on page timers, which stall in background tabs.

use std::time::Duration;

/// `retryWithProgressAndBackoff`: before the first attempt, then before each re-resolution.
const OUTER_MS: [u64; 6] = [0, 20, 50, 100, 100, 500];

/// `_retryAction`: the first attempt starts at once, then each retry waits upstream's
/// `waitTime` (0, 20, 100, 100, 500 ms).
const INNER_MS: [u64; 6] = [0, 0, 20, 100, 100, 500];

/// One retry schedule: the wait before each attempt, the last wait repeating forever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Backoff {
    /// Never empty; a prefix of one of the schedules above.
    waits_ms: &'static [u64],
}

impl Backoff {
    /// The outer loop, which re-resolves the locator. With a timeout, the trailing waits above
    /// a fifth of it are dropped first, as upstream does, so a short timeout still retries
    /// several times; `None` is Playwright's `timeout: 0`.
    pub(crate) fn outer(timeout: Option<Duration>) -> Self {
        let Some(timeout) = timeout else {
            return Self {
                waits_ms: &OUTER_MS,
            };
        };
        // Upstream pops `wait > timeout / 5` from the end; the schedule never decreases, so
        // what stays is a prefix. `5 * wait > timeout` is that test without rounding.
        let kept = OUTER_MS
            .iter()
            .skip(1)
            .take_while(|wait| u128::from(**wait) * 5 <= timeout.as_millis())
            .count();
        Self {
            waits_ms: OUTER_MS.get(..=kept).unwrap_or(&OUTER_MS),
        }
    }

    /// The inner loop, which retries the same pinned element.
    pub(crate) fn inner() -> Self {
        Self {
            waits_ms: &INNER_MS,
        }
    }

    /// The wait before attempt `attempt`, counting the first attempt as 0.
    pub(crate) fn wait(self, attempt: usize) -> Duration {
        let last = self.waits_ms.last().copied().unwrap_or_default();
        Duration::from_millis(self.waits_ms.get(attempt).copied().unwrap_or(last))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Backoff;

    fn waits(backoff: Backoff, attempts: usize) -> Vec<u64> {
        (0..attempts)
            .map(|attempt| u64::try_from(backoff.wait(attempt).as_millis()).unwrap_or(u64::MAX))
            .collect()
    }

    /// `retryWithProgressAndTimeouts` waits `timeouts[min(i, len - 1)]` before attempt `i`,
    /// with `timeouts = [0, ...backoffScale]`.
    #[test]
    fn tests_that_the_outer_loop_waits_as_upstream() {
        assert_eq!(
            waits(Backoff::outer(None), 8),
            [0, 20, 50, 100, 100, 500, 500, 500]
        );
        assert_eq!(
            waits(Backoff::outer(Some(Duration::from_secs(5))), 8),
            [0, 20, 50, 100, 100, 500, 500, 500],
            "5 s keeps every wait (500 is a tenth)"
        );
        assert_eq!(
            waits(Backoff::outer(Some(Duration::from_millis(2500))), 7),
            [0, 20, 50, 100, 100, 500, 500],
            "500 is not above 2500 / 5"
        );
        assert_eq!(
            waits(Backoff::outer(Some(Duration::from_millis(2499))), 7),
            [0, 20, 50, 100, 100, 100, 100],
            "500 is above 2499 / 5, so it goes"
        );
        assert_eq!(
            waits(Backoff::outer(Some(Duration::from_secs(1))), 7),
            [0, 20, 50, 100, 100, 100, 100],
            "a 1 s timeout polls every 100 ms"
        );
        assert_eq!(
            waits(Backoff::outer(Some(Duration::from_millis(99))), 4),
            [0, 0, 0, 0],
            "below 100 ms every wait goes, and upstream retries at once"
        );
    }

    /// `_retryAction` waits nothing before attempt 0, then `waitTime[min(retry - 1, 4)]`.
    #[test]
    fn tests_that_the_inner_loop_waits_as_upstream() {
        assert_eq!(
            waits(Backoff::inner(), 8),
            [0, 0, 20, 100, 100, 500, 500, 500]
        );
    }
}
