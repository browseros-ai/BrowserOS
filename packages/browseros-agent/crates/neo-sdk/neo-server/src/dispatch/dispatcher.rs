//! The door.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::browser::Seen;
use browseros_core::pages::PageInfo;
use browseros_core::viewport::Size;
use browseros_core::{BrowserView, PageId, TabId};
use neo_trace::{Delivery, Effect, OperationId, PageState, TaskId, TraceSink};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex as AsyncMutex, oneshot};
use tokio::time;
use ts_rs::TS;

use super::close_release::close_release;
use super::op_ctx::{OpCore, OpState, PageChange, PageReport};
use super::operation_guard::{Ending, OpStart, OperationGuard, StartTab};
use super::page_target::TargetHandle;
use super::refusal_brand::RefusalBrand;
use super::registry::DecodedCall;
use super::{
    ActedTarget, Answer, Claim, ErrorCode, LeafError, MethodSpec, Observation, Observer, OpAbort,
    OpFailure, OperationError, PageScript, PageTarget, Policy, PolicyCheck, Registry, RunCtx,
    TabNote, TaskKind, TaskNote, Visibility, operation_error,
};
use crate::browser::{BrowserLink, PageRef, TabRegistry, TaskRecord};
use crate::config::Defaults;

/// A tab's refresh bookkeeping, a per-tab slot dropped with the tab: the event seq through which
/// the door last refreshed its entry in core's page cache, so a navigation the page made itself
/// is refreshed once (`Dispatcher::refresh_if_navigated`), and the turn its refreshes take.
#[derive(Debug, Default)]
struct RefreshedThrough {
    through: AtomicU64,
    /// Core keeps the entry another refresh wrote while one was in flight and drops that one's
    /// read (`refresh_with_epoch`), so two overlapping refreshes could leave the older read under
    /// the newer mark. The door's refreshes of a tab take turns, each checking the mark again.
    turn: AsyncMutex<()>,
}

/// The method the door calls, as Neo, to open a run's lazy default page: with `{}`, returning
/// Playwright's page entry `{ page, url }` (`OpenedPage`) after reporting the page
/// `PageChange::Opened`.
const DEFAULT_PAGE_OPENER: &str = "context.newPage";
/// How many characters of a method name its refusal keeps: a script can send any string. The
/// trace writer clips the name its record keeps.
const NAME_CHARS: usize = 128;
/// The spec recorded for a name with no method. `Write`, so an unknown call is never recorded
/// as a harmless read.
const UNKNOWN_METHOD: MethodSpec = MethodSpec::new("unknown", Effect::Write);

/// The one door every SDK operation walks through (skeleton D7). Built once at startup and
/// shared by every run.
///
/// ```text
///  submit ─ admit (sync): seq, name → method, decode, op_started ─┐ refused: answered now
///          └▶ task on the run's JoinSet ─ run_op:                 ▼
///               gates ─ target (cached, tab checked) ─ rules ─
///               Method::call raced with the stop ─ reads checked again ─
///               page facts ─ OperationGuard::finish
///  OpCtx::invoke ─ the same admit and run_op, inline in the parent's task
/// ```
///
/// Everything up to the start record is synchronous, so every op id handed out is recorded
/// and answered even if its task never runs. An op on the run's lazy page records its start
/// once the door resolved the page, before the rules, and its guard records it on any path that
/// ends the op sooner.
pub(crate) struct Dispatcher {
    registry: Registry,
    /// Fixed at startup, in the one order the app writes down (see [`Policy`]).
    policies: Vec<Arc<dyn Policy>>,
    trace: TraceSink,
    browser: Arc<BrowserLink>,
    tabs: Arc<TabRegistry>,
    observer: Observer,
}

/// Who is calling. Only Neo's own calls may name `Internal` methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Caller {
    /// The agent's script, through the bridge.
    Script,
    /// Neo's own Rust: a method's child op, the lazy default page, the idle-tab janitor.
    Neo,
}

/// One call as it arrives at the door. A plain data bag.
#[derive(Debug)]
pub(crate) struct Call {
    /// The method name as the caller wrote it.
    pub(crate) name: String,
    /// The input, decoded by the door into the method's type.
    pub(crate) input: Value,
    /// Who is calling.
    pub(crate) caller: Caller,
    /// Where in the script the call was made.
    pub(crate) site: Option<CallSite>,
    /// The script's switch for aborting this op (Playwright's `signal`), when it can. A child
    /// op takes its parent's, so an abort reaches whichever of them is waiting.
    pub(crate) abort: Option<OpAbort>,
}

/// Where in the script a call was made (`run:LINE:COL`), for reply lines and persisted
/// operation records. A plain data bag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CallSite {
    /// The line in the script, from 1.
    pub(crate) line: u32,
    /// The column, from 1.
    pub(crate) column: u32,
}

