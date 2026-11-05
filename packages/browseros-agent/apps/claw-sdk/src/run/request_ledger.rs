//! `requestId` admission: a repeat never runs again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use browseros_trace::{EpochMillis, RequestId, RunId, TaskId, TraceError, TraceSink, TraceStore};
use tokio::runtime::Handle;
#[cfg(test)]
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::{RunHandle, RunMeta, RunReply, Waiter};
use crate::config::Defaults;
use crate::dispatch::{ErrorCode, OperationError};

/// Decides whether a `run` call runs, so a host's repeat of a `requestId` never runs twice
/// (sdk-traces D14).
///
/// Memory answers first, for runs that are running (a repeat joins) and runs that ended within
/// `Defaults::REPLAY_WINDOW`. The store alone cannot: its writer commits in batches and its
/// request index is not unique, so a second run could be admitted before the first one's row
/// lands. Once the store shows a run's replay, memory keeps only a marker and the store
/// answers, which also covers runs from before a Neo restart. A store read error refuses the
/// call (`neo / trace_unavailable`) rather than risk running it twice.
///
/// Memory lets go of an ended run only once the store has shown it the run: the writer drops a
/// batch it could not commit, and a request whose start record was lost would otherwise run
/// again once its reply expired. Such a request stays as a marker for Neo's life, and
/// `LedgerLimits::max_unrecorded` of them stop new request ids rather than grow without bound.
///
/// The match is D14's: with a `taskId`, an earlier run of that task with the same `requestId`;
/// without one, any run with that `requestId`.
pub(crate) struct RequestLedger {
    store: TraceStore,
    trace: TraceSink,
    limits: LedgerLimits,
    /// One std lock, never held across an await.
    entries: Mutex<HashMap<RequestId, Vec<Entry>>>,
    /// Holds the next call between its memory miss and its store lookup, so tests can land
    /// another call's run in that gap.
    #[cfg(test)]
    store_lookup_pause: Mutex<Option<StoreLookupPause>>,
}

/// The pause [`RequestLedger::pause_next_store_lookup`] sets: `reached` fires when a call is
/// held, `resume` lets it go on.
#[cfg(test)]
struct StoreLookupPause {
    reached: oneshot::Sender<()>,
    resume: oneshot::Receiver<()>,
}

/// How long memory answers for an ended run, and how many unrecorded requests it holds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LedgerLimits {
    /// `Defaults::REPLAY_WINDOW` in the app.
    pub(crate) replay_window: Duration,
    /// `Defaults::MAX_UNRECORDED_REQUESTS` in the app.
    pub(crate) max_unrecorded: usize,
}

impl Default for LedgerLimits {
    fn default() -> Self {
        Self {
            replay_window: Defaults::REPLAY_WINDOW,
            max_unrecorded: Defaults::MAX_UNRECORDED_REQUESTS,
        }
    }
}

/// One run of a request id, as memory knows it.
struct Entry {
    task: Option<TaskId>,
    run: RunId,
    state: EntryState,
    /// When a run that ended stops answering from memory; `None` while it runs.
    until: Option<Instant>,
}

enum EntryState {
    /// Running: a repeat joins it.
    Running(Arc<RunHandle>),
    /// Ended, and its replay is not in the store (not yet, its batch was lost, or it crashed):
    /// a repeat gets this reply until `until`. `recorded` once the store has shown the run,
    /// so that after `until` the store refuses a repeat and memory can let go.
    Finished {
        reply: Arc<RunReply>,
        recorded: bool,
    },
    /// Ended, and the store has the run: the store answers. Kept without the reply until the
    /// window ends, so a call whose own store lookup ran before this run's row landed still
    /// finds it here.
    Stored,
    /// Ended, its reply expired, and the store never showed the run: this marker is the only
    /// proof the request ran, so a repeat is refused, for Neo's life.
    Unrecorded,
}

/// What the store showed of an ended run.
enum Kept {
    /// Its replay: the store answers a repeat in full.
    Replay,
    /// Its row without a replay (the run crashed, or its end was lost): the store refuses a
    /// repeat as a duplicate.
    Row,
}

