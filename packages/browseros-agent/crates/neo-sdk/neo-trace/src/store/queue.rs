//! Neo: A nonblocking, byte-budgeted handoff from run/collector threads to one SQL writer.
//! Reservations cover queued commands and the writer's active batch until they are consumed.

use std::mem;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tracing::warn;

use super::writer::Command;

/// Neo: Bound pending trace data to 64 MiB even if the `SQLite` writer stalls.
const QUEUE_BYTES: usize = 64 * 1024 * 1024;
/// Neo: Conservatively include channel bookkeeping as well as each command's owned payload.
const CHANNEL_OVERHEAD: usize = 64;

/// Shared admission state lets collector and run threads reserve memory without waiting
/// for the writer or taking a producer lock. Shutdown closes this same admission boundary.
pub(super) struct Budget {
    bytes: AtomicUsize,
    limit: usize,
    closed: AtomicBool,
    warned: AtomicBool,
    dropped: AtomicU64,
}

impl Budget {
    pub(super) fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    fn dropped(&self) {
        let dropped = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
        if !self.warned.swap(true, Ordering::Relaxed) {
            warn!(target: "neo_trace", dropped, "trace store unavailable or queue full; dropping persistence commands");
        }
    }
}

/// A reservation is released on send failure, transaction failure, or ordinary consumption.
/// Keeping it beside the command makes every exit path return capacity without a lock.
pub(super) struct Reservation {
    budget: Arc<Budget>,
    bytes: usize,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.bytes.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

pub(super) struct Queued {
    pub(super) command: Command,
    pub(super) _reservation: Reservation,
}

pub(super) enum Message {
    Write(Box<Queued>),
    Close,
}

#[derive(Clone)]
/// Producers share one byte budget; only the receiver may consume commands or stop the writer.
pub(super) struct Queue {
    sender: UnboundedSender<Message>,
    pub(super) budget: Arc<Budget>,
}

impl Queue {
    pub(super) fn new() -> (Self, UnboundedReceiver<Message>) {
        Self::with_limit(QUEUE_BYTES)
    }

    pub(super) fn with_limit(limit: usize) -> (Self, UnboundedReceiver<Message>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let budget = Arc::new(Budget {
            bytes: AtomicUsize::new(0),
            limit,
            closed: AtomicBool::new(false),
            warned: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
        });
        (Self { sender, budget }, receiver)
    }

    pub(super) fn push(&self, command: Command) {
        let bytes = mem::size_of::<Message>()
            .saturating_add(mem::size_of::<Queued>())
            .saturating_add(CHANNEL_OVERHEAD)
            .saturating_add(command.heap_bytes());
        if self.budget.closed.load(Ordering::Acquire)
            || self
                .budget
                .bytes
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
                    held.checked_add(bytes)
                        .filter(|next| *next <= self.budget.limit)
                })
                .is_err()
        {
            self.budget.dropped();
            return;
        }
        let queued = Queued {
            command,
            _reservation: Reservation {
                budget: Arc::clone(&self.budget),
                bytes,
            },
        };
        if self.sender.send(Message::Write(Box::new(queued))).is_err() {
            self.budget.dropped();
        }
    }

    pub(super) fn close(&self) {
        self.budget.close();
        // Shutdown bypasses the byte budget. The writer closes its receiver, then drains
        // everything already accepted, including a producer that raced this marker.
        let _sent = self.sender.send(Message::Close);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{Message, Queue};
    use crate::store::writer::Command;

    fn command() -> Command {
        Command::Delivery {
            run_id: "run".to_owned(),
            seq: 1,
            delivery: None,
        }
    }

    #[test]
    fn a_stalled_writer_cannot_exceed_the_byte_budget_and_drops_are_counted() {
        let (queue, mut receiver) = Queue::with_limit(4096);
        for _ in 0..100 {
            queue.push(command());
        }
        assert!(queue.budget.bytes.load(Ordering::Relaxed) <= 4096);
        assert!(queue.budget.dropped.load(Ordering::Relaxed) > 0);
        assert!(
            !receiver.is_empty(),
            "commands fitting the budget remain queued"
        );
        while receiver.try_recv().is_ok() {}
        assert_eq!(
            queue.budget.bytes.load(Ordering::Relaxed),
            0,
            "consumption returns every reservation"
        );
        queue.push(command());
        assert!(
            matches!(receiver.try_recv(), Ok(Message::Write(_))),
            "capacity can be reused"
        );
    }

    #[test]
    fn closing_a_full_queue_never_needs_room_and_rejected_sends_return_capacity() {
        let (queue, mut receiver) = Queue::with_limit(4096);
        for _ in 0..100 {
            queue.push(command());
        }
        queue.close();
        let mut closed = false;
        while let Ok(message) = receiver.try_recv() {
            if matches!(message, Message::Close) {
                closed = true;
            }
        }
        assert!(closed, "shutdown bypasses the byte budget");
        assert_eq!(queue.budget.bytes.load(Ordering::Relaxed), 0);
        drop(receiver);
        queue.push(command());
        assert_eq!(queue.budget.bytes.load(Ordering::Relaxed), 0);
    }
}
