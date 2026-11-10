//! The numbered event history behind `GET /v1/events`.
//!
//! The trace writer's broadcast has no ids and no history, so one numbering task gives every
//! event a sequence number and keeps the newest 1024 in a ring that every SSE client reads.
//! Ids are `<boot>-<seq>`, with `boot` random per process, so an id from before a restart
//! never matches and resets instead of silently skipping what the restart lost.
//!
//! ```text
//!  TraceSink ─ broadcast ─▶ numbering task ─ hydrate: one store read ─▶ publish ─▶ ring
//!  Locks ───── broadcast ─▶ lock task ─ the lock is the payload ──────▶ publish ─┤
//!  Takeovers ─ broadcast ─▶ takeover task ─ the takeover is the payload ▶ publish ┤
//!  Service::tab_closed ───────────────────────────────────────────────▶ publish ─┤
//!                                                                                   │
//!                                              each SSE client reads after its seq ◀┘
//! ```
//!
//! `EventLog::publish` is the only way in, so every source of events joins the same numbering
//! and resume by calling it. Each source keeps its own events in order; the ring orders the
//! sources by the moment each event is published.
//!
//! An event's JSON is built once, when it enters the ring, so a hundred clients cost one store
//! read. Hydration reads the record as the store holds it when the task gets there, which can
//! be later than the event: a `run.started` may carry a run that has already ended. The type
//! says what happened; the payload is the record's current state.
//!
//! A lagged broadcast means events no client will ever see, so the task publishes a
//! `stream.reset` entry, which passes every filter, and clients re-read the state endpoints.

use crate::event_resume::{self, Bounds, Resume};
use crate::limits::EVENT_RING;
use crate::wire;
use browseros_core::TabId;
use neo_sdk_api as api;
use neo_takeover::{Lock, Takeover};
use neo_trace::{
    EpochMillis, OperationId, PrivacyRecord, RunId, TaskId, TraceRecordEvent, TraceStore,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch};
use ulid::Ulid;

/// What an event is: its SSE `event:` name and its `Event.type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EventKind {
    RunStarted,
    RunEnded,
    OperationStarted,
    OperationEnded,
    TabPrivate,
    TabClosed,
    LockState,
    TakeoverState,
    StreamReset,
}

impl EventKind {
    /// The wire name.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::RunStarted => "run.started",
            Self::RunEnded => "run.ended",
            Self::OperationStarted => "operation.started",
            Self::OperationEnded => "operation.ended",
            Self::TabPrivate => "tab.private",
            Self::TabClosed => "tab.closed",
            Self::LockState => "lock.state",
            Self::TakeoverState => "takeover.state",
            Self::StreamReset => "stream.reset",
        }
    }
}

/// One event as the log keeps it: what it is about, for the filters, and its `Event` JSON.
#[derive(Debug)]
pub(crate) struct LogEvent {
    pub(crate) kind: EventKind,
    pub(crate) task: Option<TaskId>,
    pub(crate) tab: Option<TabId>,
    pub(crate) data: Arc<str>,
}

impl LogEvent {
    /// An event about `task` and `tab`, as the contract's `Event`.
    pub(crate) fn new(
        kind: EventKind,
        task: Option<TaskId>,
        tab: Option<TabId>,
        event: &api::Event,
    ) -> Self {
        match serde_json::to_string(event) {
            Ok(data) => Self {
                kind,
                task,
                tab,
                data: Arc::from(data),
            },
            // Never taken: every map in these models has string keys. If it were, the lost
            // event still reaches clients, as a reset.
            Err(error) => {
                tracing::error!(%error, "an event could not be serialized");
                Self::reset(EpochMillis::from_millis(event.at))
            }
        }
    }

    /// `stream.reset`: the contract's `Event` with only its type and time.
    pub(crate) fn reset(at: EpochMillis) -> Self {
        Self {
            kind: EventKind::StreamReset,
            task: None,
            tab: None,
            data: Arc::from(format!(
                r#"{{"type":"{}","at":{}}}"#,
                EventKind::StreamReset.as_str(),
                at.millis()
            )),
        }
    }
}

/// A numbered event in the ring.
#[derive(Debug)]
pub(crate) struct Entry {
    pub(crate) seq: u64,
    pub(crate) event: LogEvent,
}

/// `GET /v1/events?taskId=&tab=`: each given filter must match the event's key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EventFilter {
    pub(crate) task: Option<TaskId>,
    pub(crate) tab: Option<TabId>,
}

