//! Event methods taken on dedicated channels instead of the client's shared broadcast ring.
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
//! **While a method has a live subscriber, the broadcast does not carry it.** A reader of such a
//! method has to subscribe here too; filtering the broadcast for it reads nothing. Once every
//! receiver of the method is dropped, the broadcast carries it again.
//!
//! **Each method counts the events it handed to subscribers.** A reader that numbers what its
//! receiver took ([`TargetedEvents::subscribe_counted`]) can tell, from the count read after a
//! command's reply, when it has seen every event of the method that came before that reply.

use crate::CdpEvent;
use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard, PoisonError},
};
use tokio::sync::mpsc;

/// Each method's subscribers, oldest first.
type Subscribers = HashMap<String, Vec<mpsc::UnboundedSender<CdpEvent>>>;

/// The subscribers and each method's count of events they took, under one lock, so a counted
/// subscription and its base come together.
#[derive(Debug, Default)]
struct Routes {
    subscribers: Subscribers,
    delivered: HashMap<String, u64>,
}

/// The targeted subscribers of one connection's event methods. [`crate::CdpClient`] routes
/// every event through [`Self::deliver`] before its broadcast; test transports that do the same
/// route events exactly as a browser connection does.
#[derive(Debug, Default)]
pub struct TargetedEvents {
    routes: Mutex<Routes>,
}

impl TargetedEvents {
    /// A new subscriber of `method`. It gets every later event of that method until its receiver
    /// is dropped, alongside any other subscriber of the method.
    #[must_use]
    pub fn subscribe(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.subscribe_counted(method).0
    }

    /// [`Self::subscribe`], with the count of `method`'s events subscribers took before this
    /// one existed, which it will never receive ([`Self::delivered`]).
    #[must_use]
    pub fn subscribe_counted(&self, method: &str) -> (mpsc::UnboundedReceiver<CdpEvent>, u64) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut routes = self.routes();
        let before = routes.delivered.get(method).copied().unwrap_or(0);
        let senders = routes.subscribers.entry(method.to_owned()).or_default();
        // Subscribers that came and went with no event in between would otherwise pile up.
        senders.retain(|sender| !sender.is_closed());
        senders.push(tx);
        (rx, before)
    }

    /// How many of `method`'s events subscribers took, ever. Each counts once however many
    /// subscribers got it; one the broadcast carried instead is not counted.
    #[must_use]
    pub fn delivered(&self, method: &str) -> u64 {
        self.routes().delivered.get(method).copied().unwrap_or(0)
    }

    /// Hands `event` to every live subscriber of its method and forgets those whose receiver is
    /// gone. Gives the event back when no subscriber took it, for the caller to broadcast.
    pub fn deliver(&self, event: CdpEvent) -> Option<CdpEvent> {
        let mut routes = self.routes();
        let Routes {
            subscribers,
            delivered,
        } = &mut *routes;
        let Some(senders) = subscribers.get_mut(&event.method) else {
            return Some(event);
        };
        senders.retain(|sender| !sender.is_closed());
        let Some((last, others)) = senders.split_last() else {
            subscribers.remove(&event.method);
            return Some(event);
        };
        let mut taken = false;
        for sender in others {
            taken |= sender.send(event.clone()).is_ok();
        }
        let method = event.method.clone();
        // The last subscriber gets the original, so a lone one, such as a screencast, is never
        // handed a copy of a large frame.
        match last.send(event) {
            Ok(()) => {}
            // Its receiver closed after the prune and no other subscriber took the event.
            Err(refused) if !taken => return Some(refused.0),
            Err(_) => {}
        }
        *delivered.entry(method).or_default() += 1;
        None
    }

    /// Never held across an await. A panic while it was held cannot leave the map half-updated,
    /// so a poisoned lock is still usable.
    fn routes(&self) -> MutexGuard<'_, Routes> {
        self.routes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::TargetedEvents;
    use crate::CdpEvent;

    const CALLED: &str = "Runtime.bindingCalled";

    fn called() -> CdpEvent {
        CdpEvent {
            method: CALLED.to_owned(),
            params: json!({}),
            session_id: None,
        }
    }

    #[test]
    fn tests_that_a_counted_subscription_starts_after_the_events_subscribers_took() {
        let routes = TargetedEvents::default();
        assert!(
            routes.deliver(called()).is_some(),
            "with no subscriber the broadcast carries it, and the method counts nothing"
        );
        let (mut first, before) = routes.subscribe_counted(CALLED);
        assert_eq!(before, 0);
        assert!(routes.deliver(called()).is_none());
        let (mut second, before) = routes.subscribe_counted(CALLED);
        assert_eq!(before, 1, "the event the first subscriber took");
        assert!(routes.deliver(called()).is_none());
        assert_eq!(
            routes.delivered(CALLED),
            2,
            "one count per event, however many subscribers got it"
        );
        assert!(first.try_recv().is_ok() && first.try_recv().is_ok());
        assert!(second.try_recv().is_ok() && second.try_recv().is_err());
        assert_eq!(routes.delivered("Page.screencastFrame"), 0);
    }
}
