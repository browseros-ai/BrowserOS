//! One run's side of the event log: what it listens for, from when, and how far its pump has
//! read for each listener. Only the run's own event ops write here; the watcher only asks whether
//! the run wants an event, to decide what to record and which dialogs to hold.

use std::collections::{HashMap, VecDeque};
use std::mem;
use std::pin::pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use browseros_core::file_transfers::FileChooserInterception;
use browseros_core::{RuntimeLease, TabId, TargetId};
use tokio::sync::Notify;
use tokio::time;

use crate::events::event_hub::LOG_CAPACITY;
use crate::events::page_event::BindingCalled;
use crate::events::{ListenerKey, PageEventKind, Subscription};

/// How long an op ending with events still unread waits for a pump to start, when none is in
/// flight: the prelude calls the next `events.next` as soon as it handled the last, so a pump
/// that does not come within this has stopped, and the op stops waiting for it.
const PUMP_GRACE: Duration = Duration::from_millis(200);

/// Behind `RunState`. A std mutex, held for a map update and never across an await.
#[derive(Debug, Default)]
pub(crate) struct RunEvents {
    inner: Mutex<Inner>,
    /// Woken when a pump starts or ends, or moves a listener's place.
    changed: Notify,
    /// How many `events.next` replies the run's pump has been given. Every reply envelope
    /// carries it, and the prelude holds an op's result until it has dispatched that many, so a
    /// pump reply that completed first runs its listeners first, whatever order the host's
    /// promises settle in (`RunState::pump_replies`).
    pump_replies: AtomicU64,
}

#[derive(Debug, Default)]
struct Inner {
    /// Explicit per-interest leases; run teardown releases any setup whose JS cleanup was cut.
    intercepting: HashMap<u64, Intercepting>,
    /// `page.on` and `context.on` listeners.
    listening: HashMap<Subscription, Listening>,
    /// `waitForEvent` ops in flight, by what they wait for: each one's `since`, because two may
    /// wait for the same thing at once.
    waiting: HashMap<Subscription, Vec<u64>>,
    /// `events.next` calls in flight.
    pumps: usize,
    /// Binding calls the watcher handed this run, oldest first, until its pump takes them. Kept
    /// apart from the bounded log: each is a request the page may be waiting on.
    calls: VecDeque<QueuedCall>,
    /// Calls dropped because [`QUEUED_CALLS`] were already waiting, since the pump last took
    /// them: the pump reports them in its `lost` count.
    dropped_calls: u64,
    /// View-approved identities remain usable by retained events after the tab disappears.
    known_pages: VecDeque<KnownPage>,
    /// The `Runtime` leases this run's listeners hold (console, page errors, bindings): core
    /// leaves `Runtime` off on page sessions, so these keep its events flowing exactly while
    /// someone listens. Removing the listener, or ending the run, drops them.
    runtime: HashMap<Subscription, Vec<RuntimeLease>>,
}

/// Marks one `events.next` in flight until dropped.
pub(crate) struct Pumping<'run> {
    events: &'run RunEvents,
}

impl Drop for Pumping<'_> {
    fn drop(&mut self) {
        let mut inner = self.events.lock();
        inner.pumps = inner.pumps.saturating_sub(1);
        drop(inner);
        self.events.changed.notify_waiters();
    }
}

/// How many binding calls wait for one run's pump at most. A page calls faster than the realm
/// answers only in a flood; past this many the oldest unread stay and the rest are counted lost,
/// so a page cannot grow Neo's memory without bound while the run is busy.
pub(crate) const QUEUED_CALLS: usize = 4_096;

/// A page's call of a binding this run exposed, as the watcher handed it over.
#[derive(Clone, Debug)]
pub(crate) struct QueuedCall {
    /// The log's head when the call came: the pump orders it after the records before it.
    pub(crate) seq: u64,
    pub(crate) at_ms: u64,
    /// The run's tab for the call's page.
    pub(crate) tab: TabId,
    pub(crate) call: BindingCalled,
}

