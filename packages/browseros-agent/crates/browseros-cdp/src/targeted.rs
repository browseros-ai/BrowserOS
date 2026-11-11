//! Targeted delivery and passive observation outside the client's shared broadcast ring.
//!
//! A consumer takes a method here for one of two reasons. The event may be large and frequent
//! (`Page.screencastFrame`), so it should be freed once read instead of holding a slot in the
//! 4096-event ring. Or the consumer must never lose one: the ring drops what a reader that fell
//! behind has not read, and these unbounded channels drop nothing.
//!
//! **Every subscriber of a method gets each of its events.** Independent features read the same
//! method. `Runtime.bindingCalled` carries the sealed world's journal entries: neo-playwright
//! reads them for the frames the agent set up, and neo-takeover's control journal reads
//! them for the frames the owner types into during a takeover. If only one subscriber per method
//! got events, whichever subscribed last would take every call and the other journal would
//! register nothing, so values the owner typed would reach the agent unmasked.
//!
//! **A live targeted subscriber takes its method out of the broadcast.** A reader of such a
//! method has to subscribe here too; filtering the broadcast for it reads nothing. Once every
//! receiver of the method is dropped, the broadcast carries it again.
//!
//! Passive observers copy selected methods into a single ordered queue and leave ordinary
//! delivery unchanged. Dialog handling uses this so opening/closing cannot be lost to ring
//! overflow or reordered across separate queues, while core still sees its normal events.

use crate::CdpEvent;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

const LOG_BACKLOG: usize = 4096;

/// One ordered feed of selected events. Lifecycle events are lossless so a stalled reader
/// cannot strand a dialog or lose attachment order. At most 4096 console/error messages wait
/// unread; further chatty messages are dropped until reading frees space. Other delivery is
/// unaffected, including the ordinary broadcast's own bound.
#[derive(Debug)]
pub struct ObservedEvents {
    receiver: mpsc::UnboundedReceiver<ObservedEvent>,
}

impl ObservedEvents {
    pub async fn recv(&mut self) -> Option<CdpEvent> {
        self.receiver.recv().await.map(|queued| queued.event)
    }

    pub fn try_recv(&mut self) -> Result<CdpEvent, mpsc::error::TryRecvError> {
        self.receiver.try_recv().map(|queued| queued.event)
    }

    /// Includes the producing socket's generation so reconnecting consumers can reject queued
    /// events from an old socket. Custom unversioned producers report `None`.
    pub async fn recv_with_epoch(&mut self) -> Option<(CdpEvent, Option<u64>)> {
        self.receiver
            .recv()
            .await
            .map(|queued| (queued.event, queued.epoch))
    }

    pub fn try_recv_with_epoch(
        &mut self,
    ) -> Result<(CdpEvent, Option<u64>), mpsc::error::TryRecvError> {
        self.receiver
            .try_recv()
            .map(|queued| (queued.event, queued.epoch))
    }
}

/// Holds a chatty event's budget slot only while it waits in the queue, not during handling.
#[derive(Debug)]
struct ObservedEvent {
    event: CdpEvent,
    epoch: Option<u64>,
    _log_slot: Option<OwnedSemaphorePermit>,
}

/// Each method's subscribers, oldest first.
type Subscribers = HashMap<String, Vec<mpsc::UnboundedSender<CdpEvent>>>;

/// A passive subscriber's single queue preserves order across its selected methods.
#[derive(Debug)]
struct Observer {
    methods: HashSet<String>,
    sender: mpsc::UnboundedSender<ObservedEvent>,
    log_slots: Arc<Semaphore>,
}

impl Observer {
    /// False removes a closed registration. A full log budget drops only that message;
    /// lifecycle events still enter the same queue in wire order.
    fn offer(&self, event: &CdpEvent, epoch: Option<u64>) -> bool {
        if self.sender.is_closed() {
            return false;
        }
        if !self.methods.contains(&event.method) {
            return true;
        }
        let log_slot = if matches!(
            event.method.as_str(),
            "Runtime.consoleAPICalled" | "Runtime.exceptionThrown"
        ) {
            let Ok(slot) = Arc::clone(&self.log_slots).try_acquire_owned() else {
                return true;
            };
            Some(slot)
        } else {
            None
        };
        self.sender
            .send(ObservedEvent {
                event: event.clone(),
                epoch,
                _log_slot: log_slot,
            })
            .is_ok()
    }
}

#[derive(Debug, Default)]
struct EventSubscribers {
    targeted: Subscribers,
    observers: Vec<Observer>,
}

/// Targeted subscribers and passive observers of one connection. [`crate::CdpClient`] routes
/// every event through [`Self::deliver`] before its broadcast; test transports that do the same
/// route events exactly as a browser connection does.
#[derive(Debug, Default)]
pub struct TargetedEvents {
    subscribers: Mutex<EventSubscribers>,
}

impl TargetedEvents {
    /// A new subscriber of `method`. It gets every later event of that method until its receiver
    /// is dropped, alongside any other subscriber of the method.
    #[must_use]
    pub fn subscribe(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut subscribers = self.subscribers();
        let senders = subscribers.targeted.entry(method.to_owned()).or_default();
        // Subscribers that came and went with no event in between would otherwise pile up.
        senders.retain(|sender| !sender.is_closed());
        senders.push(tx);
        rx
    }

