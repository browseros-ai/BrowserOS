//! The process-wide event log: one writer, many readers, one ring of records per tab.
//!
//! ```text
//!  EventWriter::record ── seq from one counter ──▶ the record's tab's ring, or the no-tab ring
//!                                                  open: live tabs' rings + the no-tab ring
//!                                                  closed: closed tabs' rings, earliest first
//!  past LOG_CAPACITY in one ring   its oldest record goes
//!  past LOG_TOTAL in all rings     1. the earliest-closed ring is cut to its `Closed`
//!                                  2. only then, the fullest open ring's oldest record goes
//!  past CLOSED_RINGS closed rings  the earliest-closed ring goes, folded into one watermark
//!
//!  read_tab_after(tab)  that tab's ring + the no-tab ring; loss from those two only
//!  read_after           every ring, merged by seq; loss from all of them
//! ```
//!
//! **Why per tab.** A tab's records go into its own ring, so a busy tab, touched or not (an ad
//! page churning cross-site iframes writes 20 to 45 records a second, measured), pushes out only
//! its own history: a reader of another tab keeps its records however long it does not read.
//! **Why one counter.** Seqs stay global, given once by the one writer as it applies an event,
//! so a merged read has the order the single ring had, and every reader's place is one number.

use std::cmp::Reverse;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::{iter, mem};

use browseros_core::{FrameId, TabId, TargetId};
use neo_trace::EpochMillis;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Notify;
use tokio::sync::futures::Notified;

use crate::RunState;
use crate::events::dialog_table::{self, DialogAnswer, DialogTable, Unanswerable};
use crate::events::download_table::DownloadTable;
use crate::events::frame_lifecycle::EventFrame;
use crate::events::log_lease::LogLeases;
use crate::events::page_event::{BindingCalled, DialogId, DialogKind, PageEvent};
use crate::events::run_events::QueuedCall;
use crate::events::tab_frames::{MissingSeeds, TabFrameList, TabFrames};
use crate::events::{PageEventKind, Subscription};

/// How many records each ring keeps: one tab's, or the no-tab ring's. A reader that falls further
/// behind on a ring it reads is told it lost records.
pub(crate) const LOG_CAPACITY: usize = 1024;

/// How many records all rings hold together: about 8 to 24 MB at 0.5 to 1.5 KB a record
/// (estimated from the types). Past it, closed tabs' rings give way first, then the open ring
/// holding the most ([`EventLog::shed`]), so an open tab keeps its share of the open rings.
pub(crate) const LOG_TOTAL: usize = 16 * LOG_CAPACITY;

/// How many closed tabs' rings stay. Past it the earliest-closed ring goes, folded into
/// `EventLog::folded`, so a long session's closed tabs cannot grow the log or crowd open tabs.
pub(crate) const CLOSED_RINGS: usize = 1024;

/// The log of page events since the `Playwright` started, the runs that read it, the dialogs
/// held for them, and each download's progress. Shared by the facade (readers) and the target
/// watcher (the writer).
#[derive(Debug, Default)]
pub(crate) struct EventHub {
    /// A std mutex: every holder copies or appends and lets go, never across an await.
    log: Mutex<EventLog>,
    /// Live worker endpoints are separate from the bounded event journal.
    pub(crate) workers: super::worker_table::WorkerTable,
    /// Woken after every record, and when a run adds a listener, so readers re-read.
    changed: Notify,
    /// Runs that used an event op, weakly: a run's state dies with the run. Pruned as it goes.
    runs: Mutex<Vec<Weak<RunState>>>,
    dialogs: Mutex<DialogTable>,
    downloads: DownloadTable,
    /// Which run owns each binding name on a tab, weakly: a name is the run's until the run
    /// ends. The page's controller keeps one binding per name, so two runs on one tab must not
    /// share one, or each would answer and remove the other's.
    bindings: Mutex<HashMap<(TargetId, String), Weak<RunState>>>,
    /// `Log` on the sessions whose console a run listens to (pages and their frames) and where
    /// `addScriptTag` reads CSP reports; the watcher reads each span's replay floor from it.
    log_leases: LogLeases,
}

#[derive(Debug, Default)]
struct EventLog {
    /// Records the watcher could not file under a tab: a frame record of a session it could not
    /// place, a download of no page. Always open; every one-tab read includes it.
    no_tab: Ring,
    /// Each live tab's ring, made at its first record.
    open: HashMap<TargetId, Ring>,
    /// Each closed tab's ring: the record of its close made it closed ([`EventLog::close`]).
    closed: HashMap<TargetId, Ring>,
    /// `closed`'s targets, earliest close first: the order rings are cut and removed in.
    closing_order: VecDeque<TargetId>,
    /// How many of `closing_order`, from the front, are cut to their `Closed` already. Cutting
    /// moves front to back and removal takes the front, so the cut ones are always a prefix.
    cut: usize,
    /// The newest record of every ring removed past [`CLOSED_RINGS`]: a reader of such a tab,
    /// whose ring is gone, reads its loss from here.
    folded: Mark,
    /// How many records the removed rings held or had dropped, to bound that loss's count.
    folded_count: u64,
    /// The last [`CLOSED_RINGS`] removed rings, each with its own newest record and count, in
    /// `folded_order`. A reader of such a tab reads its loss from its own fold, so other tabs'
    /// later removals never read as its loss; a record that arrives for one late (a popup's
    /// window lookup filing under an opener long gone) goes into its fold, rather than opening a
    /// ring for a dead tab that would count as live forever.
    folded_targets: HashMap<TargetId, Fold>,
    folded_order: VecDeque<TargetId>,
    /// Records held across every ring, against [`LOG_TOTAL`].
    total: usize,
    /// The seq the last record got; seqs start at 1, so 0 reads "from the start".
    last_seq: u64,
    /// Each open tab's newest main-frame navigation, kept apart from the rings so a bounded ring
    /// cannot evict one the host has not refreshed its URL facts for. The watcher marks
    /// them (`EventWriter::main_frame_navigated`); a tab's entry goes with its recorded close.
    navigated: HashMap<TargetId, u64>,
    /// Every tab's frames and frame-tree version, and each target's tab id ([`TabFrames`]):
    /// under the log's lock, so a list, its version and the records it includes change together.
    /// The version moves when a child frame attaches, detaches or navigates, when an
    /// out-of-process frame's session comes or goes, and when the main frame commits, so a run
    /// that holds the tab's tree knows it changed (every reply carries them,
    /// `EventHub::frame_versions`). Only the browser's own events move it, never a read of the
    /// tree. Kept apart from the rings, as `navigated` is; a tab's version goes with its close.
    /// Its tab ids also tell a live tab with no ring yet from one whose closed ring was removed
    /// ([`EventLog::tab_lost`]).
    frames: TabFrames,
}

impl EventLog {
    /// Appends `event` as the next seq to the ring of the tab it names ([`PageEvent::tab`]).
    /// The frame-only kinds name none: the watcher files those with [`EventWriter::record_in`].
    fn append(&mut self, event: PageEvent) -> u64 {
        let tab = self.ring_of(&event);
        self.append_in(tab, event)
    }

    /// The ring `event` goes to: the tab it names ([`PageEvent::tab`]), except a popup's
    /// `Opened` once its opener is known closed. `page.on('popup')` cannot fire on a closed
    /// opener, and its ring may be cut or removed, which would drop the record a context `page`
    /// listener needs for a live tab; it goes to the popup's own ring instead.
    fn ring_of(&self, event: &PageEvent) -> Option<TargetId> {
        match event {
            PageEvent::Opened(opened) => match &opened.opener {
                Some(opener)
                    if !self.closed.contains_key(opener)
                        && !self.folded_targets.contains_key(opener) =>
                {
                    Some(opener.clone())
                }
                _ => Some(opened.target.clone()),
            },
            _ => event.tab().cloned(),
        }
    }

    /// Appends `event` as the next seq to `tab`'s ring (the no-tab ring for `None`), keeping
    /// the rings within their caps ([`EventLog::shed`]). A tab's `Closed` forgets its navigation
    /// mark, version and tab id, and closes its ring.
    fn append_in(&mut self, tab: Option<TargetId>, event: PageEvent) -> u64 {
        self.last_seq += 1;
        let seq = self.last_seq;
        let closes = match &event {
            PageEvent::Closed(target) => {
                self.forget(target);
                (tab.as_ref() == Some(target)).then(|| target.clone())
            }
            _ => None,
        };
        self.push(
            tab,
            EventRecord {
                seq,
                at_ms: now_ms(),
                event,
            },
        );
        if let Some(target) = closes {
            self.close(&target);
        }
        self.shed();
        seq
    }
}

