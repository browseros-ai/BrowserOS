//! How far the target watcher has read the connection's events, so an op can wait until every
//! event that arrived before its last reply is in the log, or in a run's binding-call queue.
//!
//! Upstream handles protocol messages one at a time, so the `Runtime.consoleAPICalled` an
//! evaluate caused, or the binding call it made, has reached the script before the evaluate's
//! reply resolves. In Neo the watcher reads events on a task of its own, and an op can finish
//! before the watcher handled an event that came first on the wire. The watcher reads two
//! streams, and the CDP client numbers both: the broadcast
//! (`BrowserSession::cdp_events_broadcast`), and the `Runtime.bindingCalled` events the targeted
//! router hands its subscribers (`BrowserSession::cdp_events_targeted_delivered`). The watcher
//! counts what it handled on the same numbering, so once its counts reach the client's counts
//! read after a reply ([`Fence`]), every earlier event was handled.

use std::pin::pin;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::Notify;

/// The client's counts read after an op's last reply: what the watcher must have handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fence {
    /// Events broadcast.
    pub(crate) events: u64,
    /// `Runtime.bindingCalled` events the targeted router handed out.
    pub(crate) calls: u64,
}

/// Shared by the watcher (the only writer) and every op that waits for it.
#[derive(Debug, Default)]
pub(crate) struct WatcherProgress {
    /// Broadcast events handled, on the client's numbering: the client's count when the watcher
    /// subscribed, then one per event handled and every event a lag skipped. `u64::MAX` once
    /// the watcher stopped, so no one waits for it.
    handled: AtomicU64,
    /// Binding calls handled, on the router's numbering for `Runtime.bindingCalled`: its count
    /// when the watcher subscribed, then one per call handled. `u64::MAX` once stopped.
    calls: AtomicU64,
    changed: Notify,
}

impl WatcherProgress {
    /// The watcher subscribed when the client had broadcast `events` events and handed out
    /// `calls` binding calls, which its receivers never get (`cdp_events_counted` and
    /// `cdp_events_targeted_counted` take each count with its subscription).
    pub(crate) fn start(&self, events: u64, calls: u64) {
        self.handled.store(events, Ordering::Release);
        self.calls.store(calls, Ordering::Release);
        self.changed.notify_waiters();
    }

    /// Broadcast events handled so far, on the client's numbering (`u64::MAX` once stopped).
    pub(crate) fn events_handled(&self) -> u64 {
        self.handled.load(Ordering::Acquire)
    }

    /// The watcher handled, or a lag skipped, `count` more broadcast events.
    pub(crate) fn handled(&self, count: u64) {
        self.handled.fetch_add(count, Ordering::AcqRel);
        self.changed.notify_waiters();
    }

    /// The watcher handled one more binding call: it is in the run's queue, if a run wants it.
    pub(crate) fn call_handled(&self) {
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.changed.notify_waiters();
    }

    /// The watcher ended (its connection closed, or no runtime ran it).
    pub(crate) fn stopped(&self) {
        self.handled.store(u64::MAX, Ordering::Release);
        self.calls.store(u64::MAX, Ordering::Release);
        self.changed.notify_waiters();
    }

    /// Returns once the watcher handled every event and call `fence` counts. The caller bounds
    /// the wait.
    pub(crate) async fn reached(&self, fence: Fence) {
        loop {
            let mut changed = pin!(self.changed.notified());
            changed.as_mut().enable();
            if self.handled.load(Ordering::Acquire) >= fence.events
                && self.calls.load(Ordering::Acquire) >= fence.calls
            {
                return;
            }
            changed.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::time;

    use super::{Fence, WatcherProgress};

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_wait_ends_when_the_watcher_counts_past_the_fence() {
        let progress = Arc::new(WatcherProgress::default());
        progress.start(10, 4);
        let waiter = tokio::spawn({
            let progress = Arc::clone(&progress);
            async move {
                progress
                    .reached(Fence {
                        events: 12,
                        calls: 5,
                    })
                    .await;
            }
        });
        progress.handled(1);
        time::sleep(Duration::from_millis(1)).await;
        assert!(!waiter.is_finished(), "one event short of the fence");
        progress.handled(1);
        time::sleep(Duration::from_millis(1)).await;
        assert!(!waiter.is_finished(), "every event, but one call short");
        progress.call_handled();
        assert!(time::timeout(Duration::from_secs(1), waiter).await.is_ok());
        let earlier = Fence {
            events: 5,
            calls: 0,
        };
        assert!(
            time::timeout(Duration::from_secs(1), progress.reached(earlier))
                .await
                .is_ok(),
            "an earlier fence is reached already"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_stopped_watcher_holds_no_one() {
        let progress = WatcherProgress::default();
        progress.stopped();
        let fence = Fence {
            events: u64::MAX - 1,
            calls: u64::MAX - 1,
        };
        assert!(
            time::timeout(Duration::from_secs(1), progress.reached(fence))
                .await
                .is_ok()
        );
    }
}