/// What admission decided.
pub(crate) enum Admission {
    /// Run it: the ticket holds the request id's place until the run's reply is out.
    Fresh(RunTicket),
    /// The same request is running: wait for its reply.
    Join(Waiter),
    /// The same request ended within the window: its reply, `id` included.
    Replay(Arc<RunReply>),
    /// The same request ended before the window: refused, naming that run.
    Duplicate(RunId),
    /// The store could not be read, so the call is refused rather than risk a second run.
    Unavailable(OperationError),
}

/// The right to run one call, held by its run task, which publishes the reply through it.
/// Every path publishes: `finish` after the run, `release` for a call refused before it ran,
/// and `Drop` (the run task unwound) a crashed reply, so waiters always get an answer and a
/// crashed request never runs a second time.
#[must_use]
pub(crate) struct RunTicket {
    handle: Arc<RunHandle>,
    /// `None` without a `requestId`: nothing to remember.
    place: Option<Place>,
    published: bool,
}

/// A request id's entry in the ledger.
struct Place {
    ledger: Arc<RequestLedger>,
    request: RequestId,
}

/// A memory hit.
enum Found {
    Answer(Admission),
    /// Ended and flushed: the store has its replay.
    Stored(RunId),
}

impl RequestLedger {
    /// The ledger over the process's one trace store and writer.
    #[must_use]
    pub(crate) fn new(store: TraceStore, trace: TraceSink, limits: LedgerLimits) -> Arc<Self> {
        Arc::new(Self {
            store,
            trace,
            limits,
            entries: Mutex::default(),
            #[cfg(test)]
            store_lookup_pause: Mutex::default(),
        })
    }