/// What the door hands back: the outcome, and fresh facts about the
/// pages the op touched, read from core's cache so the prelude's sync accessors
/// (`page.url()`) stay current without a round trip. A plain data bag.
#[derive(Debug)]
pub(crate) struct OpReply {
    /// The value, or the failure with its owner and delivery.
    pub(crate) outcome: Result<Value, OpFailure>,
    /// The target first, then the pages the op reported.
    pub(crate) pages: Vec<PageFact>,
    /// Orders this fact batch, absent when the operation completed no observation.
    pub(crate) observed: Option<Observation>,
    /// Set when the op resolved the run's default page, so the script's `page` learns its id.
    pub(crate) default_page: Option<PageRef>,
    /// Set when the op bound the run's lazy page to a tab without acting on it (`neo.task`'s
    /// pick, `OpCtx::bind_run_page`). Unlike `default_page` it is an offer the prelude may
    /// decline: it binds the script's `page` now only if no other Page object for that tab has
    /// reached the script, and otherwise leaves `page` to that tab's `default_page` at its first
    /// action, so a handle the script holds keeps its listeners and bindings.
    pub(crate) picked_page: Option<PageRef>,
    /// The event-pump replies the script dispatches before it takes this one
    /// ([`OpCtx::answer_after_pump`]); 0 for an op that set none.
    pub(crate) pumped: u64,
    /// Completion facts supplied by the registered method, opaque to the door and runtime.
    pub(crate) completed_publications: Vec<u64>,
}

/// One page as a reply reports it. Deliberately not a page brand (no `$neo`), so the
/// bootstrap never revives a fact into a `Page` object. Its viewport configuration initializes
/// synchronous accessors in each new agent realm, independently of the owner's temporary fit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub(crate) struct PageFact {
    /// The browser's tab id.
    pub(crate) tab: TabId,
    /// The owning task now; task-less tabs serialize as null.
    #[serde(rename = "taskId")]
    pub(crate) task_id: Option<String>,
    /// `None` (null) once closed.
    pub(crate) url: Option<String>,
    /// `None` (null) once closed.
    pub(crate) title: Option<String>,
    /// Whether the tab is gone.
    pub(crate) closed: bool,
    /// Script configuration, carried to each fresh realm; owner fits never replace it.
    #[serde(rename = "viewportSize", skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) viewport_size: Option<Size>,
    /// Captured under the same observation serial as the page cache read.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) workers: Option<Vec<super::observation::WorkerFact>>,
}

/// The answer to a submitted call. It always arrives: once the call has an op id, its guard
/// answers on every path, panics and aborted tasks included.
#[derive(Debug)]
pub(crate) struct PendingReply {
    operation: OperationId,
    reply: oneshot::Receiver<OpReply>,
}

/// An op past its start record whose input decoded: what the door's async part needs.
struct Admitted {
    guard: OperationGuard,
    decoded: Box<dyn DecodedCall>,
    state: Arc<OpState>,
    spec: MethodSpec,
    caller: Caller,
    /// Whether the script itself made the call: the bootstrap names the script line it came
    /// from. The prelude's own work (its frame refreshes, listeners and event pulls) comes with
    /// an explicit empty site (bootstrap.js `invoke`, call-site.ts `INTERNAL_SITE`), so it is
    /// never the task acting on a tab.
    scripted: bool,
    target: Option<PageTarget>,
}

/// What [`DEFAULT_PAGE_OPENER`] returns, as far as the door reads it: the brand of the tab it
/// opened. Its `url` is the prelude's, so unknown fields pass.
#[derive(Deserialize)]
struct OpenedPage {
    page: PageTarget,
}

/// The associated identities and liveness ticket from one core-cache admission.
struct ResolvedPage {
    page: PageRef,
    /// Core identity is an op-local adapter, never part of a brand or retained tab state.
    core_page: PageId,
    seen: Seen,
}

/// An op's target after the door checked it.
struct Resolved {
    target: Option<ResolvedPage>,
    /// The run's default page, when the target was it.
    default_page: Option<PageRef>,
}

impl Call {
    /// A call from the script.
    #[must_use]
    pub(crate) fn script(name: String, input: Value, site: Option<CallSite>) -> Self {
        Self {
            name,
            input,
            caller: Caller::Script,
            site,
            abort: None,
        }
    }

    /// The same call, which the script can abort through `abort`.
    #[must_use]
    pub(crate) fn abortable(self, abort: OpAbort) -> Self {
        Self {
            abort: Some(abort),
            ..self
        }
    }
}

impl Caller {
    fn may_call(self, visibility: Visibility) -> bool {
        match (self, visibility) {
            (Self::Script, Visibility::Script)
            | (Self::Neo, Visibility::Script | Visibility::Internal) => true,
            (Self::Script, Visibility::Internal) => false,
        }
    }
}

impl CallSite {
    /// Reads the bootstrap's `run:LINE:COL`. Anything else is no site.
    #[must_use]
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (line, column) = text.strip_prefix("run:")?.split_once(':')?;
        Some(Self {
            line: line.parse().ok()?,
            column: column.parse().ok()?,
        })
    }
}

impl fmt::Display for CallSite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "run:{}:{}", self.line, self.column)
    }
}

impl Dispatcher {
    /// Builds the door. `policies` is in its fixed order.
    #[must_use]
    pub(crate) fn new(
        registry: Registry,
        policies: Vec<Arc<dyn Policy>>,
        trace: TraceSink,
        browser: Arc<BrowserLink>,
        tabs: Arc<TabRegistry>,
        observer: Observer,
    ) -> Self {
        Self {
            registry,
            policies,
            trace,
            browser,
            tabs,
            observer,
        }
    }

