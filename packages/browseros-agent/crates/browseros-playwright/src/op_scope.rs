//! One operation in flight, and the proof a leaf must hold to change the page.
//!
//! ```text
//!  Playwright::invoke ── decode `call` ── OpScope::new ── family leaf
//!                                            │ admit(page)    once per op, through the run's view
//!                                            │ log(line)      Playwright's call log, mirrored to the trace
//!                                            │ sleep, within  Rust's clock against the op's own timeout
//!                                            │ require(cap)   at a capability's gateway: declared?
//!                                            └ send_effect()  recheck, then mark_sent ──▶ Sent
//!                                                             └▶ input, navigation, element edits,
//!                                                                main-world script: each takes &Sent
//! ```
//!
//! Delivery is derived from one fact, whether the op was marked sent before its first write
//! (skeleton D7.5), so that fact is not left to convention. The host's env is private to the
//! scope, [`OpScope::send_effect`] is the only code that reaches `OpEnv::mark_sent`, it always
//! re-runs the host's attempt-stage policies first, and the [`Sent`] it returns is the only
//! way to call a function that changes the page. A leaf that never calls `send_effect`, which
//! is every read, can neither mark itself sent nor reach an effect.
//!
//! The scope also owns the op's CDP object group: every remote object the op asks for (an
//! iframe it hops through) lives in it, and dropping the scope releases it, whether the op
//! returned or the host dropped its future.

use std::future::Future;
use std::mem;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use browseros_core::pages::PageSession;
use browseros_core::{BrowserView, CoreError, PageId, ProtocolSession};
use browseros_trace::Effect;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::time::{self, Instant};

use crate::call_log::CallLog;
use crate::op::Op;
use crate::op_spec::Capability;
use crate::target::CallHeader;
use crate::{ErrorCode, OpEnv, PlaywrightError, RunState};

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
}

/// A page admitted for one op through the run's view: the profile and window scope check ran
/// once, and the op's calls go to this session.
#[derive(Debug)]
pub(crate) struct AdmittedPage {
    pub(crate) id: PageId,
    pub(crate) session: PageSession,
}

/// Names the op's objects so they go together: one group per op, released on every session
/// that holds one of them when the scope drops.
struct ObjectGroup {
    /// `browseros-playwright-op-<n>`, unique in the process.
    name: String,
    /// The sessions that hold an object in the group. A std mutex, never held across an await.
    sessions: Mutex<Vec<ProtocolSession>>,
}

/// Numbers the ops' object groups.
static NEXT_GROUP: AtomicU64 = AtomicU64::new(1);

impl ObjectGroup {
    fn new() -> Self {
        Self {
            name: format!(
                "browseros-playwright-op-{}",
                NEXT_GROUP.fetch_add(1, Ordering::Relaxed)
            ),
            sessions: Mutex::default(),
        }
    }
}

/// Releases the group with the op, cancelled or not. The release is spawned, so it runs after
/// a dropped future too; it is idempotent, and a session that is gone has nothing left to free.
impl Drop for ObjectGroup {
    fn drop(&mut self) {
        let sessions =
            mem::take(&mut *self.sessions.lock().unwrap_or_else(PoisonError::into_inner));
        if sessions.is_empty() {
            return;
        }
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        let name = mem::take(&mut self.name);
        drop(runtime.spawn(async move {
            for session in sessions {
                let _released = session
                    .send::<_, Value>("Runtime.releaseObjectGroup", json!({ "objectGroup": name }))
                    .await;
            }
        }));
    }
}

