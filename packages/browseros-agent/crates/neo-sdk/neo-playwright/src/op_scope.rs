//! One operation in flight, and the proof a leaf must hold to change the page.
//!
//! ```text
//!  Playwright::invoke ── decode `call` ── OpScope::new ── family leaf
//!                                            │ admit(page)    once per op, through the run's view
//!                                            │ admit_target   instead, for an op the browser
//!                                            │                carries out alone (close, opener)
//!                                            │ log(line)      Playwright's call log, mirrored to the trace
//!                                            │ sleep, within  Rust's clock against the op's own timeout
//!                                            │ require(cap)   at a capability's gateway: declared?
//!                                            └ send_effect()  recheck, then mark_sent ──▶ Sent
//!                                                             (then sending_main_world, past MainWorld's gateway)
//!                                                             └▶ input, navigation, element edits,
//!                                                                main-world script: each takes &Sent
//! ```
//!
//! Delivery is derived from one fact, whether the op was marked sent before its first write
//! (skeleton D7.5), so that fact is not left to convention. The host's env is private to the
//! scope, [`OpScope::send_effect`] is the only code that reaches `OpEnv::mark_sent`, it always
//! re-runs the host's attempt-stage policies first, and the [`Sent`] it returns is the only way
//! to call a function that changes the page. A leaf that never calls it, which is every read,
//! can neither mark itself sent nor reach an effect.
//!
//! The scope also tells the host which ops send script into a page's main world, where a site
//! can notice it ([`OpScope::sending_main_world`]), right before each such send: a `MainWorld`
//! gateway's `send_effect`, Neo's own script that gateway sends before its recheck (the evaluate
//! guard's evaluator, a worker's global lookup), each of the run's context scripts and bindings
//! a write's admission adds to its tab, a binding's cleanup when `disposable.dispose` removes
//! it, and a `javascript:` URL a `goto` sends. The op's static capabilities would miss the last
//! three: a `goto` declares none, yet it carries `context.exposeBinding`'s stub into a new tab.
//!
//! The scope also owns the op's CDP object group: every remote object the op asks for (an
//! iframe it hops through) lives in it, and dropping the scope releases it, whether the op
//! returned or the host dropped its future.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use browseros_core::pages::{PageInfo, PageSession};
use browseros_core::{
    BrowserSession, BrowserView, CoreError, PageId, ProtocolSession, TabId, TargetId,
};
use futures_util::future::{self, Either};
use neo_trace::Effect;
use serde_json::Value;
use tokio::time::{self, Instant};

use crate::call_log::CallLog;
use crate::object_group::ObjectGroup;
use crate::op::Op;
use crate::op_spec::Capability;
use crate::target::CallHeader;
use crate::{ErrorCode, OpEnv, PlaywrightError, ReadPath, RunState, WritePath};

pub(crate) use sent::Sent;

/// One operation in flight. Lives for one `Playwright::invoke`; every method takes `&self`,
/// so a leaf can log and read its deadline while it holds a [`Sent`].
pub(crate) struct OpScope<'env> {
    /// The host's side of this op. Private: leaves reach `mark_sent` only via `send_effect`.
    env: &'env dyn OpEnv,
    op: Op,
    /// The method the script called (`page.click`); every message starts with it.
    api: String,
    started: Instant,
    /// Playwright's timeout for this op. `None` is `timeout: 0`: the op waits until the host
    /// drops it at the run's deadline.
    timeout: Option<Duration>,
    /// A std mutex: lines are pushed between awaits, never across one.
    log: Mutex<CallLog>,
    /// The page this op acts on, admitted once.
    page: OnceLock<AdmittedPage>,
    objects: ObjectGroup,
    /// Set once the op passed its `MainWorld` gateway: every effect it sends from then on is
    /// the gateway's main-world script, so `send_effect` reports it to the host.
    main_world: AtomicBool,
}

/// A page admitted for one op through the run's view: the profile and window scope check ran
/// once, and the op's calls go to this session.
#[derive(Debug)]
pub(crate) struct AdmittedPage {
    pub(crate) tab: TabId,
    /// Core cache identity: private to this admission and never serialized.
    pub(crate) core_page: PageId,
    pub(crate) session: PageSession,
}

/// The op's own wait ended: its Playwright timeout passed, or the script aborted it through
/// `signal`. The leaf turns it into its timed-out code with [`OpScope::timed_out`], which
/// reports an abort instead when there was one, so every leaf's timeout path is its abort path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Expired;

impl<'env> OpScope<'env> {
    /// Opens the scope for `op` with the header the prelude sent. Its clock starts now.
    pub(crate) fn new(env: &'env dyn OpEnv, op: Op, call: CallHeader) -> Self {
        Self {
            env,
            op,
            api: call.api.unwrap_or_else(|| op.spec().name.to_owned()),
            started: Instant::now(),
            timeout: call
                .timeout_ms
                .filter(|millis| *millis > 0)
                .map(Duration::from_millis),
            log: Mutex::default(),
            page: OnceLock::new(),
            objects: ObjectGroup::new(),
            main_world: AtomicBool::new(false),
        }
    }

    /// The object group for remote objects this op asks `session` for. Naming it registers
    /// the session, so the group is released there when the op ends.
    pub(crate) fn object_group(&self, session: &ProtocolSession) -> &str {
        self.objects.on(session)
    }

