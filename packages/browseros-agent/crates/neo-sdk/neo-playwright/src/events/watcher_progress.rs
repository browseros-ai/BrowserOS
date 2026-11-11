//! How far the target watcher has read the connection's events, so an op can wait until every
//! event that arrived before its last reply is in the log.
//!
//! Upstream handles protocol messages one at a time, so the `Runtime.consoleAPICalled` an
//! evaluate caused has reached its listeners before the evaluate's reply resolves. In Neo the
//! watcher reads events on a task of its own, and an op can finish before the watcher logged an
//! event that came first on the wire. The CDP client numbers the events it broadcasts
//! (`BrowserSession::cdp_events_broadcast`), and the watcher counts what it handled on the same
//! numbering, so once the watcher's count reaches the client's count read after a reply, every
//! earlier event is logged.

use std::pin::pin;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::Notify;

/// Shared by the watcher (the only writer) and every op that waits for it.
#[derive(Debug, Default)]
pub(crate) struct WatcherProgress {
    /// Events handled, on the client's numbering: the client's count when the watcher
    /// subscribed, then one per event handled and every event a lag skipped. `u64::MAX` once
    /// the watcher stopped, so no one waits for it.
    handled: AtomicU64,
    changed: Notify,
}

impl WatcherProgress {
    /// The watcher subscribed when the client had broadcast `base` events. Read after the
    /// subscription, so it can only count an event the receiver also gets, never miss one.
    pub(crate) fn start(&self, base: u64) {
        self.handled.store(base, Ordering::Release);
        self.changed.notify_waiters();
    }

    /// The watcher handled, or a lag skipped, `count` more events.
    pub(crate) fn handled(&self, count: u64) {
        self.handled.fetch_add(count, Ordering::AcqRel);
        self.changed.notify_waiters();
    }

    /// The watcher ended (its connection closed, or no runtime ran it).
    pub(crate) fn stopped(&self) {
        self.handled.store(u64::MAX, Ordering::Release);
        self.changed.notify_waiters();
    }

    /// Returns once the watcher handled every event the client had broadcast when it counted
    /// `broadcast`. The caller bounds the wait.
    pub(crate) async fn reached(&self, broadcast: u64) {
        loop {
            let mut changed = pin!(self.changed.notified());
            changed.as_mut().enable();
            if self.handled.load(Ordering::Acquire) >= broadcast {
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

    use super::WatcherProgress;

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_wait_ends_when_the_watcher_counts_past_the_fence() {
        let progress = Arc::new(WatcherProgress::default());
        progress.start(10);
        let waiter = tokio::spawn({
            let progress = Arc::clone(&progress);
            async move { progress.reached(12).await }
        });
        progress.handled(1);
        time::sleep(Duration::from_millis(1)).await;
        assert!(!waiter.is_finished(), "one event short of the fence");
        progress.handled(1);
        assert!(time::timeout(Duration::from_secs(1), waiter).await.is_ok());
        assert!(
            time::timeout(Duration::from_secs(1), progress.reached(5))
                .await
                .is_ok(),
            "an earlier fence is reached already"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_stopped_watcher_holds_no_one() {
        let progress = WatcherProgress::default();
        progress.stopped();
        assert!(
            time::timeout(Duration::from_secs(1), progress.reached(u64::MAX - 1))
                .await
                .is_ok()
        );
    }
}
