//! Neo: The synchronous boundary from arbitrary runtime threads to the writer. Byte permits travel
//! with each queued event through retries and are released only after commit (or counted loss).
//! No path here waits for a runtime, a database, another producer, or a shared run registry.

use std::mem;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use neo_runtime::{
    CallId, CallResult, GetterResult, RunEvent, RunId, RunSink, RunStarted, TraceSink,
};
use serde_json::Value;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};

use crate::{AfterCall, SessionKey, StoreConfig, StoreError, StoreHealth};

/// Shared aggregate counters contain no run map. Each capture owns its own loss atomics.
#[derive(Default)]
pub(crate) struct State {
    pub(crate) stopped: AtomicBool,
    pub(crate) producers: AtomicUsize,
    pub(crate) queued: AtomicU64,
    pub(crate) bytes: AtomicU64,
    pub(crate) unflushed_events: AtomicU64,
    pub(crate) unflushed_bytes: AtomicU64,
    pub(crate) stubbed: AtomicU64,
    pub(crate) dropped: AtomicU64,
    pub(crate) settles_lost: AtomicU64,
    pub(crate) writes_dropped: AtomicU64,
    pub(crate) artifacts_dropped: AtomicU64,
    pub(crate) failing_since: AtomicU64,
    #[cfg(test)]
    pub(crate) test_readonly: AtomicBool,
    #[cfg(test)]
    pub(crate) test_failures: AtomicU64,
}

impl State {
    pub(crate) fn health(&self, read_only: bool) -> StoreHealth {
        let failing = self.failing_since.load(Ordering::Acquire);
        StoreHealth {
            read_only,
            closed: self.stopped.load(Ordering::Acquire),
            queued_events: self.queued.load(Ordering::Relaxed),
            queued_bytes: self.bytes.load(Ordering::Relaxed),
            unflushed_events: self.unflushed_events.load(Ordering::Relaxed),
            unflushed_bytes: self.unflushed_bytes.load(Ordering::Relaxed),
            failing_since_ms: (failing != 0).then_some(failing),
            events_stubbed: self.stubbed.load(Ordering::Relaxed),
            events_dropped: self.dropped.load(Ordering::Relaxed),
            settles_lost: self.settles_lost.load(Ordering::Relaxed),
            writes_dropped: self.writes_dropped.load(Ordering::Relaxed),
            artifacts_dropped: self.artifacts_dropped.load(Ordering::Relaxed),
            ..StoreHealth::default()
        }
    }
}

/// Control messages use the same FIFO as events, so fences and privacy changes cannot overtake
/// accepted writes. Only event payloads consume byte permits; async fences have no trace data.
pub(crate) enum Command {
    Started(RunStarted),
    Event {
        event: RunEvent,
        stub: bool,
        loss: (u64, u64),
    },
    Artifact(CallId, AfterCall),
    Withhold(SessionKey),
    Forget(SessionKey, oneshot::Sender<Result<u64, StoreError>>),
    Flush(oneshot::Sender<()>),
    Stop,
    #[cfg(test)]
    Pause(oneshot::Sender<()>, oneshot::Receiver<()>),
}

/// An entry owns its budget even while a failed transaction is retried. Drop accounts for
/// queued memory in one place, including shutdown cancellation and a disconnected receiver.
pub(crate) struct Entry {
    pub(crate) command: Command,
    _permit: Option<OwnedSemaphorePermit>,
    state: Arc<State>,
    bytes: u64,
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.state.queued.fetch_sub(1, Ordering::Relaxed);
        self.state.bytes.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

/// Cloneable producer endpoint: owns the channel and byte budgets shared by run captures.
#[derive(Clone)]
pub(crate) struct Queue {
    pub(crate) tx: mpsc::UnboundedSender<Entry>,
    pub(crate) state: Arc<State>,
    normal: Arc<Semaphore>,
    reserve: Arc<Semaphore>,
}

impl Queue {
    pub(crate) fn new(config: &StoreConfig) -> (Self, mpsc::UnboundedReceiver<Entry>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Self {
                tx,
                state: Arc::default(),
                normal: Arc::new(Semaphore::new(
                    config.queue_bytes.min(Semaphore::MAX_PERMITS),
                )),
                reserve: Arc::new(Semaphore::new(
                    config.reserve_bytes.min(Semaphore::MAX_PERMITS),
                )),
            },
            rx,
        )
    }

