//! Neo: The host side's inbox: one bounded FIFO from the producers, which push on a script's path
//! (the run path, a `RunPass` dropping), to the one consumer, the session ledger's task.
//!
//! A push never waits. [`Tap::try_push`] holds the lock for one `VecDeque` push and either
//! queues the event behind everything pushed before it or refuses it, counted. The queue has
//! two bands:
//!
//! ```text
//!  0 ──────────── any event ──────────── TAP_QUEUE ── lifecycle only ── TAP_QUEUE + band
//!                                                     band = LIFECYCLE_PER_RUN × max_runs
//! ```
//!
//! Per-call events go first because losing one costs something bounded and known: the current
//! tab lags until the session's next call, and a call whose `CallEnded` is refused gets no page
//! facts or screenshot, for good. Losing a lifecycle event would leave the ledger unable to prove
//! a session complete, so a lifecycle event refused past its band marks its session damaged, and
//! the ledger never reports a damaged session complete (the neo-host-events design, §6.5).
//!
//! The mark is set before the refused push returns, so it precedes every later event of that
//! session's runs, `PassReleased` included. The consumer takes the marks ([`Tap::take_damaged`])
//! before each event it applies, so damage always reaches the ledger before the pass that would
//! let the session complete.

use std::collections::{HashSet, VecDeque};
use std::mem;
use std::sync::Mutex;

use tokio::sync::Notify;
use tracing::warn;

use crate::host::events::HostEvent;
use crate::host::ids::SessionKey;

/// Neo: refuse per-call events past 16384 queued rows so a starved host task cannot grow memory on the script's path.
/// Per-call events queued past this are refused. Far above any real burst: a run has at most 64
/// calls in flight (`method_router.rs` `MAX_CALLS_IN_FLIGHT`), each puts two per-call events here,
/// and the ledger applies one in microseconds, so a full queue means its task was starved for a
/// long time. Neo's own figure (design §11).
pub(crate) const TAP_QUEUE: usize = 16_384;

/// Neo: reserve four lifecycle events per admitted run so call-event bursts cannot prevent session completion.
/// The lifecycle events budgeted per admitted run: `RunStarted`, `RunEnded`, its pass's
/// `PassReleased` and one `PageCreated`. A run that creates more pages spends more of the band.
pub(crate) const LIFECYCLE_PER_RUN: usize = 4;

pub(crate) struct Tap {
    queue: Mutex<Queue>,
    /// Wakes the consumer after a push or `close`. One stored permit is enough: `recv` takes
    /// one event per call and only waits once the queue is empty.
    ready: Notify,
    /// The queue length past which lifecycle events are refused: `TAP_QUEUE` plus the band.
    lifecycle_limit: usize,
}

struct Queue {
    events: VecDeque<HostEvent>,
    closed: bool,
    refused: Refused,
    /// Sessions that lost a lifecycle event since the consumer last took them. The ledger keeps
    /// the mark for good once it has it.
    damaged: HashSet<SessionKey>,
    /// The last per-call push was refused, so the next refusal belongs to the same burst and is
    /// not logged again.
    refusing_calls: bool,
}

/// The events the tap refused since it was built, by band.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Refused {
    pub(crate) calls: u64,
    pub(crate) lifecycle: u64,
}

/// What became of one pushed event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pushed {
    /// In the queue, behind every event pushed before it.
    Queued,
    /// A per-call event refused by the full queue, and counted.
    Refused,
    /// A lifecycle event refused past its band: counted, and its session is now damaged.
    Damaged,
    /// The consumer stopped (`close`), so nothing is queued any more.
    Closed,
}

impl Tap {
    /// A tap whose lifecycle band holds `max_runs` runs' worth of lifecycle events.
    pub(crate) fn new(max_runs: usize) -> Self {
        Self {
            queue: Mutex::new(Queue {
                // The whole per-call band up front, so no push below it reallocates on a script's
                // path. Only a push into the lifecycle band can grow the queue.
                events: VecDeque::with_capacity(TAP_QUEUE),
                closed: false,
                refused: Refused::default(),
                damaged: HashSet::new(),
                refusing_calls: false,
            }),
            ready: Notify::new(),
            lifecycle_limit: TAP_QUEUE.saturating_add(LIFECYCLE_PER_RUN.saturating_mul(max_runs)),
        }
    }

