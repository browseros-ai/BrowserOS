//! The door.

use std::fmt;
use std::sync::Arc;

use browseros_core::pages::PageInfo;
use browseros_core::{PageId, TabId};
use browseros_trace::{Delivery, Effect, OperationId, PageState, SecretSet, TraceSink};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::op_ctx::{OpCore, OpState, PageChange, PageReport};
use super::operation_guard::{Ending, OpStart, OperationGuard};
use super::page_target::TargetHandle;
use super::registry::DecodedCall;
use super::{
    ErrorCode, LeafError, MethodSpec, OpFailure, OperationError, PageTarget, Policy, PolicyCheck,
    Registry, RunCtx, Visibility,
};
use crate::browser::{BrowserLink, PageRef, TabRegistry};

/// The method the door calls, as Neo, to open a run's lazy default page: with `{}`, returning
/// Playwright's page entry `{ page, url }` (`OpenedPage`) after reporting the page
/// `PageChange::Opened`.
const DEFAULT_PAGE_OPENER: &str = "context.newPage";
/// How much of a method name the trace and the error keep: a script can send any string.
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
///               gates ─ target (cached, tab checked) ─ rules ─ Method::call raced with
///               the stop ─ reads checked again ─ page facts ─ OperationGuard::finish
///  OpCtx::invoke ─ the same admit and run_op, inline in the parent's task
/// ```
///
/// Everything up to the start record is synchronous, so every op id handed out is recorded
/// and answered even if its task never runs.
pub(crate) struct Dispatcher {
    registry: Registry,
    /// Fixed at startup, in the one order the app writes down (see [`Policy`]).
    policies: Vec<Arc<dyn Policy>>,
    secrets: Arc<SecretSet>,
    trace: TraceSink,
    browser: Arc<BrowserLink>,
    tabs: Arc<TabRegistry>,
}

/// Who is calling. A script's results are masked; Neo's own may name `Internal` methods.
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
}

/// Where in the script a call was made (`run:LINE:COL`), for the run's reply lines. A plain
/// data bag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CallSite {
    /// The line in the script, from 1.
    pub(crate) line: u32,
    /// The column, from 1.
    pub(crate) column: u32,
}

/// What the door hands back: the outcome (masked for a script), and fresh facts about the
/// pages the op touched, read from core's cache so the prelude's sync accessors
/// (`page.url()`) stay current without a round trip. A plain data bag.
#[derive(Debug)]
pub(crate) struct OpReply {
    /// The value, or the failure with its owner and delivery.
    pub(crate) outcome: Result<Value, OpFailure>,
    /// The target first, then the pages the op reported.
    pub(crate) pages: Vec<PageFact>,
    /// Set when the op resolved the run's default page, so the script's `page` learns its id.
    pub(crate) default_page: Option<PageRef>,
}

/// One page as a reply reports it. Deliberately not a page brand (no `$neo`), so the
/// bootstrap never revives a fact into a `Page` object. A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct PageFact {
    /// Core's process-local page id.
    pub(crate) id: PageId,
    /// The browser's tab id.
    pub(crate) tab: TabId,
    /// `None` once closed.
    pub(crate) url: Option<String>,
    /// `None` once closed.
    pub(crate) title: Option<String>,
    /// Whether the tab is gone.
    pub(crate) closed: bool,
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
    target: Option<PageTarget>,
}

/// What [`DEFAULT_PAGE_OPENER`] returns, as far as the door reads it: the brand of the tab it
/// opened. Its `url` is the prelude's, so unknown fields pass.
#[derive(Deserialize)]
struct OpenedPage {
    page: PageTarget,
}

