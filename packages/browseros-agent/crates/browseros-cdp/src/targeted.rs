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
    sync::{Mutex, MutexGuard, PoisonError},
};
use tokio::sync::mpsc;

/// Each method's subscribers, oldest first.
type Subscribers = HashMap<String, Vec<mpsc::UnboundedSender<CdpEvent>>>;

/// A passive subscriber's single queue preserves order across its selected methods.
#[derive(Debug)]
struct Observer {
    methods: HashSet<String>,
    sender: mpsc::UnboundedSender<CdpEvent>,
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

    /// Copies selected methods into one ordered, unbounded queue without taking them away
    /// from existing broadcast or targeted consumers. Used for low-volume lifecycle events
    /// whose loss could strand a page, such as dialog opening and closing.
    #[must_use]
    pub fn observe(&self, methods: &[&str]) -> mpsc::UnboundedReceiver<CdpEvent> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let mut subscribers = self.subscribers();
        subscribers
            .observers
            .retain(|observer| !observer.sender.is_closed());
        subscribers.observers.push(Observer {
            methods: methods.iter().map(|method| (*method).to_owned()).collect(),
            sender,
        });
        receiver
    }

    /// Copies `event` to passive observers, then hands it to every targeted subscriber of its
    /// method. Gives it back when no targeted subscriber took it, for the caller to broadcast.
    pub fn deliver(&self, event: CdpEvent) -> Option<CdpEvent> {
        let mut subscribers = self.subscribers();
        subscribers.observers.retain(|observer| {
            !observer.sender.is_closed()
                && (!observer.methods.contains(&event.method)
                    || observer.sender.send(event.clone()).is_ok())
        });
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
    use super::TargetedEvents;
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
}