    /// Releases the op's remote objects now, keeping its group for later ones
    /// ([`ObjectGroup::release_now`]). Only for a leaf whose tries share no remote object.
    pub(crate) async fn release_objects(&self) {
        self.objects.release_now().await;
    }

    /// The op's Playwright timeout; `None` waits until the host drops the op.
    pub(crate) fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// The tab this op admitted, once it has.
    pub(crate) fn admitted_tab(&self) -> Option<TabId> {
        self.page.get().map(|admitted| admitted.tab)
    }

    /// The run's admitted view, for what core does on the run's behalf: listing, opening and
    /// closing tabs, captures. A page this op acts on is still admitted once with
    /// [`OpScope::admit`], and a write through the view still follows [`OpScope::send_effect`].
    pub(crate) fn view(&self) -> &BrowserView {
        self.env.view()
    }

    /// This run's Playwright state, which the host drops with the run.
    pub(crate) fn run_state(&self) -> &Arc<RunState> {
        self.env.run_state()
    }

    /// The browser's startup mode, supplied by the host for this operation.
    pub(crate) fn hosted_mode(&self) -> bool {
        self.env.hosted_mode()
    }

    /// Checks the live read policy before a gateway stats or reads a script-supplied path.
    /// Only legacy standalone hosts may omit the policy; SDK constructors require one.
    pub(crate) async fn read_path(&self, path: &Path) -> Result<ReadPath, PlaywrightError> {
        match self.env.file_access() {
            Some(access) => access
                .read(path)
                .await
                .map_err(|error| self.fail(ErrorCode::InvalidInput, &error.to_string())),
            None => Ok(ReadPath::unchecked(path).await),
        }
    }

    /// Checks protected destinations before delivery is marked or parent folders are made.
    pub(crate) async fn write_path(&self, path: &Path) -> Result<WritePath, PlaywrightError> {
        match self.env.file_access() {
            Some(access) => access
                .write(path)
                .await
                .map_err(|error| self.fail(ErrorCode::InvalidInput, &error.to_string())),
            None => Ok(WritePath::unchecked(path)),
        }
    }

    /// Carries delivered-close evidence to the host before the operation returns its value.
    pub(crate) fn report_close(&self, tab: TabId, target: &TargetId, seq: u64) {
        self.run_state().frames.forget(tab);
        self.env.report_close(tab, target, seq);
    }

    /// Carries a page event's possible URL change to hosts that keep synchronous page facts.
    pub(crate) fn report_navigation(&self, tab: TabId) {
        self.env.report_navigation(tab);
    }

    /// Resolves a native tab in core's cache, listing once only on a miss, then performs the
    /// existing view admission. Core's id lives only in this operation's admitted handle.
    pub(crate) async fn admit(
        &self,
        browser: &BrowserSession,
        tab: TabId,
    ) -> Result<&AdmittedPage, PlaywrightError> {
        if self.page.get().is_none() {
            let admitted = self
                .observe_page(browser, tab)
                .await?
                .ok_or_else(|| self.fail(ErrorCode::TargetClosed, "the tab was closed"))?;
            // A write may load a document, so the run's context init scripts reach the tab
            // first (page/init_script.rs). A read loads none and adds nothing.
            if self.op.spec().effect == Effect::Write {
                self.run_state()
                    .init_scripts
                    .add_context_scripts(self, browser, &admitted)
                    .await?;
            }
            let _ = self.page.set(admitted);
        }
        match self.page.get() {
            Some(admitted) if admitted.tab == tab => {
                // Navigation and frame-tree reads admit directly, without OpPage. Report at
                // this common boundary so every payload names its actual source binding.
                self.env.report_target(tab, &admitted.session.target_id);
                Ok(admitted)
            }
            _ => Err(self.fail(ErrorCode::Internal, "one operation acts on one tab")),
        }
    }

    /// View admission for one record in a batch read. Unlike `admit`, this does not select
    /// the operation's action target or report a single payload source to the host: an event
    /// pump can observe several tabs. Writes still need the separate single-tab admission.
    pub(crate) async fn observe_page(
        &self,
        browser: &BrowserSession,
        tab: TabId,
    ) -> Result<Option<AdmittedPage>, PlaywrightError> {
        let Some(info) = self.known_tab(browser, tab).await? else {
            return Ok(None);
        };
        let session = self
            .view()
            .page_session(info.page_id.clone())
            .await
            .map_err(|error| self.core_error(&error))?;
        // Raw SDK calls bypass core's operation leases. Recover an interrupted earlier
        // resize here, before any page read can observe its rejected metrics. This path
        // takes no lease when the viewport is already applied and leaves dialogs usable.
        self.view()
            .repair_viewport(&info.page_id, &session)
            .await
            .map_err(|error| self.core_error(&error))?;
        self.run_state()
            .events
            .remember_page(&session.target_id, tab);
        Ok(Some(AdmittedPage {
            tab,
            core_page: info.page_id,
            session,
        }))
    }

