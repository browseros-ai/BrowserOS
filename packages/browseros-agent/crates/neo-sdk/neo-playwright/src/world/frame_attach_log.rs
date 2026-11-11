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

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use browseros_core::{FrameId, TargetId};

/// How many events one tab queues before it is compacted. Only a tab no op reads fills up (an
/// install takes its tab's queue), such as an owner's tab whose ads keep replacing iframes.
const TAB_CAPACITY: usize = 512;

/// One lifecycle fact of a tab's frames, in the order CDP reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FrameEvent {
    /// A child frame attached (`Page.frameAttached`): Playwright numbers it now.
    Attached(FrameId),
    /// A child frame was removed from its parent (`Page.frameDetached`, reason `remove`). A
    /// frame that only moved to another process (`swap`) keeps its number and is not reported.
    Removed(FrameId),
    /// The main frame committed a new document at `url` (`Page.frameNavigated`). Playwright
    /// renumbers the main frame here when the document it leaves had a real URL (neither empty
    /// nor `about:blank`), before the frames the new document attaches.
    MainCommitted { url: String },
    /// This many frames attached and were removed again while the queue waited. Upstream
    /// spent a seq on each, so the replay spends as many; compaction writes it.
    Skipped(u32),
    /// Compaction dropped events here, a main commit perhaps, so the replay treats it as a gap.
    Lost,
}

/// The tabs' queues, shared by the target watcher (the only writer) and every page's
/// `FrameWorlds` (each takes its own tab's queue). A std mutex: holders push or take and let go.
#[derive(Debug, Default)]
pub(crate) struct FrameAttachLog {
    tabs: Mutex<HashMap<TargetId, VecDeque<FrameEvent>>>,
    /// How many times the watcher fell behind CDP and lost events, for every tab at once. A
    /// page that sees it moved treats its main frame as navigated away from a real page, so a
    /// lost commit can never leave a new document under the old one's seq.
    gaps: AtomicU64,
}

/// One tab's take: its events, and how many gaps the log has had in all.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Taken {
    pub(crate) events: Vec<FrameEvent>,
    pub(crate) gaps: u64,
}

impl FrameAttachLog {
    /// Appends `event` to `tab`'s queue, compacting a full one.
    pub(crate) fn record(&self, tab: &TargetId, event: FrameEvent) {
        let mut tabs = self.lock();
        let queue = tabs.entry(tab.clone()).or_default();
        queue.push_back(event);
        if queue.len() > TAB_CAPACITY {
            compact(queue);
        }
    }

    /// Everything `tab` recorded since the last take, oldest first, and the log's gap count.
    pub(crate) fn take(&self, tab: &TargetId) -> Taken {
        let events = self.lock().remove(tab).map(Vec::from).unwrap_or_default();
        Taken {
            events,
            gaps: self.gaps.load(Ordering::Acquire),
        }
    }

    /// The watcher lost events: any tab may have committed documents nobody logged.
    pub(crate) fn lost_events(&self) {
        self.gaps.fetch_add(1, Ordering::AcqRel);
    }