    /// Holds the next call that misses memory before its store lookup. Returns the signal that
    /// it is held and the sender that resumes it.
    #[cfg(test)]
    pub(crate) fn pause_next_store_lookup(&self) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (reached, held) = oneshot::channel();
        let (resume, resumed) = oneshot::channel();
        *self
            .store_lookup_pause
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(StoreLookupPause {
            reached,
            resume: resumed,
        });
        (held, resume)
    }

    #[cfg(test)]
    async fn pause_if_asked(&self) {
        let pause = self
            .store_lookup_pause
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(pause) = pause {
            let _held = pause.reached.send(());
            let _resumed = pause.resume.await;
        }
    }

    /// Admits one call, which becomes run `handle` if it runs.
    pub(crate) async fn admit(
        self: &Arc<Self>,
        meta: &RunMeta,
        handle: &Arc<RunHandle>,
    ) -> Admission {
        let Some(request) = meta.request_id.as_ref() else {
            return Admission::Fresh(RunTicket::untracked(handle));
        };
        let task = meta.task_id.as_ref();
        // 1. Memory, for running and recent runs.
        if let Some(found) = self.find(request, task) {
            return self.answer(found).await;
        }
        #[cfg(test)]
        self.pause_if_asked().await;
        // 2. The store, with no lock held: runs past memory's window, or from before a restart.
        match self.store.run_by_request(request, task).await {
            Ok(Some(first)) => {
                // The row may be a run admitted after step 1 whose start committed during the
                // lookup. Memory still has it, running (the repeat joins it) or just ended (it
                // replays); the store answers only for a run memory has let go of.
                let found = self.find(request, task).unwrap_or(Found::Stored(first.id));
                return self.answer(found).await;
            }
            Ok(None) => {}
            Err(error) => return Admission::Unavailable(unreadable(&error)),
        }
        // 3. Memory again, under the lock that claims the id: two concurrent first calls both
        //    passed step 2, and the second to get here finds the first one running.
        match self.claim(request, task, handle) {
            Ok(ticket) => Admission::Fresh(ticket),
            Err(found) => self.answer(found).await,
        }
    }

    fn find(&self, request: &RequestId, task: Option<&TaskId>) -> Option<Found> {
        let mut entries = self.lock();
        prune(&mut entries, Instant::now());
        find_in(&entries, request, task)
    }

    /// Records `handle` as running `request`, unless a match got there first.
    fn claim(
        self: &Arc<Self>,
        request: &RequestId,
        task: Option<&TaskId>,
        handle: &Arc<RunHandle>,
    ) -> Result<RunTicket, Found> {
        let mut entries = self.lock();
        let unrecorded = prune(&mut entries, Instant::now());
        if let Some(found) = find_in(&entries, request, task) {
            return Err(found);
        }
        if unrecorded >= self.limits.max_unrecorded {
            return Err(Found::Answer(Admission::Unavailable(too_many_unrecorded(
                unrecorded,
            ))));
        }
        entries.entry(request.clone()).or_default().push(Entry {
            task: task.cloned(),
            run: handle.id(),
            state: EntryState::Running(Arc::clone(handle)),
            until: None,
        });
        Ok(RunTicket {
            handle: Arc::clone(handle),
            place: Some(Place {
                ledger: Arc::clone(self),
                request: request.clone(),
            }),
            published: false,
        })
    }

    async fn answer(&self, found: Found) -> Admission {
        match found {
            Found::Answer(admission) => admission,
            Found::Stored(run) => self.stored(run).await,
        }
    }

    /// A run the store has: its replay within the window, else a duplicate naming it. A
    /// replay that no longer decodes (written by another Neo version) is a duplicate too.
    async fn stored(&self, run: RunId) -> Admission {
        match self.store.replay(run, EpochMillis::now()).await {
            Ok(Some(replay)) => match serde_json::from_value::<RunReply>(replay) {
                Ok(reply) => Admission::Replay(Arc::new(reply)),
                Err(error) => {
                    tracing::warn!(%error, %run, "a stored replay does not decode; refusing the repeat");
                    Admission::Duplicate(run)
                }
            },
            Ok(None) => Admission::Duplicate(run),
            Err(error) => Admission::Unavailable(unreadable(&error)),
        }
    }

    /// The run ended with `reply`, and its end record is on its way to the writer. Memory
    /// answers repeats with the reply until the store shows the run's replay, then the store
    /// does.
    fn ended(self: &Arc<Self>, request: &RequestId, run: RunId, reply: Arc<RunReply>) {
        {
            let mut entries = self.lock();
            if let Some(entry) = entry_of(&mut entries, request, run) {
                entry.state = EntryState::Finished {
                    reply,
                    recorded: false,
                };
                entry.until = Some(Instant::now() + self.limits.replay_window);
            }
        }
        // A ticket dropped while the runtime itself shuts down has no runtime to spawn on, and
        // `tokio::spawn` would panic inside that unwind. The process is ending; nothing to keep.
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        let ledger = Arc::clone(self);
        let request = request.clone();
        runtime.spawn(async move { ledger.confirm(&request, run).await });
    }

    /// Asks the store what it kept of ended run `run`, once the writer has committed what was
    /// sent before, and lets memory rely on that. A flush error does not answer it: the writer
    /// reports a lost batch to whichever flush comes next, maybe another run's, so only the
    /// store says what landed.
    async fn confirm(&self, request: &RequestId, run: RunId) {
        if let Err(error) = self.trace.flush().await {
            tracing::warn!(%error, %run, "the trace lost a batch; checking what it kept of the run");
        }
        let kept = match self.kept(run).await {
            Ok(Some(kept)) => kept,
            Ok(None) => {
                tracing::warn!(%run, "the trace lost the run's start; memory refuses its requestId for Neo's life");
                return;
            }
            Err(error) => {
                tracing::warn!(%error, %run, "the trace cannot show the run; memory refuses its requestId for Neo's life");
                return;
            }
        };
        let mut entries = self.lock();
        let Some(entry) = entry_of(&mut entries, request, run) else {
            return;
        };
        match (kept, &mut entry.state) {
            (Kept::Replay, _) | (Kept::Row, EntryState::Unrecorded) => {
                entry.state = EntryState::Stored;
            }
            (Kept::Row, EntryState::Finished { recorded, .. }) => *recorded = true,
            (Kept::Row, EntryState::Running(_) | EntryState::Stored) => {}
        }
    }

    async fn kept(&self, run: RunId) -> Result<Option<Kept>, TraceError> {
        if self.store.replay(run, EpochMillis::now()).await?.is_some() {
            return Ok(Some(Kept::Replay));
        }
        Ok(self.store.run_summary(run).await?.map(|_| Kept::Row))
    }

    /// A call refused before it ran: its id is free for the retry that should run.
    fn forget(&self, request: &RequestId, run: RunId) {
        let mut entries = self.lock();
        if let Some(runs) = entries.get_mut(request) {
            runs.retain(|entry| entry.run != run);
            if runs.is_empty() {
                entries.remove(request);
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<RequestId, Vec<Entry>>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl RunTicket {
    fn untracked(handle: &Arc<RunHandle>) -> Self {
        Self {
            handle: Arc::clone(handle),
            place: None,
            published: false,
        }
    }

    /// Publishes the run's reply: the waiters get it, and repeats within the window replay it.
    /// The run's end record must already be on its way to the writer.
    pub(crate) fn finish(mut self, reply: Arc<RunReply>) {
        self.published = true;
        self.handle.finish(Arc::clone(&reply));
        if let Some(place) = self.place.take() {
            place.ledger.ended(&place.request, self.handle.id(), reply);
        }
    }

    /// Publishes the reply of a call refused before it ran, and frees its request id: nothing
    /// ran, so a retry should.
    pub(crate) fn release(mut self, reply: Arc<RunReply>) {
        self.published = true;
        self.handle.finish(reply);
        if let Some(place) = self.place.take() {
            place.ledger.forget(&place.request, self.handle.id());
        }
    }
}

impl Drop for RunTicket {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        // The run task unwound (its guard has recorded the run's end as a crash). Its ops may
        // have sent anything, so the request id answers with the crash, never with a new run.
        let crashed = Arc::new(RunReply::crashed(self.handle.id()));
        self.handle.finish(Arc::clone(&crashed));
        if let Some(place) = self.place.take() {
            place
                .ledger
                .ended(&place.request, self.handle.id(), crashed);
        }
    }
}

fn find_in(
    entries: &HashMap<RequestId, Vec<Entry>>,
    request: &RequestId,
    task: Option<&TaskId>,
) -> Option<Found> {
    // The earliest match, as the store's lookup returns the earliest run.
    let entry = entries
        .get(request)?
        .iter()
        .find(|entry| task.is_none_or(|task| entry.task.as_ref() == Some(task)))?;
    Some(match &entry.state {
        EntryState::Running(handle) => Found::Answer(Admission::Join(handle.waiter())),
        EntryState::Finished { reply, .. } => Found::Answer(Admission::Replay(Arc::clone(reply))),
        EntryState::Stored => Found::Stored(entry.run),
        EntryState::Unrecorded => Found::Answer(Admission::Duplicate(entry.run)),
    })
}

fn entry_of<'a>(
    entries: &'a mut HashMap<RequestId, Vec<Entry>>,
    request: &RequestId,
    run: RunId,
) -> Option<&'a mut Entry> {
    entries
        .get_mut(request)?
        .iter_mut()
        .find(|entry| entry.run == run)
}