    /// The methods, for the `run` description.
    #[must_use]
    pub(crate) fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Shares one observation order between operation replies and run initialization.
    pub(crate) fn observer(&self) -> &Observer {
        &self.observer
    }

    /// The trace writer every run and op records through.
    pub(super) fn trace(&self) -> &TraceSink {
        &self.trace
    }

    /// A call from the script. The op id is taken and the start recorded now, in the script's
    /// call order; the rest runs as a task on the run's `JoinSet`, so a slow op never holds
    /// the script's runtime and a panicking method stays inside its own task.
    pub(crate) fn submit(self: &Arc<Self>, run: &Arc<RunCtx>, call: Call) -> PendingReply {
        let (pending, admitted) = self.admit(run, call, None);
        if let Some(admitted) = admitted {
            run.spawn_op(Arc::clone(self).run_op(Arc::clone(run), admitted));
        }
        pending
    }

    /// A child op, run inline inside its parent's task: the same door, Neo as the caller.
    /// `parent` is `None` for Neo's own top-level ops (the lazy default page, the janitor).
    pub(super) async fn invoke_child(
        self: &Arc<Self>,
        run: &Arc<RunCtx>,
        parent: Option<&Arc<OpState>>,
        name: &str,
        input: Value,
    ) -> Result<Value, OpFailure> {
        let call = Call {
            name: name.to_owned(),
            input,
            caller: Caller::Neo,
            site: None,
            abort: None,
        };
        let (pending, admitted) = self.admit(run, call, parent.map(Arc::clone));
        if let Some(admitted) = admitted {
            // Boxed: the door's future nests here once per level of child ops.
            Box::pin(Arc::clone(self).run_op(Arc::clone(run), admitted)).await;
        }
        pending.wait().await.outcome
    }

    /// Resolves the lazy page in the current task, opening one if needed. The opener is a Neo
    /// op with no parent, so its send never counts toward the op that first touched `page`.
    pub(super) async fn open_default_page(
        self: &Arc<Self>,
        run: &Arc<RunCtx>,
    ) -> Result<PageRef, OperationError> {
        let task = run.current_task().and_then(|id| self.tabs.task(&id).ok());
        if let Some(tab) = task
            .as_ref()
            .and_then(|task| task.page_tab(|tab| !run.closed_it(tab) && self.tabs.is_open(tab)))
        {
            return Ok(PageRef { tab });
        }
        let opened = self
            .invoke_child(run, None, DEFAULT_PAGE_OPENER, json!({}))
            .await
            .map_err(|failure| failure.error)?;
        match serde_json::from_value::<OpenedPage>(opened) {
            Ok(OpenedPage {
                page: PageTarget(TargetHandle::Page { tab }),
            }) => {
                let page = PageRef { tab };
                if let Some(task) = task {
                    // A concurrent close wins ownership, but cannot fail an already-open tab.
                    let _ = self.tabs.own(&page, &task.id);
                }
                Ok(page)
            }
            Ok(OpenedPage {
                page: PageTarget(TargetHandle::RunDefault),
            }) => Err(OperationError::neo(
                ErrorCode::INTERNAL_ERROR,
                format_args!("{DEFAULT_PAGE_OPENER} returned no page"),
            )),
            Err(error) => Err(OperationError::neo(
                ErrorCode::INTERNAL_ERROR,
                format_args!("{DEFAULT_PAGE_OPENER} returned no page: {error}"),
            )),
        }
    }

    /// `OpCtx::recheck`: the run's gates and the rules again, right before a write is sent.
    pub(super) fn recheck(&self, op: &OpCore) -> Result<(), OperationError> {
        if let Some(reason) = op.run.stop_reason() {
            return Err(reason.error());
        }
        if !self.browser.is_current(op.run.epoch()) {
            return Err(disconnected());
        }
        self.check(&op.policy_check())
    }

    pub(super) fn browser(&self) -> &BrowserLink {
        &self.browser
    }

    pub(super) fn tabs(&self) -> &Arc<TabRegistry> {
        &self.tabs
    }

    /// Gather current task tabs after the run's operations drained; saved replies never re-read them.
    pub(crate) async fn task_notes(&self, run: &RunCtx) -> Vec<TaskNote> {
        let mut notes = run.task_notes();
        if notes.is_empty() {
            return notes;
        }
        // The script has stopped, so its cancellation token is already cancelled. Bound this
        // final read by the teardown grace and keep cached URLs if the browser is unavailable.
        let _listing = time::timeout(Defaults::TEARDOWN_GRACE, run.view().list_pages()).await;
        for note in &mut notes {
            if note.kind == TaskKind::Closed {
                continue;
            }
            let Ok(id) = TaskId::parse(&note.id) else {
                continue;
            };
            let Ok(task) = self.tabs.task(&id) else {
                continue;
            };
            for tab in task.tabs {
                if let Some(info) = self.browser.page(tab).await {
                    note.tabs.push(TabNote {
                        tab: tab.0,
                        url: info.url,
                    });
                }
            }
        }
        notes
    }