/// An op's target after the door checked it.
struct Resolved {
    page: Option<PageRef>,
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
        secrets: Arc<SecretSet>,
        trace: TraceSink,
        browser: Arc<BrowserLink>,
        tabs: Arc<TabRegistry>,
    ) -> Self {
        Self {
            registry,
            policies,
            secrets,
            trace,
            browser,
            tabs,
        }
    }

    /// The methods, for the `run` description.
    #[must_use]
    pub(crate) fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The process's known secrets, which the door masks out of everything a script sees.
    #[must_use]
    pub(crate) fn secrets(&self) -> &Arc<SecretSet> {
        &self.secrets
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
        };
        let (pending, admitted) = self.admit(run, call, parent.map(Arc::clone));
        if let Some(admitted) = admitted {
            // Boxed: the door's future nests here once per level of child ops.
            Box::pin(Arc::clone(self).run_op(Arc::clone(run), admitted)).await;
        }
        pending.wait().await.outcome
    }

    /// Opens the run's lazy default page through [`DEFAULT_PAGE_OPENER`], as a Neo op with no
    /// parent. It belongs to the run, not to whichever op first touched `page`, so its send
    /// never counts toward that op's delivery.
    pub(super) async fn open_default_page(
        self: &Arc<Self>,
        run: &Arc<RunCtx>,
    ) -> Result<PageRef, OperationError> {
        let opened = self
            .invoke_child(run, None, DEFAULT_PAGE_OPENER, json!({}))
            .await
            .map_err(|failure| failure.error)?;
        match serde_json::from_value::<OpenedPage>(opened) {
            Ok(OpenedPage {
                page: PageTarget(TargetHandle::Page { id, tab }),
            }) => Ok(PageRef { id, tab }),
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

    pub(super) fn tabs(&self) -> &TabRegistry {
        &self.tabs
    }

    /// The synchronous front of the door: the op id, the method, the decoded input, and the
    /// start record. A call refused here (unknown or hidden name, bad input) is answered at
    /// once; anything else comes back to run.
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
        } = call;
        let state = OpState::new(run.next_operation(), parent);
        let (reply, receiver) = oneshot::channel();
        let pending = PendingReply {
            operation: state.id(),
            reply: receiver,
        };
        let registered = self.registry.get(&name);
        let shown = shown_name(name, &self.secrets);
        // An `Internal` name gets the same answer as a missing one, so a script cannot probe
        // for hidden methods; the record keeps the real spec and its redaction.
        let decoded = match registered {
            Some(registered) if caller.may_call(registered.spec().visibility) => {
                registered.decode(&input)
            }
            Some(_) | None => Err(OperationError::script(
                ErrorCode::METHOD_UNAVAILABLE,
                format_args!("`{shown}` is not a Neo method"),
            )),
        };
        let (spec, redaction) = registered.map_or_else(
            || (UNKNOWN_METHOD, Arc::default()),
            |registered| (registered.spec(), Arc::clone(registered.redaction())),
        );
        let target = decoded.as_ref().ok().and_then(|decoded| decoded.target());
        // The tab the call names, unchecked: the record lists what the script asked for, even
        // when the door then refuses it. A lazy default page has no tab until it opens.
        let tab = match target.as_ref().map(|target| &target.0) {
            Some(TargetHandle::Page { tab, .. }) => Some(*tab),
            Some(TargetHandle::RunDefault) => run.known_default_tab(),
            None => None,
        };
        let start = OpStart {
            state: Arc::clone(&state),
            name: shown,
            spec,
            redaction,
            args: input,
            tab,
            site,
            caller,
            reply,
        };
        let guard = OperationGuard::start(&self.trace, &self.secrets, run, start);
        match decoded {
            Ok(decoded) => {
                let admitted = Admitted {
                    guard,
                    decoded,
                    state,
                    spec,
                    caller,
                    target,
                };
                (pending, Some(admitted))
            }
            Err(refusal) => {
                guard.finish(Ending::Refused(refusal), Vec::new(), None);
                (pending, None)
            }
        }
    }

    /// The door's sequence after the start record. Every return goes through the guard, so
    /// `op_ended` and the reply happen exactly once on every path.
    async fn run_op(self: Arc<Self>, run: Arc<RunCtx>, admitted: Admitted) {
        let Admitted {
            guard,
            decoded,
            state,
            spec,
            caller,
            target,
        } = admitted;
        // A stopped run admits nothing. A replaced connection means the browser may have
        // restarted, so the run's view no longer describes it: refuse, never retarget.
        if let Some(reason) = run.stop_reason() {
            return guard.finish(Ending::Stopped(reason), Vec::new(), None);
        }
        if !self.browser.is_current(run.epoch()) {
            return guard.finish(Ending::Refused(disconnected()), Vec::new(), None);
        }
        let Resolved { page, default_page } = match self.resolve_target(&run, target.as_ref()).await
        {
            Ok(resolved) => resolved,
            Err(ending) => return guard.finish(ending, Vec::new(), None),
        };
        if let (Caller::Script, Some(page)) = (caller, &page) {
            self.tabs.touched(page, run.task_id());
        }
        // The rules see the checked tab: the handle's, once verified, or the default page's,
        // once open.
        let check = PolicyCheck {
            method: &spec,
            caller,
            tab: page.as_ref().map(|page| page.tab),
            operation: state.id(),
        };
        if let Err(refusal) = self.check(&check) {
            return guard.finish(Ending::Refused(refusal), Vec::new(), default_page);
        }
        let op = OpCore {
            run: Arc::clone(&run),
            dispatcher: Arc::clone(&self),
            state: Arc::clone(&state),
            page: page.clone(),
            spec,
            caller,
            cancel: run.cancel_token().child_token(),
        };
        // Biased: when the method and the stop are ready together, the stop wins, so an answer
        // that raced a cancel is never read as definite. The token fires only after `stop`
        // recorded its reason.
        let ending = tokio::select! {
            biased;
            () = run.cancel_token().cancelled() => run.stop_reason().map_or(Ending::Lost, Ending::Stopped),
            result = decoded.run(op) => ending_of(&run, &state, result),
        };
        // A takeover that blocks reads may have begun while this read ran (sdk-traces D13,
        // reads: block), so a read's result is released only past the rules again.
        let ending = match ending {
            Ending::Returned(value) if spec.effect == Effect::Read => match self.check(&check) {
                Ok(()) => Ending::Returned(value),
                Err(refusal) => Ending::Refused(refusal),
            },
            ending => ending,
        };
        let pages = self.page_facts(&run, page.as_ref(), &state).await;
        guard.finish(ending, pages, default_page);
    }

    async fn resolve_target(
        self: &Arc<Self>,
        run: &Arc<RunCtx>,
        target: Option<&PageTarget>,
    ) -> Result<Resolved, Ending> {
        match target.map(|target| &target.0) {
            None => Ok(Resolved {
                page: None,
                default_page: None,
            }),
            Some(TargetHandle::Page { id, tab }) => {
                let page = self.resolve(run, id, *tab).await.map_err(Ending::Refused)?;
                Ok(Resolved {
                    page: Some(page),
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
                let page = self
                    .resolve(run, &default.id, default.tab)
                    .await
                    .map_err(Ending::Refused)?;
                Ok(Resolved {
                    page: Some(page.clone()),
                    default_page: Some(page),
                })
            }
        }
    }

    /// Checks a handle against core's cache, with no CDP call, and names the owner of a dead
    /// one: this script if it closed the page itself, Neo otherwise.
    async fn resolve(
        &self,
        run: &RunCtx,
        id: &PageId,
        tab: TabId,
    ) -> Result<PageRef, OperationError> {
        match self.browser.cached_page(id).await {
            Some(info) if info.tab_id == tab => Ok(PageRef::from(&info)),
            Some(_) => Err(OperationError::script(
                ErrorCode::STALE_PAGE,
                "this page handle is from before Neo restarted; get the page again from context.pages()",
            )),
            None if run.closed_it(id) => Err(OperationError::script(
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
    async fn page_facts(
        &self,
        run: &RunCtx,
        target: Option<&PageRef>,
        state: &OpState,
    ) -> Vec<PageFact> {
        let reports = state.take_reports();
        let mut pages: Vec<PageRef> = target.into_iter().cloned().collect();
        let mut refreshed: Vec<&PageId> = Vec::new();
        for PageReport { page, change } in &reports {
            match change {
                PageChange::Opened => self.tabs.opened_by_agent(page, run.task_id()),
                PageChange::Navigated if !refreshed.contains(&&page.id) => {
                    refreshed.push(&page.id);
                    refresh(run, &page.id).await;
                }
                PageChange::Navigated => {}
                PageChange::Closed => run.record_closed(&page.id),
            }
            if !pages.contains(page) {
                pages.push(page.clone());
            }
        }
        let closed: Vec<&PageId> = reports
            .iter()
            .filter(|report| report.change == PageChange::Closed)
            .map(|report| &report.page.id)
            .collect();
        let mut facts = Vec::with_capacity(pages.len());
        for page in pages {
            let info = if closed.contains(&&page.id) {
                None
            } else {
                self.browser.cached_page(&page.id).await
            };
            facts.push(PageFact::new(page, info));
        }
        facts
    }
}

impl PageFact {
    fn new(page: PageRef, info: Option<PageInfo>) -> Self {
        let PageRef { id, tab } = page;
        match info {
            Some(info) => Self {
                id,
                tab,
                url: Some(info.url),
                title: Some(info.title),
                closed: false,
            },
            None => Self {
                id,
                tab,
                url: None,
                title: None,
                closed: true,
            },
        }
    }

    /// Replaces every known secret in what a script would see of the page: its URL and title.
    pub(crate) fn mask(&mut self, secrets: &SecretSet) {
        for text in [&mut self.url, &mut self.title].into_iter().flatten() {
            secrets.mask_str(text);
        }
    }

    /// The fact as the trace's `page_after`. The writer masks and clips it.
    pub(super) fn page_state(&self) -> PageState {
        PageState {
            page_id: Some(self.id.0),
            tab_id: Some(self.tab),
            url: self.url.clone(),
            title: self.title.clone(),
        }
    }
}

/// An open tab, as core's cache describes it.
impl From<PageInfo> for PageFact {
    fn from(info: PageInfo) -> Self {
        Self::new(PageRef::from(&info), Some(info))
    }
}

impl OpReply {
    /// Replaces every known secret in what a script would see: the value, the error message,
    /// and each page's URL and title.
    pub(super) fn mask(&mut self, secrets: &SecretSet) {
        match &mut self.outcome {
            Ok(value) => {
                secrets.mask_json(value);
            }
            Err(failure) => failure.error.mask(secrets),
        }
        for page in &mut self.pages {
            page.mask(secrets);
        }
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
                default_page: None,
            }
        })
    }
}

/// How a run step that came back ended. An error that surfaces while the run is stopping is
/// the stop's doing; otherwise a standing `recheck` refusal outranks the method's own error.
fn ending_of(run: &RunCtx, state: &OpState, result: Result<Value, LeafError>) -> Ending {
    match (result, run.stop_reason()) {
        (Ok(value), _) => Ending::Returned(value),
        (Err(_), Some(reason)) => Ending::Stopped(reason),
        (Err(leaf), None) => Ending::Failed(state.refusal().map_or(leaf, LeafError::definite)),
    }
}

/// One `Browser.getTabInfo` so core's cache matches the page after an unbound navigation.
/// Raced with the stop: a stopping run does not wait on it.
async fn refresh(run: &RunCtx, page: &PageId) {
    tokio::select! {
        biased;
        () = run.cancel_token().cancelled() => {}
        refreshed = run.view().refresh_page(page.clone()) => {
            if let Err(error) = refreshed {
                tracing::debug!(%error, %page, "could not refresh a navigated page; its facts may be stale");
            }
        }
    }
}

fn disconnected() -> OperationError {
    OperationError::neo(
        ErrorCode::BROWSER_DISCONNECTED,
        "the browser connection dropped or was replaced since this run began",
    )
}

/// A script can send any string as a method name; the record and the error keep its start,
/// masked first. The writer does not mask names, and a cut through a known secret would leave a
/// prefix no mask recognizes.
fn shown_name(mut name: String, secrets: &SecretSet) -> String {
    secrets.mask_str(&mut name);
    name.chars().take(NAME_CHARS).collect()
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