    /// Admits `tab` for an op the browser carries out on the tab itself, never asking its page:
    /// closing it, naming its opener. The view decides as it does for [`OpScope::admit`]'s page
    /// session, minus the attach: a bound view checks profile and window scope on the browser's
    /// fresh tab facts, and an unbound one (every Neo run's) takes core's entry, as
    /// `PageManager::get_session` does. The host learns the target as it does there. Nothing
    /// attaches, enables a domain, repairs a viewport or adds the run's context scripts, because
    /// each of those waits on the page's renderer: while a committed page's navigation is in
    /// flight, Chromium answers no page command from any session until it commits or fails
    /// (measured on BrowserOS; the class #3317 met for reads), so an attach would hold such an
    /// op to the CDP request timeout. Returns the tab's page target as the view knows it; unlike
    /// `admit`, nothing is kept, since these ops send nothing to the page afterwards.
    pub(crate) async fn admit_target(
        &self,
        browser: &BrowserSession,
        tab: TabId,
    ) -> Result<TargetId, PlaywrightError> {
        let core = |error: CoreError| self.core_error(&error);
        let info = self
            .known_tab(browser, tab)
            .await?
            .ok_or_else(|| self.fail(ErrorCode::TargetClosed, "the tab was closed"))?;
        let bound = self
            .view()
            .scope()
            .map_err(|error| core(error.into()))?
            .is_some();
        let info = if bound {
            self.view().page(info.page_id).await.map_err(core)?
        } else {
            info
        };
        self.run_state().events.remember_page(&info.target_id, tab);
        self.env.report_target(tab, &info.target_id);
        Ok(info.target_id)
    }

    /// Core's cached entry for a native tab, listing the browser's tabs once on a miss: a tab
    /// opened outside this process (by the owner, or before a restart) is unknown until then.
    async fn known_tab(
        &self,
        browser: &BrowserSession,
        tab: TabId,
    ) -> Result<Option<PageInfo>, PlaywrightError> {
        if let Some(info) = browser.pages.get_info_by_tab(tab).await {
            return Ok(Some(info));
        }
        self.view()
            .resolve_tab_ids(&[tab])
            .await
            .map_err(|error| self.core_error(&error))?;
        Ok(browser.pages.get_info_by_tab(tab).await)
    }

    /// The gateway check: refuses, as `neo / internal` with nothing sent, an op whose spec does
    /// not declare `capability`. Each capability's gateway calls it before anything else. A
    /// host holds and polices ops by their declarations, so an op routed to a gateway without
    /// its declaration would otherwise be served unpoliced; here it fails on first use instead.
    ///
    /// Passing the `MainWorld` gateway arms the op's main-world report, which its next
    /// `send_effect` makes. Not here: the gateway runs before the op's input is checked, its
    /// page admitted and its element found, and a script that never went out (a refused script,
    /// a closed tab, an element that never appeared, the owner's takeover) ran nowhere.
    pub(crate) fn require(&self, capability: Capability) -> Result<(), PlaywrightError> {
        if self.op.spec().uses(capability) {
            if capability == Capability::MainWorld {
                self.main_world.store(true, Ordering::Relaxed);
            }
            Ok(())
        } else {
            Err(self.fail(
                ErrorCode::Internal,
                &format!(
                    "the op reached Capability::{capability:?}, which its OpSpec does not declare"
                ),
            ))
        }
    }

    /// Re-runs the host's attempt-stage policies on the op's tab without marking the op sent:
    /// for a write's preparation that changes no document the page has now, a context init
    /// script added at the op's admission. The op's own effect still goes through
    /// [`OpScope::send_effect`].
    pub(crate) fn recheck(&self) -> Result<(), PlaywrightError> {
        self.env.recheck().map_err(PlaywrightError::from)
    }

    /// Tells the host this op is about to send script into a page's main world
    /// (`OpEnv::main_world`). Call it right before the send, once whatever guards the send
    /// passed (its recheck, a binding name's claim), so a send that never goes out reports
    /// nothing. A `MainWorld` gateway's [`OpScope::send_effect`] calls it; a send an op makes
    /// without that gateway's effect calls it itself, as a write's admission does when it adds
    /// the run's context scripts and bindings to its tab (`page/init_script.rs`): that is how a
    /// `goto` or a click that carries `context.exposeBinding`'s stub into its tab is seen,
    /// though its spec declares no `MainWorld`.
    pub(crate) fn sending_main_world(&self) {
        self.env.main_world();
    }

