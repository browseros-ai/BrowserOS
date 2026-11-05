//! One run as the door sees it, and the exactly-once end of its record.

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::mem;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use browseros_core::{BrowserView, PageId, TabId};
use browseros_trace::{
    Delivery, Effect, EpochMillis, ErrorRecord, OperationId, Outcome, RequestId, RunEffects,
    RunEnded, RunId, RunStarted, RunStatus, TaskId, TraceSink,
};
use serde_json::Value;
use tokio::sync::OnceCell;
use tokio::task::JoinSet;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::operation_guard::Settlement;
use super::{CallSite, Dispatcher, ErrorCode, Lifetime, MethodSpec, OperationError};
use crate::browser::{PageRef, RunView};

/// One `run` call as the door sees it: its identity, the browser view it captured, its budget,
/// why it stopped, the ops it started, what they changed, and the state features keep per run.
///
/// Created only by [`RunCtx::start`], which records `run_started`, so no op runs outside a
/// recorded run. It lives in `dispatch/` so the dependency points one way (`run/` uses it).
/// The run service, the bridge's host futures and every op task hold an `Arc` to it.
pub(crate) struct RunCtx {
    id: RunId,
    task_id: Option<TaskId>,
    view: BrowserView,
    /// The CDP epoch the view was taken at. The unbound view would silently follow a new
    /// socket, so the door refuses once the link's epoch differs.
    epoch: u64,
    deadline: Instant,
    /// Cancelled exactly when [`RunCtx::stop`] is first called, and only by it.
    cancel: CancellationToken,
    /// Why the run stopped. Write-once; set before `cancel` fires.
    stop: OnceLock<StopReason>,
    default_page: DefaultPage,
    next_seq: AtomicU32,
    ledger: Mutex<RunLedger>,
    ops: Mutex<OpTasks>,
    slots: Mutex<RunSlots>,
}

/// Why a run stopped. Each reason decides who owns the ops it cut short.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopReason {
    /// Every caller waiting on the run left, and none came back in time.
    CallerLeft,
    /// The run's time budget ran out.
    Deadline,
    /// The script settled while ops still ran.
    ScriptReturned,
    /// Neo received SIGTERM.
    Shutdown,
    /// The run's own task panicked, or ended without recording its end.
    Crashed,
}

/// What [`RunCtx::start`] needs. A plain data bag.
pub(crate) struct RunStart {
    /// Neo's id for the run.
    pub(crate) id: RunId,
    /// The script, as given. The trace writer masks and bounds it.
    pub(crate) code: String,
    /// The host's id for this call; a repeat of it never runs twice.
    pub(crate) request_id: Option<RequestId>,
    /// The host's task; it picks the `page` default and groups runs in the trace.
    pub(crate) task_id: Option<TaskId>,
    /// The host's own data, stored and never read.
    pub(crate) host_data: Option<Value>,
    /// The browser as this run sees it.
    pub(crate) view: RunView,
    /// When the run's budget ends.
    pub(crate) deadline: Instant,
    /// The task's tab when the run service knows it; otherwise the run opens one on first use.
    pub(crate) default_page: Option<PageRef>,
}

/// The run's end record. A plain data bag.
#[derive(Clone, Debug)]
pub(crate) struct RunEnd {
    /// How the run ended.
    pub(crate) status: RunStatus,
    /// The uncaught error, when the script threw.
    pub(crate) error: Option<ErrorRecord>,
    /// The script's return value.
    pub(crate) result: Option<Value>,
    /// The masked reply, kept by the trace to answer a repeated `requestId`.
    pub(crate) replay: Option<Value>,
}

/// What a run's ops did, for the run's reply. `RunEnded` has no effects field (the trace
/// writer derives the same roll-up from the rows), so the run reports its own. A plain data
/// bag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpsReport {
    /// `none` if no write was sent, `unknown` if any sent write is unknown, else `done`: the
    /// trace store's rule.
    pub(crate) effects: RunEffects,
    /// Writes that were sent and never confirmed. Check the page before repeating them.
    pub(crate) unknown: Vec<OpLine>,
    /// Ops still running when the script returned; they were cut short.
    pub(crate) not_awaited: Vec<OpLine>,
}