    fn permit(semaphore: &Arc<Semaphore>, bytes: usize) -> Option<OwnedSemaphorePermit> {
        Arc::clone(semaphore)
            .try_acquire_many_owned(u32::try_from(bytes).ok()?)
            .ok()
    }

    pub(crate) fn send(&self, command: Command, permit: Option<OwnedSemaphorePermit>) -> bool {
        let bytes = permit
            .as_ref()
            .map_or(0, |permit| permit.num_permits() as u64);
        self.state.queued.fetch_add(1, Ordering::Relaxed);
        self.state.bytes.fetch_add(bytes, Ordering::Relaxed);
        self.tx
            .send(Entry {
                command,
                _permit: permit,
                state: Arc::clone(&self.state),
                bytes,
            })
            .is_ok()
    }

    /// Producer admission is a double checked atomic gate. Close seals it then waits for the
    /// admitted producers before putting Stop in the FIFO; no producer can send behind Stop.
    pub(crate) fn enter(&self) -> Option<Producer<'_>> {
        self.state.producers.fetch_add(1, Ordering::AcqRel);
        let producer = Producer(&self.state);
        (!self.state.stopped.load(Ordering::Acquire)).then_some(producer)
    }

    pub(crate) fn control(&self, command: Command) -> bool {
        let Some(_producer) = self.enter() else {
            return false;
        };
        self.send(command, None)
    }

    pub(crate) fn artifact(&self, call: CallId, mut after: AfterCall) {
        let Some(_producer) = self.enter() else {
            return;
        };
        if after
            .jpeg
            .as_ref()
            .is_some_and(|jpeg| jpeg.len() > 64 << 10)
        {
            after.jpeg = None;
        }
        let bytes = mem::size_of::<Entry>()
            + 256
            + after.jpeg.as_ref().map_or(0, Vec::len)
            + after.url.as_ref().map_or(0, String::len)
            + after.title.as_ref().map_or(0, String::len);
        if let Some(permit) = Self::permit(&self.normal, bytes)
            && self.send(Command::Artifact(call, after), Some(permit))
        {
            return;
        }
        self.state.artifacts_dropped.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) struct Producer<'a>(&'a State);
impl Drop for Producer<'_> {
    fn drop(&mut self) {
        self.0.producers.fetch_sub(1, Ordering::AcqRel);
    }
}

impl TraceSink for Queue {
    fn run(&self, started: &RunStarted) -> Option<Arc<dyn RunSink>> {
        let _producer = self.enter()?;
        let capture = RunCapture {
            queue: self.clone(),
            stubbed: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        };
        let bytes = mem::size_of::<Entry>() + mem::size_of::<RunStarted>() + 8192;
        let permit =
            Self::permit(&self.normal, bytes).or_else(|| Self::permit(&self.reserve, bytes));
        if !permit.is_some_and(|permit| self.send(Command::Started(started.clone()), Some(permit)))
        {
            capture.loss(false);
        }
        Some(Arc::new(capture))
    }
}

/// A run's counters live here, never in a locked global map. `RunSettled` carries a snapshot of
/// their absolute values, so transaction rollback or replay cannot multiply them.
struct RunCapture {
    queue: Queue,
    stubbed: AtomicU64,
    dropped: AtomicU64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Lifecycle,
    Record,
    Hint,
}

