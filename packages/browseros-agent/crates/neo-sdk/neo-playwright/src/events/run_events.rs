//! One run's side of the event log: what it listens for, from when, and how far its pump has
//! read for each listener. Only the run's own event ops write here; the watcher only asks whether
//! the run wants an event, to decide what to record and which dialogs to hold.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};

use browseros_core::file_transfers::FileChooserInterception;
use browseros_core::{TabId, TargetId};

use crate::events::page_event::BindingCalled;
use crate::events::{ListenerKey, PageEventKind, Subscription};

/// Behind `RunState`. A std mutex, held for a map update and never across an await.
#[derive(Debug, Default)]
pub(crate) struct RunEvents {
    inner: Mutex<Inner>,
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
    /// Binding calls the watcher handed this run, oldest first, until its pump takes them. Kept
    /// apart from the bounded log: each is a request the page may be waiting on.
    calls: VecDeque<QueuedCall>,
}

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

    /// Queues a binding call for the pump.
    pub(crate) fn queue_call(&self, call: QueuedCall) {
        self.lock().calls.push_back(call);
    }

    /// The binding calls queued since the pump last took them.
    pub(crate) fn take_calls(&self) -> Vec<QueuedCall> {
        self.lock().calls.drain(..).collect()
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
    /// meanwhile stays removed.
    pub(crate) fn advance(&self, read: &[(Subscription, u64)]) {
        let mut inner = self.lock();
        for (subscription, seq) in read {
            if let Some(listening) = inner.listening.get_mut(subscription) {
                listening.after = listening.after.max(*seq);
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use browseros_core::{TabId, TargetId};

    use super::RunEvents;
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
        events.advance(&[(dialogs(1, "T"), 9)]);
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
}