    /// Re-runs the host's attempt-stage policies, then marks the op sent, and returns the
    /// proof every effect takes. Call it right before each attempt's effect: an action that
    /// retries its input re-checks before every attempt, because the owner can take over
    /// between a missed attempt and its retry. A refusal passes through with the policy's
    /// owner and code, and nothing is marked. A main-world send runs the rules on the op's own
    /// tab only, as every other effect does, though its script can reach the tab's opener and
    /// popups.
    pub(crate) fn send_effect(&self) -> Result<Sent<'_>, PlaywrightError> {
        match self.op.spec().effect {
            // `OpSpec` promises hosts that a read never marks itself sent.
            Effect::Read => Err(self.fail(
                ErrorCode::Internal,
                "a read operation cannot change the page",
            )),
            Effect::Write => {
                let sent = Sent::after_recheck(self.env).map_err(PlaywrightError::from)?;
                if self.main_world.load(Ordering::Relaxed) {
                    self.sending_main_world();
                }
                Ok(sent)
            }
        }
    }

    /// Records one call-log line, as Playwright's server logs it (leading spaces nest it under
    /// the line before), and mirrors it to the trace unless it repeats the line before.
    pub(crate) fn log(&self, line: String) {
        let changed = self.lock_log().push(&line);
        // The host is called outside the lock.
        if changed {
            self.env.note(line);
        }
    }

    /// Starts the error's call log over, for the actions upstream's client runs as two calls
    /// (`_withElement`: a selector wait, then the handle's own call): once the element is found,
    /// the wait's lines belong to a call that succeeded, and the error shows the handle's
    /// alone. The trace keeps every line it was already given.
    pub(crate) fn restart_log(&self) {
        self.lock_log().clear();
    }

    /// Sleeps on Rust's clock, never the page's, so a background tab's throttled timers cannot
    /// stall a retry. `Err` once the op's timeout passes or the script aborts it, either of
    /// which may cut the sleep short.
    pub(crate) async fn sleep(&self, wait: Duration) -> Result<(), Expired> {
        let deadline = self.deadline();
        self.unless_aborted(Box::pin(async move {
            let Some(deadline) = deadline else {
                time::sleep(wait).await;
                return Ok(());
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            if wait < remaining {
                time::sleep(wait).await;
                Ok(())
            } else {
                time::sleep(remaining).await;
                Err(Expired)
            }
        }))
        .await
    }

    /// Asks the host to bring `tab` forward before the op sends it input or captures it
    /// ([`OpEnv::bring_forward`]), within the op's timeout. Whatever happens, the op goes on: a
    /// tab left behind is slower, not wrong, and a timeout that passed here is the op's own
    /// timeout, which its next wait reports.
    pub(crate) async fn bring_forward(&self, tab: TabId) {
        let _whatever_happened = self.within(self.env.bring_forward(tab)).await;
    }

    /// Runs `future` until the op's timeout or the script's abort; `Err` if either comes first,
    /// dropping it.
    pub(crate) async fn within<F: Future>(&self, future: F) -> Result<F::Output, Expired> {
        let deadline = self.deadline();
        self.unless_aborted(Box::pin(async move {
            match deadline {
                Some(deadline) => time::timeout_at(deadline, future)
                    .await
                    .map_err(|_elapsed| Expired),
                None => Ok(future.await),
            }
        }))
        .await
    }

    /// `wait`'s own answer, unless the script aborts the op first. An abort already made wins
    /// at once, so a leaf that retries never starts another attempt after it. `wait` is boxed:
    /// the leaf's future stays on the heap, so racing it adds no copy of it to the future of
    /// every leaf that waits (clippy's `large_futures`).
    async fn unless_aborted<T>(
        &self,
        wait: Pin<Box<impl Future<Output = Result<T, Expired>>>>,
    ) -> Result<T, Expired> {
        if self.env.abort_reason().is_some() {
            return Err(Expired);
        }
        match future::select(self.env.aborted(), wait).await {
            Either::Left(((), _)) => Err(Expired),
            Either::Right((waited, _)) => waited,
        }
    }

    /// Playwright's timeout error for `code`, one of the table's timed-out codes:
    /// `locator.click: Timeout 5000ms exceeded.` and the call log. When the wait ended because
    /// the script aborted the op, the abort's error instead ([`OpScope::aborted_error`]).
    pub(crate) fn timed_out(&self, code: ErrorCode) -> PlaywrightError {
        if let Some(aborted) = self.aborted_error() {
            return aborted;
        }
        let millis = self.timeout.map_or(0, |timeout| timeout.as_millis());
        self.fail(code, &format!("Timeout {millis}ms exceeded."))
    }

    /// The op's answer, unless the script aborted it while it ran. Upstream's progress race
    /// rejects an aborted call even when its work would have finished, and a leaf's last steps
    /// do not all wait through this scope (for example, a snapshot's frame stitching), so an op that answered after the abort reports the abort instead. A failure
    /// stands: it is what the op met first.
    pub(crate) fn settle(
        &self,
        answered: Result<Value, PlaywrightError>,
    ) -> Result<Value, PlaywrightError> {
        match answered {
            Ok(value) => self.aborted_error().map_or(Ok(value), Err),
            Err(error) => Err(error),
        }
    }

    /// Upstream's abort of an op (`Progress.abort` and the client's `AbortError`): the call log
    /// gains `operation was aborted: <reason>`, and the error reads `<api>: <reason>` with it.
    /// `None` while the script has not aborted the op. A leaf with a timeout message of its own
    /// asks this first.
    pub(crate) fn aborted_error(&self) -> Option<PlaywrightError> {
        let reason = self.env.abort_reason()?;
        self.log(format!("operation was aborted: {reason}"));
        Some(self.fail(ErrorCode::Aborted, &reason))
    }

    /// `<api>: <message>` and the call log, as Playwright prints a failed call.
    pub(crate) fn fail(&self, code: ErrorCode, message: &str) -> PlaywrightError {
        let log = self.lock_log().render();
        PlaywrightError::new(code, format!("{}: {message}{log}", self.api))
    }

    /// `head` and the call log, for a failed `expect` matcher: Playwright prints those as
    /// `expect(locator).toBeVisible() failed` and their own lines, not as `<api>: <message>`.
    pub(crate) fn fail_matcher(&self, code: ErrorCode, head: &str) -> PlaywrightError {
        let log = self.lock_log().render();
        PlaywrightError::new(code, format!("{head}{log}"))
    }

    /// The call log's lines as recorded, for a failed matcher's `matcherResult.log`.
    pub(crate) fn log_lines(&self) -> Vec<String> {
        self.lock_log().raw_lines()
    }

    /// A core or CDP failure the leaf could not recover from, as one of the crate's codes.
    pub(crate) fn core_error(&self, error: &CoreError) -> PlaywrightError {
        let code = ErrorCode::from_core(error);
        if code == ErrorCode::TargetClosed {
            self.fail(code, "Target page, context or browser has been closed")
        } else {
            self.fail(code, &error.to_string())
        }
    }

    /// `neo / internal`: this op's leaf is not built yet.
    pub(crate) fn not_built(&self) -> PlaywrightError {
        self.fail(ErrorCode::Internal, "not built yet")
    }

    /// When the op's timeout passes. `None` without a timeout, and for one too far away to
    /// name, which a script can ask for and must not crash the op.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.timeout
            .and_then(|timeout| self.started.checked_add(timeout))
    }

    fn lock_log(&self) -> MutexGuard<'_, CallLog> {
        self.log.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Kept in its own module so that nothing outside it can build a [`Sent`]: its field is private
