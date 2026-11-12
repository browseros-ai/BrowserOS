//! What a method gets: its op, its run, its checked target, and the door's services.

use std::future::{self, Future};
use std::marker::PhantomData;
use std::mem;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use browseros_cdp::CdpError;
use browseros_core::viewport::ViewportLease;
use browseros_core::{CoreError, PageId, TabId, TargetError, TargetId};
use neo_trace::{EpochMillis, OperationId};
use serde_json::Value;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{
    Caller, Dispatcher, ErrorCode, LeafError, MethodSpec, OpAbort, OpFailure, OperationError,
    PolicyCheck, RunCtx, StopReason,
};
use crate::browser::{PageRef, TabRegistry};

/// How many call-log lines one op keeps; later lines are dropped.
const NOTES_PER_OP: usize = 64;

/// What a method gets for one op: its id, its run, its checked target, and the door's
/// services. sdk-playwright implements its `OpEnv` over it.
///
/// Minted only by the door ([`OpCtx::mint`] is private to `dispatch/`) and typed by the method
/// it was minted for, so a method holding `&OpCtx<Close>` cannot run `Title::call`, which
/// wants `&OpCtx<Title>`. The one way to another action is [`OpCtx::invoke`], a child op.
/// Methods get it by reference for the length of their call.
pub(crate) struct OpCtx<M> {
    core: OpCore,
    method: PhantomData<fn() -> M>,
}

/// The untyped part the door builds for every op. Private to `dispatch/`.
pub(super) struct OpCore {
    pub(super) run: Arc<RunCtx>,
    pub(super) dispatcher: Arc<Dispatcher>,
    pub(super) state: Arc<OpState>,
    pub(super) page: Option<PageRef>,
    /// Private core adapter for this operation; never retained in a tab registry or wire value.
    pub(super) core_page: Option<PageId>,
    pub(super) spec: MethodSpec,
    pub(super) caller: Caller,
    /// A child of the run's token, so a crate given it cannot cancel the whole run.
    pub(super) cancel: CancellationToken,
}

/// A page a method changed or observed, reported so the door does bookkeeping and computes
/// fresh facts once for agent-opened tabs, navigations, closes and delivered events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PageChange {
    /// The method opened this tab for the agent. The idle-tab janitor may close it later
    /// (core has no public `is_agent_created`), and the reply reports it.
    Opened,
    /// The result names this tab; report its current fact without changing ownership.
    Linked,
    /// A newly observed popup carries its opener's ownership before its first fact is sent.
    Popup { opener: PageRef },
    /// A delivered close names this tab's destroyed target. It contributes reply facts
    /// without changing ownership or treating the event as a close performed by the script.
    Gone { target: TargetId, seq: u64 },
    /// The release policy withheld this close record; retain its internal dead evidence.
    WithheldClose { seq: u64 },
    /// The binding that produced this operation's page payload.
    Acted { target: TargetId },
    /// The method changed the page's URL. Core's cache is stale after an unbound navigation,
    /// so the door refreshes the page once (one `Browser.getTabInfo`) before reading facts.
    Navigated,
    /// The method closed the tab. A later use of it is the script's mistake.
    Closed,
}

/// One reported change. A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PageReport {
    pub(super) page: PageRef,
    pub(super) change: PageChange,
}