/// One tab's records, or the no-tab ring's, oldest first, and what it dropped.
#[derive(Debug, Default)]
struct Ring {
    /// Seqs grow from front to back and skip the seqs other rings took.
    records: VecDeque<EventRecord>,
    /// How many records this ring ever dropped.
    dropped: u64,
    /// The newest record it dropped: a reader whose place is older lost something here.
    newest_dropped: Mark,
    state: RingState,
}

/// Where a ring stands in [`EventLog::shed`]'s order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RingState {
    /// A live tab's ring, or the no-tab ring: it shares what is left once closed rings are cut.
    #[default]
    Open,
    /// Its tab's close was recorded; it waits in `EventLog::closing_order` to be cut.
    Closed,
    /// Cut to its `Closed` records under pressure. A record replayed into it later is dropped at
    /// once ([`EventLog::push`]), so a cut ring never holds more than its close and the cut order
    /// never needs revisiting.
    Cut,
}

/// What a removed ring leaves behind: its newest record and how many records it held or had
/// dropped, for its tab's readers ([`EventLog::tab_lost`]).
#[derive(Clone, Copy, Debug, Default)]
struct Fold {
    newest: Mark,
    count: u64,
}

/// A record's seq and stamp, kept for what a ring dropped; seq 0 is "nothing".
#[derive(Clone, Copy, Debug, Default)]
struct Mark {
    seq: u64,
    at_ms: u64,
}

impl Mark {
    fn of(record: &EventRecord) -> Self {
        Self {
            seq: record.seq,
            at_ms: record.at_ms,
        }
    }

    /// Records newer than `after` among `count` dropped ones whose newest is this mark: non-zero
    /// exactly when one was newer. The count is exact when those records took every seq in
    /// between (one ring writing alone), and an upper bound otherwise; readers act on `> 0`.
    fn lost_after(self, count: u64, after: u64) -> u64 {
        if self.seq <= after {
            return 0;
        }
        count.min(self.seq - after)
    }

    /// The newer of two marks.
    fn max_seq(self, other: Self) -> Self {
        if other.seq > self.seq { other } else { self }
    }

    /// This mark's seq when it is older than `since_ms`, for [`EventHub::position_at`].
    fn before(self, since_ms: u64) -> Option<u64> {
        (self.seq > 0 && self.at_ms < since_ms).then_some(self.seq)
    }
}

impl Ring {
    /// The records after seq `after`, oldest first.
    fn after(&self, after: u64) -> impl Iterator<Item = &EventRecord> {
        let start = self.records.partition_point(|record| record.seq <= after);
        self.records.range(start..)
    }

    /// How many records after seq `after` this ring still holds.
    fn held_after(&self, after: u64) -> usize {
        self.records.len() - self.records.partition_point(|record| record.seq <= after)
    }

    fn lost_after(&self, after: u64) -> u64 {
        self.newest_dropped.lost_after(self.dropped, after)
    }

    fn note_dropped(&mut self, record: &EventRecord) {
        self.dropped += 1;
        if record.seq > self.newest_dropped.seq {
            self.newest_dropped = Mark::of(record);
        }
    }

    /// Drops the oldest record; whether there was one.
    fn drop_oldest(&mut self) -> bool {
        let Some(record) = self.records.pop_front() else {
            return false;
        };
        self.note_dropped(&record);
        true
    }

    /// Keeps only its `Closed` records, which the host's close lookup and a reader's close
    /// both need, and returns how many records went.
    fn cut(&mut self) -> usize {
        let before = self.records.len();
        for record in mem::take(&mut self.records) {
            if matches!(record.event, PageEvent::Closed(_)) {
                self.records.push_back(record);
            } else {
                self.note_dropped(&record);
            }
        }
        self.state = RingState::Cut;
        before - self.records.len()
    }

    /// The newest record this ring knows of, held or dropped: what removing it folds.
    fn newest(&self) -> Mark {
        self.records
            .back()
            .map_or(self.newest_dropped, Mark::of)
            .max_seq(self.newest_dropped)
    }

    /// The newest seq this ring knows to be older than `since_ms`, held or dropped. A dropped
    /// record counts only through the newest one, as it is the only one this ring remembers.
    /// A binary search: stamps rise within a ring (one writer, one clock), so the lock is held
    /// for a few steps per ring, not a walk of every record.
    fn newest_before(&self, since_ms: u64) -> Option<u64> {
        let older = self
            .records
            .partition_point(|record| record.at_ms < since_ms);
        let held = older
            .checked_sub(1)
            .and_then(|last| self.records.get(last))
            .map(|record| record.seq);
        held.max(self.newest_dropped.before(since_ms))
    }
}

impl EventLog {
    /// Every ring, open and closed, the no-tab ring first.
    fn rings(&self) -> impl Iterator<Item = &Ring> {
        iter::once(&self.no_tab)
            .chain(self.open.values())
            .chain(self.closed.values())
    }

    fn tab_ring(&self, target: &TargetId) -> Option<&Ring> {
        self.open.get(target).or_else(|| self.closed.get(target))
    }

    /// What a reader of `target` placed at `after` lost: drops from that tab's ring and the
    /// no-tab ring, never another tab's. A removed ring answers from its own fold, so a reader
    /// that read past its tab's records hears nothing of other tabs' later removals. A tab with
    /// no ring that the hub still knows as live never had one (only closed rings are removed), so
    /// it lost nothing; a ring removed longer ago than the folds remember answers with the
    /// shared watermark: conservative, never silent.
    fn tab_lost(&self, target: &TargetId, after: u64) -> u64 {
        let own = match (self.tab_ring(target), self.folded_targets.get(target)) {
            (Some(ring), _) => ring.lost_after(after),
            (None, Some(fold)) => fold.newest.lost_after(fold.count, after),
            (None, None) if self.frames.knows(target) => 0,
            (None, None) => self.folded.lost_after(self.folded_count, after),
        };
        own + self.no_tab.lost_after(after)
    }

    /// Records after `after` that no ring holds any more. Exact: every seq went to one ring.
    fn all_lost(&self, after: u64) -> u64 {
        let held: usize = self.rings().map(|ring| ring.held_after(after)).sum();
        self.last_seq
            .saturating_sub(after)
            .saturating_sub(u64::try_from(held).unwrap_or(u64::MAX))
    }

    /// Every ring's records after `after`, merged into seq order: the single ring's order.
    fn merged_after(&self, after: u64) -> Vec<EventRecord> {
        let mut records: Vec<EventRecord> = self
            .rings()
            .flat_map(|ring| ring.after(after))
            .cloned()
            .collect();
        records.sort_unstable_by_key(|record| record.seq);
        records
    }

    /// `target`'s ring and the no-tab ring after `after`, merged into seq order.
    fn tab_after(&self, target: &TargetId, after: u64) -> Vec<EventRecord> {
        self.tabs_after(&[target], after)
    }

    /// The rings of `tabs` and the no-tab ring after `after`, merged into seq order.
    fn tabs_after(&self, tabs: &[&TargetId], after: u64) -> Vec<EventRecord> {
        let mut records: Vec<EventRecord> = self
            .no_tab
            .after(after)
            .chain(
                tabs.iter()
                    .filter_map(|tab| self.tab_ring(tab))
                    .flat_map(|ring| ring.after(after)),
            )
            .cloned()
            .collect();
        records.sort_unstable_by_key(|record| record.seq);
        records
    }

    /// Files `record` in `tab`'s ring (the no-tab ring for `None`), within [`LOG_CAPACITY`].
    fn push(&mut self, tab: Option<TargetId>, record: EventRecord) {
        let ring = match tab {
            None => &mut self.no_tab,
            Some(target) => match self.closed.get_mut(&target) {
                Some(ring) => ring,
                None => match self.folded_targets.get_mut(&target) {
                    Some(fold) => {
                        fold.newest = fold.newest.max_seq(Mark::of(&record));
                        fold.count += 1;
                        self.folded = self.folded.max_seq(Mark::of(&record));
                        self.folded_count += 1;
                        return;
                    }
                    None => self.open.entry(target).or_default(),
                },
            },
        };
        // A record replayed into a cut ring (a held event of a tab already closed and trimmed)
        // goes at once: its tab's history is already cut, and keeping it would let an open
        // tab lose a record while this ring still held more than its close.
        if ring.state == RingState::Cut {
            ring.note_dropped(&record);
            return;
        }
        ring.records.push_back(record);
        self.total += 1;
        if ring.records.len() > LOG_CAPACITY && ring.drop_oldest() {
            self.total -= 1;
        }
    }