/// The op's own Playwright timeout passed. The leaf turns it into its timed-out code with
/// [`OpScope::timed_out`].
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
        }
    }

    /// The object group for remote objects this op asks `session` for. Naming it registers
    /// the session, so the group is released there when the op ends.
    pub(crate) fn object_group(&self, session: &ProtocolSession) -> &str {
        let mut sessions = self
            .objects
            .sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !sessions.iter().any(|known| known.same_session(session)) {
            sessions.push(session.clone());
        }
        &self.objects.name
    }

    /// The op's Playwright timeout; `None` waits until the host drops the op.
    pub(crate) fn timeout(&self) -> Option<Duration> {
        self.timeout
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

    /// Admits `page` for this op through the run's view, the first time only. One op acts on
    /// one page; a second one is a Neo bug.
    pub(crate) async fn admit(&self, page: &PageId) -> Result<&AdmittedPage, PlaywrightError> {
        if self.page.get().is_none() {
            let session = self
                .env
                .view()
                .page_session(page.clone())
                .await
                .map_err(|error| self.core_error(&error))?;
            // A concurrent admit inside the same op may have won; the check below decides.
            let _ = self.page.set(AdmittedPage {
                id: page.clone(),
                session,
            });
        }
        match self.page.get() {
            Some(admitted) if admitted.id == *page => Ok(admitted),
            _ => Err(self.fail(ErrorCode::Internal, "one operation acts on one page")),
        }
    }

    /// The gateway check: refuses, as `neo / internal` with nothing sent, an op whose spec does
    /// not declare `capability`. Each capability's gateway calls it before anything else. A
    /// host holds and polices ops by their declarations, so an op routed to a gateway without
    /// its declaration would otherwise be served unpoliced; here it fails on first use instead.
    pub(crate) fn require(&self, capability: Capability) -> Result<(), PlaywrightError> {
        if self.op.spec().uses(capability) {
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

    /// Re-runs the host's attempt-stage policies, then marks the op sent, and returns the
    /// proof every effect takes. Call it right before each attempt's effect: an action that
    /// retries its input re-checks before every attempt, because the owner can take over
    /// between a missed attempt and its retry. A refusal passes through with the policy's
    /// owner and code, and nothing is marked.
    pub(crate) fn send_effect(&self) -> Result<Sent<'_>, PlaywrightError> {
        match self.op.spec().effect {
            // `OpSpec` promises hosts that a read never marks itself sent.
            Effect::Read => Err(self.fail(
                ErrorCode::Internal,
                "a read operation cannot change the page",
            )),
            Effect::Write => Sent::after_recheck(self.env).map_err(PlaywrightError::from),
        }
    }

    /// The op's target is a password, card or one-time-code field: the trace redacts its
    /// sensitive arguments.
    pub(crate) fn mark_sensitive_target(&self) {
        self.env.mark_sensitive_target();
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

    /// Sleeps on Rust's clock, never the page's, so a background tab's throttled timers cannot
    /// stall a retry. `Err` once the op's timeout passes, which may cut the sleep short.
    pub(crate) async fn sleep(&self, wait: Duration) -> Result<(), Expired> {
        let Some(deadline) = self.deadline() else {
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
    }

    /// Runs `future` until the op's timeout; `Err` if the timeout comes first, dropping it.
    pub(crate) async fn within<F: Future>(&self, future: F) -> Result<F::Output, Expired> {
        match self.deadline() {
            Some(deadline) => time::timeout_at(deadline, future)
                .await
                .map_err(|_elapsed| Expired),
            None => Ok(future.await),
        }
    }

    /// Playwright's timeout error for `code`, one of the table's timed-out codes:
    /// `locator.click: Timeout 5000ms exceeded.` and the call log.
    pub(crate) fn timed_out(&self, code: ErrorCode) -> PlaywrightError {
        let millis = self.timeout.map_or(0, |timeout| timeout.as_millis());
        self.fail(code, &format!("Timeout {millis}ms exceeded."))
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

    /// A core or CDP failure the leaf could not recover from, as one of the crate's codes.
    pub(crate) fn core_error(&self, error: &CoreError) -> PlaywrightError {
        self.fail(ErrorCode::from_core(error), &error.to_string())
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
/// `recheck` and then `mark_sent`, so even a second caller could not skip either.
mod sent {
    use std::marker::PhantomData;

    use crate::{OpEnv, Refusal};

    /// Proof that the host re-ran its attempt-stage policies and marked this op sent. Every
    /// function that dispatches input, navigates, edits an element or runs main-world script
    /// takes `&Sent`. Zero-sized; it borrows its `OpScope`, so it cannot outlive the op.
    #[derive(Debug)]
    pub(crate) struct Sent<'scope> {
        _scope: PhantomData<&'scope ()>,
    }

    impl<'scope> Sent<'scope> {
        /// `recheck`, then `mark_sent`, in the order STYLE P6 requires: a refusal returns
        /// before anything is marked.
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
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use browseros_cdp::{CdpError, CdpEvent, TargetedEvents};
    use browseros_core::{CdpConnection, PageId, ProtocolSession, SessionId};
    use browseros_trace::ErrorOwner;
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use tokio::sync::{broadcast, mpsc};
    use tokio::task;
    use tokio::time::{self, Instant};

    use super::{Expired, OpScope};
    use crate::op::Op;
    use crate::op_spec::Capability;
    use crate::target::CallHeader;
    use crate::test_env::{EnvCall, TestEnv};
    use crate::{Answer, ErrorCode, Refusal, TimedOut};

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
        let scope = OpScope::new(&env, op("page.screenshot")?, CallHeader::default());
        scope.require(Capability::Capture)?;
        let error = scope
            .require(Capability::MainWorld)
            .err()
            .ok_or("page.screenshot declares Capture, not MainWorld")?;
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Neo, "internal"),
            "a missing declaration is Neo's bug: {error}"
        );
        assert!(
            error.message().starts_with("page.screenshot: ")
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

    #[tokio::test]
    async fn tests_that_marking_a_sensitive_target_reaches_the_host() -> Result<(), Box<dyn Error>>
    {
        let env = TestEnv::new();
        let scope = OpScope::new(&env, op("locator.fill")?, CallHeader::default());
        scope.mark_sensitive_target();
        assert_eq!(env.calls(), [EnvCall::MarkSensitiveTarget]);
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
            .admit(&PageId(7))
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
}