    /// The synchronous front of the door: the op id, the method, the decoded input, and the
    /// start record. A call refused here (a script's refusal brand, an unknown or hidden name,
    /// bad input) is answered at once; anything else comes back to run.
    fn admit(
        &self,
        run: &Arc<RunCtx>,
        call: Call,
        parent: Option<Arc<OpState>>,
    ) -> (PendingReply, Option<Admitted>) {
        let Call {
            name,
            input,
            caller,
            site,
            abort,
        } = call;
        let abort = abort.or_else(|| parent.as_ref().and_then(|parent| parent.abort().cloned()));
        let state = OpState::new(run.next_operation(), parent, abort);
        let (reply, receiver) = oneshot::channel();
        let pending = PendingReply {
            operation: state.id(),
            reply: receiver,
        };
        let registered = self.registry.get(&name);
        let callable =
            registered.is_some_and(|registered| caller.may_call(registered.spec().visibility));
        // A refusal brand (FR/RC C4) is the prelude's refusal of a method Neo does not serve. It is
        // honoured from the script alone: Neo's own Rust never sends one, so a brand in a Neo
        // call's input is input like any other, script data a method passes on to a child op
        // included.
        let brand = match caller {
            Caller::Script => RefusalBrand::of(&input),
            Caller::Neo => None,
        };
        // An `Internal` name gets the same answer as a missing one, so a script cannot probe
        // for hidden methods; the record keeps the real spec. A brand is
        // decided before visibility is, so it gets one answer under every name, and it is never
        // decoded or run, its page never opened and no rule asked.
        //
        // A name the caller may not call is the script's own string. The start record gets it as
        // typed, and the trace writer clips it there. A refusal is the door's own text: the name
        // and a brand's reason in it are cut here, which bounds its size.
        let decoded = match (brand, registered) {
            (Some(brand), _) => {
                Err(brand.refusal(&operation_error::clip(name.clone(), NAME_CHARS)))
            }
            (None, Some(registered)) if callable => registered.decode(&input),
            (None, Some(_) | None) => {
                let shown = operation_error::clip(name.clone(), NAME_CHARS);
                Err(OperationError::script(
                    ErrorCode::METHOD_UNAVAILABLE,
                    format_args!("`{shown}` is not a Neo method"),
                ))
            }
        };
        let spec = registered.map_or(UNKNOWN_METHOD, super::registry::Registered::spec);
        let target = decoded.as_ref().ok().and_then(|decoded| decoded.target());
        // The tab the call names, unchecked: the record lists what the script asked for, even
        // when the door then refuses it. A lazy default page has no tab until it opens.
        let tab = match target.as_ref().map(|target| &target.0) {
            Some(TargetHandle::Page { tab, .. }) => StartTab::Named(Some(*tab)),
            Some(TargetHandle::RunDefault) => run
                .known_default_tab()
                .map_or(StartTab::Lazy, |tab| StartTab::Named(Some(tab))),
            None => StartTab::Named(None),
        };
        let scripted = caller == Caller::Script && site.is_some();
        let start = OpStart {
            state: Arc::clone(&state),
            name,
            spec,
            args: input,
            tab,
            site,
            reply,
        };
        let guard = OperationGuard::start(&self.trace, run, start);
        match decoded {
            Ok(decoded) => {
                let admitted = Admitted {
                    guard,
                    decoded,
                    state,
                    spec,
                    caller,
                    scripted,
                    target,
                };
                (pending, Some(admitted))
            }
            Err(refusal) => {
                guard.finish(Ending::Refused(refusal), Vec::new(), None, None);
                (pending, None)
            }
        }
    }

