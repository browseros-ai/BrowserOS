//! What a method gets: its op, its run, its checked target, and the door's services.

use std::future::Future;
use std::marker::PhantomData;
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use browseros_cdp::CdpError;
use browseros_core::viewport::ViewportLease;
use browseros_core::{CoreError, PageId, TabId, TargetError};
use browseros_trace::{EpochMillis, OperationId};
use serde_json::Value;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{
    Caller, Dispatcher, ErrorCode, LeafError, MethodSpec, OpFailure, OperationError, PolicyCheck,
    RunCtx, StopReason,
};
use crate::browser::PageRef;

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
    pub(super) spec: MethodSpec,
    pub(super) caller: Caller,
    /// A child of the run's token, so a crate given it cannot cancel the whole run.
    pub(super) cancel: CancellationToken,
}

/// A change a method made to a page, reported so the door does the bookkeeping once, for
/// every method: agent-opened marks, fresh facts after a navigation, closes this run made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageChange {
    /// The method opened this tab for the agent. The idle-tab janitor may close it later
    /// (core has no public `is_agent_created`), and the reply reports it.
    Opened,
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
    sensitive: AtomicBool,
    /// The latest `recheck` refusal, until a later recheck passes. If the method then fails in
    /// any way, this is the op's error, so a refusal survives a crate's own error mapping
    /// (sdk-playwright returns its own marker type and never names the app's error).
    refusal: Mutex<Option<OperationError>>,
    /// Call-log lines. `OperationEnded` has no notes field yet, so the guard writes them to
    /// the log at the op's end; when sdk-traces adds the column, only the guard changes.
    notes: Mutex<Vec<String>>,
    reports: Mutex<Vec<PageReport>>,
    /// Pages whose viewport lease this op holds. Core's leases do not nest, so a child op
    /// inherits its ancestor's lease instead of taking it again and deadlocking.
    viewports: Mutex<Vec<PageId>>,
}

/// A page's viewport lease, held or inherited from an ancestor op. A method keeps it for as
/// long as it observes, types or navigates; dropping it releases the page.
pub(crate) struct PageViewport {
    /// `None` when an ancestor op (or this op, earlier) holds the lease. Kept for its drop.
    _held: Option<HeldViewport>,
}

struct HeldViewport {
    _lease: ViewportLease,
    page: PageId,
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

    /// The target turned out to be a password, card or one-time-code field: the trace redacts
    /// the method's sensitive input fields.
    pub(crate) fn mark_sensitive_target(&self) {
        self.core.state.sensitive.store(true, Ordering::Relaxed);
    }

    /// One call-log line for the op's record. The first lines are kept; a runaway method
    /// cannot grow its op without bound.
    pub(crate) fn note(&self, line: String) {
        let mut notes = lock(&self.core.state.notes);
        if notes.len() < NOTES_PER_OP {
            notes.push(line);
        }
    }

    /// Reports a page this op opened, navigated or closed.
    pub(crate) fn report_page(&self, page: &PageRef, change: PageChange) {
        lock(&self.core.state.reports).push(PageReport {
            page: page.clone(),
            change,
        });
    }

    /// Per-tab feature state for the target (sdk-playwright's facade, sdk-privacy's journal),
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
        if self.core.state.holds_viewport(&page.id) {
            return Ok(PageViewport { _held: None });
        }
        let lease = self
            .race(self.core.run.view().prepare_viewport(page.id.clone()))
            .await?;
        lock(&self.core.state.viewports).push(page.id.clone());
        Ok(PageViewport {
            _held: Some(HeldViewport {
                _lease: lease,
                page: page.id.clone(),
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
    /// A fresh op under `parent`.
    pub(super) fn new(id: OperationId, parent: Option<Arc<OpState>>) -> Arc<Self> {
        Arc::new(Self {
            id,
            parent,
            sent_at: OnceLock::new(),
            sensitive: AtomicBool::new(false),
            refusal: Mutex::new(None),
            notes: Mutex::new(Vec::new()),
            reports: Mutex::new(Vec::new()),
            viewports: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn id(&self) -> OperationId {
        self.id
    }

    pub(super) fn parent_id(&self) -> Option<OperationId> {
        self.parent.as_ref().map(|parent| parent.id)
    }

    /// When the op, or one of its children, marked itself sent.
    pub(super) fn sent_at(&self) -> Option<EpochMillis> {
        self.sent_at.get().copied()
    }

    pub(super) fn is_sensitive(&self) -> bool {
        self.sensitive.load(Ordering::Relaxed)
    }

    /// The standing `recheck` refusal, if the latest recheck refused.
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

    fn holds_viewport(&self, page: &PageId) -> bool {
        let mut op = Some(self);
        while let Some(state) = op {
            if lock(&state.viewports).contains(page) {
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
        if let Some(index) = viewports.iter().position(|page| *page == self.page) {
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
