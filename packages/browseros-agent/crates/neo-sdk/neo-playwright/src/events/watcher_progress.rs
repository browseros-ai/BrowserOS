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
//!
//! An event the watcher read but holds back (a tab's event waiting behind that tab's frame-tree
//! read) is not in the log yet. The watcher publishes the oldest held position of each tab
//! ([`Held`]), so a wait names whose held events it needs ([`EventScope`]): an op on one tab
//! waits for that tab's, and another tab's seed read, up to 200 ms while its renderer is busy,
//! never holds it. The other tab's held events land in the log later, behind the op's reply.
//!
//! ```text
//!  read:  ... e7(X) e8(Y, held) e9(X) e10(Y, held) e11(X)       handled = 12
//!  held:  Y -> 8
//!  wait for fence 12 on [X]:        applied through 12: done
//!  wait for fence 12 on every tab:  applied through min(12, 8) = 8: waits for Y's read
//! ```

use std::collections::HashMap;
use std::pin::pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use browseros_core::TargetId;
use tokio::sync::Notify;

/// The client's counts read after an op's last reply: what the watcher must have handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fence {
    /// Events broadcast.
    pub(crate) events: u64,
    /// `Runtime.bindingCalled` events the targeted router handed out.
    pub(crate) calls: u64,
}

/// Whose held events a wait for the watcher covers ([`WatcherProgress::applied_through`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum EventScope {
    /// Every tab's: the whole log before the fence, as a context-level listener or a caller
    /// that reads every tab's records needs.
    #[default]
    Every,
    /// Only these tabs' held events, and the download begins no tab owns yet (each could turn
    /// out to be one of theirs). Another tab's events held behind its tree read do not hold the
    /// wait: they are recorded later, after the events the wait did cover.
    Tabs(Vec<TargetId>),
}

impl EventScope {
    /// Whether a wait on this scope covered `target`'s held events: a tab whose target changed
    /// after the scope was taken (core replaced it) was not waited for.
    #[must_use]
    pub fn covers(&self, target: &TargetId) -> bool {
        match self {
            Self::Every => true,
            Self::Tabs(tabs) => tabs.contains(target),
        }
    }
}

/// The broadcast positions the watcher holds back, by whose events they are: what it publishes
/// with [`WatcherProgress::hold`] each time a line or a waiting download changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Held {
    /// Each tab's oldest held position. A tab counts its own line's, and a popup still waiting
    /// to be reported counts its line's for its opener too: a download in a tab that opened only
    /// to download is the opener's (`Watcher::download`).
    pub(crate) tabs: HashMap<TargetId, u64>,
    /// The oldest download begin no tab owns yet (`UnownedDownload`): it may be any tab's, so
    /// every scope counts it.
    pub(crate) unowned: Option<u64>,
}

impl Held {
    /// The oldest position `scope` must wait for, or `None` when it holds none of them.
    fn oldest(&self, scope: &EventScope) -> Option<u64> {
        let tabs = match scope {
            EventScope::Every => self.tabs.values().min().copied(),
            EventScope::Tabs(tabs) => tabs
                .iter()
                .filter_map(|tab| self.tabs.get(tab))
                .min()
                .copied(),
        };
        tabs.into_iter().chain(self.unowned).min()
    }
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
    /// The broadcast events the watcher read but holds back, by tab (a tab's frame event
    /// waiting behind a frame-tree read, `target_watcher.rs`). `handled` counts a held event;
    /// the log does not have it yet. A short lock: the watcher replaces it whole, and a waiter
    /// reads one minimum.
    held: Mutex<Held>,
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

    /// Broadcast events read so far, held ones included, on the client's numbering (`u64::MAX`
    /// once stopped): how far the watcher's own reading got.
    pub(crate) fn events_handled(&self) -> u64 {
        self.handled.load(Ordering::Acquire)
    }

    /// How many broadcast events are in the log for `scope`, counted from the first: every event
    /// before this position was handled, and none of `scope`'s is held back.
    pub(crate) fn applied_through(&self, scope: &EventScope) -> u64 {
        let handled = self.handled.load(Ordering::Acquire);
        let held = self.lock_held().oldest(scope);
        held.map_or(handled, |held| handled.min(held))
    }

    /// The watcher now holds back exactly `held`.
    pub(crate) fn hold(&self, held: Held) {
        *self.lock_held() = held;
        self.changed.notify_waiters();
    }