/// The facts an op reports while it runs, shared by its `OpCtx`, its guard and its children.
///
/// The method and its children only report facts here; nothing here is a decision. The guard
/// reads them once, when the op ends, and decides there, so concurrent reporters never race on
/// an outcome. The sent flag is write-once, so a late or repeated mark cannot move it.
pub(super) struct OpState {
    id: OperationId,
    parent: Option<Arc<OpState>>,
    /// When the op, or one of its children, marked itself sent. Write-once: the first mark
    /// wins, and nothing unsets it.
    sent_at: OnceLock<EpochMillis>,
    /// The latest `recheck` refusal, until a later `recheck` passes. However the method then
    /// ends, a value included, this is the op's error, so a refusal survives a crate's own error
    /// mapping (sdk-playwright returns its own marker type and never names the app's error).
    /// Always the public form the agent may see.
    refusal: Mutex<Option<OperationError>>,
    /// Call-log lines. `OperationEnded` has no notes field yet, so the guard writes them to
    /// the log at the op's end; when sdk-traces adds the column, only the guard changes.
    notes: Mutex<Vec<String>>,
    reports: Mutex<Vec<PageReport>>,
    /// Pages whose viewport lease this op holds. Core's leases do not nest, so a child op
    /// inherits its ancestor's lease instead of taking it again and deadlocking.
    viewports: Mutex<Vec<TabId>>,
    /// The script's abort switch for this op (Playwright's `signal`), or its parent's.
    abort: Option<OpAbort>,
    /// How many event-pump replies the script must have dispatched before it takes this op's
    /// answer ([`OpCtx::answer_after_pump`]); the reply carries it as `pumped`.
    after_pump: AtomicU64,
    /// Run-local completion identities reported by a method, carried as reply metadata only.
    completed_publications: Mutex<Vec<u64>>,
    /// The run's lazy page, when this op bound it ([`OpCtx::bind_run_page`]). The reply names
    /// it as `defaultPage`, as it does for an op that opened the page.
    bound_page: OnceLock<PageRef>,
}

/// A page's viewport lease, held or inherited from an ancestor op. A method keeps it for as
/// long as it observes, types or navigates; dropping it releases the page.
pub(crate) struct PageViewport {
    /// `None` when an ancestor op (or this op, earlier) holds the lease. Kept for its drop.
    _held: Option<HeldViewport>,
}

struct HeldViewport {
    _lease: ViewportLease,
    tab: TabId,
    op: Arc<OpState>,
}

impl<M> OpCtx<M> {
    /// Called by the registry's `Decoded::run` only.
    pub(super) fn mint(core: OpCore) -> Self {
        Self {
            core,
            method: PhantomData,
        }
    }

    /// This op's id, `{run}.{seq}`.
    #[must_use]
    pub(crate) fn id(&self) -> OperationId {
        self.core.state.id
    }

    /// The run this op belongs to: its view, its task, its budget.
    #[must_use]
    pub(crate) fn run(&self) -> &RunCtx {
        &self.core.run
    }

    /// Task records share the tab registry's close lifecycle.
    pub(crate) fn tabs(&self) -> &TabRegistry {
        self.core.dispatcher.tabs()
    }

    /// The target, resolved and checked by the door. A method that declared no target gets
    /// `neo / internal_error`: that is a bug in the method, not in the script.
    pub(crate) fn page(&self) -> Result<&PageRef, LeafError> {
        self.core.page.as_ref().ok_or_else(|| {
            LeafError::definite(OperationError::neo(
                ErrorCode::INTERNAL_ERROR,
                format_args!("{} declares no target page", self.core.spec.name),
            ))
        })
    }

    /// Core's private page key, resolved alongside the tab when this operation was admitted.
    pub(crate) fn core_page(&self) -> Result<&PageId, LeafError> {
        self.core.core_page.as_ref().ok_or_else(|| {
            LeafError::definite(OperationError::neo(
                ErrorCode::INTERNAL_ERROR,
                "this operation has no admitted core page",
            ))
        })
    }

    /// The run's gates and the rules again. Call it right before `mark_sent` (STYLE.md P6): an
    /// action can wait seconds for actionability, and the owner can take over meanwhile.
    pub(crate) fn recheck(&self) -> Result<(), LeafError> {
        match self.core.dispatcher.recheck(&self.core) {
            Ok(()) => {
                self.core.state.set_refusal(None);
                Ok(())
            }
            Err(refusal) => {
                self.core.state.set_refusal(Some(refusal.clone()));
                Err(LeafError::definite(refusal))
            }
        }
    }

    /// The door's rules for this op as if it acted on `tab`: for a result that carries another
    /// tab's data, such as an event pull, which hands back what every page the run listens to
    /// did. Unlike [`OpCtx::recheck`], a refusal here is not the op's standing error: the method
    /// decides what becomes of that tab's data.
    pub(crate) fn check_tab(&self, tab: TabId) -> Result<(), OperationError> {
        let check = PolicyCheck {
            tab: Some(tab),
            ..self.core.policy_check()
        };
        self.core.dispatcher.check(&check)
    }

