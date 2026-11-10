//! The process-wide event log: one writer, many readers.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use browseros_core::TargetId;
use neo_trace::EpochMillis;
use tokio::sync::Notify;
use tokio::sync::futures::Notified;

use crate::RunState;
use crate::events::dialog_table::{self, DialogAnswer, DialogTable, Unanswerable};
use crate::events::page_event::{DialogId, DialogKind, PageEvent};
use crate::events::{PageEventKind, Subscription};

/// How many records the log keeps. A reader that falls further behind is told how many it lost.
pub(crate) const LOG_CAPACITY: usize = 1024;

/// The log of page events since the `Playwright` started, the runs that read it, and the
/// dialogs held for them. Shared by the facade (readers) and the target watcher (the writer).
#[derive(Debug, Default)]
pub(crate) struct EventHub {
    /// A std mutex: every holder copies or appends and lets go, never across an await.
    log: Mutex<EventLog>,
    /// Woken after every record, and when a run adds a listener, so readers re-read.
    changed: Notify,
    /// Runs that used an event op, weakly: a run's state dies with the run. Pruned as it goes.
    runs: Mutex<Vec<Weak<RunState>>>,
    dialogs: Mutex<DialogTable>,
}

#[derive(Debug, Default)]
struct EventLog {
    records: VecDeque<EventRecord>,
    /// The seq the last record got; seqs start at 1, so 0 reads "from the start".
    last_seq: u64,
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
    /// Records after the reader's place that fell off the ring before it read them.
    pub(crate) lost: u64,
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

    /// The records after seq `after`.
    pub(crate) fn read_after(&self, after: u64) -> LogRead {
        let log = self.lock_log();
        let oldest = log
            .records
            .front()
            .map_or(log.last_seq + 1, |first| first.seq);
        LogRead {
            records: log
                .records
                .iter()
                .filter(|record| record.seq > after)
                .cloned()
                .collect(),
            head: log.last_seq,
            lost: oldest.saturating_sub(after + 1),
        }
    }

    /// Where a listener attached at `since_ms` starts reading: after the newest record older
    /// than that, or at the oldest record kept when every one is newer.
    pub(crate) fn position_at(&self, since_ms: u64) -> u64 {
        let log = self.lock_log();
        let oldest = log
            .records
            .front()
            .map_or(log.last_seq + 1, |first| first.seq);
        log.records
            .iter()
            .take_while(|record| record.at_ms < since_ms)
            .last()
            .map_or(oldest.saturating_sub(1), |record| record.seq)
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

    /// The seq of the newest record: a reader's place for "from now on".
    pub(crate) fn head(&self) -> u64 {
        self.lock_log().last_seq
    }

    /// The type of a dialog recorded on `target` after seq `after` that is still open, if
    /// any. An action whose input such a dialog blocks names it when it times out: Chromium
    /// acknowledges no input and answers no page call until the dialog closes (measured).
    pub(crate) fn open_dialog_after(&self, after: u64, target: &TargetId) -> Option<&'static str> {
        let opened: Vec<(DialogId, DialogKind)> = self
            .lock_log()
            .records
            .iter()
            .filter(|record| record.seq > after)
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
    /// Appends `event`, dropping the oldest record past [`LOG_CAPACITY`], and wakes the readers.
    pub(crate) fn record(&mut self, event: PageEvent) -> u64 {
        let seq = {
            let mut log = self.hub.lock_log();
            log.last_seq += 1;
            let seq = log.last_seq;
            log.records.push_back(EventRecord {
                seq,
                at_ms: now_ms(),
                event,
            });
            while log.records.len() > LOG_CAPACITY {
                log.records.pop_front();
            }
            seq
        };
        // Outside the lock: woken readers take it at once.
        self.hub.changed.notify_waiters();
        seq
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
    use std::sync::Arc;

    use browseros_core::TargetId;
    use neo_trace::EpochMillis;

    use super::{EventHub, LOG_CAPACITY};
    use crate::RunState;
    use crate::events::page_event::PageEvent;
    use crate::events::{PageEventKind, Subscription};

    fn closed(target: &str) -> PageEvent {
        PageEvent::Closed(TargetId(target.to_owned()))
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

    /// claw-sdk compares `at_ms` with takeover lock times, which `EpochMillis::now` stamps.
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
        let _waiting = listening.events.waiting(Subscription::Page {
            page: browseros_core::PageId(1),
            tab: browseros_core::TabId(1),
            target: target.clone(),
            event: PageEventKind::Dialog,
        });
        assert_eq!(hub.listeners(&target, PageEventKind::Dialog).len(), 1);
        assert!(hub.listeners(&target, PageEventKind::Console).is_empty());
    }
}