    /// Moves `target`'s ring among the closed rings, once. Past [`CLOSED_RINGS`] the
    /// earliest-closed ring is removed and folded: its tab is gone, and what its readers need
    /// once it is (its close, the loss watermark) is answered from `folded`.
    fn close(&mut self, target: &TargetId) {
        let Some(mut ring) = self.open.remove(target) else {
            return;
        };
        ring.state = RingState::Closed;
        self.closed.insert(target.clone(), ring);
        self.closing_order.push_back(target.clone());
        while self.closing_order.len() > CLOSED_RINGS {
            let Some(gone) = self.closing_order.pop_front() else {
                break;
            };
            self.cut = self.cut.saturating_sub(1);
            let mut fold = Fold::default();
            if let Some(ring) = self.closed.remove(&gone) {
                let held = ring.records.len();
                self.total -= held;
                fold = Fold {
                    newest: ring.newest(),
                    count: ring.dropped + u64::try_from(held).unwrap_or(u64::MAX),
                };
                self.folded_count += fold.count;
                self.folded = self.folded.max_seq(fold.newest);
            }
            self.folded_targets.insert(gone.clone(), fold);
            self.folded_order.push_back(gone);
            while self.folded_order.len() > CLOSED_RINGS {
                if let Some(old) = self.folded_order.pop_front() {
                    self.folded_targets.remove(&old);
                }
            }
        }
    }

    /// Forgets what the log keeps beside the rings for `target`, which is gone: its newest
    /// main-frame navigation, its frame-tree version, and its tab id. Without the tab id, a
    /// reader of the tab whose ring was removed reads the folded loss ([`EventLog::tab_lost`]).
    fn forget(&mut self, target: &TargetId) {
        self.navigated.remove(target);
        self.frames.closed(target);
    }

    /// Brings the total back under [`LOG_TOTAL`]. Closed tabs give way first, earliest close
    /// first, each cut to its `Closed`; only when none is left to cut does the open ring holding
    /// the most lose its oldest record (ties: the ring whose oldest record is oldest). So closed
    /// tabs never crowd a live one, and a live tab is cut only past its share of the open rings.
    /// The scan in the second step covers the open rings only: live tabs plus the no-tab ring.
    fn shed(&mut self) {
        while self.total > LOG_TOTAL {
            if let Some(target) = self.closing_order.get(self.cut) {
                if let Some(ring) = self.closed.get_mut(target) {
                    self.total -= ring.cut();
                }
                self.cut += 1;
                continue;
            }
            let fullest = iter::once(&mut self.no_tab)
                .chain(self.open.values_mut())
                .filter(|ring| !ring.records.is_empty())
                .max_by_key(|ring| {
                    let oldest = ring.records.front().map_or(0, |record| record.seq);
                    (ring.records.len(), Reverse(oldest))
                });
            // None: only cut rings hold records, at most `CLOSED_RINGS` of them.
            let Some(ring) = fullest else {
                break;
            };
            if ring.drop_oldest() {
                self.total -= 1;
            }
        }
    }
}

/// One event, numbered and stamped when the watcher recorded it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EventRecord {
    pub(crate) seq: u64,
    /// Milliseconds since the Unix epoch, from [`now_ms`]: the clock that stamps takeover's lock
    /// times. The log also compares it with the `Date.now()` stamps the prelude takes when the
    /// script calls `waitForEvent` or `on`, which read the same system clock in the same process.
    pub(crate) at_ms: u64,
    pub(crate) event: PageEvent,
}

/// The records after a reader's place, and where the log stands.
#[derive(Debug, Default)]
pub(crate) struct LogRead {
    pub(crate) records: Vec<EventRecord>,
    /// The newest seq in the log: the reader's place once it handled `records`.
    pub(crate) head: u64,
    /// Records after the reader's place that fell off before it read them, from the rings this
    /// read covers (`EventHub::read_tab_after`, `EventHub::read_after`). Readers act on `> 0`.
    pub(crate) lost: u64,
}

/// One pump listener's place in the log ([`EventHub::read_for`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ListenerPlace<'a> {
    /// The tab a page listener reads, or `None` for a context listener, which reads every tab.
    pub(crate) tab: Option<&'a TargetId>,
    /// The newest seq the listener has read.
    pub(crate) after: u64,
}

/// What the pump read in one cut ([`EventHub::read_for`]).
#[derive(Debug)]
pub(crate) struct PumpRead {
    /// Every ring's records after the lowest place, in seq order.
    pub(crate) records: Vec<EventRecord>,
    /// The newest seq in the log: each listener's place once the pump handled `records`.
    pub(crate) head: u64,
    /// Each listener's loss, in the order of the places it was given.
    pub(crate) losses: Vec<u64>,
}

/// The only handle that appends to the log. Exactly one exists, made with the hub and moved
/// into the target watcher task, so every record has one writer and one order.
#[derive(Debug)]
pub(crate) struct EventWriter {
    hub: Arc<EventHub>,
}

impl EventHub {
    /// A hub and its one writer.
    pub(crate) fn new() -> (Arc<Self>, EventWriter) {
        let hub = Arc::new(Self::default());
        let writer = EventWriter {
            hub: Arc::clone(&hub),
        };
        (hub, writer)
    }