    /// The door's sequence after the start record. Every return goes through the guard, so
    /// `op_ended` and the reply happen exactly once on every path.
    async fn run_op(self: Arc<Self>, run: Arc<RunCtx>, admitted: Admitted) {
        let Admitted {
            mut guard,
            decoded,
            state,
            spec,
            caller,
            scripted,
            target,
        } = admitted;
        // A stopped run admits nothing. A replaced connection means the browser may have
        // restarted, so the run's view no longer describes it: refuse, never retarget.
        if let Some(reason) = run.stop_reason() {
            return guard.finish(Ending::Stopped(reason), Vec::new(), None, None);
        }
        if !self.browser.is_current(run.epoch()) {
            return guard.finish(Ending::Refused(disconnected()), Vec::new(), None, None);
        }
        let Resolved {
            target,
            default_page,
        } = match self.resolve_target(&run, target.as_ref()).await {
            Ok(resolved) => resolved,
            Err(ending) => return guard.finish(ending, Vec::new(), None, None),
        };
        let page = target.as_ref().map(|target| &target.page);
        guard.started_on(page.map(|page| page.tab));
        self.note_use(&run, caller, scripted, &spec, target.as_ref());
        // The rules see the checked tab: the handle's, once verified, or the default page's,
        // once open.
        let check = PolicyCheck {
            method: &spec,
            caller,
            tab: page.map(|page| page.tab),
            operation: state.id(),
            lock_pass: run.lock_pass(),
        };
        if let Err(refusal) = self.check(&check) {
            return guard.finish(Ending::Refused(refusal), Vec::new(), default_page, None);
        }
        let opening_task = self.opening_task(&run, &spec, caller);
        let op = OpCore {
            run: Arc::clone(&run),
            dispatcher: Arc::clone(&self),
            state: Arc::clone(&state),
            page: page.cloned(),
            core_page: target.as_ref().map(|target| target.core_page.clone()),
            spec,
            caller,
            cancel: run.cancel_token().child_token(),
        };
        // Biased: when the method and the stop are ready together, the stop wins, so an answer
        // that raced a cancel is never read as definite. The token fires only after `stop`
        // recorded its reason.
        let started = Instant::now();
        let ending = tokio::select! {
            biased;
            () = run.cancel_token().cancelled() => run.stop_reason().map_or(Ending::Lost, Ending::Stopped),
            result = decoded.run(op) => ending_of(&run, &state, result),
        };
        let handled = started.elapsed();
        // A takeover that blocks reads may have begun while the op ran, so what the page
        // answered is released only past the rules again. The rules see it as a read of the
        // op's tab: a main-world script is a write, and any lock refuses
        // a write, which would withhold a script that read the page before the owner took it.
        // The takeover lock applies to the tab an op targets; a script that reaches another tab
        // through `window.opener` is not checked there, as Playwright's scripts are not.
        let read = MethodSpec {
            effect: Effect::Read,
            ..spec
        };
        let mut ending = if released_past_the_rules(&spec, &ending) {
            let release = PolicyCheck {
                method: &read,
                ..check
            };
            match self.check(&release) {
                Ok(()) => ending,
                Err(refusal) => Ending::Refused(refusal),
            }
        } else {
            ending
        };
        let facts = self
            .page_facts(
                &run,
                page,
                &state,
                PolicyCheck {
                    method: &read,
                    ..check
                },
                &mut ending,
                opening_task.as_ref().map(|task| &task.id),
            )
            .await;
        // With the observation's own line (`observation.rs`), this splits a slow reply into the
        // op's work and the wait for its page facts.
        tracing::debug!(
            op = spec.name,
            handler_ms = handled.as_millis(),
            facts_ms = started.elapsed().saturating_sub(handled).as_millis(),
            "operation timing"
        );
        guard.finish(ending, facts.pages, default_page, facts.observed);
    }

    /// An op from the script's realm keeps its tab in use (the idle clock, the janitor), but
    /// only the script's own call acts on it, and only an act moves a task's built-in `page`,
    /// among that task's own tabs ([`TabRegistry::acted_on`]). A subscription (`page.on` on
    /// every tab the run knows) names a tab without acting on it. The prelude's own reads are not
    /// acts either: `waitForTimeout` refreshes the frames of every tab the run lists, and counting
    /// those moved the task's next `page` to whichever tab answered last. Neo's own ops (`caller`
    /// Neo) touch nothing here.
    fn note_use(
        &self,
        run: &RunCtx,
        caller: Caller,
        scripted: bool,
        spec: &MethodSpec,
        target: Option<&ResolvedPage>,
    ) {
        let (Caller::Script, Some(target)) = (caller, target) else {
            return;
        };
        if neo_trace::is_subscription(spec.name) {
            return;
        }
        if scripted {
            let host = run.task_id();
            let picked = run.current_task();
            let tasks: Vec<&TaskId> = host.into_iter().chain(picked.as_ref()).collect();
            self.tabs.acted_on(&target.page, target.seen, &tasks);
        } else {
            self.tabs.touched(&target.page, target.seen);
        }
        self.tabs.used_in_run(target.page.tab, run.id());
    }

    /// Capture the latest pick before a script's opener sends. Its record pins an empty task
    /// until ownership is assigned; a missing task simply leaves the tab unowned.
    fn opening_task(&self, run: &RunCtx, spec: &MethodSpec, caller: Caller) -> Option<TaskRecord> {
        if caller == Caller::Script && spec.name == DEFAULT_PAGE_OPENER {
            return run.current_task().and_then(|id| self.tabs.task(&id).ok());
        }
        None
    }

    async fn resolve_target(
        self: &Arc<Self>,
        run: &Arc<RunCtx>,
        target: Option<&PageTarget>,
    ) -> Result<Resolved, Ending> {
        match target.map(|target| &target.0) {
            None => Ok(Resolved {
                target: None,
                default_page: None,
            }),
            Some(TargetHandle::Page { tab }) => {
                let target = self.resolve(run, *tab).await.map_err(Ending::Refused)?;
                Ok(Resolved {
                    target: Some(target),
                    default_page: None,
                })
            }
            Some(TargetHandle::RunDefault) => {
                let default = match run.default_page(self).await {
                    Ok(page) => page,
                    // An open cut short by the stop is the stop's doing.
                    Err(error) => {
                        return Err(run
                            .stop_reason()
                            .map_or(Ending::Refused(error), Ending::Stopped));
                    }
                };
                // Checked like any handle, so a default page closed earlier in the run is
                // `page_closed` too.
                let target = self
                    .resolve(run, default.tab)
                    .await
                    .map_err(Ending::Refused)?;
                Ok(Resolved {
                    default_page: Some(target.page.clone()),
                    target: Some(target),
                })
            }
        }
    }