    /// Drops `tab`'s queue: the tab closed.
    pub(crate) fn forget(&self, tab: &TargetId) {
        self.lock().remove(tab);
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<TargetId, VecDeque<FrameEvent>>> {
        self.tabs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Folds every frame that attached and was removed within the queue into a `Skipped` count at
/// its attach's place, so the replay spends the same seqs in the same order and only frames
/// still attached keep an entry. A queue still full after that (more live frames than it
/// holds) drops its oldest entries and starts with `Lost` instead: those frames lose their
/// attach order, and the main frame moves on as after a gap, in case a commit was among them.
fn compact(queue: &mut VecDeque<FrameEvent>) {
    let attached: HashSet<FrameId> = queue
        .iter()
        .filter_map(|event| match event {
            FrameEvent::Attached(frame) => Some(frame.clone()),
            _ => None,
        })
        .collect();
    let gone: HashSet<FrameId> = queue
        .iter()
        .filter_map(|event| match event {
            FrameEvent::Removed(frame) if attached.contains(frame) => Some(frame.clone()),
            _ => None,
        })
        .collect();
    let mut kept = VecDeque::with_capacity(queue.len());
    for event in queue.drain(..) {
        let spent = match &event {
            FrameEvent::Attached(frame) if gone.contains(frame) => 1,
            FrameEvent::Removed(frame) if gone.contains(frame) => continue,
            FrameEvent::Skipped(spent) => *spent,
            _ => 0,
        };
        if spent == 0 {
            kept.push_back(event);
        } else if let Some(FrameEvent::Skipped(total)) = kept.back_mut() {
            *total = total.saturating_add(spent);
        } else {
            kept.push_back(FrameEvent::Skipped(spent));
        }
    }
    if kept.len() > TAB_CAPACITY {
        while kept.len() >= TAB_CAPACITY {
            kept.pop_front();
        }
        if kept.front() != Some(&FrameEvent::Lost) {
            kept.push_front(FrameEvent::Lost);
        }
    }
    *queue = kept;
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_core::{FrameId, TargetId};

    use super::{FrameAttachLog, FrameEvent, TAB_CAPACITY};

    type TestResult = Result<(), Box<dyn Error>>;

    fn frame(id: &str) -> FrameId {
        FrameId(id.to_owned())
    }

    #[test]
    fn tests_that_each_tab_takes_its_own_events_in_order_once() {
        let log = FrameAttachLog::default();
        let (tab, other) = (TargetId("tab".to_owned()), TargetId("other".to_owned()));
        log.record(&tab, FrameEvent::Attached(frame("a")));
        log.record(
            &other,
            FrameEvent::MainCommitted {
                url: "about:blank".to_owned(),
            },
        );
        log.record(&tab, FrameEvent::Removed(frame("a")));
        assert_eq!(
            log.take(&tab).events,
            [
                FrameEvent::Attached(frame("a")),
                FrameEvent::Removed(frame("a"))
            ]
        );
        assert!(log.take(&tab).events.is_empty(), "a take empties the queue");
        assert_eq!(
            log.take(&other).events,
            [FrameEvent::MainCommitted {
                url: "about:blank".to_owned()
            }]
        );
    }

    /// A full queue keeps every frame still attached, in order, and counts the removed ones
    /// where they attached.
    #[test]
    fn tests_that_a_full_queue_folds_removed_frames_into_a_count() -> TestResult {
        let log = FrameAttachLog::default();
        let tab = TargetId("tab".to_owned());
        log.record(&tab, FrameEvent::Attached(frame("first")));
        log.record(&tab, FrameEvent::Removed(frame("before")));
        for n in 0..TAB_CAPACITY {
            log.record(&tab, FrameEvent::Attached(frame(&n.to_string())));
            log.record(&tab, FrameEvent::Removed(frame(&n.to_string())));
        }
        log.record(&tab, FrameEvent::Attached(frame("last")));
        let taken = log.take(&tab).events;
        assert!(taken.len() <= TAB_CAPACITY, "{} events", taken.len());
        assert_eq!(taken.first(), Some(&FrameEvent::Attached(frame("first"))));
        assert_eq!(
            taken.get(1),
            Some(&FrameEvent::Removed(frame("before"))),
            "a frame numbered before the queue started is still reported removed"
        );
        assert_eq!(taken.last(), Some(&FrameEvent::Attached(frame("last"))));
        let spent: u32 = taken
            .iter()
            .map(|event| match event {
                FrameEvent::Skipped(spent) => *spent,
                FrameEvent::Attached(_) => 1,
                FrameEvent::Removed(_) | FrameEvent::MainCommitted { .. } | FrameEvent::Lost => 0,
            })
            .sum();
        assert_eq!(
            usize::try_from(spent)?,
            TAB_CAPACITY + 2,
            "every attach spends a seq"
        );
        Ok(())
    }

    /// More live frames than the queue holds: the oldest go, and a `Lost` marks where.
    #[test]
    fn tests_that_a_queue_of_live_frames_drops_its_oldest_behind_a_marker() {
        let log = FrameAttachLog::default();
        let tab = TargetId("tab".to_owned());
        log.record(
            &tab,
            FrameEvent::MainCommitted {
                url: "https://a.test/".to_owned(),
            },
        );
        for n in 0..=TAB_CAPACITY {
            log.record(&tab, FrameEvent::Attached(frame(&n.to_string())));
        }
        let taken = log.take(&tab).events;
        assert_eq!(taken.len(), TAB_CAPACITY);
        assert_eq!(taken.first(), Some(&FrameEvent::Lost));
        assert_eq!(
            taken.last(),
            Some(&FrameEvent::Attached(frame(&TAB_CAPACITY.to_string())))
        );
    }
}