/// One op, for a line in the run's reply. A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpLine {
    /// The op, `{run}.{seq}`.
    pub(crate) operation: OperationId,
    /// Its method.
    pub(crate) name: &'static str,
    /// Where the script called it.
    pub(crate) site: Option<CallSite>,
}

/// Ends the run's record exactly once: [`RunGuard::finish`] on the normal path, `Drop` when
/// the run's task unwound. On drop it also stops the run and aborts its op tasks, which settle
/// themselves through their own guards.
#[must_use]
pub(crate) struct RunGuard {
    run: Option<Arc<RunCtx>>,
    trace: TraceSink,
}

enum DefaultPage {
    Known(PageRef),
    /// Opened on first use. Single-flight: concurrent ops on the lazy page open one tab.
    Lazy(OnceCell<PageRef>),
}

#[derive(Default)]
struct RunLedger {
    wrote: bool,
    unknown: Vec<OpLine>,
    not_awaited: Vec<OpLine>,
    /// Pages this run closed, so a later use is the script's mistake, not Neo's.
    closed_by_run: HashSet<PageId>,
}

/// The run's op tasks. Closed at teardown; a closed set admits nothing.
#[derive(Default)]
struct OpTasks {
    set: JoinSet<()>,
    closed: bool,
}

/// Per-run feature state, one value per type ([`RunCtx::slot`]).
///
/// The guard empties it when the run's record ends, on the normal path and on a crash alike,
/// rather than leaving it to the `RunCtx`'s own drop: a run's state must not live on in
/// whatever still holds the `RunCtx`. sdk-playwright's `RunState` is why: the event hub holds
/// runs weakly, so dropping it is what ends the run's listeners and releases the dialogs it
/// held. A side table keyed by run id would leak on every abnormal end instead.
#[derive(Default)]
struct RunSlots {
    values: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// Set once the run ended. A later `slot` gets a value that nothing keeps.
    ended: bool,
}

impl RunCtx {
    /// Opens the run: records `run_started` (a channel send, never a wait on disk) and returns
    /// the guard that records its end.
    pub(crate) fn start(trace: &TraceSink, start: RunStart) -> (Arc<Self>, RunGuard) {
        let RunStart {
            id,
            code,
            request_id,
            task_id,
            host_data,
            view,
            deadline,
            default_page,
        } = start;
        trace.run_started(RunStarted {
            id,
            code,
            request_id,
            task_id: task_id.clone(),
            host_data,
            at: EpochMillis::now(),
        });
        let run = Arc::new(Self {
            id,
            task_id,
            view: view.view,
            epoch: view.epoch,
            deadline,
            cancel: CancellationToken::new(),
            stop: OnceLock::new(),
            default_page: default_page
                .map_or_else(|| DefaultPage::Lazy(OnceCell::new()), DefaultPage::Known),
            next_seq: AtomicU32::new(1),
            ledger: Mutex::default(),
            ops: Mutex::default(),
            slots: Mutex::default(),
        });
        let guard = RunGuard {
            run: Some(Arc::clone(&run)),
            trace: trace.clone(),
        };
        (run, guard)
    }

    /// Neo's id for this run.
    #[must_use]
    pub(crate) fn id(&self) -> RunId {
        self.id
    }

    /// The host's task, if it sent one.
    #[must_use]
    pub(crate) fn task_id(&self) -> Option<&TaskId> {
        self.task_id.as_ref()
    }

    /// The browser as this run sees it: unbound (every tab), with the process's viewports.
    #[must_use]
    pub(crate) fn view(&self) -> &BrowserView {
        &self.view
    }