    /// Checks a handle against core's cache, with no CDP call, and names the owner of a dead
    /// one: this script if it closed the page itself, Neo otherwise.
    async fn resolve(&self, run: &RunCtx, tab: TabId) -> Result<ResolvedPage, OperationError> {
        // Neo's blank tab keeps the browser open (`NeoTab`): no listing gives it to a script, and
        // a handle a script built for it itself is refused here, before anything reaches it.
        if self.tabs.is_neo(tab) {
            return Err(OperationError::script(
                ErrorCode::INVALID_INPUT,
                "this tab is Neo's own blank tab, which keeps the browser open; a script cannot use it",
            ));
        }
        // Sample immediately before the cache read. A detach after this read invalidates the
        // registry write even if its close record was evicted before the write resumes.
        let seen = self.tabs.seen();
        match self.browser.page(tab).await {
            Some(info) => Ok(ResolvedPage {
                page: PageRef::from(&info),
                core_page: info.page_id,
                seen,
            }),
            None if run.closed_it(tab) => Err(OperationError::script(
                ErrorCode::PAGE_CLOSED,
                "this script closed the page",
            )),
            None => Err(OperationError::neo(
                ErrorCode::PAGE_CLOSED,
                "the page was closed",
            )),
        }
    }

    /// Runs every rule in order; the first refusal wins.
    pub(super) fn check(&self, check: &PolicyCheck<'_>) -> Result<(), OperationError> {
        for policy in &self.policies {
            policy.check(check)?;
        }
        Ok(())
    }