/// Historical native identity, admitted by this run rather than guessed from a dead target.
#[derive(Debug)]
struct KnownPage {
    target: TargetId,
    tab: TabId,
}

#[derive(Debug)]
struct Intercepting {
    tab: TabId,
    lease: FileChooserInterception,
}

/// One listener's place in the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Listening {
    /// The script's `Date.now()` when it attached the listener: earlier events are not its.
    pub(crate) since_ms: u64,
    /// The last log seq the pump looked at for this listener. Each listener has its own, so a
    /// listener added after the pump read past an event still gets that event, if it is newer
    /// than `since_ms`.
    pub(crate) after: u64,
}

/// Marks one `waitForEvent` in flight until dropped, which is also when its op ends.
pub(crate) struct Waiting<'run> {
    events: &'run RunEvents,
    subscription: Subscription,
    since_ms: u64,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let mut inner = self.events.lock();
        if let Some(sinces) = inner.waiting.get_mut(&self.subscription) {
            if let Some(at) = sinces.iter().position(|since| *since == self.since_ms) {
                sinces.swap_remove(at);
            }
            if sinces.is_empty() {
                inner.waiting.remove(&self.subscription);
            }
        }
    }
}

impl RunEvents {
    pub(crate) fn keep_interception(&self, id: u64, tab: TabId, lease: FileChooserInterception) {
        self.lock()
            .intercepting
            .insert(id, Intercepting { tab, lease });
    }

    pub(crate) fn take_interception(&self, id: u64, tab: TabId) -> Option<FileChooserInterception> {
        let mut inner = self.lock();
        if inner
            .intercepting
            .get(&id)
            .is_some_and(|held| held.tab == tab)
        {
            inner.intercepting.remove(&id).map(|held| held.lease)
        } else {
            None
        }
    }
    /// Starts listening for `subscription` from `since_ms`, reading the log after seq `after`.
    /// Listening again keeps the first attachment, which is the earlier one.
    pub(crate) fn listen(&self, subscription: Subscription, since_ms: u64, after: u64) {
        self.lock()
            .listening
            .entry(subscription)
            .or_insert(Listening { since_ms, after });
    }

    /// Stops the listener `key` names, whatever target its page has now.
    pub(crate) fn unlisten(&self, key: &ListenerKey) {
        let mut inner = self.lock();
        inner
            .listening
            .retain(|subscription, _| subscription.key() != *key);
        inner
            .runtime
            .retain(|subscription, _| subscription.key() != *key);
    }

    /// Whether the run holds a `Runtime` lease for `subscription`.
    pub(crate) fn holds_runtime(&self, subscription: &Subscription) -> bool {
        self.lock()
            .runtime
            .get(subscription)
            .is_some_and(|leases| !leases.is_empty())
    }

    /// Releases the lease the run's bindings on `target` hold, once its last binding there is
    /// disposed. The binding-call listener stays: a later binding leases again.
    pub(crate) fn release_binding_runtime(&self, target: &TargetId) {
        self.lock().runtime.retain(|subscription, _| {
            subscription.page_event() != Some((target, PageEventKind::Binding))
        });
    }

    /// Keeps `leases` for as long as `subscription` listens. One removed while they were taken
    /// releases them at once.
    pub(crate) fn hold_runtime(&self, subscription: &Subscription, leases: Vec<RuntimeLease>) {
        let mut inner = self.lock();
        if inner.listening.contains_key(subscription) {
            inner
                .runtime
                .entry(subscription.clone())
                .or_default()
                .extend(leases);
        }
    }