fn class(event: &RunEvent) -> Class {
    match event {
        RunEvent::RunEnded { .. }
        | RunEvent::RunSettled { .. }
        | RunEvent::PageOpened { .. }
        | RunEvent::PageClosed { .. } => Class::Lifecycle,
        RunEvent::AfterCall { .. } | RunEvent::DeliverySettled { .. } | RunEvent::Getter { .. } => {
            Class::Record
        }
        RunEvent::BeforeCall { .. } | RunEvent::InputSent { .. } => Class::Hint,
    }
}

impl RunCapture {
    fn loss(&self, stub: bool) {
        let (run, store) = if stub {
            (&self.stubbed, &self.queue.state.stubbed)
        } else {
            (&self.dropped, &self.queue.state.dropped)
        };
        run.fetch_add(1, Ordering::Relaxed);
        store.fetch_add(1, Ordering::Relaxed);
    }
}

impl RunSink for RunCapture {
    fn emit(&self, event: &RunEvent) {
        let Some(_producer) = self.queue.enter() else {
            self.loss(false);
            if matches!(event, RunEvent::RunSettled { .. }) {
                self.queue
                    .state
                    .settles_lost
                    .fetch_add(1, Ordering::Relaxed);
            }
            return;
        };
        let settling = matches!(event, RunEvent::RunSettled { .. });
        let (event, permit, stub) =
            if let Some(permit) = Queue::permit(&self.queue.normal, weight(event)) {
                (event.clone(), permit, false)
            } else if class(event) != Class::Hint {
                let stub = stub(event);
                let Some(permit) = Queue::permit(&self.queue.reserve, weight(&stub)) else {
                    self.loss(false);
                    if matches!(event, RunEvent::RunSettled { .. }) {
                        self.queue
                            .state
                            .settles_lost
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    return;
                };
                let stubbed =
                    class(event) == Class::Record || matches!(event, RunEvent::RunEnded { .. });
                if stubbed {
                    self.loss(true);
                }
                (stub, permit, stubbed)
            } else {
                self.loss(false);
                return;
            };
        let loss = (
            self.stubbed.load(Ordering::Relaxed),
            self.dropped.load(Ordering::Relaxed),
        );
        if !self
            .queue
            .send(Command::Event { event, stub, loss }, Some(permit))
        {
            self.loss(false);
            if settling {
                self.queue
                    .state
                    .settles_lost
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Copies only the small record fields. In particular, do not clone a `CallRecord`'s log just
/// to discard it: emit also runs in Drop on the realm's thread.
pub(crate) fn stub(event: &RunEvent) -> RunEvent {
    match event {
        RunEvent::AfterCall {
            run,
            call,
            parent,
            page,
            record,
            offset_us,
            ..
        } => {
            let record = neo_runtime::CallRecord {
                call_id: record.call_id,
                guid: record.guid.clone(),
                method: record.method.clone(),
                title: None,
                params: Value::Null,
                location: record.location,
                timeout_ms: record.timeout_ms,
                started_at_ms: record.started_at_ms,
                duration: record.duration,
                result: match &record.result {
                    CallResult::Ok => CallResult::Ok,
                    CallResult::Unfinished { reason } => CallResult::Unfinished { reason: *reason },
                    CallResult::Error { name, .. } => CallResult::Error {
                        name: name.clone(),
                        message: String::new(),
                    },
                },
                delivery: record.delivery,
                log: Vec::new(),
            };
            RunEvent::AfterCall {
                run: *run,
                call: *call,
                parent: *parent,
                page: page.clone(),
                record: Arc::new(record),
                params_truncated: true,
                offset_us: *offset_us,
            }
        }
        RunEvent::Getter {
            run,
            no,
            object,
            field,
            page,
            result,
            offset_us,
            duration_us,
        } => {
            let result = match result {
                GetterResult::Ok { .. } => GetterResult::Ok {
                    summary: Value::Null,
                },
                GetterResult::Error { name, .. } => GetterResult::Error {
                    name: name.clone(),
                    message: String::new(),
                },
            };
            RunEvent::Getter {
                run: *run,
                no: *no,
                object: object.clone(),
                field: *field,
                page: page.clone(),
                result,
                offset_us: *offset_us,
                duration_us: *duration_us,
            }
        }
        RunEvent::RunEnded {
            run,
            status,
            cause,
            error,
            calls_made,
            unreadable_calls,
            duration_us,
            ..
        } => RunEvent::RunEnded {
            run: *run,
            status: *status,
            cause: *cause,
            error: error.as_ref().map(|error| neo_runtime::RunError {
                name: error.name.clone(),
                message: String::new(),
            }),
            value: None,
            value_truncated: true,
            logs: Arc::from([]),
            script: None,
            script_truncated: true,
            calls_made: *calls_made,
            unreadable_calls: *unreadable_calls,
            duration_us: *duration_us,
        },
        _ => event.clone(),
    }
}

/// Conservative owned-memory charge, including enum/allocator overhead and shared payloads.
/// Value traversal counts memory without serializing or copying a large JSON result on emit.
fn weight(event: &RunEvent) -> usize {
    let mut bytes = mem::size_of::<Entry>() + mem::size_of::<RunEvent>() + 1024;
    bytes += match event {
        RunEvent::AfterCall { record, .. } => {
            record.guid.len()
                + record.method.len()
                + record.title.as_ref().map_or(0, String::len)
                + json_bytes(&record.params)
                + record.log.iter().map(|line| line.len() + 32).sum::<usize>()
                + error_bytes(&record.result)
        }
        RunEvent::Getter { object, result, .. } => {
            object.len()
                + match result {
                    GetterResult::Ok { summary } => json_bytes(summary),
                    GetterResult::Error { name, message } => name.len() + message.len(),
                }
        }
        RunEvent::RunEnded {
            value,
            logs,
            script,
            error,
            ..
        } => {
            value.as_deref().map_or(0, json_bytes)
                + logs.iter().map(|line| line.text.len() + 32).sum::<usize>()
                + script.as_ref().map_or(0, |script| script.len())
                + error
                    .as_ref()
                    .map_or(0, |error| error.name.len() + error.message.len())
        }
        RunEvent::BeforeCall {
            object,
            method,
            title,
            selector,
            ..
        } => {
            object.len()
                + method.len()
                + title.as_ref().map_or(0, String::len)
                + selector.as_ref().map_or(0, String::len)
        }
        RunEvent::InputSent { .. }
        | RunEvent::DeliverySettled { .. }
        | RunEvent::PageOpened { .. }
        | RunEvent::PageClosed { .. }
        | RunEvent::RunSettled { .. } => 0,
    };
    bytes
}

fn error_bytes(result: &CallResult) -> usize {
    match result {
        CallResult::Error { name, message } => name.len() + message.len(),
        _ => 0,
    }
}
fn json_bytes(value: &Value) -> usize {
    mem::size_of::<Value>()
        + match value {
            Value::String(text) => text.len(),
            Value::Array(items) => items.iter().map(json_bytes).sum(),
            Value::Object(items) => items
                .iter()
                .map(|(key, value)| key.len() + 64 + json_bytes(value))
                .sum(),
            _ => 0,
        }
}

pub(crate) fn run_id(command: &Command) -> Option<RunId> {
    match command {
        Command::Started(started) => Some(started.run.id),
        Command::Artifact(call, _) => Some(call.run),
        Command::Event { event, .. } => Some(match event {
            RunEvent::BeforeCall { run, .. }
            | RunEvent::InputSent { run, .. }
            | RunEvent::AfterCall { run, .. }
            | RunEvent::DeliverySettled { run, .. }
            | RunEvent::PageOpened { run, .. }
            | RunEvent::PageClosed { run, .. }
            | RunEvent::Getter { run, .. }
            | RunEvent::RunEnded { run, .. }
            | RunEvent::RunSettled { run } => *run,
        }),
        _ => None,
    }
}