    /// The CDP epoch the view was taken at.
    #[must_use]
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }

    /// When the run's budget ends.
    #[must_use]
    pub(crate) fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Fires when the run stops, for whatever reason.
    #[must_use]
    pub(crate) fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Stops the run. The first reason wins, and it is recorded before the token fires, so
    /// whatever wakes on the token reads the reason that woke it.
    pub(crate) fn stop(&self, reason: StopReason) {
        let _ = self.stop.set(reason);
        self.cancel.cancel();
    }

    /// Why the run stopped; `None` while it runs.
    #[must_use]
    pub(crate) fn stop_reason(&self) -> Option<StopReason> {
        self.stop.get().copied()
    }

    /// What the ops did so far. Complete once teardown drained them.
    #[must_use]
    pub(crate) fn report(&self) -> OpsReport {
        let ledger = self.lock_ledger();
        let effects = if !ledger.unknown.is_empty() {
            RunEffects::Unknown
        } else if ledger.wrote {
            RunEffects::Done
        } else {
            RunEffects::None
        };
        OpsReport {
            effects,
            unknown: ledger.unknown.clone(),
            not_awaited: ledger.not_awaited.clone(),
        }
    }

    /// Teardown: refuses new ops and hands back the running ones, to drain with a grace and
    /// then drop. Dropping the set aborts what is left, and each aborted op settles itself.
    #[must_use]
    pub(crate) fn close_ops(&self) -> JoinSet<()> {
        let mut ops = self.lock_ops();
        ops.closed = true;
        mem::take(&mut ops.set)
    }

    /// The run's one `T`, created on first use and shared by every op of the run (sdk-playwright
    /// keeps its `RunState` here). It goes when the run's record ends, however the run ends; an
    /// op still holding it keeps its own reference only until that op settles. Called after the
    /// end, it returns a fresh `T` that nothing keeps.
    pub(crate) fn slot<T: Default + Send + Sync + 'static>(&self) -> Arc<T> {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        if slots.ended {
            return Arc::new(T::default());
        }
        let slot = slots
            .values
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Arc::new(T::default()));
        // Keyed by `TypeId::of::<T>()`, so the downcast cannot miss.
        Arc::clone(slot)
            .downcast::<T>()
            .unwrap_or_else(|_other| Arc::new(T::default()))
    }

    /// The run's record ended: its feature state goes now. Dropped after the lock is released,
    /// because a value's `Drop` may do anything, such as release the dialogs a run held.
    fn end_slots(&self) {
        let values = {
            let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            slots.ended = true;
            mem::take(&mut slots.values)
        };
        drop(values);
    }

    /// The next op id, in call order.
    pub(super) fn next_operation(&self) -> OperationId {
        OperationId::new(self.id, self.next_seq.fetch_add(1, Ordering::Relaxed))
    }

    /// Runs one op as its own task. Once teardown closed the set, the op is dropped instead,
    /// and its guard settles it as stopped.
    pub(super) fn spawn_op(&self, op: impl Future<Output = ()> + Send + 'static) {
        let refused = {
            let mut ops = self.lock_ops();
            if ops.closed {
                Some(op)
            } else {
                // Reaped here so a long run with many ops keeps a small set.
                while ops.set.try_join_next().is_some() {}
                ops.set.spawn(op);
                None
            }
        };
        // Outside the lock: settling records the op and touches the ledger.
        drop(refused);
    }

    /// The run's default page, opened on first use by a Neo op (single-flight).
    pub(super) async fn default_page(
        self: &Arc<Self>,
        dispatcher: &Arc<Dispatcher>,
    ) -> Result<PageRef, OperationError> {
        match &self.default_page {
            DefaultPage::Known(page) => Ok(page.clone()),
            DefaultPage::Lazy(cell) => cell
                .get_or_try_init(|| dispatcher.open_default_page(self))
                .await
                .cloned(),
        }
    }

    /// The default page's tab when it is known without opening anything.
    pub(super) fn known_default_tab(&self) -> Option<TabId> {
        match &self.default_page {
            DefaultPage::Known(page) => Some(page.tab),
            DefaultPage::Lazy(cell) => cell.get().map(|page| page.tab),
        }
    }

    /// An op settled: the effects roll-up and the reply's lines.
    pub(super) fn settled(&self, line: OpLine, spec: &MethodSpec, settled: &Settlement) {
        let unknown_write = match (spec.effect, settled.delivery) {
            (Effect::Write, Delivery::Unknown) => true,
            (Effect::Write, Delivery::NotSent | Delivery::Done)
            | (Effect::Read, Delivery::NotSent | Delivery::Done | Delivery::Unknown) => false,
        };
        let sent_write = match (spec.effect, settled.delivery) {
            (Effect::Write, Delivery::Done | Delivery::Unknown) => true,
            (Effect::Write, Delivery::NotSent)
            | (Effect::Read, Delivery::NotSent | Delivery::Done | Delivery::Unknown) => false,
        };
        let cut_short = settled.outcome == Outcome::Cancelled
            && self.stop_reason() == Some(StopReason::ScriptReturned)
            && match spec.lifetime {
                Lifetime::Operation => true,
                Lifetime::Background => false,
            };
        let mut ledger = self.lock_ledger();
        ledger.wrote |= sent_write;
        match (unknown_write, cut_short) {
            (true, true) => {
                ledger.unknown.push(line.clone());
                ledger.not_awaited.push(line);
            }
            (true, false) => ledger.unknown.push(line),
            (false, true) => ledger.not_awaited.push(line),
            (false, false) => {}
        }
    }

    /// This run closed `page`.
    pub(super) fn record_closed(&self, page: &PageId) {
        self.lock_ledger().closed_by_run.insert(page.clone());
    }

    /// Whether this run closed `page`, which decides who owns `page_closed`.
    pub(super) fn closed_it(&self, page: &PageId) -> bool {
        self.lock_ledger().closed_by_run.contains(page)
    }

    fn lock_ledger(&self) -> MutexGuard<'_, RunLedger> {
        self.ledger.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_ops(&self) -> MutexGuard<'_, OpTasks> {
        self.ops.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl StopReason {
    /// The error of an op this stop cut short. The owner is whoever caused the stop.
    #[must_use]
    pub(crate) fn error(self) -> OperationError {
        match self {
            Self::CallerLeft => OperationError::host(
                ErrorCode::CANCELLED,
                "the caller went away before this operation finished",
            ),
            Self::Deadline => {
                OperationError::neo(ErrorCode::RUN_TIMEOUT, "the run reached its time limit")
                    .timed_out()
            }
            Self::ScriptReturned => OperationError::script(
                ErrorCode::NOT_AWAITED,
                "the script returned without awaiting this operation",
            ),
            Self::Shutdown => OperationError::neo(ErrorCode::SHUTTING_DOWN, "Neo is shutting down"),
            Self::Crashed => OperationError::neo(
                ErrorCode::INTERNAL_ERROR,
                "the run stopped on an internal error",
            ),
        }
    }
}