    /// A wake-up for the next change. A reader enables it before reading, so a record that
    /// lands between the read and the wait still wakes it.
    pub(crate) fn changed(&self) -> Notified<'_> {
        self.changed.notified()
    }

    /// Wakes every reader to read again, as after a run adds a listener.
    pub(crate) fn wake(&self) {
        self.changed.notify_waiters();
    }

    /// Every ring's records after seq `after`, in seq order, for a reader of every tab (a
    /// context subscription). `lost` counts every ring's drops after `after`.
    pub(crate) fn read_after(&self, after: u64) -> LogRead {
        let log = self.lock_log();
        LogRead {
            records: log.merged_after(after),
            head: log.last_seq,
            lost: log.all_lost(after),
        }
    }

    /// `tab`'s records after seq `after`, in seq order: its own ring plus the no-tab ring, so a
    /// frame record the watcher could not file under a tab still retires a route or ends a wait.
    /// `lost` counts only those two rings' drops: another tab's records never read as this tab's
    /// loss, however busy that tab is.
    pub(crate) fn read_tab_after(&self, tab: &TargetId, after: u64) -> LogRead {
        let log = self.lock_log();
        LogRead {
            records: log.tab_after(tab, after),
            head: log.last_seq,
            lost: log.tab_lost(tab, after),
        }
    }

    /// The pump's read: the rings its places name after the lowest place, and each listener's
    /// loss from its own place, counted under the same lock. Page listeners name their tabs'
    /// rings and the no-tab ring; a context listener names every ring. So a run that listens on
    /// its own pages never copies another tab's records under the lock. A page listener's loss
    /// is its tab's ring and the no-tab ring's; a context listener's is every ring's. Counting
    /// after the read would let a record this read delivers count as lost once a later record
    /// pushed it out.
    pub(crate) fn read_for(&self, places: &[ListenerPlace<'_>]) -> PumpRead {
        let log = self.lock_log();
        let lowest = places
            .iter()
            .map(|place| place.after)
            .min()
            .unwrap_or(log.last_seq);
        let named: Option<Vec<&TargetId>> = places.iter().map(|place| place.tab).collect();
        let records = match named {
            Some(named) => {
                // Each tab once: a run often holds several listeners on one page.
                let mut tabs: Vec<&TargetId> = Vec::with_capacity(named.len());
                for tab in named {
                    if !tabs.contains(&tab) {
                        tabs.push(tab);
                    }
                }
                log.tabs_after(&tabs, lowest)
            }
            None => log.merged_after(lowest),
        };
        PumpRead {
            records,
            head: log.last_seq,
            losses: places
                .iter()
                .map(|place| match place.tab {
                    Some(tab) => log.tab_lost(tab, place.after),
                    None => log.all_lost(place.after),
                })
                .collect(),
        }
    }

    /// Where a reader started at `since_ms` reads from: after the newest record known to be older
    /// than that, held or dropped, in any ring. Counting each ring's newest dropped record, and
    /// the removed rings' folded one, keeps a ring that dropped only older records from reading
    /// as a loss to a reader started later. With no record known to be older, it reads from the
    /// start: every ring's newest drop is then at or after `since_ms`, a real loss its readers
    /// must see, and the records it returns are the ones held, which readers filter by time.
    pub(crate) fn position_at(&self, since_ms: u64) -> u64 {
        let log = self.lock_log();
        log.rings()
            .filter_map(|ring| ring.newest_before(since_ms))
            .chain(log.folded.before(since_ms))
            .max()
            .unwrap_or(0)
    }

    /// Registers `run` as a reader, once. Its listeners count from now on when the watcher
    /// decides what to record and which dialogs to hold.
    pub(crate) fn bind(&self, run: &Arc<RunState>) {
        let mut runs = self.runs.lock().unwrap_or_else(PoisonError::into_inner);
        runs.retain(|known| known.strong_count() > 0);
        if !runs.iter().any(|known| ptr_eq(known, run)) {
            runs.push(Arc::downgrade(run));
        }
    }

    /// The live runs that listen for, or wait on, `event` on the tab whose target is `target`.
    pub(crate) fn listeners(&self, target: &TargetId, event: PageEventKind) -> Vec<Weak<RunState>> {
        let runs = self.runs.lock().unwrap_or_else(PoisonError::into_inner);
        runs.iter()
            .filter(|known| {
                known
                    .upgrade()
                    .is_some_and(|run| run.events.wants(target, event))
            })
            .map(Weak::clone)
            .collect()
    }

    /// Whether some live run listens for, or waits on, `event` on the tab whose target is
    /// `target`. The watcher records console messages and page errors only then.
    pub(crate) fn wants(&self, target: &TargetId, event: PageEventKind) -> bool {
        let runs = self.runs.lock().unwrap_or_else(PoisonError::into_inner);
        runs.iter().any(|known| {
            known
                .upgrade()
                .is_some_and(|run| run.events.wants(target, event))
        })
    }

    /// The earliest moment any live run asked for the console messages and errors of the
    /// dedicated workers of the tab whose target is `target` (`RunEvents::workers_asked_since`).
    pub(crate) fn workers_asked_since(&self, target: &TargetId) -> Option<u64> {
        let runs = self.runs.lock().unwrap_or_else(PoisonError::into_inner);
        runs.iter()
            .filter_map(Weak::upgrade)
            .filter_map(|run| run.events.workers_asked_since(target))
            .min()
    }

    /// Claims binding `name` on the tab whose target is `target` for `run`: false when another
    /// live run holds it there. A run may claim its own name again; a name stays claimed until
    /// its binding is disposed ([`EventHub::release_binding`]) or the run that holds it ends.
    /// The claim is also the route: a call of the name reaches its holder alone.
    pub(crate) fn claim_binding(&self, target: &TargetId, name: &str, run: &Arc<RunState>) -> bool {
        let mut owners = self.bindings.lock().unwrap_or_else(PoisonError::into_inner);
        owners.retain(|_, owner| owner.strong_count() > 0);
        let key = (target.clone(), name.to_owned());
        if owners.get(&key).is_some_and(|owner| !ptr_eq(owner, run)) {
            return false;
        }
        owners.insert(key, Arc::downgrade(run));
        true
    }

    /// Lets go of `name` on the tab whose target is `target`, if `run` holds it: its binding
    /// was disposed, so its calls go nowhere and another run may expose the name there.
    pub(crate) fn release_binding(&self, target: &TargetId, name: &str, run: &Arc<RunState>) {
        let mut owners = self.bindings.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (target.clone(), name.to_owned());
        if owners.get(&key).is_some_and(|owner| ptr_eq(owner, run)) {
            owners.remove(&key);
        }
    }

    /// Whether a run explicitly requested lifecycle events on every context page.
    pub(crate) fn wants_context_lifecycle(&self) -> bool {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|run| {
                run.upgrade()
                    .is_some_and(|run| run.events.wants_context_lifecycle())
            })
    }

    /// The `Log` leases on page and frame sessions (`log_lease.rs`).
    pub(crate) fn log_leases(&self) -> &LogLeases {
        &self.log_leases
    }

    /// The seq of the newest record: a reader's place for "from now on".
    pub(crate) fn head(&self) -> u64 {
        self.lock_log().last_seq
    }

    /// The newest close of this target its ring still holds. A closed ring keeps its `Closed`
    /// through every cut, and goes only past [`CLOSED_RINGS`] later closes.
    pub(crate) fn closed_at(&self, target: &TargetId) -> Option<u64> {
        self.lock_log()
            .tab_ring(target)?
            .records
            .iter()
            .rev()
            .find_map(|record| match &record.event {
                PageEvent::Closed(closed) if closed == target => Some(record.seq),
                _ => None,
            })
    }

    /// The newest retained navigation of `target`'s main frame (a new document or a
    /// same-document change: a link, `pushState`, a fragment, the Navigation API), for the
    /// host's URL facts. Core's page cache does not follow a navigation the page makes itself,
    /// so the host refreshes it when this moves past the seq it last refreshed through.
    pub(crate) fn navigated_at(&self, target: &TargetId) -> Option<u64> {
        self.lock_log().navigated.get(target).copied()
    }

    /// Every tab's frame-tree version whose tab id the watcher knows: every reply carries them,
    /// so a run learns that a tab's frames changed even from a reply about another tab.
    pub(crate) fn frame_versions(&self) -> Vec<(TabId, u64)> {
        self.lock_log().frames.versions()
    }

    /// `tab`'s frames as the watcher's events and seeds left them, main frame first, with its
    /// version, whether its seed landed and the newest record the list includes
    /// ([`TabFrames::get`]). Nothing in the product reads it yet.
    pub(crate) fn tab_frames(&self, tab: &TargetId) -> Option<TabFrameList> {
        self.lock_log().frames.get(tab)
    }

    /// The tab a live frame belongs to, same-process children included ([`TabFrames::owner`]).
    pub(crate) fn frame_owner(&self, frame: &FrameId) -> Option<TargetId> {
        self.lock_log().frames.owner(frame).cloned()
    }

    /// `tab`'s main frame, once its commit or seed was applied ([`TabFrames::main`]).
    pub(crate) fn main_frame(&self, tab: &TargetId) -> Option<EventFrame> {
        self.lock_log().frames.main(tab)
    }

    /// `tab`'s frame-tree version, `0` while no writer moved it.
    pub(crate) fn frame_version(&self, tab: &TargetId) -> u64 {
        self.lock_log().frames.version(tab)
    }

    /// The reads `tab`'s list lacks before it is whole ([`TabFrames::missing_seeds`]).
    pub(crate) fn missing_seeds(&self, tab: &TargetId) -> Option<MissingSeeds> {
        self.lock_log().frames.missing_seeds(tab)
    }

    /// The type of a dialog recorded on `target` after seq `after` that is still open, if
    /// any. An action whose input such a dialog blocks names it when it times out: Chromium
    /// acknowledges no input and answers no page call until the dialog closes (measured).
    pub(crate) fn open_dialog_after(&self, after: u64, target: &TargetId) -> Option<&'static str> {
        let opened: Vec<(DialogId, DialogKind)> = self
            .lock_log()
            .tab_ring(target)
            .into_iter()
            .flat_map(|ring| ring.after(after))
            .filter_map(|record| match &record.event {
                PageEvent::Dialog(dialog) if dialog.target == *target => {
                    Some((dialog.id, dialog.kind))
                }
                _ => None,
            })
            .collect();
        let dialogs = self.dialogs();
        opened
            .into_iter()
            .find(|(id, _)| dialogs.is_open(*id, target))
            .map(|(_, kind)| kind.as_str())
    }

    /// The held dialogs. A std mutex, never held across an await.
    pub(crate) fn dialogs(&self) -> MutexGuard<'_, DialogTable> {
        dialog_table::lock(&self.dialogs)
    }

    /// Every download the watcher saw begin, with its progress.
    pub(crate) fn downloads(&self) -> &DownloadTable {
        &self.downloads
    }

    /// Starts a script's answer to dialog `id` on the tab `target`. The guard keeps the dialog
    /// answering until the browser acknowledges it, and reopens it if it drops before then.
    pub(crate) fn answer_dialog(
        &self,
        id: DialogId,
        target: &TargetId,
    ) -> Result<DialogAnswer<'_>, Unanswerable> {
        DialogAnswer::start(&self.dialogs, id, target)
    }

    fn lock_log(&self) -> MutexGuard<'_, EventLog> {
        self.log.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl EventWriter {
    /// Appends `event` to the ring the log files it in ([`EventLog::append`]), and wakes the
    /// readers. The frame-only kinds name no tab: the watcher files those with
    /// [`Self::record_in`].
    pub(crate) fn record(&mut self, event: PageEvent) -> u64 {
        let seq = self.hub.lock_log().append(event);
        // Outside the lock: woken readers take it at once.
        self.hub.changed.notify_waiters();
        seq
    }

    /// Appends `event` to `tab`'s ring (the no-tab ring for `None`) with the next seq
    /// ([`EventLog::append_in`]), and wakes the readers.
    pub(crate) fn record_in(&mut self, tab: Option<TargetId>, event: PageEvent) -> u64 {
        let seq = self.hub.lock_log().append_in(tab, event);
        // Outside the lock: woken readers take it at once.
        self.hub.changed.notify_waiters();
        seq
    }

    /// Closes `target`'s ring, if it has one, with no record: a tab that goes while Pending with
    /// no opener is not news (`target_watcher.rs`), but its ring must not count as live forever,
    /// and the hub forgets it as a recorded close would.
    pub(crate) fn close_ring(&mut self, target: &TargetId) {
        let mut log = self.hub.lock_log();
        log.forget(target);
        log.close(target);
        log.shed();
    }

    /// Hands a binding call to the run that holds its name on its tab
    /// ([`EventHub::claim_binding`]), into that run's own queue rather than the bounded log: a
    /// call is a request the page may await, so it must reach the run however many records
    /// follow it before the pump reads. Only the holder gets it, so no other run on the tab
    /// answers it too; a name nobody holds (disposed, or the page's own call of the raw binding)
    /// goes nowhere. Numbered with the log's head, so the pump orders it after the records that
    /// came before it.
    pub(crate) fn deliver_binding(&mut self, call: &BindingCalled) {
        #[derive(Deserialize)]
        struct Named {
            name: String,
        }
        let Ok(Named { name }) = serde_json::from_str::<Named>(&call.payload) else {
            return;
        };
        let owner = self
            .hub
            .bindings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(call.target.clone(), name))
            .and_then(Weak::upgrade);
        if let Some(run) = owner
            && let Some(tab) = run.events.binding_tab(&call.target)
        {
            run.events.queue_call(QueuedCall {
                seq: self.hub.head(),
                at_ms: now_ms(),
                tab,
                call: call.clone(),
            });
            self.hub.changed.notify_waiters();
        }
    }

    /// Records that the navigation recorded as `seq` was `tab`'s main frame's. The watcher calls
    /// it, as only the watcher knows which tab a session's frame belongs to: a child frame's
    /// navigation is never kept, so the map holds one entry per open tab.
    pub(crate) fn main_frame_navigated(&mut self, tab: TargetId, seq: u64) {
        self.hub.lock_log().navigated.insert(tab, seq);
    }

    /// Records `target`'s tab id, which the per-tab versions are keyed by on the wire.
    pub(crate) fn tab_id(&mut self, target: TargetId, tab: TabId) {
        self.hub.lock_log().frames.tab_id(target, tab);
    }

    /// Moves `tab`'s frame-tree version for an out-of-process frame's session attaching or
    /// ending (see `EventLog::frames`). The watcher calls it, as only the watcher knows which tab
    /// a frame's session belongs to; a frame event moves it in [`Self::frame_event`].
    pub(crate) fn frames_changed(&mut self, tab: TargetId) {
        self.hub.lock_log().frames.changed(tab);
    }

    /// Seeds `tab`'s list from a `Page.getFrameTree` reply, whole as of the log's head
    /// ([`TabFrames::seed`]). A seed moves no version.
    pub(crate) fn seed_frames(&mut self, tab: &TargetId, tree: Value) {
        let mut log = self.hub.lock_log();
        let head = log.last_seq;
        log.frames.seed(tab, tree, head);
    }

    /// Applies one CDP frame event of `tab`'s lifecycle source to its list, writes its
    /// transitions as records when `write` (only while a session of core's is on the tab, or
    /// for a source whose records main wrote, `Watcher::records_page_events`), and moves the
    /// tab's version on any transition or any commit, as the version always moved. All of it
    /// under one lock, so a reader never sees the list, its version and the records it includes
    /// disagree; the list names the newest record it includes.
    pub(crate) fn frame_event(
        &mut self,
        tab: &TargetId,
        method: &str,
        params: &Value,
        write: bool,
    ) {
        let written = {
            let mut log = self.hub.lock_log();
            let transitions = log.frames.record(tab, method, params);
            let changed = !transitions.is_empty();
            let written = write && changed;
            if write {
                for transition in transitions {
                    log.append(PageEvent::FrameLifecycle(transition));
                }
            }
            if changed {
                let head = log.last_seq;
                log.frames.included(tab, head);
            }
            if changed || method == "Page.frameNavigated" {
                log.frames.changed(tab.clone());
            }
            written
        };
        if written {
            self.hub.changed.notify_waiters();
        }
    }

    /// An out-of-process frame's session became `frame`'s source on `tab`: its list is not
    /// whole until that session's own subtree lands ([`TabFrames::session_attached`]).
    pub(crate) fn frame_session_attached(&mut self, tab: &TargetId, frame: FrameId) {
        self.hub.lock_log().frames.session_attached(tab, frame);
    }

    /// `frame`'s session ended with no other to take over ([`TabFrames::session_ended`]).
    pub(crate) fn frame_session_ended(&mut self, tab: &TargetId, frame: &FrameId) {
        self.hub.lock_log().frames.session_ended(tab, frame);
    }

    /// Forgets `tab`'s list as the watcher drops the tab; its version goes with its recorded
    /// close ([`TabFrames::forget`]).
    pub(crate) fn forget_frames(&mut self, tab: &TargetId) {
        self.hub.lock_log().frames.forget(tab);
    }

    /// Forgets every tab's list after a reconnect lost the events between
    /// ([`TabFrames::reset_lists`]).
    pub(crate) fn reset_frame_lists(&mut self) {
        self.hub.lock_log().frames.reset_lists();
    }

    /// The hub this writer appends to.
    pub(crate) fn hub(&self) -> &EventHub {
        &self.hub
    }
}