impl EventFilter {
    /// `stream.reset` passes every filter: the events it stands for may have matched.
    pub(crate) fn admits(&self, event: &LogEvent) -> bool {
        event.kind == EventKind::StreamReset
            || (self
                .task
                .as_ref()
                .is_none_or(|task| event.task.as_ref() == Some(task))
                && self.tab.is_none_or(|tab| event.tab == Some(tab)))
    }
}

/// What a reader at some seq reads next.
pub(crate) enum Read {
    /// The entries after its seq, oldest first; none when it is up to date.
    Entries(Vec<Arc<Entry>>),
    /// The ring dropped entries it never read.
    Missed { newest: u64 },
}

/// The event history. Publishers and readers share one short `std` mutex around the ring;
/// nothing awaits while holding it.
pub(crate) struct EventLog {
    boot: String,
    ring: Mutex<Ring>,
    /// Bumped on every publish, so a waiting reader wakes.
    published: watch::Sender<()>,
}

struct Ring {
    capacity: usize,
    /// The seq the next entry gets. Seqs start at 1, so 0 reads as "before every event".
    next: u64,
    entries: VecDeque<Arc<Entry>>,
}

impl Ring {
    fn bounds(&self) -> Bounds {
        Bounds {
            oldest: self.entries.front().map_or(self.next, |entry| entry.seq),
            newest: self.next.saturating_sub(1),
        }
    }
}

impl EventLog {
    /// An empty log holding the newest `capacity` events, with a fresh boot id.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            boot: Ulid::new().to_string(),
            ring: Mutex::new(Ring {
                capacity,
                next: 1,
                entries: VecDeque::with_capacity(capacity),
            }),
            published: watch::channel(()).0,
        }
    }

    /// Starts the numbering task over `trace`, a receiver the caller subscribed before this
    /// call so that no event committed after it is missed. The task ends when the writer's
    /// last sink is gone.
    pub(crate) fn start(
        store: TraceStore,
        trace: broadcast::Receiver<TraceRecordEvent>,
    ) -> Arc<Self> {
        let log = Arc::new(Self::new(EVENT_RING));
        tokio::spawn(number(Arc::clone(&log), store, trace));
        log
    }

    /// Starts the task that numbers lock changes as `lock.state` events, over `changes`, a
    /// receiver the caller subscribed before this call. It ends when the locks are dropped.
    pub(crate) fn follow_locks(self: &Arc<Self>, changes: broadcast::Receiver<Lock>) {
        tokio::spawn(lock_states(Arc::clone(self), changes));
    }

    /// Starts the task that numbers takeover changes as `takeover.state` events, over `changes`,
    /// a receiver the caller subscribed before this call. It ends when the takeovers are dropped.
    pub(crate) fn follow_takeovers(self: &Arc<Self>, changes: broadcast::Receiver<Takeover>) {
        tokio::spawn(takeover_states(Arc::clone(self), changes));
    }

    /// Numbers `event` and appends it, dropping the oldest entry once the ring is full.
    pub(crate) fn publish(&self, event: LogEvent) {
        {
            let mut ring = self.lock();
            let seq = ring.next;
            ring.next = seq.saturating_add(1);
            if ring.entries.len() >= ring.capacity {
                ring.entries.pop_front();
            }
            ring.entries.push_back(Arc::new(Entry { seq, event }));
        }
        // After the entry is in the ring, so a reader woken by this finds it there.
        self.published.send_replace(());
    }

    /// This process's boot id, the first half of every event id.
    pub(crate) fn boot(&self) -> &str {
        &self.boot
    }

    /// Where a client that sent `last_event_id` starts.
    pub(crate) fn resume(&self, last_event_id: Option<&str>) -> Resume {
        let bounds = self.lock().bounds();
        event_resume::resume(last_event_id, &self.boot, bounds)
    }

    /// What a reader that has read every entry up to `seq` reads next.
    pub(crate) fn read_after(&self, seq: u64) -> Read {
        let ring = self.lock();
        let bounds = ring.bounds();
        if event_resume::missed(seq, bounds) {
            return Read::Missed {
                newest: bounds.newest,
            };
        }
        Read::Entries(
            ring.entries
                .iter()
                .filter(|entry| entry.seq > seq)
                .cloned()
                .collect(),
        )
    }

    /// Wakes when an entry is published. A reader marks it seen before it reads the ring, so a
    /// publish between its read and its wait still wakes it.
    pub(crate) fn subscribe(&self) -> watch::Receiver<()> {
        self.published.subscribe()
    }

    fn lock(&self) -> MutexGuard<'_, Ring> {
        self.ring.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The numbering task: the one reader of the trace writer's broadcast. Every event becomes one