    fn lock_held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
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
        *self.lock_held() = Held::default();
        self.changed.notify_waiters();
    }

    /// Returns once the watcher handled every event and call `fence` counts, and none of
    /// `scope`'s before it is held back. The caller bounds the wait.
    pub(crate) async fn reached(&self, fence: Fence, scope: &EventScope) {
        loop {
            let mut changed = pin!(self.changed.notified());
            changed.as_mut().enable();
            if self.applied_through(scope) >= fence.events
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
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use browseros_core::TargetId;
    use tokio::time;

    use super::{EventScope, Fence, Held, WatcherProgress};

    fn tab(name: &str) -> TargetId {
        TargetId(name.to_owned())
    }

    /// `tabs` hold from their positions, and no download waits.
    fn held(tabs: &[(&str, u64)]) -> Held {
        Held {
            tabs: tabs.iter().map(|(name, at)| (tab(name), *at)).collect(),
            unowned: None,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_wait_ends_when_the_watcher_counts_past_the_fence() {
        let progress = Arc::new(WatcherProgress::default());
        progress.start(10, 4);
        let waiter = tokio::spawn({
            let progress = Arc::clone(&progress);
            async move {
                progress
                    .reached(
                        Fence {
                            events: 12,
                            calls: 5,
                        },
                        &EventScope::Every,
                    )
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
            time::timeout(
                Duration::from_secs(1),
                progress.reached(earlier, &EventScope::Every)
            )
            .await
            .is_ok(),
            "an earlier fence is reached already"
        );
    }

    /// An event the watcher read but holds back is not in the log yet: a fence past it waits
    /// until the watcher lets it go, though it counted the event as read.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_held_event_keeps_a_fence_past_it_waiting() {
        let progress = Arc::new(WatcherProgress::default());
        progress.start(0, 0);
        progress.handled(5);
        progress.hold(held(&[("x", 2)]));
        assert_eq!(progress.events_handled(), 5);
        assert_eq!(
            progress.applied_through(&EventScope::Every),
            2,
            "events 0 and 1 are in the log"
        );
        let fence = Fence {
            events: 3,
            calls: 0,
        };
        let waiter = tokio::spawn({
            let progress = Arc::clone(&progress);
            async move {
                progress
                    .reached(fence, &EventScope::Tabs(vec![tab("x")]))
                    .await;
            }
        });
        time::sleep(Duration::from_millis(1)).await;
        assert!(!waiter.is_finished(), "event 2 is held");
        progress.hold(Held::default());
        assert!(time::timeout(Duration::from_secs(1), waiter).await.is_ok());
    }

    /// One tab's held events hold only the waits that name it: an op on another tab goes on
    /// while the busy tab's tree read is out, and a wait on every tab still waits for them.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_wait_on_one_tab_passes_another_tabs_held_events() {
        let progress = Arc::new(WatcherProgress::default());
        progress.start(0, 0);
        progress.handled(12);
        progress.hold(held(&[("busy", 8), ("slow", 10)]));
        let on = |names: &[&str]| EventScope::Tabs(names.iter().map(|name| tab(name)).collect());
        assert_eq!(
            progress.applied_through(&on(&["quiet"])),
            12,
            "nothing of its own held"
        );
        assert_eq!(progress.applied_through(&on(&["busy"])), 8);
        assert_eq!(progress.applied_through(&on(&["quiet", "slow"])), 10);
        assert_eq!(
            progress.applied_through(&on(&["slow", "busy"])),
            8,
            "the oldest it names"
        );
        assert_eq!(progress.applied_through(&EventScope::Every), 8);
        let fence = Fence {
            events: 12,
            calls: 0,
        };
        assert!(
            time::timeout(
                Duration::from_secs(1),
                progress.reached(fence, &on(&["quiet"]))
            )
            .await
            .is_ok(),
            "the quiet tab's wait does not wait for the busy tab"
        );
        let every = tokio::spawn({
            let progress = Arc::clone(&progress);
            async move { progress.reached(fence, &EventScope::Every).await }
        });
        time::sleep(Duration::from_millis(1)).await;
        assert!(
            !every.is_finished(),
            "a wait on every tab waits for the held events"
        );
        progress.hold(Held::default());
        assert!(time::timeout(Duration::from_secs(1), every).await.is_ok());
    }

    /// A download begin no tab owns yet may be any tab's, so it holds every wait, whichever tabs
    /// it names.
    #[test]
    fn tests_that_a_download_no_tab_owns_holds_every_tab() {
        let progress = WatcherProgress::default();
        progress.start(0, 0);
        progress.handled(9);
        progress.hold(Held {
            tabs: HashMap::new(),
            unowned: Some(4),
        });
        assert_eq!(
            progress.applied_through(&EventScope::Tabs(vec![tab("x")])),
            4
        );
        assert_eq!(progress.applied_through(&EventScope::Tabs(Vec::new())), 4);
        assert_eq!(progress.applied_through(&EventScope::Every), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_stopped_watcher_holds_no_one() {
        let progress = WatcherProgress::default();
        progress.hold(held(&[("x", 0)]));
        progress.stopped();
        let fence = Fence {
            events: u64::MAX - 1,
            calls: u64::MAX - 1,
        };
        assert!(
            time::timeout(
                Duration::from_secs(1),
                progress.reached(fence, &EventScope::Tabs(vec![tab("x")]))
            )
            .await
            .is_ok()
        );
    }
}
