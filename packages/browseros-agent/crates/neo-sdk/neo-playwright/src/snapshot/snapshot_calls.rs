//! A run's whole-page snapshot calls, as each snapshot op itself timed its call: when that call's
//! own timeout passes on Rust's clock.
//!
//! A v1 snapshot's call goes on after its tree: the prelude reads the tab's delta baseline and
//! records the text as pending (`page.snapshotBaseline`, `page.snapshotPending`). The tree op
//! answers with the id of its call (`snapshotCall`), and both baseline ops name it. Whether that
//! call's timeout has passed is decided here, from what the tree op recorded, and never from
//! anything the realm says: the script shares the realm, so a deadline it reported, or an error
//! it built with a timeout's name and fields, proves nothing. A baseline op whose call's timeout
//! passed fails as that call's `wait_timeout` in the phase it names, with its own op id, which
//! the prelude either passes on or answers by returning the snapshot it holds (neo-playwright-js
//! `snapshot-late.ts`).
//!
//! Each call has its own id, so two snapshots of one tab in flight at once are each held to their
//! own deadline, and one with no timeout to none. A script can name an id itself, but only ever
//! gets Rust's answer about a snapshot call it really made: an unknown id has no deadline.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

/// One run's whole-page snapshot calls by id. Owned by the run's `RunState`, so it ends with the
/// run.
#[derive(Debug, Default)]
pub(crate) struct SnapshotCalls {
    inner: Mutex<Calls>,
}

#[derive(Debug, Default)]
struct Calls {
    /// The last id handed out; ids start at 1.
    last: u32,
    /// Each call's timeout and deadline; `None` for a call with no timeout (`timeout: 0`).
    taken: HashMap<u32, Option<Taken>>,
}

/// A whole page's tree, taken by a call whose timeout passes at `deadline`.
#[derive(Clone, Copy, Debug)]
struct Taken {
    /// The call's timeout, for the message.
    timeout: Duration,
    /// When it passes: the tree op's start plus that timeout.
    deadline: Instant,
}

impl SnapshotCalls {
    /// A whole page's tree was taken by a call whose timeout is `timeout`, passing at
    /// `deadline`; the call's id, which the tree op answers with. A call with no timeout
    /// (`timeout: 0`) has none to pass.
    pub(crate) fn taken(&self, timeout: Option<Duration>, deadline: Option<Instant>) -> u32 {
        let mut calls = self.lock();
        calls.last = calls.last.wrapping_add(1);
        let id = calls.last;
        let taken = timeout
            .zip(deadline)
            .map(|(timeout, deadline)| Taken { timeout, deadline });
        calls.taken.insert(id, taken);
        id
    }

    /// The timeout of call `id` if it had passed by `now`; `None` while it has not, for a call
    /// with no timeout, and for an id this run never handed out.
    pub(crate) fn passed(&self, id: u32, now: Instant) -> Option<Duration> {
        self.lock()
            .taken
            .get(&id)
            .copied()
            .flatten()
            .filter(|taken| now >= taken.deadline)
            .map(|taken| taken.timeout)
    }

    fn lock(&self) -> MutexGuard<'_, Calls> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::Instant;

    use super::SnapshotCalls;

    const TIMEOUT: Duration = Duration::from_millis(5_000);

    #[test]
    fn a_call_times_out_only_once_its_own_deadline_passed() {
        let calls = SnapshotCalls::default();
        let start = Instant::now();
        let deadline = start + TIMEOUT;
        let call = calls.taken(Some(TIMEOUT), Some(deadline));
        assert_eq!(calls.passed(call, start), None, "at the start");
        assert_eq!(
            calls.passed(call, deadline - Duration::from_millis(1)),
            None,
            "1 ms before"
        );
        assert_eq!(calls.passed(call, deadline), Some(TIMEOUT), "on it");
        assert_eq!(
            calls.passed(call, deadline + Duration::from_millis(640)),
            Some(TIMEOUT),
            "640 ms after, as the owner's run 2"
        );
    }

    #[test]
    fn calls_in_flight_together_are_each_held_to_their_own_deadline() {
        let calls = SnapshotCalls::default();
        let now = Instant::now();
        let late = calls.taken(Some(TIMEOUT), Some(now));
        let unlimited = calls.taken(None, None);
        let later = calls.taken(Some(TIMEOUT), Some(now + TIMEOUT));
        assert_ne!(late, unlimited);
        assert_eq!(calls.passed(late, now), Some(TIMEOUT));
        assert_eq!(
            calls.passed(unlimited, now + Duration::from_secs(3_600)),
            None,
            "timeout 0 stays unlimited beside a call that timed out"
        );
        assert_eq!(calls.passed(later, now), None);
    }

    #[test]
    fn an_id_this_run_never_handed_out_has_no_deadline() {
        let calls = SnapshotCalls::default();
        let late = Instant::now() + Duration::from_secs(3_600);
        assert_eq!(calls.passed(1, late), None);
        let call = calls.taken(Some(TIMEOUT), Some(Instant::now()));
        assert_eq!(calls.passed(call.wrapping_add(1), late), None);
    }
}
