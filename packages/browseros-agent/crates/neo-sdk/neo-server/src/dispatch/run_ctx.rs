//! One run as the door sees it, and the exactly-once end of its record.

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::mem;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use browseros_core::{BrowserView, TabId};
use neo_takeover::LockId;
use neo_trace::{
    Delivery, Effect, EpochMillis, ErrorRecord, OperationId, Outcome, RequestId, RunEffects,
    RunEnded, RunId, RunStarted, RunStatus, TaskId, TraceSink,
};
use serde_json::Value;
use tokio::sync::OnceCell;
use tokio::task::JoinSet;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::opens_in_flight::{OpenInFlight, OpensInFlight};
use super::operation_guard::Settlement;
use super::run_tasks::RunTasks;
use super::{
    CallSite, Dispatcher, ErrorCode, Lifetime, MethodSpec, OperationError, TaskKind, TaskNote,
};
use crate::browser::{PageRef, RunScope, RunView, Sight, TabRegistry, TabsScope, TaskRecord};

/// One `run` call as the door sees it: its identity, the browser view it captured, its budget,
/// why it stopped, the ops it started, what they changed, and the state features keep per run.
///
/// Created only by [`RunCtx::start`], which records `run_started`, so no op runs outside a
/// recorded run. It lives in `dispatch/` so the dependency points one way (`run/` uses it).
/// The run service, the bridge's host futures and every op task hold an `Arc` to it.
pub(crate) struct RunCtx {
    id: RunId,
    task_id: Option<TaskId>,
    /// The family of the host's task, read once at the run's start ([`RunCtx::family`]).
    family: Option<TaskId>,
    /// What the run may reach, fixed at its start ([`RunCtx::sight`]).
    scope: RunScope,
    /// Read for the run's sight of each tab, live: the scope is fixed, which tabs it covers is not.
    tabs: Arc<TabRegistry>,
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
    tasks: Mutex<RunTasks>,
    /// The host lock this run's ops pass ([`RunCtx::pass_lock`]). Write-once.
    lock_pass: OnceLock<LockId>,
    /// A bound run's opens whose tab the browser has not named yet ([`RunCtx::opening`]).
    opens: OpensInFlight,
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
    /// The script, as given. The trace writer bounds it.
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
    /// The task's tab when the run service knows it; otherwise resolved on first use.
    pub(crate) default_page: Option<PageRef>,
    /// The tab scope setting as the run read it at its start. With the run's task it decides
    /// what the run may reach ([`RunScope::decide`]). Neo's own runs (the janitor, private
    /// typing) read `Browser` always: they reach every tab.
    pub(crate) tabs_scope: TabsScope,
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
    /// The exact reply, kept by the trace to answer a repeated `requestId`.
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
/// themselves through their own guards. Either way the tabs the run used stop being in use
/// for the idle-tab janitor (`TabRegistry::run_ended`).
#[must_use]
pub(crate) struct RunGuard {
    run: Option<Arc<RunCtx>>,
    trace: TraceSink,
    tabs: Arc<TabRegistry>,
}

enum DefaultPage {
    Known(PageRef),
    /// Chosen on first use. Single-flight: concurrent ops on the lazy page share one tab.
    Lazy(OnceCell<PageRef>),
}

#[derive(Default)]
struct RunLedger {
    wrote: bool,
    unknown: Vec<OpLine>,
    not_awaited: Vec<OpLine>,
    /// Pages this run closed, so a later use is the script's mistake, not Neo's.
    closed_by_run: HashSet<TabId>,
    /// Tabs this bound run's own opens named ([`RunCtx::opened`]). Bounded by the run's opens.
    opened_by_run: HashSet<TabId>,
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
    /// Opens the run at `door`: records `run_started` through the door's trace (a channel send,
    /// never a wait on disk) and returns the guard that records its end.
    pub(crate) fn start(door: &Dispatcher, start: RunStart) -> (Arc<Self>, RunGuard) {
        let RunStart {
            id,
            code,
            request_id,
            task_id,
            host_data,
            view,
            deadline,
            default_page,
            tabs_scope,
        } = start;
        let trace = door.trace();
        if let Some(task) = &task_id {
            // Register the host's task once. A missing or full task book must not block opens,
            // and a task closed later in this run must not be silently recreated.
            let _ = door.tabs().ensure_task(task);
        }
        // Fixed now, so a task this run creates joins the host task's family even after the run
        // closed that task. A task the full book could not register roots its own family.
        let family = task_id.as_ref().map(|task| {
            door.tabs()
                .task_family(task)
                .unwrap_or_else(|| task.clone())
        });
        let scope = RunScope::decide(tabs_scope, family.as_ref());
        let tasks = Mutex::new(RunTasks::new(task_id.clone()));
        // In use from the start: the janitor must not close the task's tab between the run's
        // start and its first op.
        door.tabs()
            .run_started(id, default_page.as_ref().map(|page| page.tab));
        trace.run_started(RunStarted {
            id,
            code,
            request_id,
            task_id: task_id.clone(),
            host_data,
            tabs_scope: scope.record(),
            at: EpochMillis::now(),
        });
        let run = Arc::new(Self {
            id,
            task_id,
            family,
            scope,
            tabs: Arc::clone(door.tabs()),
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
            tasks,
            lock_pass: OnceLock::new(),
            opens: OpensInFlight::default(),
        });
        let guard = RunGuard {
            run: Some(Arc::clone(&run)),
            trace: trace.clone(),
            tabs: Arc::clone(door.tabs()),
        };
        (run, guard)
    }

    /// Neo's id for this run.
    #[must_use]
    pub(crate) fn id(&self) -> RunId {
        self.id
    }