    /// Counts one `waitForEvent` for `subscription`, which began at the script's `since_ms`,
    /// until the returned guard drops.
    pub(crate) fn waiting(&self, subscription: Subscription, since_ms: u64) -> Waiting<'_> {
        self.lock()
            .waiting
            .entry(subscription.clone())
            .or_default()
            .push(since_ms);
        Waiting {
            events: self,
            subscription,
            since_ms,
        }
    }

    /// Queues a binding call for the pump, or counts it lost when [`QUEUED_CALLS`] wait already.
    pub(crate) fn queue_call(&self, call: QueuedCall) {
        let mut inner = self.lock();
        if inner.calls.len() < QUEUED_CALLS {
            inner.calls.push_back(call);
        } else {
            inner.dropped_calls += 1;
        }
    }

    /// The binding calls queued since the pump last took them, and how many were dropped.
    pub(crate) fn take_calls(&self) -> (Vec<QueuedCall>, u64) {
        let mut inner = self.lock();
        let dropped = mem::take(&mut inner.dropped_calls);
        (inner.calls.drain(..).collect(), dropped)
    }

    /// The tab this run exposes bindings on whose target is `target`.
    pub(crate) fn binding_tab(&self, target: &TargetId) -> Option<TabId> {
        self.lock()
            .listening
            .keys()
            .find_map(|subscription| match subscription {
                Subscription::Page {
                    tab,
                    target: listened,
                    event: PageEventKind::Binding,
                } if listened == target => Some(*tab),
                _ => None,
            })
    }

    /// Whether the run listens for, or waits on, `event` on the tab whose target is `target`.
    pub(crate) fn wants(&self, target: &TargetId, event: PageEventKind) -> bool {
        let inner = self.lock();
        let wanted =
            |subscription: &Subscription| subscription.page_event() == Some((target, event));
        inner.listening.keys().any(wanted) || inner.waiting.keys().any(wanted)
    }

    /// Since when the run wants the dedicated workers of the tab whose target is `target` to
    /// report console messages and uncaught errors, which a worker does only while `Runtime` is
    /// on in it (`target_watcher.rs`); `None` if it does not. The script's clock when the earliest
    /// ask began: its `page.on('console' | 'pageerror')` listeners on the tab that leased the
    /// page's `Runtime`, and its `waitForEvent`s for them. A listener that took no lease
    /// (`event_op`'s `lease_runtime` decides) asks nothing of workers either: they follow the page.
    pub(crate) fn workers_asked_since(&self, target: &TargetId) -> Option<u64> {
        let inner = self.lock();
        let reported = |subscription: &Subscription| {
            subscription.page_event().is_some_and(|(listened, event)| {
                listened == target
                    && matches!(event, PageEventKind::Console | PageEventKind::PageError)
            })
        };
        let listening = inner
            .listening
            .iter()
            .filter(|(subscription, _)| {
                reported(subscription)
                    && inner
                        .runtime
                        .get(*subscription)
                        .is_some_and(|leases| !leases.is_empty())
            })
            .map(|(_, listening)| listening.since_ms);
        let waiting = inner
            .waiting
            .iter()
            .filter(|(subscription, _)| reported(subscription))
            .flat_map(|(_, sinces)| sinces.iter().copied());
        listening.chain(waiting).min()
    }

    /// Context lifecycle interest explicitly asks to observe untouched pages too.
    pub(crate) fn wants_context_lifecycle(&self) -> bool {
        let inner = self.lock();
        inner
            .listening
            .keys()
            .chain(inner.waiting.keys())
            .any(|subscription| matches!(subscription, Subscription::Context(_)))
    }

    /// Retain only identities approved by this run's view. The journal bounds how much
    /// history can still be delivered, so the identity cache follows the same capacity.
    pub(crate) fn remember_page(&self, target: &TargetId, tab: TabId) {
        let mut inner = self.lock();
        inner.known_pages.retain(|page| page.target != *target);
        inner.known_pages.push_back(KnownPage {
            target: target.clone(),
            tab,
        });
        while inner.known_pages.len() > LOG_CAPACITY {
            inner.known_pages.pop_front();
        }
    }

    /// A retained event may name a tab that no live listing can resolve any longer.
    pub(crate) fn known_page(&self, target: &TargetId) -> Option<TabId> {
        self.lock()
            .known_pages
            .iter()
            .find(|page| page.target == *target)
            .map(|page| page.tab)
    }

    /// The listeners and their places, for one pass of the pump.
    pub(crate) fn listeners(&self) -> Vec<(Subscription, Listening)> {
        self.lock()
            .listening
            .iter()
            .map(|(subscription, listening)| (subscription.clone(), *listening))
            .collect()
    }

    /// Moves each listener's place forward to the seq the pump read it up to. A listener removed
    /// meanwhile stays removed. `replying` says the pump answers with what it read: that reply
    /// is counted before an op waiting on these places wakes (`taken_through`), so the op's own
    /// reply never carries a count that misses it. A count whose reply then fails still ends as
    /// one the prelude dispatched, an error.
    pub(crate) fn advance(&self, read: &[(Subscription, u64)], replying: bool) {
        let mut inner = self.lock();
        for (subscription, seq) in read {
            if let Some(listening) = inner.listening.get_mut(subscription) {
                listening.after = listening.after.max(*seq);
            }
        }
        if replying {
            self.pump_replies.fetch_add(1, Ordering::AcqRel);
        }
        drop(inner);
        self.changed.notify_waiters();
    }

    /// The `events.next` replies given so far.
    pub(crate) fn pump_replies(&self) -> u64 {
        self.pump_replies.load(Ordering::Acquire)
    }

    /// Counts one `events.next` in flight until the returned guard drops.
    pub(crate) fn pumping(&self) -> Pumping<'_> {
        self.lock().pumps += 1;
        self.changed.notify_waiters();
        Pumping { events: self }
    }

    /// Whether the run has a `page.on` or `context.on` listener.
    pub(crate) fn has_listeners(&self) -> bool {
        !self.lock().listening.is_empty()
    }

    /// Returns once the pump read every listener past log seq `head` and took every queued
    /// binding call (calls wait in their own queue, not the log), so whatever it found is on its
    /// way to the script ahead of anything that ends later. With no pump in flight it waits
    /// [`PUMP_GRACE`] for one, then gives up: a stopped pump never holds an op. The caller
    /// bounds the whole wait.
    pub(crate) async fn taken_through(&self, head: u64) {
        loop {
            let mut changed = pin!(self.changed.notified());
            changed.as_mut().enable();
            let pumping = {
                let inner = self.lock();
                if inner.calls.is_empty()
                    && inner.dropped_calls == 0
                    && inner
                        .listening
                        .values()
                        .all(|listening| listening.after >= head)
                {
                    return;
                }
                inner.pumps > 0
            };
            if pumping {
                changed.await;
            } else if time::timeout(PUMP_GRACE, changed).await.is_err() {
                return;
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use browseros_core::{TabId, TargetId};
    use tokio::time;

    use super::{PUMP_GRACE, QueuedCall, RunEvents};
    use crate::events::page_event::BindingCalled;
    use crate::events::{ListenerKey, PageEventKind, Subscription};

    fn dialogs(page: u32, target: &str) -> Subscription {
        Subscription::Page {
            tab: TabId(i64::from(page)),
            target: TargetId(target.to_owned()),
            event: PageEventKind::Dialog,
        }
    }

    #[test]
    fn tests_that_a_wait_counts_as_wanting_only_while_it_runs() {
        let events = RunEvents::default();
        let target = TargetId("T".to_owned());
        let first = events.waiting(dialogs(1, "T"), 10);
        let second = events.waiting(dialogs(1, "T"), 10);
        drop(first);
        assert!(
            events.wants(&target, PageEventKind::Dialog),
            "one wait still runs"
        );
        drop(second);
        assert!(!events.wants(&target, PageEventKind::Dialog));
        assert!(!events.wants(&target, PageEventKind::Popup));
    }

    #[test]
    fn tests_that_listening_again_keeps_the_first_attachment() {
        let events = RunEvents::default();
        events.listen(dialogs(1, "T"), 100, 0);
        events.advance(&[(dialogs(1, "T"), 9)], false);
        events.listen(dialogs(1, "T"), 500, 0);
        let listeners = events.listeners();
        assert_eq!(listeners.len(), 1);
        assert_eq!(
            listeners.first().map(|(_, listening)| *listening),
            Some(super::Listening {
                since_ms: 100,
                after: 9
            })
        );
    }

    #[test]
    fn tests_that_unlisten_removes_only_that_page_and_event() {
        let events = RunEvents::default();
        events.listen(dialogs(1, "T"), 0, 0);
        events.listen(dialogs(2, "U"), 0, 0);
        events.listen(Subscription::ContextPage, 0, 0);
        events.unlisten(&ListenerKey::Page(TabId(1), PageEventKind::Dialog));
        assert!(!events.wants(&TargetId("T".to_owned()), PageEventKind::Dialog));
        assert!(events.wants(&TargetId("U".to_owned()), PageEventKind::Dialog));
        events.unlisten(&ListenerKey::ContextPage);
        assert_eq!(events.listeners().len(), 1, "the context listener went");
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_an_op_waits_for_the_pump_in_flight_to_take_its_events_and_count_its_reply()
    {
        let events = Arc::new(RunEvents::default());
        events.listen(dialogs(1, "T"), 0, 3);
        let pumping = events.pumping();
        let waiter = tokio::spawn({
            let events = Arc::clone(&events);
            async move {
                events.taken_through(7).await;
                events.pump_replies()
            }
        });
        time::sleep(PUMP_GRACE * 4).await;
        assert!(
            !waiter.is_finished(),
            "a pump in flight is waited for past the grace"
        );
        events.advance(&[(dialogs(1, "T"), 7)], true);
        let replies = time::timeout(Duration::from_secs(1), waiter).await;
        assert!(
            matches!(replies, Ok(Ok(1))),
            "the reply is counted by the time the op wakes: {replies:?}"
        );
        drop(pumping);
        events.advance(&[(dialogs(1, "T"), 9)], false);
        assert_eq!(
            events.pump_replies(),
            1,
            "a read with nothing to reply counts none"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_no_pump_holds_an_op_only_for_the_grace() {
        let events = RunEvents::default();
        events.listen(dialogs(1, "T"), 0, 3);
        let started = time::Instant::now();
        events.taken_through(7).await;
        assert_eq!(started.elapsed(), PUMP_GRACE);
        events.listen(dialogs(2, "U"), 0, 9);
        events.unlisten(&dialogs(1, "T").key());
        let started = time::Instant::now();
        events.taken_through(7).await;
        assert_eq!(
            started.elapsed(),
            Duration::ZERO,
            "every listener is past the head"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_an_op_waits_for_the_pump_to_take_the_runs_queued_binding_calls() {
        let events = Arc::new(RunEvents::default());
        let binding = Subscription::Page {
            tab: TabId(1),
            target: TargetId("T".to_owned()),
            event: PageEventKind::Binding,
        };
        events.listen(binding.clone(), 0, 7);
        events.queue_call(QueuedCall {
            seq: 7,
            at_ms: 0,
            tab: TabId(1),
            call: BindingCalled {
                target: TargetId("T".to_owned()),
                context: 3,
                frame: None,
                payload: "{}".to_owned(),
            },
        });
        let _pumping = events.pumping();
        let waiter = tokio::spawn({
            let events = Arc::clone(&events);
            async move {
                events.taken_through(7).await;
                events.pump_replies()
            }
        });
        time::sleep(PUMP_GRACE * 4).await;
        assert!(
            !waiter.is_finished(),
            "every listener is past the head, but a call waits in the queue"
        );
        let (calls, dropped) = events.take_calls();
        assert_eq!((calls.len(), dropped), (1, 0));
        events.advance(&[(binding, 7)], true);
        let replies = time::timeout(Duration::from_secs(1), waiter).await;
        assert!(matches!(replies, Ok(Ok(1))), "{replies:?}");
    }
}