    /// The side effect may happen from now on. Write-once, and it marks every ancestor too:
    /// a parent's effect happened through its child.
    pub(crate) fn mark_sent(&self) {
        self.core.state.mark_sent(EpochMillis::now());
    }

    /// One call-log line for the op's record. The first lines are kept; a runaway method
    /// cannot grow its op without bound.
    pub(crate) fn note(&self, line: String) {
        let mut notes = lock(&self.core.state.notes);
        if notes.len() < NOTES_PER_OP {
            notes.push(line);
        }
    }

    /// The script takes this op's answer only once it has dispatched `replies` replies of the
    /// run's event pump, so the listeners for events that came before the op ran first, as
    /// upstream's have (sdk-playwright counts them, `RunState::pump_replies`). The door only
    /// carries the count: the prelude holds the answer (its pump ledger), since the host settles
    /// each reply's promise on its own and in no fixed order. Raised, never lowered.
    pub(crate) fn answer_after_pump(&self, replies: u64) {
        self.core
            .state
            .after_pump
            .fetch_max(replies, Ordering::AcqRel);
    }

    /// Carry a method's completion facts to the runtime without letting runtime reach its
    /// browser implementation. These identities affect publication ordering, never delivery.
    pub(crate) fn report_publications(&self, completed: Vec<u64>) {
        *lock(&self.core.state.completed_publications) = completed;
    }

    /// Makes `page` the run's built-in page if nothing gave it a tab yet
    /// ([`RunCtx::bind_default_page`]); the reply then names it as `defaultPage`, which binds the
    /// prelude's `page` object before the script's next statement. The page's URL and title reach
    /// the script only as a reported page's fact (`report_page`), past the read rules like
    /// every reply's, so a tab whose reads are blocked binds with nothing to read.
    pub(crate) fn bind_run_page(&self, page: &PageRef) {
        if self.core.run.bind_default_page(page) {
            let _ = self.core.state.bound_page.set(page.clone());
        }
    }

    /// Reports a page this op opened, navigated or closed.
    pub(crate) fn report_page(&self, page: &PageRef, change: PageChange) {
        lock(&self.core.state.reports).push(PageReport {
            page: page.clone(),
            change,
        });
    }

