//! The order in which each tab's frames attached and its main frame committed documents, as the
//! target watcher saw it. Playwright numbers a frame when it attaches (server/frames.ts, the
//! `Frame` constructor) and renumbers the main frame when it commits a new document, so its seqs
//! follow the page's lifecycle. Neo picks a seq when a world first installs, which is later and
//! in whatever order the reads happen to run, so `FrameWorlds` replays this log first and hands
//! out seqs in Playwright's order.
//!
//! ```text
//!  CDP: Page.frameAttached / frameDetached / frameNavigated, any session of the tab
//!    └─ TargetWatcher (one task) ── record(tab, event) ─▶ FrameAttachLog ─ queue per tab
//!                                                            │
//!  FrameWorlds::world, before it picks a seq ── take(tab) ───┘  replayed in order
//! ```
//!
//! The log only orders seqs; it is never needed for one. A frame whose attach the watcher did
//! not see (a tab opened before it started, a nested frame in a session without the Page domain,
//! an install that ran ahead of the watcher) takes the next seq when its world installs, as it
//! would without the log.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};

use browseros_core::{FrameId, TargetId};

/// How many events one tab keeps before the oldest are dropped. Only tabs no op touches fill up
/// (an op's install takes the queue), and dropping only costs those frames their attach order.
const TAB_CAPACITY: usize = 512;

/// One lifecycle fact of a tab's frames, in the order CDP reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FrameEvent {
    /// A child frame attached (`Page.frameAttached`): Playwright numbers it now.
    Attached(FrameId),
    /// A child frame was removed from its parent (`Page.frameDetached`, reason `remove`). A
    /// frame that only moved to another process (`swap`) keeps its number and is not reported.
    Removed(FrameId),
    /// The main frame committed a new document (`Page.frameNavigated`): Playwright renumbers
    /// the main frame here, before the frames the new document attaches.
    MainCommitted,
}

/// The tabs' queues, shared by the target watcher (the only writer) and every page's
/// `FrameWorlds` (each takes its own tab's queue). A std mutex: holders push or take and let go.
#[derive(Debug, Default)]
pub(crate) struct FrameAttachLog {
    tabs: Mutex<HashMap<TargetId, VecDeque<FrameEvent>>>,
}

impl FrameAttachLog {
    /// Appends `event` to `tab`'s queue.
    pub(crate) fn record(&self, tab: &TargetId, event: FrameEvent) {
        let mut tabs = self.lock();
        let queue = tabs.entry(tab.clone()).or_default();
        if queue.len() == TAB_CAPACITY {
            queue.pop_front();
        }
        queue.push_back(event);
    }

    /// Everything `tab` recorded since the last take, oldest first.
    pub(crate) fn take(&self, tab: &TargetId) -> Vec<FrameEvent> {
        self.lock().remove(tab).map(Vec::from).unwrap_or_default()
    }

    /// Drops `tab`'s queue: the tab closed.
    pub(crate) fn forget(&self, tab: &TargetId) {
        self.lock().remove(tab);
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<TargetId, VecDeque<FrameEvent>>> {
        self.tabs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use browseros_core::{FrameId, TargetId};

    use super::{FrameAttachLog, FrameEvent, TAB_CAPACITY};

    fn frame(id: &str) -> FrameId {
        FrameId(id.to_owned())
    }

    #[test]
    fn tests_that_each_tab_takes_its_own_events_in_order_once() {
        let log = FrameAttachLog::default();
        let (tab, other) = (TargetId("tab".to_owned()), TargetId("other".to_owned()));
        log.record(&tab, FrameEvent::Attached(frame("a")));
        log.record(&other, FrameEvent::MainCommitted);
        log.record(&tab, FrameEvent::Removed(frame("a")));
        assert_eq!(
            log.take(&tab),
            [
                FrameEvent::Attached(frame("a")),
                FrameEvent::Removed(frame("a"))
            ]
        );
        assert!(log.take(&tab).is_empty(), "a take empties the queue");
        assert_eq!(log.take(&other), [FrameEvent::MainCommitted]);
    }

    #[test]
    fn tests_that_a_full_queue_drops_its_oldest_event() {
        let log = FrameAttachLog::default();
        let tab = TargetId("tab".to_owned());
        for n in 0..=TAB_CAPACITY {
            log.record(&tab, FrameEvent::Attached(frame(&n.to_string())));
        }
        let taken = log.take(&tab);
        assert_eq!(taken.len(), TAB_CAPACITY);
        assert_eq!(taken.first(), Some(&FrameEvent::Attached(frame("1"))));
    }
}
