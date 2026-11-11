//! The process-wide event log: one writer, many readers.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use browseros_core::{SessionId, TargetId};
use neo_trace::EpochMillis;
use serde::Deserialize;
use tokio::sync::Notify;
use tokio::sync::futures::Notified;

use crate::RunState;
use crate::events::dialog_table::{self, DialogAnswer, DialogTable, Unanswerable};
use crate::events::download_table::DownloadTable;
use crate::events::page_event::{BindingCalled, DialogId, DialogKind, PageEvent};
use crate::events::run_events::QueuedCall;
use crate::events::{PageEventKind, Subscription};

/// How many records the log keeps. A reader that falls further behind is told how many it lost.
pub(crate) const LOG_CAPACITY: usize = 1024;

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
    /// Each page session whose `Log` domain Neo enabled or is enabling (`EventHub::start_log`).
    log_sessions: Mutex<HashMap<SessionId, LogState>>,
}

/// Where one page session's `Log` domain stands for Neo. Blink's `Log.enable` replays the
/// entries the page logged so far before it replies, and is a no-op once enabled. So an entry
/// can be a replay only until the watcher passes the reply's place on the client's broadcast
/// numbering, and is one when the page logged it before the session's first console listener
/// or wait attached: upstream, which enables `Log` on every page, would have emitted it before
/// that listener existed. Every entry after the reply is live, whatever its timestamp.
#[derive(Debug)]
enum LogState {
    Enabling {
        /// The earliest console listener's or wait's `since`, in ms since the epoch.
        since_ms: f64,
        /// The client's broadcast count when the reply came: the replay is at or before it.
        through: Option<u64>,
        /// Whether a `Log.enable` is out now; a cancelled one lets the next listener resend.
        sending: bool,
    },
    Enabled,
}

/// One `Log.enable` to send on a session ([`EventHub::start_log`]). Dropped unsettled, as by a
/// cancelled op, it keeps the replay filter, since a replay may still come, and lets the next
/// console listener send again.
pub(crate) struct LogEnabling<'hub> {
    hub: &'hub EventHub,
    session: SessionId,
    settled: bool,
}

impl LogEnabling<'_> {
    /// The enable replied when the client had broadcast `through` events. The watcher ends the
    /// replay filter once it passes that place (`EventHub::log_entry_is_new`).
    pub(crate) fn replied(mut self, through: u64) {
        if let Some(LogState::Enabling {
            through: at,
            sending,
            ..
        }) = self.hub.lock_log_sessions().get_mut(&self.session)
        {
            *at = Some(through);
            *sending = false;
        }
        self.settled = true;
    }

    /// The page refused the enable, so it replays nothing: the next listener starts over.
    pub(crate) fn refused(mut self) {
        self.hub.lock_log_sessions().remove(&self.session);
        self.settled = true;
    }
}

impl Drop for LogEnabling<'_> {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        if let Some(LogState::Enabling { sending, .. }) =
            self.hub.lock_log_sessions().get_mut(&self.session)
        {
            *sending = false;
        }
    }
}

#[derive(Debug, Default)]
struct EventLog {
    records: VecDeque<EventRecord>,
    /// The seq the last record got; seqs start at 1, so 0 reads "from the start".
    last_seq: u64,
    /// Each open tab's newest main-frame navigation, kept apart from `records` so the bounded
    /// ring cannot evict one the host has not refreshed its URL facts for. The watcher marks
    /// them (`EventWriter::main_frame_navigated`); a tab's entry goes with its recorded close.
    navigated: HashMap<TargetId, u64>,
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