    /// Queues `event` or refuses it, at once. Never awaits and never fails the caller: a refusal
    /// is counted here, so producers ignore the answer. A poisoned lock reads as closed.
    pub(crate) fn try_push(&self, event: HostEvent) -> Pushed {
        let Ok(mut queue) = self.queue.lock() else {
            return Pushed::Closed;
        };
        if queue.closed {
            return Pushed::Closed;
        }
        let lifecycle = event.is_lifecycle();
        let limit = if lifecycle {
            self.lifecycle_limit
        } else {
            TAP_QUEUE
        };
        if queue.events.len() < limit {
            if !lifecycle {
                queue.refusing_calls = false;
            }
            queue.events.push_back(event);
            drop(queue);
            self.ready.notify_one();
            return Pushed::Queued;
        }
        if lifecycle {
            queue.refused.lifecycle += 1;
            let first = queue.damaged.insert(event.session().clone());
            drop(queue);
            if first {
                warn!(
                    session = %event.session(),
                    "host tap past its lifecycle band: the session can never read complete"
                );
            }
            Pushed::Damaged
        } else {
            queue.refused.calls += 1;
            let first = !mem::replace(&mut queue.refusing_calls, true);
            drop(queue);
            if first {
                warn!(
                    queued = TAP_QUEUE,
                    "host tap full: refusing per-call events until the ledger catches up"
                );
            }
            Pushed::Refused
        }
    }
}

impl Tap {
    /// The oldest queued event, waiting for one while the queue is empty; `None` once the tap
    /// is closed and drained. For the one consumer only: two would split the FIFO. Cancel-safe:
    /// an event leaves the queue only in the poll that returns it.
    pub(crate) async fn recv(&self) -> Option<HostEvent> {
        loop {
            {
                let Ok(mut queue) = self.queue.lock() else {
                    return None;
                };
                if let Some(event) = queue.events.pop_front() {
                    return Some(event);
                }
                if queue.closed {
                    return None;
                }
            }
            self.ready.notified().await;
        }
    }

    /// Stops accepting events. The consumer still receives every queued event, then `None`.
    pub(crate) fn close(&self) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.closed = true;
        }
        self.ready.notify_one();
    }

    #[cfg(test)]
    pub(crate) fn refused(&self) -> Refused {
        self.queue
            .lock()
            .map_or_else(|_| Refused::default(), |queue| queue.refused)
    }

    /// The sessions marked damaged since the last call, each once. The consumer calls it before
    /// applying each event it received: a mark set before that event was pushed is then always
    /// taken first. A poisoned lock yields none; `recv` has ended the consumer by then.
    pub(crate) fn take_damaged(&self) -> Vec<SessionKey> {
        self.queue
            .lock()
            .map_or_else(|_| Vec::new(), |mut queue| queue.damaged.drain().collect())
    }
}

#[cfg(test)]
impl Tap {
    /// Every queued event, oldest first, without waiting and without closing the tap.
    pub(crate) fn drain(&self) -> Vec<HostEvent> {
        self.queue
            .lock()
            .map_or_else(|_| Vec::new(), |mut queue| queue.events.drain(..).collect())
    }
}

#[cfg(test)]
mod tests {
    use futures_util::FutureExt;

    use super::*;
    use crate::host::ids::{EpochMs, TabId};
    use crate::run::{CallResult, PageId, RunId, RunStatus};

    fn page() -> PageId {
        PageId::new("T".to_string())
    }

    /// One event of every kind for `session`, per-call kinds first.
    fn every_kind(session: &SessionKey) -> [HostEvent; 6] {
        let run = RunId::mint();
        let at = EpochMs::from_millis(1_794_002_916_593).unwrap();
        [
            HostEvent::CallStarted {
                session: session.clone(),
                run: run.clone(),
                call_id: 1,
                page: Some(page()),
                snapshot: true,
                at,
            },
            HostEvent::CallEnded {
                session: session.clone(),
                run: run.clone(),
                call_id: 1,
                page: Some(page()),
                result: CallResult::Ok,
            },
            HostEvent::RunStarted {
                session: session.clone(),
                run: run.clone(),
                page: page(),
                at,
            },
            HostEvent::PageCreated {
                session: session.clone(),
                run: run.clone(),
                page: page(),
                tab: TabId::parse(7),
                call_id: Some(1),
            },
            HostEvent::RunEnded {
                session: session.clone(),
                run,
                status: RunStatus::Ok,
                at,
            },
            HostEvent::PassReleased {
                session: session.clone(),
            },
        ]
    }

