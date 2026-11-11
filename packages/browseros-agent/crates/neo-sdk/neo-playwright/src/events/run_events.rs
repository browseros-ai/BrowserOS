//! One run's side of the event log: what it listens for, from when, and how far its pump has
//! read for each listener. Only the run's own event ops write here; the watcher only asks whether
//! the run wants an event, to decide what to record and which dialogs to hold.

use std::collections::HashMap;
use std::pin::pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use browseros_core::file_transfers::FileChooserInterception;
use browseros_core::{TabId, TargetId};
use tokio::sync::Notify;
use tokio::time;

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
    /// `waitForEvent` ops in flight, by what they wait for. Counted, because two may wait for
    /// the same thing at once.
    waiting: HashMap<Subscription, usize>,
    /// `events.next` calls in flight.
    pumps: usize,
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
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let mut inner = self.events.lock();
        if let Some(count) = inner.waiting.get_mut(&self.subscription) {
            *count -= 1;
            if *count == 0 {
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
        self.lock()
            .listening
            .retain(|subscription, _| subscription.key() != *key);
    }

    /// Counts one `waitForEvent` for `subscription` until the returned guard drops.
    pub(crate) fn waiting(&self, subscription: Subscription) -> Waiting<'_> {
        *self.lock().waiting.entry(subscription.clone()).or_default() += 1;
        Waiting {
            events: self,
            subscription,
        }
    }

    /// Whether the run listens for, or waits on, `event` on the tab whose target is `target`.
    pub(crate) fn wants(&self, target: &TargetId, event: PageEventKind) -> bool {
        let inner = self.lock();
        let wanted =
            |subscription: &Subscription| subscription.page_event() == Some((target, event));
        inner.listening.keys().any(wanted) || inner.waiting.keys().any(wanted)
    }

    /// Context lifecycle interest requires observing tabs the script has never acted on.
    pub(crate) fn wants_context_lifecycle(&self) -> bool {
        let inner = self.lock();
        inner
            .listening
            .keys()
            .chain(inner.waiting.keys())
            .any(|subscription| matches!(subscription, Subscription::Context(_)))
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

    /// Returns once the pump read every listener past log seq `head`, so whatever it found there
    /// is on its way to the script ahead of anything that ends later. With no pump in flight it
    /// waits [`PUMP_GRACE`] for one, then gives up: a stopped pump never holds an op. The caller
    /// bounds the whole wait.
    pub(crate) async fn taken_through(&self, head: u64) {
        loop {
            let mut changed = pin!(self.changed.notified());
            changed.as_mut().enable();
            let pumping = {
                let inner = self.lock();
                if inner
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

    use super::{PUMP_GRACE, RunEvents};
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
        let first = events.waiting(dialogs(1, "T"));
        let second = events.waiting(dialogs(1, "T"));
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
}