impl RunGuard {
    /// Records the run's end. Teardown drains the run's ops first, so their `op_ended`
    /// records precede it; the trace turns any op still open at a run's end into `unknown`.
    pub(crate) fn finish(mut self, end: RunEnd) {
        if let Some(run) = self.run.take() {
            run.end_slots();
            self.trace.run_ended(RunEnded {
                id: run.id,
                status: end.status,
                error: end.error,
                result: end.result,
                replay: end.replay,
                at: EpochMillis::now(),
            });
        }
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        let Some(run) = self.run.take() else {
            return;
        };
        run.stop(StopReason::Crashed);
        drop(run.close_ops());
        run.end_slots();
        // Ops still in flight may have sent something, so the run is not called retryable.
        let error = StopReason::Crashed.error().record(false, None);
        self.trace.run_ended(RunEnded {
            id: run.id,
            status: RunStatus::Error,
            error: Some(error),
            result: None,
            replay: None,
            at: EpochMillis::now(),
        });
    }
}

#[cfg(test)]
mod tests {
    use browseros_trace::ErrorOwner;

    use super::StopReason;
    use crate::dispatch::ErrorCode;

    #[test]
    fn a_stop_names_who_caused_it() {
        let cases = [
            (
                StopReason::CallerLeft,
                ErrorOwner::Host,
                ErrorCode::CANCELLED,
            ),
            (
                StopReason::Deadline,
                ErrorOwner::Neo,
                ErrorCode::RUN_TIMEOUT,
            ),
            (
                StopReason::ScriptReturned,
                ErrorOwner::Script,
                ErrorCode::NOT_AWAITED,
            ),
            (
                StopReason::Shutdown,
                ErrorOwner::Neo,
                ErrorCode::SHUTTING_DOWN,
            ),
            (
                StopReason::Crashed,
                ErrorOwner::Neo,
                ErrorCode::INTERNAL_ERROR,
            ),
        ];
        for (reason, owner, code) in cases {
            let error = reason.error();
            assert_eq!((error.owner(), error.code()), (owner, code), "{reason:?}");
            assert_eq!(
                error.is_timeout(),
                reason == StopReason::Deadline,
                "only the deadline throws a TimeoutError"
            );
        }
    }
}