    /// The reported pages' bookkeeping, one refresh per navigated page, then facts from core's
    /// cache for the target and every reported page: no CDP call per op otherwise.
    #[expect(
        clippy::too_many_lines,
        reason = "one ordered fact batch reconciles reports, cache observations, and release decisions"
    )]
    async fn page_facts(
        &self,
        run: &RunCtx,
        target: Option<&PageRef>,
        state: &OpState,
        check: PolicyCheck<'_>,
        ending: &mut Ending,
        opening_task: Option<&TaskId>,
    ) -> PageFacts {
        let reports = state.take_reports();
        let mut pages: Vec<PageRef> = target.into_iter().cloned().collect();
        let mut refreshed: Vec<TabId> = Vec::new();
        let mut claims: HashMap<TabId, Vec<Claim>> = HashMap::new();
        let mut acted = None;
        let mut withheld = HashSet::new();
        for PageReport { page, change } in &reports {
            match change {
                PageChange::Opened => {
                    let seen = self.tabs.seen();
                    if self.browser.page(page.tab).await.is_some() {
                        self.tabs.opened_by_agent(page, seen);
                        self.tabs.used_in_run(page.tab, run.id());
                        if let Some(task) = opening_task {
                            // The selected task can close while the browser opens this tab.
                            let _ = self.tabs.own(page, task);
                        }
                    }
                }
                PageChange::Navigated if !refreshed.contains(&page.tab) => {
                    refreshed.push(page.tab);
                    self.refresh_navigated(run, page).await;
                }
                PageChange::Navigated | PageChange::Linked => {}
                PageChange::Popup { opener } => {
                    let seen = self.tabs.seen();
                    if self.browser.page(page.tab).await.is_some() {
                        self.tabs.popup(page, opener.tab, seen);
                        // The run that received it may use it a while later, as it does a tab
                        // it opened, so the janitor leaves it while the run is in flight.
                        self.tabs.used_in_run(page.tab, run.id());
                    }
                }
                PageChange::Gone { target, seq } => {
                    claims.entry(page.tab).or_default().push(Claim {
                        target: target.clone(),
                        seq: *seq,
                        released: true,
                    });
                }
                PageChange::WithheldClose { seq } => {
                    withheld.insert((page.tab, *seq));
                }
                PageChange::Acted { target } => {
                    acted = Some(ActedTarget {
                        tab: page.tab,
                        target: target.clone(),
                    });
                }
                PageChange::Closed => run.record_closed(page.tab),
            }
            if !pages.contains(page) {
                pages.push(page.clone());
            }
        }
        for (tab, delivered) in &mut claims {
            for claim in delivered {
                claim.released = !withheld.contains(&(*tab, claim.seq));
            }
        }
        let closed: Vec<TabId> = reports
            .iter()
            .filter(|report| report.change == PageChange::Closed)
            .map(|report| report.page.tab)
            .collect();
        if pages.is_empty() {
            return PageFacts {
                pages: Vec::new(),
                observed: None,
            };
        }
        self.refresh_self_navigated(run, &pages, &closed, &refreshed)
            .await;
        let reads: Vec<TabId> = pages
            .iter()
            .filter(|page| !closed.contains(&page.tab))
            .map(|page| page.tab)
            .collect();
        let observation = self
            .observer
            .observe(&self.browser, &self.tabs, &reads, &claims, acted.as_ref())
            .await;
        let mut infos = observation.infos.into_iter();
        let mut workers = observation.workers.into_iter();
        let mut facts = Vec::with_capacity(pages.len());
        for page in pages {
            let info = if closed.contains(&page.tab) {
                None
            } else {
                infos.next().flatten()
            };
            let mut fact = PageFact::in_view(&page, info, run.view());
            fact.task_id = self.tabs.owner(page.tab).map(|id| id.to_string());
            fact.workers = if closed.contains(&page.tab) {
                Some(Vec::new())
            } else {
                workers.next().flatten()
            };
            facts.push(fact);
        }
        let mut released = self.release_facts(
            PageFacts {
                pages: facts,
                observed: Some(observation.observed),
            },
            &observation.withheld,
            check,
            &claims,
            ending,
        );
        if acted.is_some_and(|source| !released.pages.iter().any(|fact| fact.tab == source.tab))
            && let Some(observed) = &mut released.observed
        {
            observed.payload_current = None;
        }
        released
    }

    /// A navigation the page made itself (a link, `pushState`, a fragment, the Navigation API)
    /// reaches core's cache only through a refresh, so each open tab of the reply whose main
    /// frame the event log saw navigate since the door last refreshed it is refreshed once, as
    /// a navigating op's tab is. Otherwise the reply would hand the prelude the URL from before,
    /// over a fresher one it already has. `refreshed` are the tabs refreshed for this op already.
    async fn refresh_self_navigated(
        &self,
        run: &RunCtx,
        pages: &[PageRef],
        closed: &[TabId],
        refreshed: &[TabId],
    ) {
        for page in pages {
            if closed.contains(&page.tab) || refreshed.contains(&page.tab) {
                continue;
            }
            self.refresh_if_navigated(run, page).await;
        }
    }

    /// Refreshes `page` once its main frame navigated past the event seq the door last refreshed
    /// it through, then moves the mark to the log's head from just before the refresh read core.
    /// Only a refresh that read core moves it: a failed or cancelled one leaves the navigation
    /// owed, so a later reply tries again. A tab the registry does not track has no mark.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "the turn orders one tab's cache refreshes; see RefreshedThrough"
    )]
    async fn refresh_if_navigated(&self, run: &RunCtx, page: &PageRef) {
        let Some(mark) = self.tabs.slot::<RefreshedThrough>(page) else {
            return;
        };
        let Some(info) = self.browser.page(page.tab).await else {
            return;
        };
        let Some(navigated) = self.observer.navigated(&info.target_id) else {
            return;
        };
        if navigated <= mark.through.load(Ordering::Relaxed) {
            return;
        }
        let _turn = mark.turn.lock().await;
        // A refresh that had the turn while this one waited may already cover it.
        if navigated <= mark.through.load(Ordering::Relaxed) {
            return;
        }
        let head = self.observer.head();
        if refresh(run, &self.browser, page.tab).await {
            mark.through.fetch_max(head, Ordering::Relaxed);
        }
    }

    /// A navigating op's refresh, in its tab's turn, moving the mark as one above does.
    async fn refresh_navigated(&self, run: &RunCtx, page: &PageRef) {
        let mark = self.tabs.slot::<RefreshedThrough>(page);
        let _turn = match &mark {
            Some(mark) => Some(mark.turn.lock().await),
            None => None,
        };
        let head = self.observer.head();
        if refresh(run, &self.browser, page.tab).await
            && let Some(mark) = &mark
        {
            mark.through.fetch_max(head, Ordering::Relaxed);
        }
    }

    /// Facts and their reported close records cross policy together, after the async cache
    /// observation. A close cannot outlive the withheld cutoff that made its delivery safe.
    fn release_facts(
        &self,
        mut facts: PageFacts,
        withheld: &[TabId],
        check: PolicyCheck<'_>,
        claims: &HashMap<TabId, Vec<Claim>>,
        ending: &mut Ending,
    ) -> PageFacts {
        let mut refusals = HashMap::new();
        facts.pages.retain(|page| {
            match self.check(&PolicyCheck {
                tab: Some(page.tab),
                ..check
            }) {
                Ok(()) => !withheld.contains(&page.tab),
                Err(error) => {
                    refusals.insert(page.tab, error);
                    false
                }
            }
        });
        if let Some(observed) = &mut facts.observed {
            observed
                .closes
                .retain(|close| facts.pages.iter().any(|fact| fact.tab == close.tab));
        }
        close_release(ending, claims, &facts.pages, &refusals);
        if facts.pages.is_empty() {
            facts.observed = None;
        }
        facts
    }
}

/// A fact batch and the host observation that orders it in each run's prelude.
pub(crate) struct PageFacts {
    pub(crate) pages: Vec<PageFact>,
    pub(crate) observed: Option<Observation>,
}

impl PageFact {
    pub(crate) fn in_view(page: &PageRef, info: Option<PageInfo>, view: &BrowserView) -> Self {
        let viewport_size = info
            .as_ref()
            .and_then(|info| view.playwright_viewport(&info.page_id));
        Self {
            viewport_size,
            ..Self::new(page, info)
        }
    }