    /// Per-tab feature state for the target (sdk-playwright's facade),
    /// dropped when the tab closes.
    pub(crate) fn tab_slot<T: Default + Send + Sync + 'static>(&self) -> Result<Arc<T>, LeafError> {
        let page = self.page()?;
        self.core.dispatcher.tabs().slot::<T>(page).ok_or_else(|| {
            LeafError::definite(OperationError::neo(
                ErrorCode::PAGE_CLOSED,
                "the page closed",
            ))
        })
    }

    /// The reason the script gave when it aborted this op (Playwright's `signal`), once it has.
    #[must_use]
    pub(crate) fn abort_reason(&self) -> Option<String> {
        self.core.state.abort()?.reason().map(str::to_owned)
    }

    /// Resolves once the script aborts this op; never, for an op it cannot abort. The method
    /// decides what an abort ends (`op_abort.rs`): the door does not cut the op for it.
    pub(crate) async fn aborted(&self) {
        match self.core.state.abort() {
            Some(abort) => abort.requested().await,
            None => future::pending().await,
        }
    }

    /// Fires when the run stops. Crates that take a token (the CAPTCHA solver) get a clone.
    #[must_use]
    pub(crate) fn cancel_token(&self) -> &CancellationToken {
        &self.core.cancel
    }

    /// When the run's budget ends, for methods with their own wait loops (auto-wait).
    #[must_use]
    pub(crate) fn deadline(&self) -> Instant {
        self.core.run.deadline()
    }

    /// The error for "the run stopped under this op", owned by whoever stopped it, and
    /// ambiguous because a sent write may have landed. For crates that report their own
    /// cancel (the solver's `CallerCancelled`).
    #[must_use]
    pub(crate) fn stopped(&self) -> LeafError {
        let error = self.core.run.stop_reason().map_or_else(
            || OperationError::neo(ErrorCode::CANCELLED, "the operation was cancelled"),
            StopReason::error,
        );
        LeafError::ambiguous(error)
    }

    /// Awaits one browser call (STYLE.md P7) and classifies core's error once, here: a lost or
    /// replaced connection and a CDP timeout may have delivered (`unknown` if sent), a closed
    /// page or a protocol error is the browser's answer (`done` if sent), anything else is
    /// left unclassified, which the door reads as ambiguous once sent.
    pub(crate) async fn race<T>(
        &self,
        work: impl Future<Output = Result<T, CoreError>>,
    ) -> Result<T, LeafError> {
        self.race_answer(work).await?.map_err(classify_answer)
    }

    /// Like [`OpCtx::race`], but hands the browser's own error back for the method to judge
    /// (a failed navigation is `website / navigation_failed`, definite). The outer error is
    /// the door's: the run's stop, a lost or replaced connection, a CDP timeout.
    pub(crate) async fn race_answer<T>(
        &self,
        work: impl Future<Output = Result<T, CoreError>>,
    ) -> Result<Result<T, CoreError>, LeafError> {
        match self.race_classified(work).await? {
            Err(
                error @ (CoreError::Cdp(
                    CdpError::ConnectionLost
                    | CdpError::NotConnected
                    | CdpError::ConnectionEpochChanged { .. },
                )
                | CoreError::Target(TargetError::DefaultProfileUnavailable(_))),
            ) => Err(LeafError::ambiguous(OperationError::neo(
                ErrorCode::BROWSER_DISCONNECTED,
                error,
            ))),
            Err(error @ CoreError::Cdp(CdpError::Timeout { .. })) => Err(LeafError::ambiguous(
                OperationError::neo(ErrorCode::CDP_TIMEOUT, error),
            )),
            answer => Ok(answer),
        }
    }

    /// Awaits work that classifies its own failures, such as a crate's whole operation
    /// (sdk-playwright's `Playwright::invoke`): raced with the run's stop and refused when the
    /// connection was replaced under it, as [`OpCtx::race`] is, and otherwise handed back as it
    /// came, its own errors included.
    pub(crate) async fn race_classified<T>(
        &self,
        work: impl Future<Output = T>,
    ) -> Result<T, LeafError> {
        // Biased: when both are ready the stop wins, so an answer that arrives in the same
        // instant as a cancel is never read as definite.
        let result = tokio::select! {
            biased;
            () = self.core.cancel.cancelled() => return Err(self.stopped()),
            result = work => result,
        };
        // An unbound view follows a reconnect silently, so a call that straddled one may have
        // acted through another socket, even when it returned.
        if self.core.dispatcher.browser().epoch() != Some(self.core.run.epoch()) {
            return Err(LeafError::ambiguous(OperationError::neo(
                ErrorCode::BROWSER_DISCONNECTED,
                "the browser connection was replaced during this call",
            )));
        }
        Ok(result)
    }

    /// The page's viewport lease, for methods that observe, type or navigate. Inherited, not
    /// taken again, when this op or an ancestor already holds it: core's leases do not nest,
    /// so a child taking its parent's page lease would deadlock. Raced like any wait.
    pub(crate) async fn viewport(&self, page: &PageRef) -> Result<PageViewport, LeafError> {
        if self.core.state.holds_viewport(page.tab) {
            return Ok(PageViewport { _held: None });
        }
        let lease = self
            .race(
                self.core
                    .run
                    .view()
                    .prepare_viewport(self.core_page()?.clone()),
            )
            .await?;
        lock(&self.core.state.viewports).push(page.tab);
        Ok(PageViewport {
            _held: Some(HeldViewport {
                _lease: lease,
                tab: page.tab,
                op: Arc::clone(&self.core.state),
            }),
        })
    }

    /// Another action, as a child op: its own record, rules and delivery, under this op. If
    /// the child sends, this op counts as sent too.
    pub(crate) async fn invoke(&self, name: &str, input: Value) -> Result<Value, OpFailure> {
        self.core
            .dispatcher
            .invoke_child(&self.core.run, Some(&self.core.state), name, input)
            .await
    }
}

