//! Event methods taken on dedicated channels instead of the client's shared broadcast ring.
//!
//! A consumer takes a method here for one of two reasons. The event may be large and frequent
//! (`Page.screencastFrame`), so it should be freed once read instead of holding a slot in the
//! 4096-event ring. Or the consumer must never lose one: the ring drops what a reader that fell
//! behind has not read, and these unbounded channels drop nothing.
//!
//! **Every subscriber of a method gets each of its events.** Independent features read the same
//! method. `Runtime.bindingCalled` carries the sealed world's journal entries: browseros-playwright
//! reads them for the frames the agent set up, and browseros-takeover's control journal reads
//! them for the frames the owner types into during a takeover. If only one subscriber per method
//! got events, whichever subscribed last would take every call and the other journal would
//! register nothing, so values the owner typed would reach the agent unmasked.
//!
//! **While a method has a live subscriber, the broadcast does not carry it.** A reader of such a
//! method has to subscribe here too; filtering the broadcast for it reads nothing. Once every
//! receiver of the method is dropped, the broadcast carries it again.

use crate::CdpEvent;
use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard, PoisonError},
};
use tokio::sync::mpsc;

/// Each method's subscribers, oldest first.
type Subscribers = HashMap<String, Vec<mpsc::UnboundedSender<CdpEvent>>>;

/// The targeted subscribers of one connection's event methods. [`crate::CdpClient`] routes
/// every event through [`Self::deliver`] before its broadcast; test transports that do the same
/// route events exactly as a browser connection does.
#[derive(Debug, Default)]
pub struct TargetedEvents {
    subscribers: Mutex<Subscribers>,
}

impl TargetedEvents {
    /// A new subscriber of `method`. It gets every later event of that method until its receiver
    /// is dropped, alongside any other subscriber of the method.
    #[must_use]
    pub fn subscribe(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut subscribers = self.subscribers();
        let senders = subscribers.entry(method.to_owned()).or_default();
        // Subscribers that came and went with no event in between would otherwise pile up.
        senders.retain(|sender| !sender.is_closed());
        senders.push(tx);
        rx
    }

    /// Hands `event` to every live subscriber of its method and forgets those whose receiver is
    /// gone. Gives the event back when no subscriber took it, for the caller to broadcast.
    pub fn deliver(&self, event: CdpEvent) -> Option<CdpEvent> {
        let mut subscribers = self.subscribers();
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
    fn subscribers(&self) -> MutexGuard<'_, Subscribers> {
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