    pub(crate) fn new(page: &PageRef, info: Option<PageInfo>) -> Self {
        let PageRef { tab } = *page;
        match info {
            Some(info) => Self {
                tab,
                task_id: None,
                url: Some(info.url),
                title: Some(info.title),
                closed: false,
                viewport_size: None,
                workers: None,
            },
            None => Self {
                tab,
                task_id: None,
                url: None,
                title: None,
                closed: true,
                viewport_size: None,
                workers: None,
            },
        }
    }

    /// The fact as the trace's `page_after`. The writer clips it.
    pub(super) fn page_state(&self) -> PageState {
        PageState {
            tab_id: Some(self.tab),
            url: self.url.clone(),
            title: self.title.clone(),
        }
    }
}

/// An open tab, as core's cache describes it.
impl From<PageInfo> for PageFact {
    fn from(info: PageInfo) -> Self {
        Self::new(&PageRef::from(&info), Some(info))
    }
}

impl PendingReply {
    /// The op this call became.
    #[must_use]
    pub(crate) fn operation(&self) -> OperationId {
        self.operation
    }

    /// Waits for the answer.
    pub(crate) async fn wait(self) -> OpReply {
        let operation = self.operation;
        self.reply.await.unwrap_or_else(|error| {
            // The guard answers on every path, so this means the op's task was torn down in a
            // way Neo cannot see into. Unknown, so the agent looks before repeating.
            tracing::error!(%error, %operation, "an operation ended without an answer");
            OpReply {
                outcome: Err(OpFailure {
                    error: OperationError::neo(
                        ErrorCode::INTERNAL_ERROR,
                        "the operation ended without an answer",
                    ),
                    delivery: Delivery::Unknown,
                    retryable: false,
                    operation,
                }),
                pages: Vec::new(),
                observed: None,
                default_page: None,
                picked_page: None,
                pumped: 0,
                completed_publications: Vec::new(),
            }
        })
    }
}

/// Whether `ending` is page data the door checks against the rules again before release: a read's
/// value, and what a main-world script answered (decision 35c), its value or the error the page
/// answered with, whose text the script wrote. A write's own end, a click's say, reports what it
/// did, not what it read, and an unanswered failure carries Neo's text alone.
fn released_past_the_rules(spec: &MethodSpec, ending: &Ending) -> bool {
    match ending {
        Ending::Returned(_) => {
            spec.effect == Effect::Read || spec.page_script == PageScript::MainWorld
        }
        Ending::Failed(leaf) => {
            spec.page_script == PageScript::MainWorld && leaf.answer == Some(Answer::Definite)
        }
        Ending::Refused(_) | Ending::Stopped(_) | Ending::Panicked | Ending::Lost => false,
    }
}

/// How a run step that came back ended. A standing refusal outranks whatever the method made of
/// it, its value included, so the rule's own error reaches the script even when the crate drops
/// the refusal. A value with a refusal standing cannot come from the crate, whose one send gate
/// (`OpEnv::recheck`) returns every refusal; a passing `recheck` clears it at each attempt.
/// An error that surfaces while the run is stopping is the stop's doing; otherwise the standing
/// refusal outranks the method's own error.
fn ending_of(run: &RunCtx, state: &OpState, result: Result<Value, LeafError>) -> Ending {
    let result = match (result, state.refusal()) {
        (Ok(_), Some(refusal)) => Err(LeafError::definite(refusal)),
        (result, _) => result,
    };
    match (result, run.stop_reason()) {
        (Ok(value), _) => Ending::Returned(value),
        (Err(_), Some(reason)) => Ending::Stopped(reason),
        (Err(leaf), None) => Ending::Failed(state.refusal().map_or(leaf, LeafError::definite)),
    }
}

/// One `Browser.getTabInfo` so core's cache matches the page after an unbound navigation.
/// Raced with the stop: a stopping run does not wait on it.
/// Reads `tab`'s entry in core's page cache again; `true` once it did.
async fn refresh(run: &RunCtx, browser: &BrowserLink, tab: TabId) -> bool {
    let Some(info) = browser.page(tab).await else {
        return false;
    };
    tokio::select! {
        biased;
        () = run.cancel_token().cancelled() => false,
        refreshed = run.view().refresh_page(info.page_id) => match refreshed {
            Ok(_) => true,
            Err(error) => {
                tracing::debug!(%error, tab = tab.0, "could not refresh a navigated page; its facts may be stale");
                false
            }
        },
    }
}

fn disconnected() -> OperationError {
    OperationError::neo(
        ErrorCode::BROWSER_DISCONNECTED,
        "the browser connection dropped or was replaced since this run began",
    )
}

#[cfg(test)]
mod tests {
    use super::CallSite;

    #[test]
    fn a_call_site_reads_the_bootstraps_form() {
        assert_eq!(
            CallSite::parse("run:3:17"),
            Some(CallSite {
                line: 3,
                column: 17
            }),
            "the bootstrap's `run:LINE:COL`"
        );
        assert_eq!(
            CallSite::parse("run:3:17").map(|site| site.to_string()),
            Some("run:3:17".to_owned()),
            "and the reply lines print it back the same way"
        );
        for text in ["", "run:3", "run:x:1", "eval:3:17", "run:3:17:9"] {
            assert_eq!(CallSite::parse(text), None, "{text:?} is no call site");
        }
    }
}