/// Lets go of the runs whose reply expired and that the store has, keeps an expired run the
/// store never showed as a marker without its reply, and counts those markers.
fn prune(entries: &mut HashMap<RequestId, Vec<Entry>>, now: Instant) -> usize {
    let mut unrecorded = 0;
    entries.retain(|_, runs| {
        runs.retain_mut(|entry| {
            if entry.until.is_some_and(|until| until <= now) {
                match entry.state {
                    EntryState::Finished {
                        recorded: false, ..
                    } => entry.state = EntryState::Unrecorded,
                    EntryState::Unrecorded => {}
                    EntryState::Running(_)
                    | EntryState::Finished { recorded: true, .. }
                    | EntryState::Stored => return false,
                }
            }
            if matches!(entry.state, EntryState::Unrecorded) {
                unrecorded += 1;
            }
            true
        });
        !runs.is_empty()
    });
    unrecorded
}

fn too_many_unrecorded(unrecorded: usize) -> OperationError {
    OperationError::neo(
        ErrorCode::TRACE_UNAVAILABLE,
        format_args!(
            "Neo's trace lost the records of {unrecorded} earlier requests, so it cannot promise \
             this requestId runs once; it did not run the call"
        ),
    )
}

fn unreadable(error: &TraceError) -> OperationError {
    OperationError::neo(
        ErrorCode::TRACE_UNAVAILABLE,
        format_args!(
            "Neo cannot read its trace to check this requestId, so it did not run the call: {error}"
        ),
    )
}