/// entry, in commit order.
async fn number(
    log: Arc<EventLog>,
    store: TraceStore,
    mut trace: broadcast::Receiver<TraceRecordEvent>,
) {
    loop {
        let event = match trace.recv().await {
            Ok(event) => match hydrate(&store, event).await {
                Some(event) => event,
                None => continue,
            },
            Err(RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "the event log fell behind the trace writer");
                LogEvent::reset(EpochMillis::now())
            }
            Err(RecvError::Closed) => return,
        };
        log.publish(event);
    }
}

/// The lock task: each lock change is one `lock.state` event about the lock's tab. A lagged
/// broadcast lost changes no client will see, so it publishes a reset, as the trace's does.
async fn lock_states(log: Arc<EventLog>, mut changes: broadcast::Receiver<Lock>) {
    loop {
        let event = match changes.recv().await {
            Ok(lock) => LogEvent::new(
                EventKind::LockState,
                None,
                Some(lock.tab),
                &wire::lock_event(&lock),
            ),
            Err(RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "the event log fell behind the lock changes");
                LogEvent::reset(EpochMillis::now())
            }
            Err(RecvError::Closed) => return,
        };
        log.publish(event);
    }
}

/// The takeover task: each start and release is one `takeover.state` event about the takeover's
/// tab, the way the lock task numbers lock changes.
async fn takeover_states(log: Arc<EventLog>, mut changes: broadcast::Receiver<Takeover>) {
    loop {
        let event = match changes.recv().await {
            Ok(takeover) => LogEvent::new(
                EventKind::TakeoverState,
                None,
                Some(takeover.tab),
                &wire::takeover_event(&takeover),
            ),
            Err(RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "the event log fell behind the takeover changes");
                LogEvent::reset(EpochMillis::now())
            }
            Err(RecvError::Closed) => return,
        };
        log.publish(event);
    }
}

/// The contract's `Event` for one trace event, or `None` for an internal one. A record that
/// cannot be read becomes a reset: an event without its record would name nothing a client can
/// use.
async fn hydrate(store: &TraceStore, event: TraceRecordEvent) -> Option<LogEvent> {
    let event = match event {
        TraceRecordEvent::RunStarted { run, at, .. } => {
            run_event(store, EventKind::RunStarted, run, at).await
        }
        TraceRecordEvent::RunEnded { run, at, .. } => {
            run_event(store, EventKind::RunEnded, run, at).await
        }
        TraceRecordEvent::OperationStarted { id, task, at, .. } => {
            operation_event(store, EventKind::OperationStarted, id, task, at).await
        }
        TraceRecordEvent::OperationEnded { id, task, at, .. } => {
            operation_event(store, EventKind::OperationEnded, id, task, at).await
        }
        TraceRecordEvent::TabPrivate { tab, reason, at } => {
            let record = PrivacyRecord { since: at, reason };
            LogEvent::new(
                EventKind::TabPrivate,
                None,
                Some(tab),
                &wire::tab_private_event(tab, record),
            )
        }
        // It wakes `wait=`; SSE clients read `complete` from the run itself.
        TraceRecordEvent::RunComplete { .. } => return None,
    };
    Some(event)
}

/// A run event is about the run's task; a run spans tabs, so it is about none of them.
async fn run_event(store: &TraceStore, kind: EventKind, run: RunId, at: EpochMillis) -> LogEvent {
    match store.run_summary(run).await {
        Ok(Some(summary)) => LogEvent::new(
            kind,
            summary.task_id.clone(),
            None,
            &wire::run_event(kind, at, &summary),
        ),
        Ok(None) => {
            tracing::warn!(%run, "an event's run is not in the store");
            LogEvent::reset(at)
        }
        Err(error) => {
            tracing::error!(%error, %run, "could not read an event's run");
            LogEvent::reset(at)
        }
    }
}