/// Whether the weak entry is `run`.
fn ptr_eq(known: &Weak<RunState>, run: &Arc<RunState>) -> bool {
    Weak::as_ptr(known) == Arc::as_ptr(run)
}

/// The wall clock in milliseconds since the Unix epoch: [`EpochMillis::now`], the function
/// `neo_takeover::Locks` stamps a lock's `created_at` and `ended.at` with. claw-sdk tells
/// whether a lock held a tab when an event was recorded by comparing the two, so a second clock
/// here (another source, unit or epoch) would misplace every event against every lock.
pub(crate) fn now_ms() -> u64 {
    // `EpochMillis::now` reads a clock set before 1970 as 0, so the conversion cannot fail.
    u64::try_from(EpochMillis::now().millis()).unwrap_or(0)
}

/// Whether a subscription's listener, attached at `since_ms`, receives `record`.
pub(crate) fn receives(subscription: &Subscription, since_ms: u64, record: &EventRecord) -> bool {
    record.at_ms >= since_ms && subscription.matches(&record.event)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::hint;
    use std::sync::Arc;

    use browseros_core::{FrameId, TabId, TargetId};
    use neo_trace::EpochMillis;

    use super::{
        CLOSED_RINGS, EventHub, EventWriter, LOG_CAPACITY, LOG_TOTAL, ListenerPlace, LogRead,
        now_ms,
    };
    use crate::RunState;
    use crate::events::navigation_event::NavigationEvent;
    use crate::events::page_event::{BindingCalled, OpenedPage, PageEvent, WindowKind};
    use crate::events::run_events::QUEUED_CALLS;
    use crate::events::{PageEventKind, Subscription};

    fn closed(target: &str) -> PageEvent {
        PageEvent::Closed(TargetId(target.to_owned()))
    }

    fn target(name: &str) -> TargetId {
        TargetId(name.to_owned())
    }

    /// A record filed in `tab`'s ring that does not close it.
    fn crashed(tab: &str) -> PageEvent {
        PageEvent::Crashed(target(tab))
    }

    fn seqs(read: &LogRead) -> Vec<u64> {
        read.records.iter().map(|record| record.seq).collect()
    }

    /// A stamp newer than every record written so far, and older than every one written after.
    fn after_every_stamp() -> u64 {
        let stamped = now_ms();
        while now_ms() <= stamped {
            hint::spin_loop();
        }
        now_ms()
    }

    /// Row 1. A busy tab pushes out only its own records: a reader of another tab keeps that
    /// tab's record and loses nothing however far behind it is, while a read of every ring sees
    /// the busy tab's drops.
    #[test]
    fn tests_that_a_busy_tab_does_not_push_out_another_tabs_records() {
        let (hub, mut writer) = EventHub::new();
        let quiet = writer.record(crashed("A"));
        for _ in 0..5_000 {
            writer.record(crashed("B"));
        }
        let read = hub.read_tab_after(&target("A"), 0);
        assert_eq!(seqs(&read), [quiet]);
        assert_eq!(read.lost, 0, "B's drops are not A's loss");
        assert_eq!(
            hub.read_after(0).lost,
            3_976,
            "5,000 of B's written, 1,024 kept"
        );
    }

    /// Row 2. Past `LOG_TOTAL` with no closed ring to cut, the open ring holding the most gives
    /// way, never a small one, even when the small one's records are the oldest in the log.
    #[test]
    fn tests_that_past_the_total_the_fullest_open_ring_gives_way() {
        let (hub, mut writer) = EventHub::new();
        let small: Vec<u64> = (0..10).map(|_| writer.record(crashed("small"))).collect();
        let single = writer.record(crashed("single"));
        for busy in 0..LOG_TOTAL / LOG_CAPACITY {
            for _ in 0..LOG_CAPACITY {
                writer.record(crashed(&format!("busy{busy}")));
            }
        }
        let kept = hub.read_tab_after(&target("small"), 0);
        assert_eq!((seqs(&kept), kept.lost), (small, 0));
        assert_eq!(seqs(&hub.read_tab_after(&target("single"), 0)), [single]);
        assert!(hub.read_after(0).records.len() <= LOG_TOTAL);
        assert!(
            hub.read_tab_after(&target("busy0"), 0).lost > 0,
            "the fullest rings paid, oldest first among equals"
        );
    }

    /// Row 3. A one-tab reader learns of its own ring's drops past its place, and of the no-tab
    /// ring's, where the frame records the watcher could not place sit.
    #[test]
    fn tests_that_a_one_tab_reader_learns_of_its_rings_losses() {
        let (hub, mut writer) = EventHub::new();
        let place = writer.record(crashed("A"));
        for _ in 0..=LOG_CAPACITY {
            writer.record(crashed("A"));
        }
        assert!(
            hub.read_tab_after(&target("A"), place).lost > 0,
            "its own ring dropped past its place"
        );
        let later = hub.head();
        assert_eq!(hub.read_tab_after(&target("A"), later).lost, 0);
        for _ in 0..=LOG_CAPACITY {
            writer.record(navigated("unplaced-frame"));
        }
        assert!(
            hub.read_tab_after(&target("A"), later).lost > 0,
            "the no-tab ring's drops count for every one-tab reader"
        );
        assert!(hub.read_tab_after(&target("B"), later).lost > 0);
    }

    /// Row 4. Reads across rings keep the one counter's order, the single ring's, including a
    /// record applied late (a held event) with a higher seq; the pump's read counts each
    /// listener's loss in the same cut.
    #[test]
    fn tests_that_reads_across_rings_keep_the_counters_order() {
        let (hub, mut writer) = EventHub::new();
        let written: Vec<u64> = ["A", "B", "A", "C", "B", "A"]
            .into_iter()
            .map(|tab| writer.record(crashed(tab)))
            .collect();
        let unplaced = writer.record(navigated("frame"));
        let late = writer.record(crashed("A"));
        let all = seqs(&hub.read_after(0));
        assert_eq!(all, [written.clone(), vec![unplaced, late]].concat());
        assert_eq!(
            seqs(&hub.read_tab_after(&target("A"), 0)),
            [written[0], written[2], written[5], unplaced, late]
        );
        let b = target("B");
        let read = hub.read_for(&[
            ListenerPlace {
                tab: Some(&b),
                after: 0,
            },
            ListenerPlace {
                tab: None,
                after: 0,
            },
        ]);
        let pumped: Vec<u64> = read.records.iter().map(|record| record.seq).collect();
        assert_eq!((pumped, read.losses), (all, vec![0, 0]));
    }

    /// Row 5. A reader started after old drops reads no loss: a ring that dropped only records
    /// older than its start, and a closed ring removed whose records were all older, both count
    /// in `position_at`. A live tab with no ring yet (a popup, whose `Opened` sits in its
    /// opener's ring) never had one and reads 0 from any place; a removed ring read from before
    /// its records reads its loss.
    #[test]
    fn tests_that_a_reader_started_after_old_drops_reads_no_loss() {
        let (hub, mut writer) = EventHub::new();
        let older = writer.record(crashed("A"));
        for _ in 0..10 {
            writer.record(crashed("B"));
        }
        writer.record(closed("gone"));
        let since = after_every_stamp();
        for _ in 0..LOG_CAPACITY {
            writer.record(crashed("B"));
        }
        for later in 0..CLOSED_RINGS {
            writer.record(closed(&format!("later{later}")));
        }
        writer.tab_id(target("popup"), TabId(9));
        let place = hub.position_at(since);
        assert_eq!(
            hub.read_tab_after(&target("B"), place).lost,
            0,
            "B dropped only records older than the reader's start"
        );
        assert!(place > older, "past A's record, the newest older one held");
        assert_eq!(
            hub.closed_at(&target("gone")),
            None,
            "removed past CLOSED_RINGS"
        );
        assert_eq!(
            hub.read_tab_after(&target("gone"), place).lost,
            0,
            "removed, but everything it held was older than the reader's start"
        );
        assert!(hub.read_tab_after(&target("gone"), older).lost > 0);
        assert_eq!(
            hub.read_tab_after(&target("popup"), 0).lost,
            0,
            "a live tab with no ring never had one"
        );
    }

    /// Row 6. Closed tabs give way before an open one: thousands of closed tabs with records,
    /// then an open tab fills its ring and keeps all of it. Closed rings stay bounded.
    #[test]
    fn tests_that_closed_tabs_give_way_before_an_open_tab() {
        let (hub, mut writer) = EventHub::new();
        for tab in 0..2_000 {
            let name = format!("closed{tab}");
            for _ in 0..49 {
                writer.record(crashed(&name));
            }
            writer.record(closed(&name));
        }
        let place = hub.head();
        let kept: Vec<u64> = (0..LOG_CAPACITY)
            .map(|_| writer.record(crashed("open")))
            .collect();
        let read = hub.read_tab_after(&target("open"), place);
        assert_eq!((seqs(&read), read.lost), (kept, 0));
        assert!(hub.read_after(0).records.len() <= LOG_TOTAL);
        assert_eq!(
            hub.closed_at(&target("closed0")),
            None,
            "removed past CLOSED_RINGS"
        );
        assert!(
            hub.closed_at(&target("closed1999")).is_some(),
            "a cut ring keeps its close"
        );
    }

    /// A tab that goes while Pending with no opener records no close, but its ring still joins
    /// the closed ones and the hub forgets its tab id: it is removed past `CLOSED_RINGS` like any
    /// closed tab's, and a reader of it then reads its fold's loss. One that went with no record
    /// at all has no ring and no fold; forgotten, it reads the shared watermark, not a live tab's 0.
    #[test]
    fn tests_that_a_ring_closed_without_a_record_is_a_closed_tabs() {
        let (hub, mut writer) = EventHub::new();
        writer.tab_id(target("pending"), TabId(5));
        writer.record(crashed("pending"));
        writer.close_ring(&target("pending"));
        writer.tab_id(target("silent"), TabId(6));
        writer.close_ring(&target("silent"));
        for later in 0..CLOSED_RINGS {
            let name = format!("later{later}");
            writer.record(crashed(&name));
            writer.record(closed(&name));
        }
        assert!(hub.read_tab_after(&target("pending"), 0).lost > 0);
        assert!(
            hub.read_tab_after(&target("silent"), 0).lost > 0,
            "a gone tab is not read as a live one with no ring yet"
        );
    }

    #[test]
    fn tests_that_each_reader_reads_from_its_own_place() {
        let (hub, mut writer) = EventHub::new();
        let first = writer.record(closed("A"));
        let second = writer.record(closed("B"));
        assert_eq!((first, second), (1, 2));
        let all = hub.read_after(0);
        assert_eq!(all.records.len(), 2);
        assert_eq!((all.head, all.lost), (2, 0));
        let rest = hub.read_after(first);
        assert_eq!(
            rest.records
                .iter()
                .map(|record| record.seq)
                .collect::<Vec<_>>(),
            [2]
        );
        assert!(hub.read_after(second).records.is_empty());
    }

    #[test]
    fn tests_that_a_reader_that_fell_behind_learns_how_much_it_lost() {
        let (hub, mut writer) = EventHub::new();
        for _ in 0..LOG_CAPACITY + 5 {
            writer.record(closed("A"));
        }
        let read = hub.read_after(0);
        assert_eq!(read.records.len(), LOG_CAPACITY, "the ring is bounded");
        assert_eq!(
            read.lost, 5,
            "the five oldest fell off before this reader read them"
        );
        assert_eq!(
            hub.read_after(10).lost,
            0,
            "a reader past them lost nothing"
        );
    }

    fn navigated(frame: &str) -> PageEvent {
        PageEvent::Navigation(NavigationEvent {
            frame: FrameId(frame.to_owned()),
            url: "https://a.test/#moved".to_owned(),
            new_document: false,
            loader: None,
            unreachable: None,
        })
    }

    /// The host's URL refresh reads each target's newest main-frame navigation, which the bounded
    /// ring must not evict before it does, and which goes once the target closes.
    #[test]
    fn tests_that_the_newest_navigation_outlives_the_ring_until_its_target_closes() {
        let (hub, mut writer) = EventHub::new();
        let target = TargetId("A".to_owned());
        let moved = writer.record_in(Some(target.clone()), navigated("A"));
        writer.main_frame_navigated(target.clone(), moved);
        let child = writer.record(navigated("child-frame"));
        assert_eq!(
            hub.navigated_at(&TargetId("child-frame".to_owned())),
            None,
            "a navigation the watcher did not mark as a main frame's is not kept: {child}"
        );
        for _ in 0..LOG_CAPACITY + 5 {
            writer.record(crashed("A"));
        }
        assert!(
            !seqs(&hub.read_tab_after(&target, 0)).contains(&moved),
            "A's own ring dropped the navigation's record"
        );
        assert_eq!(hub.navigated_at(&target), Some(moved));
        writer.record(closed("A"));
        assert_eq!(hub.navigated_at(&target), None);
    }

    /// claw-sdk compares `at_ms` with takeover lock times, which `EpochMillis::now` stamps.
    #[test]
    fn tests_that_each_tabs_frame_version_grows_with_its_changes_and_leaves_with_its_close() {
        let (hub, mut writer) = EventHub::new();
        let (a, b) = (TargetId("A".to_owned()), TargetId("B".to_owned()));
        writer.tab_id(a.clone(), TabId(1));
        writer.tab_id(b.clone(), TabId(2));
        assert!(hub.frame_versions().is_empty(), "no change yet");
        writer.frames_changed(a.clone());
        writer.frames_changed(b.clone());
        writer.frames_changed(a);
        let mut versions = hub.frame_versions();
        versions.sort_unstable();
        assert_eq!(versions, [(TabId(1), 3), (TabId(2), 2)]);
        writer.frames_changed(TargetId("unknown-tab".to_owned()));
        assert_eq!(
            hub.frame_versions().len(),
            2,
            "a target whose tab id is unknown is not listed"
        );
        writer.record(closed("B"));
        assert_eq!(hub.frame_versions(), [(TabId(1), 3)]);
    }

    #[test]
    fn tests_that_a_record_is_stamped_on_the_lock_clock() {
        let (hub, mut writer) = EventHub::new();
        let before = EpochMillis::now().millis();
        writer.record(closed("A"));
        let after = EpochMillis::now().millis();
        let stamped = hub
            .read_after(0)
            .records
            .first()
            .map(|record| i64::try_from(record.at_ms));
        assert!(
            matches!(stamped, Some(Ok(at)) if before <= at && at <= after),
            "stamped {stamped:?}, outside the EpochMillis reads {before}..={after}"
        );
    }

    #[test]
    fn tests_that_only_live_runs_that_want_an_event_listen_for_it() {
        let (hub, _writer) = EventHub::new();
        let target = TargetId("T".to_owned());
        let listening = Arc::new(RunState::default());
        let idle = Arc::new(RunState::default());
        hub.bind(&listening);
        hub.bind(&listening);
        hub.bind(&idle);
        let _waiting = listening.events.waiting(
            Subscription::Page {
                tab: TabId(1),
                target: target.clone(),
                event: PageEventKind::Dialog,
            },
            0,
        );
        assert_eq!(hub.listeners(&target, PageEventKind::Dialog).len(), 1);
        assert!(hub.listeners(&target, PageEventKind::Console).is_empty());
    }

    /// A binding call goes to the queue of the run holding its name on its tab, whatever the log
    /// holds after it, and never into the log, so a flood of records cannot evict it. Another run
    /// listening for that tab's calls gets none; a released name routes nowhere; past
    /// `QUEUED_CALLS` waiting calls the rest are counted lost.
    #[test]
    fn tests_that_a_binding_call_reaches_its_names_holder_past_any_number_of_records() {
        let (hub, mut writer) = EventHub::new();
        let target = TargetId("T".to_owned());
        let holder = Arc::new(RunState::default());
        let other = Arc::new(RunState::default());
        for run in [&holder, &other] {
            hub.bind(run);
            run.events.listen(
                Subscription::Page {
                    tab: TabId(7),
                    target: target.clone(),
                    event: PageEventKind::Binding,
                },
                0,
                0,
            );
        }
        assert!(hub.claim_binding(&target, "f", &holder));
        let call = BindingCalled {
            target: target.clone(),
            context: 3,
            frame: None,
            payload: r#"{"name":"f","seq":1,"serializedArgs":[]}"#.to_owned(),
        };
        writer.deliver_binding(&call);
        for _ in 0..=LOG_CAPACITY {
            writer.record(closed("U"));
        }
        let (queued, dropped) = holder.events.take_calls();
        assert_eq!(
            (queued.len(), dropped),
            (1, 0),
            "the call outlived the ring"
        );
        assert_eq!((queued[0].tab, queued[0].seq), (TabId(7), 0));
        assert!(other.events.take_calls().0.is_empty(), "only the holder");
        assert!(
            hub.read_after(0)
                .records
                .iter()
                .all(|record| matches!(record.event, PageEvent::Closed(_))),
            "the log holds no call"
        );

        for _ in 0..QUEUED_CALLS + 2 {
            writer.deliver_binding(&call);
        }
        let (queued, dropped) = holder.events.take_calls();
        assert_eq!(
            (queued.len(), dropped),
            (QUEUED_CALLS, 2),
            "a flood is counted lost"
        );

        hub.release_binding(&target, "f", &holder);
        writer.deliver_binding(&call);
        assert!(
            holder.events.take_calls().0.is_empty(),
            "a released name goes nowhere"
        );
    }

    /// A binding name on a tab belongs to one live run: another run is refused it until the
    /// first ends, and the holder may claim it again.
    #[test]
    fn tests_that_a_binding_name_belongs_to_one_live_run_per_tab() {
        let (hub, _writer) = EventHub::new();
        let tab = TargetId("T".to_owned());
        let first = Arc::new(RunState::default());
        let second = Arc::new(RunState::default());
        assert!(hub.claim_binding(&tab, "add", &first));
        assert!(hub.claim_binding(&tab, "add", &first), "its own again");
        assert!(!hub.claim_binding(&tab, "add", &second));
        assert!(
            hub.claim_binding(&TargetId("U".to_owned()), "add", &second),
            "another tab's"
        );
        drop(first);
        assert!(
            hub.claim_binding(&tab, "add", &second),
            "once the first ended"
        );
    }

    /// A record that arrives late for a tab whose ring was removed (a popup's window lookup
    /// filing under an opener long gone) opens no ring for the dead tab: it goes into the folded
    /// watermark, so its readers see the loss and the live tabs keep their share.
    #[test]
    fn tests_that_a_late_record_of_a_removed_ring_opens_none() {
        let (hub, mut writer) = EventHub::new();
        writer.record(closed("gone"));
        for later in 0..CLOSED_RINGS {
            writer.record(closed(&format!("later{later}")));
        }
        let place = hub.head();
        writer.record(crashed("gone"));
        let read = hub.read_tab_after(&target("gone"), place);
        assert!(read.records.is_empty(), "no ring holds it");
        assert!(read.lost > 0, "its reader learns of it");
    }

    /// A cut ring holds only its close: a record replayed into it later is dropped even with room
    /// in the log, so no open tab loses a record while a closed tab's ring holds more.
    #[test]
    fn tests_that_a_cut_ring_drops_a_replay_even_with_room() {
        let (hub, mut writer) = EventHub::new();
        for _ in 0..100 {
            writer.record(crashed("closed"));
        }
        let close = writer.record(closed("closed"));
        // Past the total once (the closed ring is cut, freeing 100), then back under it.
        press_past_the_total(&mut writer);
        assert!(hub.read_after(0).records.len() < LOG_TOTAL, "room again");
        writer.record(crashed("closed"));
        assert_eq!(seqs(&hub.read_tab_after(&target("closed"), 0)), [close]);
    }

    /// A reader started before every record still learns of a drop: with no record known to be
    /// older than its start, it reads from the start, so a ring that dropped a record at or
    /// after that start reports it.
    #[test]
    fn tests_that_a_reader_started_before_every_record_learns_of_a_drop() {
        let (hub, mut writer) = EventHub::new();
        let since = now_ms();
        for _ in 0..=LOG_CAPACITY {
            writer.record(crashed("A"));
        }
        let place = hub.position_at(since);
        assert!(hub.read_tab_after(&target("A"), place).lost > 0);
    }

    /// Fills the log past `LOG_TOTAL` once with busy open tabs, then leaves room again: any
    /// closed ring at the time is cut to its `Closed`.
    fn press_past_the_total(writer: &mut EventWriter) {
        for busy in 0..16 {
            let count = if busy < 15 { LOG_CAPACITY } else { 940 };
            for _ in 0..count {
                writer.record(crashed(&format!("busy{busy}")));
            }
        }
    }

    /// A popup whose opener closed before its window lookup came back goes to the popup's own
    /// ring, not the opener's cut ring that would drop it: a context `page` listener still gets
    /// a tab that is alive, with no loss.
    #[test]
    fn tests_that_a_popup_of_a_closed_opener_is_kept() {
        let (hub, mut writer) = EventHub::new();
        for _ in 0..100 {
            writer.record(crashed("opener"));
        }
        writer.record(closed("opener"));
        press_past_the_total(&mut writer);
        let place = hub.head();
        let opened = writer.record(PageEvent::Opened(OpenedPage {
            target: target("popup"),
            opener: Some(target("opener")),
            url: "https://a.test/popup".to_owned(),
            window: WindowKind::Tab,
            tab: Some(TabId(3)),
        }));
        let read = hub.read_after(place);
        assert_eq!((seqs(&read), read.lost), (vec![opened], 0));
    }

    /// The pump copies only the rings its places name when every listener is a page listener:
    /// another tab's records never enter its read under the lock.
    #[test]
    fn tests_that_a_page_only_pump_reads_only_its_rings() {
        let (hub, mut writer) = EventHub::new();
        let mine = writer.record(crashed("mine"));
        writer.record(crashed("other"));
        let unplaced = writer.record(navigated("frame"));
        writer.record(crashed("other"));
        let mine_too = writer.record(crashed("mine"));
        let page = target("mine");
        let place = ListenerPlace {
            tab: Some(&page),
            after: 0,
        };
        let read = hub.read_for(&[place, place]);
        let pumped: Vec<u64> = read.records.iter().map(|record| record.seq).collect();
        assert_eq!(
            pumped,
            [mine, unplaced, mine_too],
            "each tab once, no other tab's"
        );
    }

    /// `position_at`'s binary search gives what a walk of every held record gives: the newest
    /// seq stamped before `since`, or 0, across rings written over several milliseconds.
    #[test]
    fn tests_that_position_at_matches_a_walk_of_every_record() {
        let (hub, mut writer) = EventHub::new();
        for round in 0..4 {
            for tab in ["A", "B", "C"] {
                for _ in 0..=round {
                    writer.record(crashed(tab));
                }
            }
            after_every_stamp();
        }
        let held = hub.read_after(0).records;
        let mut stamps: Vec<u64> = held.iter().map(|record| record.at_ms).collect();
        stamps.push(held.first().map_or(0, |record| record.at_ms) - 1);
        stamps.push(now_ms() + 1);
        for since in stamps {
            let walked = held
                .iter()
                .filter(|record| record.at_ms < since)
                .map(|record| record.seq)
                .max()
                .unwrap_or(0);
            assert_eq!(hub.position_at(since), walked, "since {since}");
        }
    }

    /// A reader of a removed tab that read past its tab's records hears nothing of other tabs'
    /// later removals: its loss comes from its own tab's fold, not the shared watermark, so a
    /// listener left on a closed page cannot fail a live page's click with a loss it never had.
    #[test]
    fn tests_that_a_removed_tabs_reader_hears_only_its_own_fold() {
        let (hub, mut writer) = EventHub::new();
        writer.record(crashed("gone"));
        writer.record(closed("gone"));
        let place = hub.head();
        for later in 0..CLOSED_RINGS + 10 {
            let name = format!("later{later}");
            writer.record(crashed(&name));
            writer.record(closed(&name));
        }
        assert_eq!(hub.closed_at(&target("gone")), None, "its ring was removed");
        assert_eq!(
            hub.read_tab_after(&target("gone"), place).lost,
            0,
            "it read past its tab's records"
        );
        assert!(
            hub.read_tab_after(&target("gone"), 0).lost > 0,
            "read from before them, the loss shows"
        );
    }

    /// A close is found while its tab's ring holds it: another tab's records never push it out,
    /// only the tab's own past `LOG_CAPACITY`.
    #[test]
    fn closed_at_tracks_only_retained_closes_of_the_requested_target() {
        let (hub, mut writer) = EventHub::new();
        writer.record(closed("A"));
        writer.record(closed("B"));
        let latest = writer.record(closed("A"));
        assert_eq!(hub.closed_at(&TargetId("A".into())), Some(latest));
        assert_eq!(hub.closed_at(&TargetId("never".into())), None);
        for _ in 0..LOG_CAPACITY {
            writer.record(closed("B"));
        }
        assert_eq!(
            hub.closed_at(&TargetId("A".into())),
            Some(latest),
            "another tab's records keep it"
        );
        for _ in 0..LOG_CAPACITY {
            writer.record(crashed("A"));
        }
        assert_eq!(hub.closed_at(&TargetId("A".into())), None);
    }

    #[test]
    fn an_already_read_close_can_outlive_ring_evidence_without_a_lost_signal()
    -> Result<(), Box<dyn Error>> {
        let (hub, mut writer) = EventHub::new();
        let close = writer.record(closed("A"));
        let read = hub.read_after(0);
        let pending = read
            .records
            .first()
            .ok_or("the waiter read its close")?
            .clone();
        assert_eq!(read.lost, 0);
        assert_eq!(hub.closed_at(&TargetId("A".into())), Some(close));
        for _ in 0..LOG_CAPACITY {
            writer.record(crashed("A"));
        }
        assert_eq!(
            pending.seq, close,
            "the waiter can still deliver the copied record"
        );
        assert_eq!(hub.closed_at(&TargetId("A".into())), None);
        assert!(hub.head() >= pending.seq);
        assert_eq!(read.lost, 0, "the copied response still reports no loss");
        assert!(hub.read_after(0).lost > 0, "a new read sees the eviction");
        Ok(())
    }
}