    fn per_call(session: &SessionKey) -> HostEvent {
        every_kind(session)[0].clone()
    }

    fn lifecycle(session: &SessionKey) -> HostEvent {
        every_kind(session)[5].clone()
    }

    #[test]
    fn only_call_events_are_per_call() {
        let session = SessionKey::new("s");
        let lifecycle: Vec<bool> = every_kind(&session)
            .iter()
            .map(HostEvent::is_lifecycle)
            .collect();
        assert_eq!(lifecycle, [false, false, true, true, true, true]);
    }

    #[tokio::test]
    async fn the_consumer_receives_events_in_push_order_then_none_once_closed() {
        let tap = Tap::new(1);
        let session = SessionKey::new("s");
        let sent = every_kind(&session);
        for event in &sent {
            assert_eq!(tap.try_push(event.clone()), Pushed::Queued);
        }
        tap.close();
        assert_eq!(tap.try_push(lifecycle(&session)), Pushed::Closed);

        let mut received = Vec::new();
        while let Some(event) = tap.recv().await {
            received.push(event);
        }
        assert_eq!(received, sent);
    }

    /// The consumer parks on an empty tap and the next push wakes it: without that wake the
    /// second poll would wait forever.
    #[tokio::test]
    async fn recv_waits_for_the_next_push() {
        let tap = Tap::new(1);
        let mut receiving = Box::pin(tap.recv());
        assert_eq!(
            receiving.as_mut().now_or_never(),
            None,
            "nothing pushed yet"
        );

        let event = lifecycle(&SessionKey::new("s"));
        assert_eq!(tap.try_push(event.clone()), Pushed::Queued);
        assert_eq!(receiving.await, Some(event));
    }

    #[test]
    fn a_full_queue_refuses_per_call_events_and_still_takes_lifecycle_ones() {
        let tap = Tap::new(1);
        let session = SessionKey::new("s");
        for _ in 0..TAP_QUEUE {
            assert_eq!(tap.try_push(per_call(&session)), Pushed::Queued);
        }

        assert_eq!(tap.try_push(per_call(&session)), Pushed::Refused);
        assert_eq!(tap.try_push(per_call(&session)), Pushed::Refused);
        assert_eq!(tap.try_push(lifecycle(&session)), Pushed::Queued);
        assert_eq!(
            tap.refused(),
            Refused {
                calls: 2,
                lifecycle: 0
            }
        );
        assert!(
            tap.take_damaged().is_empty(),
            "only per-call events were lost"
        );
    }

    /// The consumer learns of each damaged session once, and of no other, however many of its
    /// lifecycle events were lost before it looked.
    #[test]
    fn a_lifecycle_event_past_its_band_damages_its_session_only() {
        let tap = Tap::new(2);
        let (healthy, unlucky) = (SessionKey::new("healthy"), SessionKey::new("unlucky"));
        for _ in 0..TAP_QUEUE {
            assert_eq!(tap.try_push(per_call(&healthy)), Pushed::Queued);
        }
        for _ in 0..2 * LIFECYCLE_PER_RUN {
            assert_eq!(tap.try_push(lifecycle(&healthy)), Pushed::Queued);
        }

        assert_eq!(tap.try_push(lifecycle(&unlucky)), Pushed::Damaged);
        assert_eq!(tap.try_push(lifecycle(&unlucky)), Pushed::Damaged);
        assert_eq!(tap.try_push(per_call(&healthy)), Pushed::Refused);
        assert_eq!(tap.take_damaged(), [SessionKey::new("unlucky")]);
        assert!(tap.take_damaged().is_empty(), "each mark is taken once");
        assert_eq!(tap.try_push(lifecycle(&unlucky)), Pushed::Damaged);
        assert_eq!(tap.take_damaged(), [unlucky], "a later loss marks it again");
        assert_eq!(
            tap.refused(),
            Refused {
                calls: 1,
                lifecycle: 3
            }
        );
        assert_eq!(
            tap.drain().len(),
            TAP_QUEUE + 2 * LIFECYCLE_PER_RUN,
            "nothing refused was queued"
        );
    }
}