/// An operation event is about its run's task and the tab it targets.
async fn operation_event(
    store: &TraceStore,
    kind: EventKind,
    id: OperationId,
    task: Option<TaskId>,
    at: EpochMillis,
) -> LogEvent {
    match store.operation(id).await {
        Ok(Some(op)) => {
            let event = wire::operation_event(kind, at, task.as_ref(), &op);
            LogEvent::new(kind, task, op.tab, &event)
        }
        Ok(None) => {
            tracing::warn!(operation = %id, "an event's operation is not in the store");
            LogEvent::reset(at)
        }
        Err(error) => {
            tracing::error!(%error, operation = %id, "could not read an event's operation");
            LogEvent::reset(at)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EventFilter, EventKind, EventLog, LogEvent, Read, number};
    use browseros_core::TabId;
    use neo_trace::{EpochMillis, PrivateReason, TaskId, TraceRecordEvent, TraceStore};
    use std::error::Error;
    use std::sync::Arc;

    fn tab_event(tab: i64) -> LogEvent {
        LogEvent::reset(EpochMillis::from_millis(tab)).with_keys(
            EventKind::TabPrivate,
            None,
            Some(TabId(tab)),
        )
    }

    impl LogEvent {
        fn with_keys(self, kind: EventKind, task: Option<TaskId>, tab: Option<TabId>) -> Self {
            Self {
                kind,
                task,
                tab,
                ..self
            }
        }
    }

    fn seqs(read: Read) -> Option<Vec<u64>> {
        match read {
            Read::Entries(entries) => Some(entries.iter().map(|entry| entry.seq).collect()),
            Read::Missed { .. } => None,
        }
    }

    #[test]
    fn the_ring_keeps_the_newest_entries_and_reports_what_a_reader_missed() {
        let log = EventLog::new(4);
        for tab in 1..=10 {
            log.publish(tab_event(tab));
        }
        assert_eq!(
            seqs(log.read_after(6)),
            Some(vec![7, 8, 9, 10]),
            "the oldest kept is 7"
        );
        assert_eq!(seqs(log.read_after(10)), Some(vec![]));
        assert!(
            matches!(log.read_after(5), Read::Missed { newest: 10 }),
            "6 was dropped"
        );
    }

    #[test]
    fn filters_match_their_keys_and_resets_pass_them_all() {
        let task = TaskId::parse("checkout").ok();
        let filter = EventFilter {
            task: task.clone(),
            tab: Some(TabId(4)),
        };
        let event = |task: Option<TaskId>, tab: Option<i64>| {
            LogEvent::reset(EpochMillis::from_millis(0)).with_keys(
                EventKind::OperationEnded,
                task,
                tab.map(TabId),
            )
        };
        assert!(filter.admits(&event(task.clone(), Some(4))));
        assert!(!filter.admits(&event(task.clone(), Some(3))), "another tab");
        assert!(!filter.admits(&event(None, Some(4))), "no task");
        assert!(
            !filter.admits(&event(task, None)),
            "a run event is about no tab"
        );
        assert!(filter.admits(&LogEvent::reset(EpochMillis::from_millis(0))));
        assert!(EventFilter::default().admits(&event(None, None)));
    }

    #[test]
    fn a_reset_is_the_contracts_event() -> Result<(), Box<dyn Error>> {
        let reset = LogEvent::reset(EpochMillis::from_millis(42));
        let event: neo_sdk_api::Event = serde_json::from_str(&reset.data)?;
        assert_eq!(event, neo_sdk_api::Event::new("stream.reset".into(), 42));
        Ok(())
    }

    /// The broadcast holds two events here, so five sent before the task reads lose three: the
    /// log says so with a reset before the two it still got.
    #[tokio::test]
    async fn a_lagged_broadcast_publishes_a_reset() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let store = TraceStore::open(&dir.path().join("trace.db")).await?;
        let (sender, receiver) = tokio::sync::broadcast::channel(2);
        for tab in 1..=5 {
            sender.send(TraceRecordEvent::TabPrivate {
                tab: TabId(tab),
                reason: PrivateReason::Host,
                at: EpochMillis::from_millis(tab),
            })?;
        }
        drop(sender);
        let log = Arc::new(EventLog::new(16));
        number(Arc::clone(&log), store, receiver).await;
        let Read::Entries(entries) = log.read_after(0) else {
            panic!("nothing was dropped from the ring");
        };
        let kinds = entries
            .iter()
            .map(|entry| (entry.event.kind, entry.event.tab))
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                (EventKind::StreamReset, None),
                (EventKind::TabPrivate, Some(TabId(4))),
                (EventKind::TabPrivate, Some(TabId(5))),
            ]
        );
        Ok(())
    }
}