    /// Lets this run's ops pass host lock `lock`, as the takeover rule reads it
    /// (`PolicyCheck::lock_pass`). Only the host's private typing calls it, before it submits its
    /// one op, with the `lockId` the host named and Neo checked is held on that op's tab: the
    /// host types through its own lock. A script's run never has one. Write-once.
    pub(crate) fn pass_lock(&self, lock: LockId) {
        let _ = self.lock_pass.set(lock);
    }

    /// The host lock this run's ops pass, if any.
    #[must_use]
    pub(crate) fn lock_pass(&self) -> Option<LockId> {
        self.lock_pass.get().copied()
    }

    /// The host's task, if it sent one.
    #[must_use]
    pub(crate) fn task_id(&self) -> Option<&TaskId> {
        self.task_id.as_ref()
    }

    /// The family of the host's task as the run started: tasks this run creates join it. `None`
    /// for a run with no host task, whose created tasks each root their own family, even after
    /// it picked another task.
    #[must_use]
    pub(crate) fn family(&self) -> Option<&TaskId> {
        self.family.as_ref()
    }

    /// What this run may reach, fixed at its start.
    #[must_use]
    pub(crate) fn scope(&self) -> &RunScope {
        &self.scope
    }

    /// Whether this run may reach `tab` now: the registry's answer for its scope, except that a
    /// bound run's own open is shown from the moment the browser names it, before its task
    /// holds it ([`RunCtx::opened`]).
    #[must_use]
    pub(crate) fn sight(&self, tab: TabId) -> Sight {
        if self.scope.is_bound() && self.lock_ledger().opened_by_run.contains(&tab) {
            return match self.tabs.sight(&RunScope::Browser, tab) {
                Sight::Reserved => Sight::Reserved,
                Sight::Shown | Sight::Closed | Sight::Foreign => Sight::Shown,
            };
        }
        self.tabs.sight(&self.scope, tab)
    }

    /// This run's own open named `tab`. Its task takes it once the open returns; until then the
    /// run sees it all the same, and after that its task holds it.
    pub(crate) fn opened(&self, tab: TabId) {
        if self.scope.is_bound() {
            self.lock_ledger().opened_by_run.insert(tab);
        }
    }

    /// The run's op `op` starts an open. Until the browser names its tab ([`RunCtx::named`]) or
    /// the op ends, the run's other ops wait before they judge a tab they do not know
    /// ([`RunCtx::others_opened`]). `None` for a run that is not bound: it sees every tab, so
    /// nothing waits.
    pub(crate) fn opening(&self, op: OperationId) -> Option<OpenInFlight<'_>> {
        self.scope.is_bound().then(|| self.opens.begin(op))
    }

    /// `op`'s open named `tab`, as soon as the browser did: the run's own from now on
    /// ([`RunCtx::opened`]), and no longer an open its other ops wait for.
    pub(crate) fn named(&self, op: OperationId, tab: TabId) {
        self.opened(tab);
        self.opens.named(op);
    }

    /// Resolves once no op of this run but `op` has an open in flight: a tab that op is opening
    /// is then the run's own, or not the run's at all.
    pub(crate) async fn others_opened(&self, op: OperationId) {
        self.opens.others_named(op).await;
    }

    /// Sets the task used by subsequent opens and the unresolved built-in page.
    pub(crate) fn picked_task(&self, task: &TaskRecord, kind: TaskKind) {
        self.lock_tasks().pick(task, kind);
    }

    /// A closed task retains its first-touch place in the reply.
    pub(crate) fn closed_task(&self, task: &TaskRecord) {
        self.lock_tasks().close(task, self.task_id());
    }

    /// The latest pick, initially the host's task or none. It may have since disappeared.
    pub(crate) fn current_task(&self) -> Option<TaskId> {
        self.lock_tasks().current().cloned()
    }

    /// First-touch task metadata, completed with live tabs when the reply is built.
    pub(super) fn task_notes(&self) -> Vec<TaskNote> {
        self.lock_tasks().notes()
    }

    fn lock_tasks(&self) -> MutexGuard<'_, RunTasks> {
        self.tasks.lock().unwrap_or_else(PoisonError::into_inner)
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

    /// The run's default page, resolved once on first use through the door.
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

    /// A task pick hands the lazy page an open tab of the task, so built-in `page` is that tab
    /// from the pick's reply on instead of from its first action. True only when this call
    /// bound it: a known page, one already resolved, and one an op is opening right now each
    /// keep theirs, because `page` holds the first tab it gets for the whole run (its listeners
    /// and the prelude's `Page` object are that tab's).
    pub(super) fn bind_default_page(&self, page: &PageRef) -> bool {
        match &self.default_page {
            DefaultPage::Known(_) => false,
            DefaultPage::Lazy(cell) => cell.set(page.clone()).is_ok(),
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
    pub(super) fn record_closed(&self, page: TabId) {
        self.lock_ledger().closed_by_run.insert(page);
    }

    /// Whether this run closed `page`, which decides who owns `page_closed`, and keeps the tab
    /// out of the run's later tab listings.
    pub(crate) fn closed_it(&self, page: TabId) -> bool {
        self.lock_ledger().closed_by_run.contains(&page)
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
    /// Records the run's end, which the run service builds from the run's reply. Teardown drains
    /// the run's ops first, so their `op_ended` records precede it; the trace turns any op still
    /// open at a run's end into `unknown`.
    pub(crate) fn finish(mut self, end: RunEnd) {
        if let Some(run) = self.run.take() {
            run.end_slots();
            self.tabs.run_ended(run.id);
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
        self.tabs.run_ended(run.id);
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
    use neo_trace::ErrorOwner;

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