impl OpCore {
    /// The rules' input for this op: the same tab its trace records.
    pub(super) fn policy_check(&self) -> PolicyCheck<'_> {
        PolicyCheck {
            method: &self.spec,
            caller: self.caller,
            tab: self.page.as_ref().map(|page| page.tab),
            operation: self.state.id,
        }
    }
}

impl OpState {
    /// A fresh op under `parent`, which the script can abort through `abort`.
    pub(super) fn new(
        id: OperationId,
        parent: Option<Arc<OpState>>,
        abort: Option<OpAbort>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            parent,
            abort,
            sent_at: OnceLock::new(),
            refusal: Mutex::new(None),
            notes: Mutex::new(Vec::new()),
            reports: Mutex::new(Vec::new()),
            viewports: Mutex::new(Vec::new()),
            after_pump: AtomicU64::new(0),
            completed_publications: Mutex::default(),
            bound_page: OnceLock::new(),
        })
    }

    /// The run's page this op bound, if it did.
    pub(super) fn bound_page(&self) -> Option<PageRef> {
        self.bound_page.get().cloned()
    }

    pub(super) fn id(&self) -> OperationId {
        self.id
    }

    pub(super) fn parent_id(&self) -> Option<OperationId> {
        self.parent.as_ref().map(|parent| parent.id)
    }

    /// The op's abort switch, which its child ops share.
    pub(super) fn abort(&self) -> Option<&OpAbort> {
        self.abort.as_ref()
    }

    /// When the op, or one of its children, marked itself sent.
    pub(super) fn sent_at(&self) -> Option<EpochMillis> {
        self.sent_at.get().copied()
    }

    pub(super) fn take_publications(&self) -> Vec<u64> {
        mem::take(&mut *lock(&self.completed_publications))
    }

    /// What [`OpCtx::answer_after_pump`] last raised it to, 0 when nothing did.
    pub(super) fn after_pump(&self) -> u64 {
        self.after_pump.load(Ordering::Acquire)
    }

    /// The refusal standing as the op's error, if any.
    pub(super) fn refusal(&self) -> Option<OperationError> {
        lock(&self.refusal).clone()
    }

    pub(super) fn take_notes(&self) -> Vec<String> {
        mem::take(&mut *lock(&self.notes))
    }

    pub(super) fn take_reports(&self) -> Vec<PageReport> {
        mem::take(&mut *lock(&self.reports))
    }

    fn mark_sent(&self, at: EpochMillis) {
        let mut op = Some(self);
        while let Some(state) = op {
            let _ = state.sent_at.set(at);
            op = state.parent.as_deref();
        }
    }

    fn set_refusal(&self, refusal: Option<OperationError>) {
        *lock(&self.refusal) = refusal;
    }

    fn holds_viewport(&self, page: TabId) -> bool {
        let mut op = Some(self);
        while let Some(state) = op {
            if lock(&state.viewports).contains(&page) {
                return true;
            }
            op = state.parent.as_deref();
        }
        false
    }
}

impl Drop for HeldViewport {
    // Runs before the lease field drops, so the op stops claiming the page first; both happen
    // synchronously on this task, so no child can slip in between.
    fn drop(&mut self) {
        let mut viewports = lock(&self.op.viewports);
        if let Some(index) = viewports.iter().position(|page| *page == self.tab) {
            viewports.swap_remove(index);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Core errors that are the browser's own answer: no lost connection, no timeout.
fn classify_answer(error: CoreError) -> LeafError {
    match error {
        CoreError::UnknownPage(_) | CoreError::UnknownPageShort(_) => {
            LeafError::definite(OperationError::neo(ErrorCode::PAGE_CLOSED, error))
        }
        CoreError::Cdp(CdpError::Protocol { .. }) => {
            LeafError::definite(OperationError::neo(ErrorCode::CDP_ERROR, error))
        }
        other => LeafError::from(OperationError::neo(ErrorCode::INTERNAL_ERROR, other)),
    }
}