    /// A console listener or wait attached at `since_ms` on `session`'s page: the `Log.enable`
    /// to send, unless one is out or replied, or the domain is on. Each listener's `since`
    /// lowers the replay cutoff, so no listener loses an entry logged after it attached.
    pub(crate) fn start_log(&self, session: &SessionId, since_ms: f64) -> Option<LogEnabling<'_>> {
        let mut sessions = self.lock_log_sessions();
        let send = match sessions.get_mut(session) {
            None => {
                sessions.insert(
                    session.clone(),
                    LogState::Enabling {
                        since_ms,
                        through: None,
                        sending: true,
                    },
                );
                true
            }
            Some(LogState::Enabled) => false,
            Some(LogState::Enabling {
                since_ms: cutoff,
                through,
                sending,
            }) => {
                *cutoff = cutoff.min(since_ms);
                let resend = !*sending && through.is_none();
                *sending |= resend;
                resend
            }
        };
        send.then(|| LogEnabling {
            hub: self,
            session: session.clone(),
            settled: false,
        })
    }

    /// Whether an entry `session` logged at `timestamp`, the broadcast's event number
    /// `position`, is news ([`LogState`]). Past the enable reply's place the replay is over.
    pub(crate) fn log_entry_is_new(
        &self,
        session: &SessionId,
        timestamp: f64,
        position: u64,
    ) -> bool {
        let mut sessions = self.lock_log_sessions();
        let Some(state) = sessions.get_mut(session) else {
            return true;
        };
        match state {
            LogState::Enabled => true,
            LogState::Enabling {
                through: Some(through),
                ..
            } if position > *through => {
                *state = LogState::Enabled;
                true
            }
            LogState::Enabling { since_ms, .. } => timestamp >= *since_ms,
        }
    }

    /// The session detached: a later one with its id starts over.
    pub(crate) fn forget_log(&self, session: &SessionId) {
        self.lock_log_sessions().remove(session);
    }

    fn lock_log_sessions(&self) -> MutexGuard<'_, HashMap<SessionId, LogState>> {
        self.log_sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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

    /// The seq of the newest record: a reader's place for "from now on".
    pub(crate) fn head(&self) -> u64 {
        self.lock_log().last_seq
    }

    /// The newest close of this target still retained by the bounded event log.
    pub(crate) fn closed_at(&self, target: &TargetId) -> Option<u64> {
        self.lock_log()
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
    /// Appends `event`, dropping the oldest record past [`LOG_CAPACITY`], and wakes the readers.
    pub(crate) fn record(&mut self, event: PageEvent) -> u64 {
        let seq = {
            let mut log = self.hub.lock_log();
            log.last_seq += 1;
            let seq = log.last_seq;
            if let PageEvent::Closed(target) = &event {
                log.navigated.remove(target);
            }
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
    use std::sync::Arc;

    use browseros_core::{FrameId, SessionId, TabId, TargetId};
    use neo_trace::EpochMillis;

    use super::{EventHub, LOG_CAPACITY};
    use crate::RunState;
    use crate::events::navigation_event::NavigationEvent;
    use crate::events::page_event::{BindingCalled, PageEvent};
    use crate::events::run_events::QUEUED_CALLS;
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
        let moved = writer.record(navigated("A"));
        writer.main_frame_navigated(target.clone(), moved);
        let child = writer.record(navigated("child-frame"));
        assert_eq!(
            hub.navigated_at(&TargetId("child-frame".to_owned())),
            None,
            "a navigation the watcher did not mark as a main frame's is not kept: {child}"
        );
        for _ in 0..LOG_CAPACITY + 5 {
            writer.record(closed("B"));
        }
        assert_eq!(hub.navigated_at(&target), Some(moved));
        writer.record(closed("A"));
        assert_eq!(hub.navigated_at(&target), None);
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
            tab: TabId(1),
            target: target.clone(),
            event: PageEventKind::Dialog,
        });
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
            writer.record(closed("B"));
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

    /// CM-008: one `Log.enable` per page session; until the watcher passes the reply's place an
    /// entry logged before the earliest console listener attached is a replay, and past it every
    /// entry is live, a repeated or earlier timestamp included. A refused enable starts over, and
    /// a cancelled one keeps its filter while the next listener sends again.
    #[test]
    fn tests_that_only_entries_before_the_reply_from_before_the_listener_are_replays()
    -> Result<(), String> {
        let (hub, _writer) = EventHub::new();
        let session = SessionId::from("S".to_owned());
        let enabling = hub
            .start_log(&session, 1_000.0)
            .ok_or("the first console listener enables Log")?;
        assert!(
            hub.start_log(&session, 900.0).is_none(),
            "one enable at a time; an earlier listener lowers the cutoff"
        );
        assert!(!hub.log_entry_is_new(&session, 800.0, 3), "a replay");
        assert!(
            hub.log_entry_is_new(&session, 950.0, 4),
            "after the earlier listener attached"
        );
        enabling.replied(10);
        assert!(
            !hub.log_entry_is_new(&session, 850.0, 10),
            "still the replay"
        );
        assert!(
            hub.log_entry_is_new(&session, 850.0, 11),
            "past the reply: live"
        );
        assert!(
            hub.log_entry_is_new(&session, 850.0, 12),
            "the same time again, still live"
        );
        assert!(hub.start_log(&session, 2_000.0).is_none(), "enabled");

        let refused = SessionId::from("T".to_owned());
        hub.start_log(&refused, 1_000.0)
            .ok_or("an enable")?
            .refused();
        assert!(
            hub.start_log(&refused, 1_000.0).is_some(),
            "a refused enable starts over"
        );

        let cancelled = SessionId::from("U".to_owned());
        drop(hub.start_log(&cancelled, 1_000.0));
        assert!(
            !hub.log_entry_is_new(&cancelled, 500.0, 1),
            "a cancelled enable keeps its filter"
        );
        assert!(
            hub.start_log(&cancelled, 1_000.0).is_some(),
            "and is sent again"
        );

        hub.forget_log(&session);
        assert!(
            hub.start_log(&session, 3_000.0).is_some(),
            "a detached session starts over"
        );
        Ok(())
    }
}