    /// Copies selected methods into one ordered feed without taking them from existing
    /// consumers. Lifecycle events are lossless; chatty traffic has [`ObservedEvents`]' bound.
    #[must_use]
    pub fn observe(&self, methods: &[&str]) -> ObservedEvents {
        let (sender, receiver) = mpsc::unbounded_channel();
        let mut subscribers = self.subscribers();
        subscribers
            .observers
            .retain(|observer| !observer.sender.is_closed());
        subscribers.observers.push(Observer {
            methods: methods.iter().map(|method| (*method).to_owned()).collect(),
            sender,
            log_slots: Arc::new(Semaphore::new(LOG_BACKLOG)),
        });
        ObservedEvents { receiver }
    }

    /// Copies `event` to passive observers, then hands it to every targeted subscriber of its
    /// method. Gives it back when no targeted subscriber took it, for the caller to broadcast.
    pub fn deliver(&self, event: CdpEvent) -> Option<CdpEvent> {
        self.deliver_observed(event, None)
    }

    /// Observations retain the socket that produced the event. Ordinary targeted and
    /// broadcast consumers keep their existing event shape and delivery.
    pub fn deliver_at_epoch(&self, event: CdpEvent, epoch: u64) -> Option<CdpEvent> {
        self.deliver_observed(event, Some(epoch))
    }

    fn deliver_observed(&self, event: CdpEvent, epoch: Option<u64>) -> Option<CdpEvent> {
        let mut subscribers = self.subscribers();
        subscribers
            .observers
            .retain(|observer| observer.offer(&event, epoch));
        let Some(senders) = subscribers.targeted.get_mut(&event.method) else {
            return Some(event);
        };
        senders.retain(|sender| !sender.is_closed());
        let Some((last, others)) = senders.split_last() else {
            subscribers.targeted.remove(&event.method);
            return Some(event);
        };
        let mut taken = false;
        for sender in others {
            taken |= sender.send(event.clone()).is_ok();
        }
        // The last subscriber gets the original, so a lone one, such as a screencast, is never
        // handed a copy of a large frame.
        match last.send(event) {
            Ok(()) => None,
            // Its receiver closed after the prune and no other subscriber took the event.
            Err(refused) if !taken => Some(refused.0),
            Err(_) => None,
        }
    }

    /// Never held across an await. A panic while it was held cannot leave the map half-updated,
    /// so a poisoned lock is still usable.
    fn subscribers(&self) -> MutexGuard<'_, EventSubscribers> {
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::{LOG_BACKLOG, TargetedEvents};
    use crate::CdpEvent;
    use serde_json::json;

    fn event(method: &str) -> CdpEvent {
        CdpEvent {
            method: method.to_owned(),
            params: json!({}),
            session_id: None,
        }
    }

    #[test]
    fn passive_observation_preserves_order_and_existing_delivery() {
        let events = TargetedEvents::default();
        let mut observer = events.observe(&["opening", "closed"]);
        let mut targeted = events.subscribe("opening");
        assert!(events.deliver(event("opening")).is_none());
        assert_eq!(
            events.deliver(event("noise")).map(|e| e.method),
            Some("noise".to_owned())
        );
        assert_eq!(
            events.deliver(event("closed")).map(|e| e.method),
            Some("closed".to_owned())
        );
        assert_eq!(
            targeted.try_recv().ok().map(|e| e.method),
            Some("opening".to_owned())
        );
        assert_eq!(
            observer.try_recv().ok().map(|e| e.method),
            Some("opening".to_owned())
        );
        assert_eq!(
            observer.try_recv().ok().map(|e| e.method),
            Some("closed".to_owned())
        );
        assert!(observer.try_recv().is_err());
    }

    #[test]
    fn observed_backlog_retains_its_producing_socket_epoch() {
        let events = TargetedEvents::default();
        let mut observer = events.observe(&["opening"]);
        let _ = events.deliver_at_epoch(event("opening"), 41);
        let _ = events.deliver_at_epoch(event("opening"), 42);
        assert_eq!(
            observer
                .try_recv_with_epoch()
                .ok()
                .and_then(|(_, epoch)| epoch),
            Some(41)
        );
        assert_eq!(
            observer
                .try_recv_with_epoch()
                .ok()
                .and_then(|(_, epoch)| epoch),
            Some(42)
        );
    }

    #[test]
    fn console_backlog_is_bounded_without_dropping_lifecycle_events() {
        let events = TargetedEvents::default();
        let mut observer = events.observe(&["Runtime.consoleAPICalled", "opening"]);
        for _ in 0..LOG_BACKLOG + 1 {
            let _ = events.deliver(event("Runtime.consoleAPICalled"));
        }
        let _ = events.deliver(event("opening"));
        let mut messages = Vec::new();
        while let Ok(event) = observer.try_recv() {
            messages.push(event.method);
        }
        assert_eq!(messages.len(), LOG_BACKLOG + 1);
        assert_eq!(messages.last().map(String::as_str), Some("opening"));
        let _ = events.deliver(event("Runtime.consoleAPICalled"));
        assert_eq!(
            observer.try_recv().ok().map(|event| event.method),
            Some("Runtime.consoleAPICalled".to_owned())
        );
    }
}