/// to this module, and its one constructor is visible only to `op_scope.rs`, whose only call is
/// in `OpScope::send_effect` (a unit test below checks that). The constructor itself runs
/// `recheck`, then `mark_sent`, so even a second caller could not skip either.
mod sent {
    use std::marker::PhantomData;

    use crate::{OpEnv, Refusal};

    /// Proof that the host re-ran its attempt-stage policies on the op's tab and marked this op
    /// sent. Every function that dispatches input, navigates, edits an element or runs
    /// main-world script takes `&Sent`. Zero-sized; it borrows its `OpScope`, so it cannot
    /// outlive the op.
    #[derive(Debug)]
    pub(crate) struct Sent<'scope> {
        _scope: PhantomData<&'scope ()>,
    }

    impl<'scope> Sent<'scope> {
        /// `recheck`, then `mark_sent`, in the order STYLE P6 requires: a refusal returns before
        /// anything is marked.
        pub(super) fn after_recheck(env: &'scope dyn OpEnv) -> Result<Self, Refusal> {
            env.recheck()?;
            env.mark_sent();
            Ok(Self {
                _scope: PhantomData,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::future::pending;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use browseros_cdp::{CdpError, CdpEvent, TargetedEvents};
    use browseros_core::{BrowserView, CdpConnection, ProtocolSession, SessionId, TabId};
    use futures_util::future::BoxFuture;
    use neo_trace::ErrorOwner;
    use serde_json::{Value, json};
    use tokio::sync::{Notify, broadcast, mpsc};
    use tokio::task;
    use tokio::time::{self, Instant};

    use super::{Expired, OpScope};
    use crate::op::Op;
    use crate::op_spec::Capability;
    use crate::target::CallHeader;
    use crate::test_env::{self, EnvCall, TestEnv};
    use crate::{Answer, ErrorCode, OpEnv, Refusal, RunState, TimedOut};

    fn op(name: &str) -> Result<Op, Box<dyn Error>> {
        Ok(Op::from_name(name).ok_or(format!("{name} is an op"))?)
    }

    fn header(api: Option<&str>, timeout_ms: Option<u64>) -> CallHeader {
        CallHeader {
            api: api.map(str::to_owned),
            timeout_ms,
        }
    }

    #[tokio::test]
    async fn tests_that_send_effect_rechecks_before_it_marks_the_op_sent()
    -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = OpScope::new(&env, op("locator.click")?, CallHeader::default());
        let _first = scope.send_effect()?;
        let _retry = scope.send_effect()?;
        assert_eq!(
            env.calls(),
            [
                EnvCall::Recheck,
                EnvCall::MarkSent,
                EnvCall::Recheck,
                EnvCall::MarkSent
            ],
            "every attempt re-runs the policies, each time right before it is marked"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_refusal_marks_nothing_and_passes_through() -> Result<(), Box<dyn Error>> {
        let refusal = Refusal {
            owner: ErrorOwner::Host,
            code: "owner_has_control".to_owned(),
            message: "The owner is using this tab.".to_owned(),
            host_code: None,
        };
        let env = TestEnv::refusing(refusal.clone());
        let scope = OpScope::new(&env, op("locator.click")?, CallHeader::default());
        let error = scope
            .send_effect()
            .err()
            .ok_or("a refused recheck must fail")?;
        assert_eq!(
            env.calls(),
            [EnvCall::Recheck],
            "nothing is marked sent after a refusal"
        );
        assert_eq!(
            (error.owner(), error.code(), error.message()),
            (
                ErrorOwner::Host,
                "owner_has_control",
                "The owner is using this tab."
            ),
            "the policy's owner, code and message reach the script unchanged"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_read_op_cannot_send() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = OpScope::new(&env, op("locator.textContent")?, CallHeader::default());
        let error = scope
            .send_effect()
            .err()
            .ok_or("a read must not get a Sent")?;
        assert!(
            env.calls().is_empty(),
            "a read neither rechecks nor marks itself sent"
        );
        assert_eq!(error.error_code(), Some(ErrorCode::Internal));
        assert_eq!(
            error.message(),
            "locator.textContent: a read operation cannot change the page"
        );
        Ok(())
    }

    /// A gateway lets an op through only for the capability it declares: one declared
    /// capability does not stand in for another, and a refusal reaches nothing on the host.
    #[tokio::test]
    async fn tests_that_require_passes_only_the_declared_capability() -> Result<(), Box<dyn Error>>
    {
        let env = TestEnv::new();
        let scope = OpScope::new(&env, op("locator.setInputFiles")?, CallHeader::default());
        scope.require(Capability::HostFiles)?;
        let error = scope
            .require(Capability::MainWorld)
            .err()
            .ok_or("locator.setInputFiles declares HostFiles, not MainWorld")?;
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Neo, "internal"),
            "a missing declaration is Neo's bug: {error}"
        );
        assert!(
            error.message().starts_with("locator.setInputFiles: ")
                && error.message().contains("MainWorld"),
            "the refusal names the method and the capability: {error}"
        );
        assert!(
            env.calls().is_empty(),
            "a refused gateway rechecks, marks and notes nothing"
        );
        Ok(())
    }

    /// The `Sent` module's guarantee, checked on this file's source: the constructor is called
    /// once outside the module that defines it, and that call is in `send_effect`.
    #[test]
    fn tests_that_only_send_effect_mints_sent() {
        let source = include_str!("op_scope.rs");
        let code = source.split("#[cfg(test)]").next().unwrap_or_default();
        let outside_module = code.split("mod sent {").next().unwrap_or_default();
        let call = concat!("Sent::", "after_recheck(");
        assert_eq!(
            outside_module.matches(call).count(),
            1,
            "one call site mints Sent"
        );
        let send_effect = outside_module
            .split("fn send_effect(")
            .nth(1)
            .and_then(|body| body.split("\n    pub(crate) fn ").next())
            .unwrap_or_default();
        assert!(
            send_effect.contains(call),
            "the one call site is send_effect"
        );
        assert!(
            code.contains("pub(super) fn after_recheck(") && code.contains("_scope: PhantomData<"),
            "the constructor and the field stay private to the module"
        );
    }

    #[tokio::test]
    async fn tests_that_the_log_mirrors_each_change_to_the_trace() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = OpScope::new(&env, op("locator.click")?, header(Some("page.click"), None));
        scope.log("waiting for getByRole('button')".to_owned());
        scope.log("  element is not visible".to_owned());
        scope.log("  element is not visible".to_owned());
        assert_eq!(
            env.calls(),
            [
                EnvCall::Note("waiting for getByRole('button')".to_owned()),
                EnvCall::Note("  element is not visible".to_owned()),
            ],
            "a repeated line is kept in the log but noted once"
        );
        let error = scope.fail(ErrorCode::NotActionable, "Timeout 5000ms exceeded.");
        assert_eq!(
            error.message(),
            "page.click: Timeout 5000ms exceeded.\nCall log:\n  - waiting for getByRole('button')\n    2 × element is not visible\n"
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_sleep_stops_at_the_op_timeout() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = OpScope::new(&env, op("locator.click")?, header(None, Some(100)));
        let started = Instant::now();
        assert_eq!(scope.sleep(Duration::from_millis(30)).await, Ok(()));
        assert_eq!(scope.sleep(Duration::from_millis(500)).await, Err(Expired));
        assert_eq!(
            started.elapsed(),
            Duration::from_millis(100),
            "cut short at the timeout"
        );
        assert_eq!(
            scope.sleep(Duration::ZERO).await,
            Err(Expired),
            "and expired after it"
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_no_timeout_never_expires() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = OpScope::new(&env, op("locator.click")?, header(None, Some(0)));
        assert_eq!(
            scope.timeout(),
            None,
            "timeout: 0 is no timeout, as in Playwright"
        );
        assert_eq!(scope.sleep(Duration::from_secs(3600)).await, Ok(()));
        assert_eq!(scope.within(async { 7 }).await, Ok(7));
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_within_drops_a_future_at_the_timeout() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = OpScope::new(&env, op("page.goto")?, header(None, Some(30_000)));
        let fast = scope.within(time::sleep(Duration::from_secs(1))).await;
        assert_eq!(fast, Ok(()));
        let slow = scope.within(time::sleep(Duration::from_secs(60))).await;
        assert_eq!(slow, Err(Expired));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_timeout_reads_as_playwrights() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = OpScope::new(&env, op("locator.click")?, header(None, Some(5000)));
        scope.log("waiting for locator('aria-ref=e7')".to_owned());
        let error = scope.timed_out(ErrorCode::NoMatch);
        assert_eq!(
            error.message(),
            "locator.click: Timeout 5000ms exceeded.\nCall log:\n  - waiting for locator('aria-ref=e7')\n",
            "the shape real Playwright printed in the eval"
        );
        assert_eq!(
            (error.owner(), error.timed_out(), error.answer()),
            (ErrorOwner::Script, TimedOut::Yes, Answer::Ambiguous)
        );
        Ok(())
    }

    /// Paused: core waits out its own attach timeout before it gives up on the page.
    #[tokio::test(start_paused = true)]
    async fn tests_that_admitting_a_page_without_a_browser_names_the_cause()
    -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = OpScope::new(&env, op("locator.click")?, CallHeader::default());
        let error = scope
            .admit(&test_env::offline_session(), TabId(7))
            .await
            .err()
            .ok_or("no browser, no page")?;
        assert_eq!(
            error.error_code(),
            Some(ErrorCode::BrowserDisconnected),
            "an unreachable browser is Neo's, not the script's"
        );
        assert_eq!(error.message(), "locator.click: CDP not connected");
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_handler_that_is_not_built_says_so() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = OpScope::new(
            &env,
            op("locator.click")?,
            header(Some("frame.click"), None),
        );
        let error = scope.not_built();
        assert_eq!(
            error.message(),
            "frame.click: not built yet",
            "messages start with the method the script called"
        );
        assert_eq!((error.owner(), error.code()), (ErrorOwner::Neo, "internal"));
        Ok(())
    }

    /// Records every CDP send and answers each with `{}`.
    #[derive(Default)]
    struct Recording {
        sent: Mutex<Vec<(String, Value, Option<SessionId>)>>,
        targeted: TargetedEvents,
    }

    impl CdpConnection for Recording {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            self.sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((method.to_owned(), params, session.cloned()));
            Box::pin(async { Ok(json!({})) })
        }

        fn send_at_epoch<'a>(
            &'a self,
            expected_epoch: u64,
            method: &'a str,
            params: Value,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            if expected_epoch != self.connection_epoch() {
                return Box::pin(async {
                    Err(CdpError::Transport("fixture epoch changed".into()))
                });
            }
            self.send(method, params, session)
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_owned()) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            broadcast::channel(1).1
        }

        fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
            self.targeted.subscribe(method)
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    #[tokio::test]
    async fn tests_that_dropping_the_scope_releases_its_object_group_on_every_session()
    -> Result<(), Box<dyn Error>> {
        let recording = Arc::new(Recording::default());
        let connection: Arc<dyn CdpConnection> = Arc::clone(&recording) as Arc<dyn CdpConnection>;
        let first = ProtocolSession::for_session(Arc::clone(&connection), SessionId::from("A"));
        let second = ProtocolSession::for_session(connection, SessionId::from("B"));
        let env = TestEnv::new();
        let scope = OpScope::new(&env, op("locator.click")?, CallHeader::default());
        let group = scope.object_group(&first).to_owned();
        assert_eq!(scope.object_group(&first), group, "one group per op");
        assert_eq!(scope.object_group(&second), group);
        let other = OpScope::new(&env, op("locator.click")?, CallHeader::default());
        assert_ne!(
            other.object_group(&first),
            group,
            "another op's release must not free this op's objects"
        );
        drop(other);
        drop(scope);
        for _ in 0..10 {
            task::yield_now().await;
        }
        let released: Vec<(String, Option<SessionId>)> = recording
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(method, params, _)| {
                method == "Runtime.releaseObjectGroup" && params["objectGroup"] == group.as_str()
            })
            .map(|(_, params, session)| (params["objectGroup"].to_string(), session.clone()))
            .collect();
        assert_eq!(
            released
                .iter()
                .map(|(_, session)| session.clone())
                .collect::<Vec<_>>(),
            [Some(SessionId::from("A")), Some(SessionId::from("B"))],
            "released once on each session that holds one of the op's objects"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_releasing_the_objects_frees_the_group_now_and_keeps_it()
    -> Result<(), Box<dyn Error>> {
        let recording = Arc::new(Recording::default());
        let connection: Arc<dyn CdpConnection> = Arc::clone(&recording) as Arc<dyn CdpConnection>;
        let session = ProtocolSession::for_session(connection, SessionId::from("A"));
        let env = TestEnv::new();
        let scope = OpScope::new(&env, op("locator.waitForFunction")?, CallHeader::default());
        let group = scope.object_group(&session).to_owned();
        let releases = || {
            recording
                .sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .filter(|(method, params, session)| {
                    method == "Runtime.releaseObjectGroup"
                        && params["objectGroup"] == group.as_str()
                        && *session == Some(SessionId::from("A"))
                })
                .count()
        };
        scope.release_objects().await;
        assert_eq!(releases(), 1, "released at once, before the op ends");
        assert_eq!(
            scope.object_group(&session),
            group,
            "the next try's objects join the same group"
        );
        drop(scope);
        for _ in 0..10 {
            task::yield_now().await;
        }
        assert_eq!(releases(), 2, "and the op's end releases them too");
        Ok(())
    }

    #[tokio::test]
    async fn an_unknown_tab_lists_once_and_sends_nothing() -> Result<(), Box<dyn Error>> {
        use browseros_core::testing::TestConnection;
        use browseros_core::{BrowserSession, BrowserSessionHooks};
        let connection =
            TestConnection::new([("Browser.getTabs", serde_json::json!({ "tabs": [] }))]);
        let browser = BrowserSession::new(
            Arc::clone(&connection) as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        );
        let env = TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None));
        let scope = OpScope::new(&env, op("locator.click")?, CallHeader::default());
        let error = scope
            .admit(&browser, TabId(999))
            .await
            .err()
            .ok_or("unknown tab must fail")?;
        assert_eq!(error.error_code(), Some(ErrorCode::TargetClosed));
        assert_eq!(connection.calls()?.len(), 1, "one listing on a cache miss");
        assert!(
            !env.calls().contains(&EnvCall::MarkSent),
            "no input before admission"
        );
        Ok(())
    }

    /// A host whose script can abort the op (Playwright's `signal`), as claw-sdk's door does
    /// through `OpCtx::aborted`.
    struct AbortingHost {
        env: TestEnv,
        reason: Mutex<Option<String>>,
        wake: Notify,
    }

    impl AbortingHost {
        fn new() -> Self {
            Self {
                env: TestEnv::new(),
                reason: Mutex::new(None),
                wake: Notify::new(),
            }
        }

        fn abort(&self, reason: &str) {
            *self.reason.lock().unwrap_or_else(PoisonError::into_inner) = Some(reason.to_owned());
            self.wake.notify_waiters();
        }
    }

    impl OpEnv for AbortingHost {
        fn view(&self) -> &BrowserView {
            self.env.view()
        }

        fn run_state(&self) -> &Arc<RunState> {
            self.env.run_state()
        }

        fn recheck(&self) -> Result<(), Refusal> {
            self.env.recheck()
        }

        fn mark_sent(&self) {
            self.env.mark_sent();
        }

        fn note(&self, line: String) {
            self.env.note(line);
        }

        fn abort_reason(&self) -> Option<String> {
            self.reason
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        fn aborted(&self) -> BoxFuture<'_, ()> {
            Box::pin(async move {
                loop {
                    let woken = self.wake.notified();
                    if self.abort_reason().is_some() {
                        return;
                    }
                    woken.await;
                }
            })
        }
    }

    /// Upstream's `Progress.abort`: the wait in flight ends, and the op's timeout path reports
    /// the abort, with the reason and an `operation was aborted` line under the call log.
    #[tokio::test(start_paused = true)]
    async fn tests_that_an_abort_ends_the_wait_in_flight_as_the_ops_abort()
    -> Result<(), Box<dyn Error>> {
        let host = AbortingHost::new();
        let scope = OpScope::new(&host, op("locator.click")?, header(None, None));
        scope.log("waiting for locator('button')".to_owned());
        let (waited, ()) = tokio::join!(scope.within(pending::<()>()), async {
            time::sleep(Duration::from_millis(500)).await;
            host.abort("foo bar");
        });
        assert_eq!(waited, Err(Expired), "an op with no timeout still ends");
        let error = scope.timed_out(ErrorCode::NotActionable);
        assert_eq!(error.error_code(), Some(ErrorCode::Aborted));
        assert_eq!(
            (error.owner(), error.timed_out(), error.answer()),
            (ErrorOwner::Script, TimedOut::No, Answer::Ambiguous),
            "the script chose to stop it, and input it sent may have landed"
        );
        assert_eq!(
            error.message(),
            "locator.click: foo bar\nCall log:\n  - waiting for locator('button')\n  - \
             operation was aborted: foo bar\n"
        );
        Ok(())
    }

    /// An op aborted between attempts never sleeps or waits again.
    #[tokio::test(start_paused = true)]
    async fn tests_that_an_aborted_op_waits_no_more() -> Result<(), Box<dyn Error>> {
        let host = AbortingHost::new();
        let scope = OpScope::new(&host, op("locator.click")?, header(None, Some(5_000)));
        host.abort("already");
        let started = Instant::now();
        assert_eq!(scope.sleep(Duration::from_secs(1)).await, Err(Expired));
        assert_eq!(scope.within(async {}).await, Err(Expired));
        assert_eq!(Instant::now(), started, "neither waited");
        Ok(())
    }

    /// An op whose last step finished after the abort reports the abort, not its value.
    #[tokio::test]
    async fn tests_that_an_answer_after_the_abort_settles_as_the_abort()
    -> Result<(), Box<dyn Error>> {
        let host = AbortingHost::new();
        let scope = OpScope::new(&host, op("locator.ariaSnapshot")?, header(None, None));
        assert_eq!(scope.settle(Ok(json!("before")))?, json!("before"));
        host.abort("stop");
        let error = scope
            .settle(Ok(json!("after")))
            .err()
            .ok_or("an answer after the abort must fail")?;
        assert_eq!(error.error_code(), Some(ErrorCode::Aborted));
        assert!(error.message().starts_with("locator.ariaSnapshot: stop"));
        let failed = scope.fail(ErrorCode::NoMatch, "gone");
        let settled = scope
            .settle(Err(failed))
            .err()
            .ok_or("a failure stays a failure")?;
        assert_eq!(
            settled.error_code(),
            Some(ErrorCode::NoMatch),
            "a failure stands"
        );
        Ok(())
    }

    /// Without an abort the timeout path reads as before, and the default host never aborts.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_timeout_stays_a_timeout_without_an_abort() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let scope = OpScope::new(&env, op("locator.click")?, header(None, Some(100)));
        assert_eq!(scope.within(pending::<()>()).await, Err(Expired));
        let error = scope.timed_out(ErrorCode::NotActionable);
        assert_eq!(error.error_code(), Some(ErrorCode::NotActionable));
        assert_eq!(error.message(), "locator.click: Timeout 100ms exceeded.");
        Ok(())
    }
}
