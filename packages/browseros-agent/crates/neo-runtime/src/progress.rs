//! Neo: One browser call's progress, Playwright's `server/progress.ts` for Neo: the call's own
//! deadline, the run ending under it, the tab closing, its call log and its delivery cell.
//!
//! A call's durable state lives in a [`CallEntry`] that the run's [`Trace`] holds, never in the
//! realm and never only in the call's task. Three things can end a call early and none may lose
//! its record: the run deadline drops the realm, the script returning drops it too, and the
//! end-of-run wait can leave a task running past the run. So the record is derived from the
//! entry by its gated publication, never assembled by whoever ended the call. The outcome
//! reads the same sealed result with the delivery known after the drain; late replies cannot
//! rewrite a sealed call, but acknowledged writes must still reach its outcome.
//!
//! ```text
//!   dispatch ──creates──▶ CallEntry ◀──holds── Trace (run)          read once, after `finish`'s wait
//!       │                    ▲   ▲
//!       └──▶ Progress ───────┘   └── DeliveryCell ◀── InputTicket / AckedInput (input writer)
//!              race(work): run deadline or cancel │ run ended │ aborted │ tab closed │ own deadline
//! ```
//!
//! An abort (`abort.rs`) is the script's `signal` reaching the call: the dispatcher's registry
//! finds the call's entry by its message id and aborts it there, so the `Progress` and the record
//! read the same reason.

use std::borrow::Cow;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(test)]
use std::thread;
use std::time::{Duration, SystemTime};
use std::{fmt, future};

use neo_runtime_spec::ParamShape;
use serde_json::Value;
use tokio::time;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::info;

use crate::abort::CallAbort;
use crate::artifacts::Artifacts;
use crate::browser_representations::bindings::Bindings;
use crate::browser_representations::clock::Clock;
use crate::browser_representations::init_scripts::InitScripts;
use crate::deadline::{Deadline, Stop};
use crate::playwright_methods::evaluate::Handles;
use crate::run::{CallLocation, CallRecord, CallResult, Delivery, DroppedCalls, RunEnd};
use crate::trace;
use crate::trace::redact::{self, Scrubber};
use crate::trace::{PageRef, RunEvent, RunTap};

/// How a call ends early: a Playwright error the script sees, or the run ending under it.
/// Every handler returns this; `?` on [`Progress::race`] is how a handler gives up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallError {
    Playwright(PlaywrightError),
    /// The run ended under the call; no realm is left to reply to.
    RunEnded(RunEnd),
}

/// The error a call replies with: Playwright's wire `{name, message}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlaywrightError {
    pub(crate) name: ErrorName,
    pub(crate) message: String,
}

/// The wire error names Neo replies with. The SDK rebuilds `TimeoutError` and `AbortError`
/// under those names and `TargetClosedError` with name `Error`, as Playwright's client does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorName {
    Error,
    TimeoutError,
    TargetClosedError,
    AbortError,
}

impl ErrorName {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::TimeoutError => "TimeoutError",
            Self::TargetClosedError => "TargetClosedError",
            Self::AbortError => "AbortError",
        }
    }
}

impl PlaywrightError {
    /// `Error` with `message`: invalid params, an unserved method, a strict mode violation, a
    /// page exception.
    pub(crate) fn error(message: impl Into<String>) -> Self {
        Self {
            name: ErrorName::Error,
            message: message.into(),
        }
    }

    /// Playwright's `TimeoutError` (`progress.ts`): `Timeout ${timeout}ms exceeded.`, naming the
    /// call's own `metadata.timeout` printed as JS prints a number.
    pub(crate) fn timeout(timeout_ms: f64) -> Self {
        Self {
            name: ErrorName::TimeoutError,
            message: format!("Timeout {}ms exceeded.", js_number(timeout_ms)),
        }
    }

    /// Playwright's server `AbortError` for an aborted call (`dispatcher.ts`,
    /// `new AbortError(params.reason)`): the message is the reason the client sent, the abort
    /// reason's `message` or its `String(…)`.
    pub(crate) fn aborted(reason: &str) -> Self {
        Self {
            name: ErrorName::AbortError,
            message: reason.to_string(),
        }
    }

    /// Playwright's `TargetClosedError` with its default message.
    pub(crate) fn target_closed() -> Self {
        Self {
            name: ErrorName::TargetClosedError,
            message: "Target page, context or browser has been closed".to_string(),
        }
    }

    /// The record of a call that replied with this error.
    pub(crate) fn to_result(&self) -> CallResult {
        CallResult::Error {
            name: self.name.as_str().to_string(),
            message: self.message.clone(),
        }
    }
}

impl From<PlaywrightError> for CallError {
    fn from(error: PlaywrightError) -> Self {
        Self::Playwright(error)
    }
}

/// What every call of a run shares about the run itself.
#[derive(Clone)]
pub(crate) struct RunScope {
    /// The run's deadline and its caller's cancellation, the same `Deadline` the realm uses.
    pub(crate) deadline: Deadline,
    /// Cancelled by `Dispatcher::finish`, or by the last `Dispatcher` dropping.
    pub(crate) ended: CancellationToken,
    /// Set once, before `ended` is cancelled, so every call reports the same reason. Unset when
    /// `ended` fired because the dispatcher dropped: read as [`RunEnd::Cancelled`]. The run's
    /// tap shares the cell, so a call cut off after the run is recorded with the same reason.
    end_reason: Arc<OnceLock<RunEnd>>,
    /// The run's call tasks, every task they start, and the run-end input releases
    /// (`Tabs::release_input`). `Dispatcher::finish` waits on it, bounded, so records read
    /// final deliveries; `Runtime::drain` waits for what is left.
    pub(crate) tasks: TaskTracker,
    /// The run's JS handles (`evaluate.rs`); closed at run end, which releases their groups.
    pub(crate) handles: Arc<Handles>,
    /// The run's init scripts (`init_scripts.rs`); removed from every session at run end.
    pub(crate) init_scripts: Arc<InitScripts>,
    /// Cancelled when the script closes the run's context (`page_and_context_methods.rs` `close_context`):
    /// every call still running stops at its next await or check, as Playwright's calls stop
    /// with its connection, and none writes input after.
    pub(crate) context_closed: CancellationToken,
    /// Where the run's `path` options write, and what they wrote (`artifacts.rs`).
    pub(crate) artifacts: Arc<Artifacts>,
    /// The run's exposed bindings (`bindings.rs`); ended in every frame at run end.
    pub(crate) bindings: Arc<Bindings>,
    /// The run's fake clock (`clock.rs`); uninstalled in every frame at run end.
    pub(crate) clock: Arc<Clock>,
}

impl RunScope {
    pub(crate) fn new(
        deadline: Deadline,
        tasks: TaskTracker,
        end_reason: Arc<OnceLock<RunEnd>>,
    ) -> Self {
        Self {
            deadline,
            ended: CancellationToken::new(),
            end_reason,
            tasks,
            handles: Arc::default(),
            init_scripts: Arc::default(),
            context_closed: CancellationToken::new(),
            artifacts: Arc::default(),
            bindings: Arc::default(),
            clock: Arc::default(),
        }
    }

    /// The run's artifact directory and its record of written files.
    pub(crate) fn with_artifacts(self, artifacts: Arc<Artifacts>) -> Self {
        Self { artifacts, ..self }
    }

    /// Records `reason` (the first one wins) and cancels `ended`.
    pub(crate) fn end(&self, reason: RunEnd) {
        let _first = self.end_reason.set(reason);
        self.ended.cancel();
    }

    fn end_reason(&self) -> RunEnd {
        self.end_reason.get().copied().unwrap_or(RunEnd::Cancelled)
    }
}

fn run_end(stop: Stop) -> RunEnd {
    match stop {
        Stop::Timeout => RunEnd::Timeout,
        Stop::Cancelled => RunEnd::Cancelled,
    }
}

/// The live handle a handler gets for one call. Narrow on purpose: a handler races and waits,
/// logs, and reaches the delivery cell; spawning belongs to whoever owns the session.
pub(crate) struct Progress {
    entry: Arc<CallEntry>,
    /// `metadata.timeout` in ms exactly as the client sent it, for the timeout message and for
    /// Playwright's backoff caps; only `0.0` means none.
    timeout_ms: f64,
    /// When this call times out on its own. `None` for no timeout, and also when its timeout
    /// reaches the run's deadline: then only the run can stop it, so the two deadlines never
    /// fire together and the outcome is always the run's `Timeout`.
    own_deadline: Option<Instant>,
    run: RunScope,
    /// Cancelled when the tab closes.
    page_closed: CancellationToken,
    /// The run's [`RunScope::context_closed`], or, for the context's own close, a token never
    /// cancelled: that call finishes its acknowledgement after stopping the others.
    context_closed: CancellationToken,
}

impl Progress {
    pub(crate) fn new(
        entry: Arc<CallEntry>,
        timeout_ms: f64,
        run: RunScope,
        page_closed: CancellationToken,
    ) -> Self {
        // Playwright's `deadline = timeout ? now + timeout : 0` (`progress.ts` `run`): only zero
        // waits forever, and a negative timeout is a deadline already past, so the call fails
        // at its first wait with `Timeout -1ms exceeded.`.
        let own_deadline = if timeout_ms == 0.0 {
            None
        } else if timeout_ms < 0.0 {
            Some(Instant::now())
        } else {
            // A timeout too large for a `Duration` is past the run's deadline anyway.
            Duration::try_from_secs_f64(timeout_ms / 1000.0)
                .ok()
                .and_then(|timeout| Instant::now().checked_add(timeout))
        }
        .filter(|at| *at < run.deadline.at());
        let context_closed = run.context_closed.clone();
        Self {
            entry,
            timeout_ms,
            own_deadline,
            run,
            page_closed,
            context_closed,
        }
    }

    /// The context's own close: it outlives the stop it gives every other call of the run.
    #[must_use]
    pub(crate) fn closing_the_context(mut self) -> Self {
        self.context_closed = CancellationToken::new();
        self
    }

    /// Stops every other call of the run (the context closed): each ends as its tab closing
    /// would, at its next await or check.
    pub(crate) fn stop_the_runs_calls(&self) {
        self.run.context_closed.cancel();
    }

    /// What every call of the run shares about the run itself.
    pub(crate) fn run_scope(&self) -> &RunScope {
        &self.run
    }
}

impl Progress {
    /// Runs `work` until it finishes or the call must stop, whichever is first. The `biased`
    /// order is the precedence: the run's deadline or cancellation, then the run ending
    /// otherwise, then the script aborting the call (`AbortError`), then the tab or the run's
    /// context closing (`TargetClosedError`), then the call's own deadline (`TimeoutError`).
    /// Every `work` passed in must be cancel-safe: dropping it may lose only its answer, never
    /// leave the page half-changed. An abort ends `work` exactly as a timeout does, so whatever
    /// a timed-out call cleans up, an aborted one cleans up too.
    pub(crate) async fn race<T>(&self, work: impl Future<Output = T>) -> Result<T, CallError> {
        let own_deadline = async {
            match self.own_deadline {
                Some(at) => time::sleep_until(at).await,
                None => future::pending().await,
            }
        };
        tokio::select! {
            biased;
            stop = self.run.deadline.reached() => Err(CallError::RunEnded(run_end(stop))),
            () = self.run.ended.cancelled() => Err(CallError::RunEnded(self.run.end_reason())),
            error = self.entry.abort.aborted() => Err(error.into()),
            () = self.page_closed.cancelled() => Err(PlaywrightError::target_closed().into()),
            () = self.context_closed.cancelled() => Err(PlaywrightError::target_closed().into()),
            () = own_deadline => Err(PlaywrightError::timeout(self.timeout_ms).into()),
            value = work => Ok(value),
        }
    }

    /// Playwright's `progress.wait(ms)`: a backoff sleep that ends early exactly as
    /// [`Self::race`] would.
    pub(crate) async fn wait(&self, delay: Duration) -> Result<(), CallError> {
        self.race(time::sleep(delay)).await
    }

    /// The checks of [`Self::race`] without waiting, for right before an irreversible step, so
    /// no input starts after the call has to stop.
    pub(crate) fn check(&self) -> Result<(), CallError> {
        if let Some(stop) = self.run_stop() {
            return Err(stop);
        }
        self.pending_abort()?;
        if self.page_closed.is_cancelled() || self.context_closed.is_cancelled() {
            return Err(PlaywrightError::target_closed().into());
        }
        if self.own_deadline.is_some_and(|at| Instant::now() >= at) {
            return Err(PlaywrightError::timeout(self.timeout_ms).into());
        }
        Ok(())
    }

    /// The run-level stops, first in [`Self::race`]'s precedence: the run's deadline or
    /// cancellation, then the run ending otherwise. Each outranks the script's abort.
    fn run_stop(&self) -> Option<CallError> {
        if let Some(stop) = self.run.deadline.stop() {
            return Some(CallError::RunEnded(run_end(stop)));
        }
        if self.run.ended.is_cancelled() {
            return Some(CallError::RunEnded(self.run.end_reason()));
        }
        None
    }

    /// The call's `AbortError` if the script aborted it already, unless the run has stopped,
    /// which outranks it. The dispatcher asks before the handler starts, as Playwright's `run`
    /// throws `_pendingAbortError` before its task: an abort can arrive between the call's
    /// admission and its task's first poll. When the run has stopped as well, the handler starts
    /// as any call first polled after the run's end does, and its first race reports the end.
    pub(crate) fn pending_abort(&self) -> Result<(), CallError> {
        if self.run_stop().is_some() {
            return Ok(());
        }
        match self.entry.abort.error() {
            Some(error) => Err(error.into()),
            None => Ok(()),
        }
    }

    /// A token cancelled when the script aborts this call, for a wait that runs without the
    /// call's `Progress` in hand (a task the call spawned, a watcher it waits on). Whoever holds
    /// it selects on it and stops; the call itself still replies through its own `race`.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "slice S's seam for waits without a Progress (coordination.md Seams); no served wait needs it yet"
        )
    )]
    pub(crate) fn abort_token(&self) -> CancellationToken {
        self.entry.abort.token()
    }

    /// Appends one raw call-log line; leading spaces are Playwright's nesting.
    pub(crate) fn log(&self, line: impl Into<String>) {
        let line = line.into();
        self.entry.update_log(|log| log.push(line));
    }

    /// Sets the log's first line to `waiting for <locator>`. Playwright logs it before anything
    /// else; Neo may learn the locator's description only after the page answers, so the line
    /// is replaced, not appended.
    pub(crate) fn name_locator(&self, locator: &str) {
        let header = format!("waiting for {locator}");
        self.entry.update_log(|log| log.header = Some(header));
    }

    /// `metadata.timeout` in ms as the client sent it, `0.0` for none: Playwright caps loop A's
    /// backoff by it.
    /// The run's JS handles, for the evaluation calls (`evaluate.rs`).
    pub(crate) fn handles(&self) -> &Handles {
        &self.run.handles
    }

    /// The run's init scripts (`init_scripts.rs`).
    pub(crate) fn init_scripts(&self) -> &Arc<InitScripts> {
        &self.run.init_scripts
    }

    /// The run's exposed bindings (`bindings.rs`).
    pub(crate) fn bindings(&self) -> &Arc<Bindings> {
        &self.run.bindings
    }

    /// The run's fake clock (`clock.rs`).
    pub(crate) fn clock(&self) -> &Arc<Clock> {
        &self.run.clock
    }

    pub(crate) fn timeout_ms(&self) -> f64 {
        self.timeout_ms
    }

    pub(crate) fn call_id(&self) -> u64 {
        self.entry.call_id
    }

    /// The call's delivery cell; `None` for a method the spec does not flag `input`.
    pub(crate) fn delivery(&self) -> Option<&Arc<DeliveryCell>> {
        self.entry.delivery.as_ref()
    }

    /// The run's artifact directory, where a `path` option writes.
    pub(crate) fn artifacts(&self) -> &Artifacts {
        &self.run.artifacts
    }
}

#[cfg(test)]
impl Progress {
    /// A progress for `click.rs`'s policy tests: an input call with Playwright's `timeout`, in
    /// a run whose deadline is far away, on a tab that never closes.
    pub(crate) fn for_test(timeout_ms: f64) -> Self {
        let entry = CallEntry::for_test(CallStart {
            timeout_ms: Some(timeout_ms),
            ..CallStart::for_test("click", true)
        });
        let run = RunScope::new(
            Deadline::new(Duration::from_hours(1), CancellationToken::new()),
            TaskTracker::new(),
            Arc::default(),
        );
        Self::new(entry, timeout_ms, run, CancellationToken::new())
    }

    /// The call log as a reply would carry it.
    pub(crate) fn folded_log(&self) -> Vec<String> {
        self.entry.folded_log()
    }
}

/// The delivery state of one input call, shared by the call's task, the input writer and the
/// trace entry, so the record reads the truth however any of them ends. Transitions happen only
/// through [`InputTicket`] and [`AckedInput`]:
///
/// ```text
///  NotSent ──begin()──▶ Unknown ──acked()──▶ Done ──blocked()──▶ NotSent
///     ▲                    │                       (verified: Neo's interceptor stopped the
///     └──── unsent() ──────┘                        press at `window`)
///  (nothing was written)   ticket dropped: stays Unknown     AckedInput dropped: stays Done
/// ```
///
/// `begin` succeeds only from `NotSent`, so after input whose fate is unknown or done no code
/// path can write it again. A two-part drag declares `expect_more` before its first write:
/// only the second acknowledgement can mark the whole call Done.
#[derive(Default)]
pub(crate) struct DeliveryCell {
    state: AtomicU8,
    more: AtomicBool,
    call: Option<(Arc<RunTap>, u64)>,
    /// Set before `AfterCall` while the entry still holds this cell. Its final Drop therefore
    /// cannot publish a correction until `AfterCall` has reached every sink.
    reported: OnceLock<Delivery>,
    input_sent: AtomicBool,
}

impl fmt::Debug for DeliveryCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeliveryCell")
            .field("delivery", &self.delivery())
            .finish_non_exhaustive()
    }
}

impl Drop for DeliveryCell {
    fn drop(&mut self) {
        if let (Some((tap, call)), Some(reported)) = (&self.call, self.reported.get()) {
            let delivery = self.delivery();
            if delivery != *reported {
                tap.emit_with(|| RunEvent::DeliverySettled {
                    run: tap.run(),
                    call: *call,
                    delivery,
                });
            }
        }
    }
}

/// Neo: atomic encoding of delivery before an input writer claims its ticket.
const NOT_SENT: u8 = 0;
/// Neo: atomic encoding of input claimed without a proven acknowledgement.
const UNKNOWN: u8 = 1;
/// Neo: atomic encoding of an acknowledged input write.
const DONE: u8 = 2;

/// Proof that this attempt may write input: the cell read `NotSent` and now reads `Unknown`.
/// It moves into whatever writes the input, which reports through it. Its states: what the
/// cell goes back to when nothing reached the page, and what `acked()` makes it.
#[must_use = "dropping the ticket leaves the call's delivery Unknown"]
#[derive(Debug)]
pub(crate) struct InputTicket(Arc<DeliveryCell>, u8, u8);

/// The browser acknowledged every input event of the attempt: the cell reads `Done`.
#[must_use = "a blocked verdict must call `blocked`"]
#[derive(Debug)]
pub(crate) struct AckedInput(Arc<DeliveryCell>, u8);

/// `begin` refused: earlier input of this call may have reached the page.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InputAlreadySent;

impl DeliveryCell {
    /// The delivery this cell proves now.
    pub(crate) fn delivery(&self) -> Delivery {
        match self.state.load(Ordering::Acquire) {
            NOT_SENT => Delivery::NotSent,
            DONE => Delivery::Done,
            _ => Delivery::Unknown,
        }
    }
}

impl DeliveryCell {
    /// Called just before an attempt hands its first input event to the writer.
    pub(crate) fn begin(self: &Arc<Self>) -> Result<InputTicket, InputAlreadySent> {
        let done = if self.more.load(Ordering::Acquire) {
            UNKNOWN
        } else {
            DONE
        };
        self.state
            .compare_exchange(NOT_SENT, UNKNOWN, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| InputAlreadySent)?;
        if !self.input_sent.swap(true, Ordering::AcqRel)
            && let Some((tap, call)) = &self.call
        {
            tap.emit_with(|| RunEvent::InputSent {
                run: tap.run(),
                call: *call,
                offset_us: tap.offset_us(),
            });
        }
        Ok(InputTicket(Arc::clone(self), NOT_SENT, done))
    }

    /// A call that writes its input in two parts (`dragAndDrop`'s move and down, then its move
    /// and up), said before the first part begins: the first part's acknowledgement, however
    /// late (after the call gave up on it), leaves the cell `Unknown`, and only the second
    /// part's makes it `Done`. So a drag that stops before its drop is never `Done`.
    pub(crate) fn expect_more(&self) {
        self.more.store(true, Ordering::Release);
    }

    /// The second part's ticket. After a first part that wrote input the cell stays `Unknown`,
    /// and nothing the second part does takes it back to `NotSent`; after one that wrote nothing
    /// (a trial) it begins as any first input does.
    pub(crate) fn begin_more(self: &Arc<Self>) -> Result<InputTicket, InputAlreadySent> {
        // The second part's acknowledgement completes the call.
        self.more.store(false, Ordering::Release);
        match self.state.load(Ordering::Acquire) {
            NOT_SENT => self.begin(),
            UNKNOWN => Ok(InputTicket(Arc::clone(self), UNKNOWN, DONE)),
            _ => Err(InputAlreadySent),
        }
    }
}

impl InputTicket {
    /// The writer proved nothing was written (browseros-cdp refuses a send at a stale epoch
    /// before writing it): back to `NotSent`.
    pub(crate) fn unsent(self) {
        self.0.state.store(self.1, Ordering::Release);
    }

    /// Every event was acknowledged.
    pub(crate) fn acked(self) -> AckedInput {
        self.0.state.store(self.2, Ordering::Release);
        AckedInput(self.0, self.1)
    }
}

impl AckedInput {
    /// Neo's hit-target interceptor stopped a press that landed on another element, before the
    /// page's handlers saw it (`Delivery::NotSent` says how far it got): back to `NotSent`.
    pub(crate) fn blocked(self) {
        self.0.state.store(self.1, Ordering::Release);
    }

    /// The events reached the page: stays `Done`.
    pub(crate) fn delivered(self) {
        drop(self);
    }
}

// The trace caps. They mirror `quickjs_bridge::Logs`, for its reason: records live in Rust, outside the
// realm's memory limit, so a script looping on browser calls could otherwise hold any amount of
// the server's memory. Like `Logs`, the first entries are kept and the rest only counted.

/// The most raw log lines one call keeps. Far above a real click's (a `covered` click retrying
/// for 1 s logs 33); a call with no timeout retrying for the whole run would otherwise grow
/// without bound.
/// Deviation: `/packages/protocol/spec/tracing.yml` `Tracing.commands.tracingStopChunk` Neo bounds the per-call in-memory log to protect host memory during unbounded retries.
const MAX_CALL_LOG_LINES: usize = 1000;

/// Records carry only a bounded copy of the reply's folded log. These limits never change
/// the raw log returned to JavaScript, which remains Playwright's.
/// Deviation: `/packages/protocol/spec/tracing.yml` `Tracing.commands.tracingStopChunk` Neo caps only the recorded folded log; the Playwright reply remains whole.
const MAX_RECORD_LOG_LINES: usize = 200;

/// Deviation: `/packages/protocol/spec/tracing.yml` `Tracing.commands.tracingStopChunk` Neo caps recorded folded logs at 16 KiB plus one omission note; the Playwright reply remains whole.
const MAX_RECORD_LOG_BYTES: usize = 16 * 1024;

/// The most call records a run keeps: the first calls to arrive.
/// Deviation: `/packages/protocol/spec/tracing.yml` `Tracing.commands.tracingStopChunk` Neo bounds `RunOutcome` calls; trace sinks still receive every call.
const MAX_CALL_RECORDS: usize = 1000;

/// The most record bytes (params plus folded log) a run keeps. A record's params are charged
/// when its call arrives, so a record that would not fit is never held; its log is charged when
/// the call finishes, or at [`Trace::take`] for one still running, because only then is it
/// final.
/// Deviation: `/packages/protocol/spec/tracing.yml` `Tracing.commands.tracingStopChunk` Neo bounds `RunOutcome` record memory; the separate store applies its own detail budget.
const MAX_TRACE_BYTES: usize = 1024 * 1024;

/// The durable state of one call, shared by its `Progress` and the run's `Trace`. Its record
/// (`RunOutcome::calls`) is derived from it, redacted ([`CallEntry::record`]).
pub(crate) struct CallEntry {
    /// The run's shared trace state: its secrets now, its sinks for the call's events.
    tap: Arc<RunTap>,
    call_id: u64,
    parent: Option<u64>,
    page: Option<PageRef>,
    published: AtomicBool,
    snapshot: OnceLock<(Arc<CallRecord>, bool)>,
    guid: String,
    method: String,
    title: Option<Cow<'static, str>>,
    /// As the SDK sent them. Only [`redact::summarize`] reads them for a record.
    params: Value,
    shape: &'static [ParamShape],
    /// `params` as JSON text, in bytes: what keeping the record costs before any log.
    params_bytes: usize,
    location: Option<CallLocation>,
    timeout_ms: Option<f64>,
    started_at: SystemTime,
    started: Instant,
    log: Mutex<CallLog>,
    /// `Some` for methods Playwright's spec flags `input`.
    delivery: Option<Arc<DeliveryCell>>,
    /// The script's abort of this call, if its `signal` aborted while it ran (`abort.rs`).
    abort: CallAbort,
    /// Set once by whoever ends the call; absent while it runs.
    end: OnceLock<CallEnd>,
    /// Test-only: how long `abort` holds between waking the call and logging, in ms.
    #[cfg(test)]
    abort_pause_ms: AtomicU64,
}

/// A call's raw log, Playwright's `metadata.log`.
#[derive(Debug, Default)]
struct CallLog {
    /// `waiting for <…>`, the one line that may be named late (`Progress::name_locator`).
    header: Option<String>,
    lines: Vec<String>,
    /// Lines past [`MAX_CALL_LOG_LINES`], counted.
    dropped: usize,
}

/// The first terminal result and its raw reply log. Keeping it separate from the redacted
/// snapshot lets a sealed call reject late finishes without changing JavaScript's reply.
struct CallEnd {
    result: CallResult,
    duration: Duration,
    /// The folded log, as the reply carried it.
    log: Vec<String>,
}

/// What a call is, known when it arrives at the door.
pub(crate) struct CallStart {
    pub(crate) parent: Option<u64>,
    pub(crate) page: Option<PageRef>,
    pub(crate) guid: String,
    pub(crate) method: String,
    /// The method's trace title, such as `Fill "{value}"`. Kept as the template: rendering it
    /// would inline the params it names.
    pub(crate) title: Option<Cow<'static, str>>,
    pub(crate) params: Value,
    /// The method's params by kind (`MethodMeta::params`); empty for a method Neo does not
    /// know, so a record keeps none of its strings.
    pub(crate) shape: &'static [ParamShape],
    pub(crate) location: Option<CallLocation>,
    pub(crate) timeout_ms: Option<f64>,
    /// Whether the method writes input, so its delivery is tracked.
    pub(crate) input: bool,
}

impl CallEntry {
    /// The entry of call `call_id` of `tap`'s run. Registers the call's `Secret`-kind params
    /// with the run before the call can log anything, so its own log and error are scrubbed.
    fn new(tap: Arc<RunTap>, call_id: u64, start: CallStart) -> Self {
        let CallStart {
            parent,
            page,
            guid,
            method,
            title,
            params,
            shape,
            location,
            timeout_ms,
            input,
        } = start;
        redact::register_secrets(&params, shape, tap.secrets());
        let delivery = input.then(|| {
            Arc::new(DeliveryCell {
                call: Some((Arc::clone(&tap), call_id)),
                state: AtomicU8::new(NOT_SENT),
                more: AtomicBool::new(false),
                reported: OnceLock::new(),
                input_sent: AtomicBool::new(false),
            })
        });
        Self {
            tap,
            call_id,
            parent,
            page,
            published: AtomicBool::new(false),
            snapshot: OnceLock::new(),
            guid,
            method,
            title,
            params_bytes: redact::json_len(&params),
            params,
            shape,
            location,
            timeout_ms,
            started_at: SystemTime::now(),
            started: Instant::now(),
            log: Mutex::default(),
            delivery,
            abort: CallAbort::default(),
            end: OnceLock::new(),
            #[cfg(test)]
            abort_pause_ms: AtomicU64::new(0),
        }
    }

    /// The call's params as the script sent them.
    pub(crate) fn params(&self) -> &Value {
        &self.params
    }

    /// Playwright's `ProgressController.abort` for this call: logs `operation was aborted:
    /// <reason>` and aborts it, unless it already ended or was aborted. Its `Progress` stops at
    /// its next race or check.
    pub(crate) fn abort(&self, reason: String) {
        if self.end.get().is_some() {
            return;
        }
        // Abort is a control frame rather than a browser call. Register its free-text reason
        // on the existing entry before its raw log/error can become a durable record.
        self.tap.secrets().register(&reason);
        let line = format!("operation was aborted: {reason}");
        // The abort wakes the call while this holds its log: the call's end folds the log
        // under the same lock, so the reply always carries the line.
        match self.log.lock() {
            Ok(mut log) => {
                if self.abort.abort(reason) {
                    #[cfg(test)]
                    thread::sleep(Duration::from_millis(
                        self.abort_pause_ms.load(Ordering::Relaxed),
                    ));
                    log.push(line);
                }
            }
            Err(_) => {
                let _aborted = self.abort.abort(reason);
            }
        }
    }

    /// The call's `AbortError`, once the script aborted it.
    #[cfg(test)]
    pub(crate) fn abort_error(&self) -> Option<PlaywrightError> {
        self.abort.error()
    }

    /// Ends the call with `result` and returns its folded log, which the reply carries. Only
    /// the first end counts.
    pub(crate) fn finish(&self, result: CallResult) -> Vec<String> {
        let log = self.folded_log();
        let _first = self.end.set(CallEnd {
            result,
            duration: self.started.elapsed(),
            log: log.clone(),
        });
        log
    }

    /// The call log after Playwright's `compressCallLog`, ending with a note when lines were
    /// dropped. A poisoned lock (a panic while logging) reads as an empty log.
    fn folded_log(&self) -> Vec<String> {
        let Ok(log) = self.log.lock() else {
            return Vec::new();
        };
        let note = (log.dropped > 0).then(|| log.dropped_note());
        let mut raw: Vec<&str> = log.header.iter().map(String::as_str).collect();
        raw.extend(log.lines.iter().map(String::as_str));
        raw.extend(note.as_deref());
        compress_call_log(&raw)
    }

    /// Changes the raw log. A poisoned lock (a panic while logging) drops the change: logging
    /// never fails a call, as `console.log` never fails the script.
    fn update_log(&self, change: impl FnOnce(&mut CallLog)) {
        if let Ok(mut log) = self.log.lock() {
            change(&mut log);
        }
    }

    /// The call's record, redacted: params summarized by kind ([`redact::summarize`]), and its
    /// log and error message scrubbed of the run's secrets. Whether the summary was cut comes
    /// with it. What the script saw (the reply) is never redacted; this is only ever a record.
    fn record(&self) -> (CallRecord, bool) {
        let (result, duration, log) = match self.end.get() {
            Some(end) => (end.result.clone(), end.duration, end.log.clone()),
            None => (
                CallResult::Unfinished {
                    reason: self.reason(),
                },
                self.started.elapsed(),
                self.folded_log(),
            ),
        };
        let scrubber = self.tap.secrets().scrubber();
        let mut summary = redact::summarize(&self.params, self.shape);
        summary.params = scrubber.scrub_value(&summary.params);
        if redact::json_len(&summary.params) > redact::MAX_SUMMARY_BYTES {
            summary.params = Value::Object(serde_json::Map::new());
            summary.truncated = true;
        }
        let record = CallRecord {
            call_id: self.call_id,
            guid: scrubber.scrub(&self.guid).into_owned(),
            method: scrubber.scrub(&self.method).into_owned(),
            title: self
                .title
                .as_deref()
                .map(|title| scrubber.scrub(title).into_owned()),
            params: summary.params,
            location: self.location,
            timeout_ms: self.timeout_ms,
            started_at_ms: trace::unix_ms(self.started_at),
            duration,
            result: scrubbed_result(result, &scrubber),
            delivery: self.delivery.as_ref().map(|cell| cell.delivery()),
            log: recorded_log(&log, &scrubber),
        };
        (record, summary.truncated)
    }

    fn reason(&self) -> RunEnd {
        self.tap
            .end_cell()
            .get()
            .copied()
            .unwrap_or(RunEnd::Cancelled)
    }

    /// First end wins, including a run sealing a task that outlived its drain. The raw reply
    /// remains the handler's answer; only the record is frozen here.
    fn seal(&self, reason: RunEnd) {
        if self.end.get().is_none() {
            self.finish(CallResult::Unfinished { reason });
        }
    }

    /// Freezes the event's record and the outcome's result. The outcome refreshes delivery at
    /// take time; sinks learn of later delivery through `DeliverySettled`. No sink is called
    /// while initializing this snapshot, so concurrent finish/seal/publish paths can wait safely.
    fn snapshot(&self) -> &(Arc<CallRecord>, bool) {
        self.snapshot.get_or_init(|| {
            self.seal(self.reason());
            let (record, truncated) = self.record();
            if let (Some(cell), Some(delivery)) = (&self.delivery, record.delivery) {
                let _reported = cell.reported.set(delivery);
            }
            (Arc::new(record), truncated)
        })
    }

    /// The only `AfterCall` door. A task publishes after sending its reply; sealing and Drop
    /// are its backstops. An entry holds its delivery cell through this synchronous emission,
    /// so the cell's last Drop can only emit `DeliverySettled` afterwards.
    pub(crate) fn publish(&self) {
        if self.published.swap(true, Ordering::AcqRel) {
            return;
        }
        if self.tap.listening() {
            let (record, params_truncated) = self.snapshot();
            self.tap.emit(&RunEvent::AfterCall {
                run: self.tap.run(),
                call: self.call_id,
                parent: self.parent,
                page: self.page.clone(),
                record: Arc::clone(record),
                params_truncated: *params_truncated,
                offset_us: self.tap.offset_us(),
            });
        }
    }

    fn before(&self) {
        self.tap.emit_with(|| {
            let scrubber = self.tap.secrets().scrubber();
            RunEvent::BeforeCall {
                run: self.tap.run(),
                call: self.call_id,
                parent: self.parent,
                object: scrubber.scrub(&self.guid).into_owned(),
                page: self.page.clone(),
                method: scrubber.scrub(&self.method).into_owned(),
                title: self
                    .title
                    .as_deref()
                    .map(|title| scrubber.scrub(title).into_owned()),
                selector: self
                    .params
                    .get("selector")
                    .and_then(Value::as_str)
                    .map(|text| scrubber.scrub_cut(text, redact::MAX_SUMMARY_BYTES)),
                location: self.location,
                started_at_ms: trace::unix_ms(self.started_at),
                offset_us: trace::micros(self.started.duration_since(self.tap.start())),
            }
        });
    }

    /// What a kept record's log costs against [`MAX_TRACE_BYTES`], on top of its params.
    fn log_bytes(&self) -> usize {
        match self.end.get() {
            Some(end) => end.log.iter().map(String::len).sum(),
            None => self.folded_log().iter().map(String::len).sum(),
        }
    }
}

impl Drop for CallEntry {
    fn drop(&mut self) {
        self.publish();
    }
}

#[cfg(test)]
impl CallEntry {
    /// An entry in a run no sink listens to, outside any trace, for tests that only need a
    /// call's `Progress`.
    pub(crate) fn for_test(start: CallStart) -> Arc<Self> {
        Arc::new(Self::new(RunTap::detached(), 1, start))
    }
}

#[cfg(test)]
impl CallStart {
    /// A call of `method` on `frame@T` with no params.
    pub(crate) fn for_test(method: &str, input: bool) -> Self {
        Self {
            parent: None,
            page: None,
            guid: "frame@T".to_string(),
            method: method.to_string(),
            title: None,
            params: Value::Null,
            shape: &[],
            location: None,
            timeout_ms: None,
            input,
        }
    }
}

/// The record/outcome snapshot owns this bounded, scrubbed copy. Charge after scrubbing
/// because replacement markers can change the byte count; leave the reply's folded log whole.
fn recorded_log(log: &[String], scrubber: &Scrubber) -> Vec<String> {
    let mut kept = Vec::new();
    let mut bytes = 0;
    for line in log.iter().take(MAX_RECORD_LOG_LINES) {
        let line = scrubber.scrub_cut(line, redact::MAX_LINE_BYTES);
        let next = bytes + line.len() + usize::from(!kept.is_empty());
        if next > MAX_RECORD_LOG_BYTES {
            break;
        }
        bytes = next;
        kept.push(line);
    }
    let dropped = log.len() - kept.len();
    if dropped > 0 {
        kept.push(format!(
            "neo-runtime dropped the last {dropped} call log lines: a record keeps at most {MAX_RECORD_LOG_LINES} lines / {MAX_RECORD_LOG_BYTES} bytes"
        ));
    }
    kept
}

/// `result` with its error message scrubbed of the run's secrets.
fn scrubbed_result(result: CallResult, scrubber: &Scrubber) -> CallResult {
    match result {
        CallResult::Error { name, message } => CallResult::Error {
            name,
            message: scrubber.scrub_cut(&message, redact::MAX_ERROR_BYTES),
        },
        CallResult::Ok | CallResult::Unfinished { .. } => result,
    }
}

impl CallLog {
    fn push(&mut self, line: String) {
        if self.lines.len() < MAX_CALL_LOG_LINES {
            self.lines.push(line);
        } else {
            self.dropped += 1;
        }
    }

    fn dropped_note(&self) -> String {
        let lines = if self.dropped == 1 { "line" } else { "lines" };
        format!(
            "neo-runtime dropped the last {} call log {lines}: a call keeps at most {MAX_CALL_LOG_LINES} lines",
            self.dropped
        )
    }
}

/// Every call of one run. The dispatcher opens each call here; the run reads once, after
/// `finish`'s wait.
///
/// Memory stays bounded: at most [`MAX_CALL_RECORDS`] entries are held, each entry's log is
/// capped by [`MAX_CALL_LOG_LINES`], and kept records total at most [`MAX_TRACE_BYTES`], params
/// charged as a call arrives and logs as it finishes. A call past a cap still runs and replies;
/// only its record is not kept. While anything can still change its delivery cell (its call
/// task or its input writer), the cell is kept, a few bytes; once nothing can, its delivery is
/// folded into one running worst. So [`DroppedCalls::delivery`] still reports the least safe
/// delivery among the dropped calls, and dropping a record never hides a write.
///
/// Calls that are kept but unfinished hold logs not yet charged; the dispatcher's in-flight cap
/// bounds how many there are.
#[derive(Clone)]
pub(crate) struct Trace(Arc<TraceShared>);

/// Outcome bookkeeping and dense call numbers shared by the run and its dispatch tasks.
/// Entries own their publication separately, so the outcome's caps cannot suppress events.
struct TraceShared {
    tap: Arc<RunTap>,
    /// The next call's number: a run's calls are numbered in arrival order from 1, nested calls
    /// included, so `calls_made` is the last number handed out.
    next_call: AtomicU64,
    /// Messages the door could not read as a call, which therefore have no number.
    unreadable: AtomicU64,
    inner: Mutex<TraceInner>,
}

#[derive(Default)]
struct TraceInner {
    /// In arrival order, each with whether its log was charged (its params always are).
    kept: Vec<(Arc<CallEntry>, bool)>,
    kept_bytes: usize,
    /// Calls ever admitted, kept or not, so the record cap keeps the first calls.
    arrived: usize,
    /// Set by the first record that did not fit the byte budget; every later call is dropped,
    /// so the kept records stay the run's first ones.
    full: bool,
    dropped_count: u64,
    /// The least safe final delivery among dropped calls whose cells nothing else holds.
    dropped_worst: Option<Delivery>,
    /// Cells of dropped calls that a call task or an input writer still holds.
    dropped_live: Vec<Arc<DeliveryCell>>,
}

impl Trace {
    pub(crate) fn new(tap: Arc<RunTap>) -> Self {
        Self(Arc::new(TraceShared {
            tap,
            next_call: AtomicU64::new(1),
            unreadable: AtomicU64::new(0),
            inner: Mutex::default(),
        }))
    }

    /// Opens the entry of a call that just arrived and registers it: kept, its params charged,
    /// while the caps allow; otherwise counted.
    pub(crate) fn call(&self, start: CallStart) -> Arc<CallEntry> {
        let call_id = self.0.next_call.fetch_add(1, Ordering::Relaxed);
        let entry = Arc::new(CallEntry::new(Arc::clone(&self.0.tap), call_id, start));
        self.push(&entry);
        entry
    }

    fn push(&self, entry: &Arc<CallEntry>) {
        let mut retired = Retired::default();
        if let Ok(mut trace) = self.0.inner.lock() {
            trace.arrived += 1;
            let fits = trace.kept_bytes + entry.params_bytes <= MAX_TRACE_BYTES;
            if trace.full || trace.arrived > MAX_CALL_RECORDS || !fits {
                trace.full |= !fits;
                trace.drop_entry(entry, &mut retired);
            } else {
                trace.kept_bytes += entry.params_bytes;
                trace.kept.push((Arc::clone(entry), false));
            }
        }
        // The realm cannot finish while this synchronous door is on its stack. Publish the
        // start outside the lock, before handing the entry back to any task that can end it.
        entry.before();
    }

    /// Counts a message the door could not read as a call.
    pub(crate) fn count_unreadable(&self) {
        self.0.unreadable.fetch_add(1, Ordering::Relaxed);
    }

    /// How many calls the run made: every call number handed out.
    pub(crate) fn calls_made(&self) -> u64 {
        self.0.next_call.load(Ordering::Relaxed) - 1
    }

    /// How many messages the door could not read as a call.
    pub(crate) fn unreadable_calls(&self) -> u64 {
        self.0.unreadable.load(Ordering::Relaxed)
    }

    /// Charges a kept call's log when it finishes. Called right after [`CallEntry::finish`].
    pub(crate) fn finished(&self, entry: &Arc<CallEntry>) {
        let mut retired = Retired::default();
        let Ok(mut trace) = self.0.inner.lock() else {
            return;
        };
        if let Some(index) = trace
            .kept
            .iter()
            .position(|(kept, _)| Arc::ptr_eq(kept, entry))
        {
            trace.charge(index, &mut retired);
        }
    }

    /// The run's records, in arrival order, and the dropped calls. A call with no end is
    /// `Unfinished { reason }`. Results keep their first published snapshot, while delivery is
    /// read after the drain so a writer's later acknowledgement is reflected in the outcome.
    /// Records are built after the lock is released, with one `tracing` event per record.
    pub(crate) fn take(&self, reason: RunEnd) -> (Vec<CallRecord>, Option<DroppedCalls>) {
        let mut retired = Retired::default();
        let (kept, dropped) = {
            let Ok(mut trace) = self.0.inner.lock() else {
                return (Vec::new(), None);
            };
            for (entry, _) in &trace.kept {
                entry.seal(reason);
            }
            let mut index = 0;
            while index < trace.kept.len() {
                if trace.kept[index].1 || trace.charge(index, &mut retired) {
                    index += 1;
                }
            }
            let kept: Vec<Arc<CallEntry>> = trace.kept.drain(..).map(|(entry, _)| entry).collect();
            let live = trace.dropped_live.iter().map(|cell| cell.delivery());
            let dropped = (trace.dropped_count > 0).then(|| DroppedCalls {
                count: trace.dropped_count,
                delivery: live.chain(trace.dropped_worst).max(),
            });
            (kept, dropped)
        };
        // Freeze ends before releasing any entry, then publish outside the trace lock.
        // A late finish loses to the seal and cannot rewrite either consumer's record.
        let records: Vec<CallRecord> = kept
            .iter()
            .map(|entry| {
                entry.publish();
                let mut record = entry.snapshot().0.as_ref().clone();
                record.delivery = entry.delivery.as_ref().map(|cell| cell.delivery());
                record
            })
            .collect();
        for record in &records {
            info!(
                call_id = record.call_id,
                method = %record.method,
                result = ?record.result,
                delivery = ?record.delivery,
                duration_ms = record.duration.as_millis(),
                "browser call"
            );
        }
        (records, dropped)
    }

    /// How many dropped calls' cells the trace still holds.
    #[cfg(test)]
    fn dropped_cells_held(&self) -> usize {
        self.0
            .inner
            .lock()
            .map_or(0, |trace| trace.dropped_live.len())
    }
}

#[cfg(test)]
impl Default for Trace {
    /// The trace of a run no sink listens to.
    fn default() -> Self {
        Self::new(RunTap::detached())
    }
}

/// Owners removed while the trace is locked. Destruction is deferred until its lock is gone:
/// the last entry or delivery owner can emit synchronously from Drop.
#[derive(Default)]
struct Retired {
    entries: Vec<Arc<CallEntry>>,
    cells: Vec<Arc<DeliveryCell>>,
}

impl TraceInner {
    /// Charges `kept[index]`'s log. Returns whether it stays kept: a record admitted before the
    /// budget ran out keeps its place while its log fits. Past the budget it moves to the
    /// dropped count, its params' charge is returned, and every later call is dropped too.
    fn charge(&mut self, index: usize, retired: &mut Retired) -> bool {
        let bytes = self.kept[index].0.log_bytes();
        if self.kept_bytes + bytes <= MAX_TRACE_BYTES {
            self.kept_bytes += bytes;
            self.kept[index].1 = true;
            return true;
        }
        self.full = true;
        let (entry, _) = self.kept.remove(index);
        self.kept_bytes -= entry.params_bytes;
        self.drop_entry(&entry, retired);
        retired.entries.push(entry);
        false
    }

    /// Counts a dropped call and keeps only its delivery cell. Cells already held that nothing
    /// else holds any more are folded into `dropped_worst` first: a cell moves only through its
    /// entry, an [`InputTicket`] or an [`AckedInput`], each of which holds it, so once the
    /// trace's `Arc` is the last one the cell's delivery is final. That keeps `dropped_live` to the dropped calls still
    /// running, however many calls a run drops.
    fn drop_entry(&mut self, entry: &CallEntry, retired: &mut Retired) {
        self.dropped_count += 1;
        let Some(cell) = entry.delivery.clone() else {
            return;
        };
        let mut worst = self.dropped_worst;
        self.dropped_live = self
            .dropped_live
            .drain(..)
            .filter_map(|held| {
                if Arc::strong_count(&held) > 1 {
                    Some(held)
                } else {
                    worst = worst.max(Some(held.delivery()));
                    retired.cells.push(held);
                    None
                }
            })
            .collect();
        self.dropped_worst = worst;
        self.dropped_live.push(cell);
    }
}

/// Playwright's `compressCallLog` (`server/callLog.ts`) with `findRepeatedSubsequences`,
/// ported line for line. Maximal repeated runs fold into `'  ' + ws + 'N × ' + first line`
/// plus continuation lines, every other line prints as `'  ' + ws + '- ' + line`, and a line's
/// leading whitespace becomes its nesting. Folding is what keeps a call that retried for a long
/// time down to a few lines of error message.
pub(crate) fn compress_call_log<S: AsRef<str>>(log: &[S]) -> Vec<String> {
    let lines: Vec<&str> = log.iter().map(AsRef::as_ref).collect();
    let mut folded = Vec::with_capacity(lines.len());
    for (sequence, count) in find_repeated_subsequences(&lines) {
        let count_prefix = format!("{count} × ");
        for (index, line) in sequence.iter().enumerate() {
            let text = line.trim_matches(is_js_whitespace);
            let nesting = &line[..line.len() - line.trim_start_matches(is_js_whitespace).len()];
            let folded_line = if count > 1 && index == 0 {
                format!("  {nesting}{count_prefix}{text}")
            } else if count > 1 {
                // JS's `countPrefix.length - 2`; every character of `N × ` is one UTF-16 unit.
                let pad = " ".repeat(count_prefix.chars().count() - 2);
                format!("  {nesting}{pad}- {text}")
            } else {
                format!("  {nesting}- {text}")
            };
            folded.push(folded_line);
        }
    }
    folded
}

/// Playwright's `findRepeatedSubsequences`: from each position, the window repeated the most
/// lines' worth of times in a row (the shortest window on a tie), else the single line.
fn find_repeated_subsequences<T: PartialEq>(s: &[T]) -> Vec<(&[T], usize)> {
    let n = s.len();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < n {
        let mut max_count = 1;
        let mut max_length = 1;
        for p in 1..=n - i {
            let window = &s[i..i + p];
            // Playwright's loop first compares the window with itself; start past that copy.
            let mut k = 1;
            while i + p * (k + 1) <= n && s[i + p * k..i + p * (k + 1)] == *window {
                k += 1;
            }
            if k > 1 && k * p > max_count * max_length {
                max_count = k;
                max_length = p;
            }
        }
        blocks.push((&s[i..i + max_length], max_count));
        i += max_length * max_count;
    }
    blocks
}

/// JS's `\s` and `String.prototype.trim` set: Rust's `White_Space` without U+0085, plus U+FEFF.
pub(crate) fn is_js_whitespace(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// A number the way JS's `${n}` prints it: `500` for 500.0, `1500.5`, `1e+21`, `NaN`.
pub(crate) fn js_number(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    // Both of Rust's float formats print the shortest digits that round-trip, as JS does; JS
    // switches to exponent notation outside [1e-6, 1e21) and signs a positive exponent.
    if (1e-6..1e21).contains(&value.abs()) {
        return format!("{value}");
    }
    let exponential = format!("{value:e}");
    match exponential.split_once('e') {
        Some((digits, exponent)) if !exponent.starts_with('-') => format!("{digits}e+{exponent}"),
        _ => exponential,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tests_that_closing_the_context_stops_every_other_call_but_itself() {
        let run = RunScope::new(
            Deadline::new(Duration::from_hours(1), CancellationToken::new()),
            TaskTracker::new(),
            Arc::default(),
        );
        let entry = || {
            CallEntry::for_test(CallStart {
                parent: None,
                page: None,
                shape: &[],
                timeout_ms: None,
                guid: "frame@T".to_string(),
                method: "click".to_string(),
                title: Some("Click".into()),
                params: Value::Null,
                location: None,
                input: true,
            })
        };
        let waiting = Progress::new(entry(), 0.0, run.clone(), CancellationToken::new());
        let closing = Progress::new(entry(), 0.0, run.clone(), CancellationToken::new())
            .closing_the_context();
        assert!(waiting.check().is_ok() && closing.check().is_ok());
        closing.stop_the_runs_calls();
        let closed: CallError = PlaywrightError::target_closed().into();
        assert_eq!(
            waiting.check(),
            Err(closed.clone()),
            "no input after the close"
        );
        assert_eq!(
            waiting.race(future::pending::<()>()).await,
            Err(closed),
            "a wait in flight ends as its tab closing would"
        );
        assert!(closing.check().is_ok(), "the close finishes its own reply");
        let later = Progress::new(entry(), 0.0, run, CancellationToken::new());
        assert!(later.check().is_err(), "and a call after it never starts");
    }

    use crate::trace::testing;

    #[test]
    fn finish_seal_and_drop_publish_once_under_threads() {
        for _ in 0..10_000 {
            let (tap, recording) = testing::tap();
            let trace = Trace::new(tap);
            let entry = entry(&trace, false);
            thread::scope(|scope| {
                let finishing = Arc::clone(&entry);
                scope.spawn(move || {
                    finishing.finish(CallResult::Ok);
                    finishing.publish();
                });
                scope.spawn(move || {
                    trace.take(RunEnd::Timeout);
                });
                drop(entry);
            });
            let events = recording.events.lock().unwrap();
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(event, RunEvent::AfterCall { .. }))
                    .count(),
                1
            );
            assert!(matches!(
                events.first(),
                Some(RunEvent::BeforeCall { call: 1, .. })
            ));
            assert!(matches!(events.last(), Some(RunEvent::RunSettled { .. })));
            assert_eq!(*recording.dropped_after.lock().unwrap(), Some(events.len()));
        }
    }

    #[test]
    fn call_and_delivery_drop_emit_without_a_tokio_runtime() {
        let (tap, recording) = testing::tap();
        let trace = Trace::new(tap);
        let entry = entry(&trace, true);
        let ticket = entry.delivery.as_ref().unwrap().begin().unwrap();
        drop(trace);
        thread::spawn(move || {
            drop(entry);
            ticket.acked().delivered();
        })
        .join()
        .unwrap();
        let events = recording.events.lock().unwrap();
        assert!(matches!(events.as_slice(), [
            RunEvent::BeforeCall { .. }, RunEvent::InputSent { .. },
            RunEvent::AfterCall { record, .. },
            RunEvent::DeliverySettled { delivery: Delivery::Done, .. },
            RunEvent::RunSettled { .. },
        ] if record.result == CallResult::Unfinished { reason: RunEnd::Cancelled }));
        assert_eq!(*recording.dropped_after.lock().unwrap(), Some(events.len()));
    }

    #[test]
    fn a_late_finish_cannot_rewrite_a_sealed_record_or_event() {
        let (tap, recording) = testing::tap();
        let trace = Trace::new(tap);
        let entry = entry(&trace, true);
        let (records, _) = trace.take(RunEnd::Timeout);
        entry.finish(CallResult::Ok);
        entry.publish();
        assert_eq!(entry.snapshot().0.as_ref(), &records[0]);
        assert_eq!(
            records[0].result,
            CallResult::Unfinished {
                reason: RunEnd::Timeout
            }
        );
        assert_eq!(
            recording
                .events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| matches!(event, RunEvent::AfterCall { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn delivery_settles_after_after_call_in_every_holder_drop_order() {
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let (tap, recording) = testing::tap();
            let trace = Trace::new(Arc::clone(&tap));
            let entry = entry(&trace, true);
            let run = scope(Duration::from_secs(10));
            let progress = Progress::new(Arc::clone(&entry), 0.0, run, CancellationToken::new());
            let ticket = entry.delivery.as_ref().unwrap().begin().unwrap();
            trace.take(RunEnd::Settled);
            drop(trace);
            drop(tap);
            // Each holder can disappear first. The acknowledgement moves delivery after
            // the frozen record; its cell outlives both entry and Progress when necessary.
            let mut entry = Some(entry);
            let mut progress = Some(progress);
            let mut ticket = Some(ticket);
            for index in order {
                match index {
                    0 => drop(entry.take()),
                    1 => drop(progress.take()),
                    _ => ticket.take().unwrap().acked().delivered(),
                }
            }
            let events = recording.events.lock().unwrap();
            assert!(matches!(events.as_slice(), [
                RunEvent::BeforeCall { .. }, RunEvent::InputSent { .. },
                RunEvent::AfterCall { record, .. },
                RunEvent::DeliverySettled { delivery: Delivery::Done, .. },
                RunEvent::RunSettled { .. },
            ] if record.delivery == Some(Delivery::Unknown)));
        }
    }

    #[test]
    fn unchanged_delivery_emits_no_correction_and_input_sent_is_once() {
        for delivered_before_record in [false, true] {
            let (tap, recording) = testing::tap();
            let trace = Trace::new(tap);
            let entry = entry(&trace, true);
            let cell = entry.delivery.as_ref().unwrap();
            let first = cell.begin().unwrap();
            assert!(cell.begin().is_err());
            first.unsent();
            let second = cell.begin().unwrap();
            if delivered_before_record {
                second.acked().delivered();
            } else {
                drop(second);
            }
            trace.take(RunEnd::Settled);
            drop(entry);
            drop(trace);
            let events = recording.events.lock().unwrap();
            assert_eq!(
                events
                    .iter()
                    .filter(|e| matches!(e, RunEvent::InputSent { .. }))
                    .count(),
                1
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|e| matches!(e, RunEvent::DeliverySettled { .. }))
                    .count(),
                0
            );
        }
        let (tap, recording) = testing::tap();
        let trace = Trace::new(tap);
        let _entry = entry(&trace, true);
        trace.take(RunEnd::Settled);
        assert!(
            !recording
                .events
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, RunEvent::InputSent { .. }))
        );
    }

    /// A click on `#go` that arrives at `trace`.
    fn entry(trace: &Trace, input: bool) -> Arc<CallEntry> {
        entry_with_params(trace, input, serde_json::json!({ "selector": "#go" }))
    }

    fn entry_with_params(trace: &Trace, input: bool, params: Value) -> Arc<CallEntry> {
        trace.call(CallStart {
            params,
            ..CallStart::for_test("click", input)
        })
    }

    fn scope(run_timeout: Duration) -> RunScope {
        RunScope::new(
            Deadline::new(run_timeout, CancellationToken::new()),
            TaskTracker::new(),
            Arc::default(),
        )
    }

    fn progress(timeout_ms: f64, run: &RunScope, page_closed: &CancellationToken) -> Progress {
        let entry = CallEntry::for_test(CallStart::for_test("click", true));
        Progress::new(entry, timeout_ms, run.clone(), page_closed.clone())
    }

    /// Fifty failed attempts fold into a few lines that still name the reason and say how
    /// often it repeated.
    #[test]
    fn tests_that_repeated_attempts_fold_into_a_short_log() {
        let mut raw = vec![
            "waiting for #go",
            "  locator resolved to <button id=\"go\">",
        ];
        for _ in 0..50 {
            raw.extend([
                "retrying click action",
                "  waiting 500ms",
                "  element is not enabled",
            ]);
        }
        let folded = compress_call_log(&raw);

        assert!(folded.len() < 10, "{folded:#?}");
        assert!(
            folded.iter().any(|line| line.contains("50 ×")),
            "the repeat count: {folded:#?}"
        );
        for text in ["waiting for #go", "element is not enabled"] {
            assert!(
                folded.iter().any(|line| line.ends_with(text)),
                "{text}: {folded:#?}"
            );
        }
    }

    /// Lines that do not repeat stay one each, in order, a nested line deeper than its parent.
    #[test]
    fn tests_that_a_folded_log_keeps_each_line_and_its_nesting() {
        let folded = compress_call_log(&["waiting for x", "  resolved to y"]);
        let indent = |line: &str| line.len() - line.trim_start().len();

        assert_eq!(folded.len(), 2, "{folded:?}");
        assert!(
            folded[0].ends_with("waiting for x") && folded[1].ends_with("resolved to y"),
            "{folded:?}"
        );
        assert!(indent(&folded[1]) > indent(&folded[0]), "{folded:?}");
        assert!(
            compress_call_log::<&str>(&[]).is_empty(),
            "nothing in, nothing out"
        );
    }

    #[test]
    fn js_number_prints_as_javascript_does() {
        let cases = [
            (500.0, "500"),
            (1500.5, "1500.5"),
            (0.5, "0.5"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1e21, "1e+21"),
            (1e-7, "1e-7"),
            (1e20, "100000000000000000000"),
            (-0.0, "0"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
        ];
        for (value, printed) in cases {
            assert_eq!(js_number(value), printed, "{value:?}");
        }
    }

    #[test]
    fn delivery_cell_never_allows_a_second_write_after_unknown_or_done() {
        let cell = Arc::new(DeliveryCell::default());
        assert_eq!(cell.delivery(), Delivery::NotSent);
        let ticket = cell.begin().expect("a fresh cell may write");
        assert_eq!(cell.delivery(), Delivery::Unknown);
        assert_eq!(cell.begin().unwrap_err(), InputAlreadySent);
        let acked = ticket.acked();
        assert_eq!(cell.delivery(), Delivery::Done);
        assert_eq!(cell.begin().unwrap_err(), InputAlreadySent);
        acked.blocked();
        assert_eq!(cell.delivery(), Delivery::NotSent, "a verified block");
        cell.begin()
            .expect("after a verified block the call may press again")
            .unsent();
        assert_eq!(cell.delivery(), Delivery::NotSent);

        let ticket = cell.begin().expect("an unsent write may be retried");
        drop(ticket);
        assert_eq!(cell.delivery(), Delivery::Unknown, "a dropped ticket");
        let cell = Arc::new(DeliveryCell::default());
        cell.begin().expect("fresh").acked().delivered();
        assert_eq!(cell.delivery(), Delivery::Done);
    }

    /// A call that writes in two parts, said before the first: the first part's acknowledgement,
    /// on time or late, leaves the cell `Unknown`; only the second part's makes it `Done`.
    #[test]
    fn delivery_cell_completes_a_two_part_call_only_with_its_second_part() {
        let cell = Arc::new(DeliveryCell::default());
        cell.expect_more();
        let first = cell.begin().expect("the first part");
        first.acked().delivered();
        assert_eq!(
            cell.delivery(),
            Delivery::Unknown,
            "half a drag is not done"
        );
        let second = cell.begin_more().expect("the second part");
        assert_eq!(cell.delivery(), Delivery::Unknown);
        second.acked().delivered();
        assert_eq!(cell.delivery(), Delivery::Done);

        // The first part acknowledged only after the call gave up: never `Done`.
        let cell = Arc::new(DeliveryCell::default());
        cell.expect_more();
        let late = cell.begin().expect("the first part");
        late.acked().delivered();
        assert_eq!(cell.delivery(), Delivery::Unknown);

        // A first part that wrote nothing (a trial): the second begins as any first input.
        let cell = Arc::new(DeliveryCell::default());
        cell.expect_more();
        cell.begin_more().expect("fresh").acked().delivered();
        assert_eq!(cell.delivery(), Delivery::Done);
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_times_out_on_its_own_deadline_with_playwrights_message() {
        let run = scope(Duration::from_secs(30));
        let progress = progress(500.0, &run, &CancellationToken::new());
        let started = Instant::now();
        let error = progress.race(future::pending::<()>()).await.unwrap_err();
        assert_eq!(
            error,
            CallError::Playwright(PlaywrightError {
                name: ErrorName::TimeoutError,
                message: "Timeout 500ms exceeded.".to_string(),
            })
        );
        assert_eq!(started.elapsed(), Duration::from_millis(500));
    }

    #[tokio::test(start_paused = true)]
    async fn a_negative_timeout_times_out_at_once_with_playwrights_message() {
        let run = scope(Duration::from_secs(30));
        let progress = progress(-1.0, &run, &CancellationToken::new());
        let expected = CallError::Playwright(PlaywrightError {
            name: ErrorName::TimeoutError,
            message: "Timeout -1ms exceeded.".to_string(),
        });
        assert_eq!(progress.check(), Err(expected.clone()));
        let started = Instant::now();
        assert_eq!(progress.race(future::pending::<()>()).await, Err(expected));
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn a_timeout_reaching_the_run_deadline_leaves_the_stop_to_the_run() {
        let run = scope(Duration::from_secs(1));
        for timeout_ms in [1000.0, 5000.0, 0.0] {
            let progress = progress(timeout_ms, &run, &CancellationToken::new());
            assert_eq!(
                progress.race(future::pending::<()>()).await.unwrap_err(),
                CallError::RunEnded(RunEnd::Timeout),
                "timeout {timeout_ms}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_run_ending_wins_a_tie_with_the_call_deadline() {
        let run = scope(Duration::from_secs(30));
        let progress = progress(100.0, &run, &CancellationToken::new());
        time::advance(Duration::from_millis(100)).await;
        run.end(RunEnd::Settled);
        run.end(RunEnd::Timeout);
        assert_eq!(
            progress.race(future::pending::<()>()).await.unwrap_err(),
            CallError::RunEnded(RunEnd::Settled),
            "the first reason recorded wins"
        );
        assert_eq!(progress.check(), Err(CallError::RunEnded(RunEnd::Settled)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_closed_tab_ends_the_call_with_target_closed() {
        let run = scope(Duration::from_secs(30));
        let closed = CancellationToken::new();
        let progress = progress(1000.0, &run, &closed);
        assert_eq!(progress.wait(Duration::from_millis(20)).await, Ok(()));
        closed.cancel();
        let expected = CallError::Playwright(PlaywrightError::target_closed());
        assert_eq!(
            progress.race(future::pending::<()>()).await,
            Err(expected.clone())
        );
        assert_eq!(progress.check(), Err(expected));
    }

    #[tokio::test(start_paused = true)]
    async fn an_abort_ends_the_call_with_the_reason_and_logs_it() {
        let run = scope(Duration::from_secs(30));
        let entry = entry(&Trace::default(), true);
        let progress = Progress::new(Arc::clone(&entry), 0.0, run, CancellationToken::new());
        let token = progress.abort_token();
        assert_eq!(progress.pending_abort(), Ok(()));
        let aborts = async {
            time::sleep(Duration::from_millis(50)).await;
            entry.abort("stop".to_string());
            entry.abort("again".to_string());
        };
        let (raced, ()) = tokio::join!(progress.race(future::pending::<()>()), aborts);
        let expected = CallError::Playwright(PlaywrightError::aborted("stop"));
        assert_eq!(raced, Err(expected.clone()));
        assert_eq!(progress.check(), Err(expected.clone()));
        assert_eq!(progress.pending_abort(), Err(expected));
        assert!(token.is_cancelled(), "the abort token follows the abort");
        assert_eq!(
            progress.folded_log(),
            vec!["  - operation was aborted: stop".to_string()],
            "one line, for the first abort only"
        );
    }

    /// The interleaving forced: the call wakes from its abort and folds its log while `abort`
    /// is held between waking it and logging. The fold waits for the line.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_call_that_folds_its_log_as_its_abort_wakes_it_has_the_abort_line() {
        let run = scope(Duration::from_secs(30));
        let entry = entry(&Trace::default(), true);
        let progress = Progress::new(Arc::clone(&entry), 0.0, run, CancellationToken::new());
        let token = progress.abort_token();
        entry.abort_pause_ms.store(300, Ordering::Relaxed);
        let woken = tokio::spawn({
            let entry = Arc::clone(&entry);
            async move {
                token.cancelled().await;
                entry.folded_log()
            }
        });
        let aborting = thread::spawn({
            let entry = Arc::clone(&entry);
            move || entry.abort("stop".to_string())
        });
        let log = woken.await.expect("the woken call");
        aborting.join().expect("the abort");
        assert_eq!(
            log.last().map(String::as_str),
            Some("  - operation was aborted: stop")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_abort_wins_a_tie_with_the_call_deadline_and_loses_one_with_the_run() {
        let run = scope(Duration::from_secs(30));
        let entry = entry(&Trace::default(), true);
        let progress = Progress::new(
            Arc::clone(&entry),
            100.0,
            run.clone(),
            CancellationToken::new(),
        );
        time::advance(Duration::from_millis(100)).await;
        entry.abort("stop".to_string());
        assert_eq!(
            progress.race(future::pending::<()>()).await,
            Err(CallError::Playwright(PlaywrightError::aborted("stop")))
        );
        run.end(RunEnd::Settled);
        assert_eq!(
            progress.race(future::pending::<()>()).await,
            Err(CallError::RunEnded(RunEnd::Settled))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_stop_outranks_a_pending_abort() {
        let run = scope(Duration::from_secs(30));
        let entry = entry(&Trace::default(), true);
        let progress = Progress::new(
            Arc::clone(&entry),
            0.0,
            run.clone(),
            CancellationToken::new(),
        );
        entry.abort("stop".to_string());
        let aborted = CallError::Playwright(PlaywrightError::aborted("stop"));
        assert_eq!(progress.pending_abort(), Err(aborted.clone()));
        assert_eq!(progress.check(), Err(aborted));
        run.end(RunEnd::Settled);
        assert_eq!(
            progress.pending_abort(),
            Ok(()),
            "the handler starts, and its first race reports the run's end"
        );
        assert_eq!(progress.check(), Err(CallError::RunEnded(RunEnd::Settled)));
        assert_eq!(
            progress.race(future::pending::<()>()).await,
            Err(CallError::RunEnded(RunEnd::Settled))
        );
    }

    #[test]
    fn an_abort_after_the_call_ended_changes_nothing() {
        let entry = entry(&Trace::default(), false);
        let _log = entry.finish(CallResult::Ok);
        entry.abort("late".to_string());
        assert!(entry.abort_error().is_none());
        assert!(entry.folded_log().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn an_ended_run_without_a_reason_reads_as_cancelled() {
        let run = scope(Duration::from_secs(30));
        let progress = progress(0.0, &run, &CancellationToken::new());
        drop(run.ended.drop_guard());
        assert_eq!(
            progress.check(),
            Err(CallError::RunEnded(RunEnd::Cancelled))
        );
    }

    #[test]
    fn an_unfinished_call_reads_unfinished_with_its_cells_delivery() {
        let trace = Trace::default();
        let finished = entry(&trace, true);
        let running = entry(&trace, true);
        finished.finish(PlaywrightError::timeout(500.0).to_result());
        trace.finished(&finished);
        let ticket = running
            .delivery
            .as_ref()
            .expect("click is an input call")
            .begin()
            .expect("fresh");

        let (records, dropped) = trace.take(RunEnd::Settled);
        drop(ticket);

        assert_eq!(dropped, None);
        let summary: Vec<_> = records
            .iter()
            .map(|record| (record.call_id, record.result.clone(), record.delivery))
            .collect();
        assert_eq!(
            summary,
            [
                (
                    1,
                    CallResult::Error {
                        name: "TimeoutError".to_string(),
                        message: "Timeout 500ms exceeded.".to_string(),
                    },
                    Some(Delivery::NotSent),
                ),
                (
                    2,
                    CallResult::Unfinished {
                        reason: RunEnd::Settled
                    },
                    Some(Delivery::Unknown),
                ),
            ]
        );
    }

    #[test]
    fn calls_past_the_record_cap_are_counted_with_their_least_safe_delivery() {
        let trace = Trace::default();
        for call_id in 1..=1002 {
            let entry = entry(&trace, true);
            if call_id == 1001 {
                let _unknown = entry.delivery.as_ref().expect("input").begin();
            }
            entry.finish(CallResult::Ok);
            trace.finished(&entry);
        }
        let (records, dropped) = trace.take(RunEnd::Settled);
        assert_eq!(records.len(), 1000);
        assert_eq!(records.last().map(|record| record.call_id), Some(1000));
        assert_eq!(
            dropped,
            Some(DroppedCalls {
                count: 2,
                delivery: Some(Delivery::Unknown),
            })
        );
    }

    #[test]
    fn records_past_the_byte_budget_are_dropped_and_later_calls_with_them() {
        let trace = Trace::default();
        let big = "x".repeat(600 * 1024);
        let entries: Vec<_> = (1..=3).map(|_| entry(&trace, false)).collect();
        for entry in &entries {
            let line = big.clone();
            entry.update_log(|log| log.push(line));
        }
        for entry in &entries {
            entry.finish(CallResult::Ok);
            trace.finished(entry);
        }
        let _late = entry(&trace, false);

        let (records, dropped) = trace.take(RunEnd::Settled);
        let kept: Vec<u64> = records.iter().map(|record| record.call_id).collect();
        assert_eq!(
            kept,
            [1],
            "one 600 KiB record fits in 1 MiB, a second does not"
        );
        assert_eq!(
            dropped,
            Some(DroppedCalls {
                count: 3,
                delivery: None,
            })
        );
    }

    /// A record whose params alone would pass the byte budget is never held, not even while its
    /// call runs: the trace keeps no `Arc` to it from the moment it arrives.
    #[test]
    fn params_past_the_byte_budget_are_not_held_while_the_call_runs() {
        let trace = Trace::default();
        let selector = "x".repeat(600 * 1024);
        let first = entry_with_params(&trace, true, serde_json::json!({ "selector": selector }));
        let second = entry_with_params(&trace, true, serde_json::json!({ "selector": selector }));

        assert_eq!(Arc::strong_count(&first), 2, "the first record is kept");
        assert_eq!(
            Arc::strong_count(&second),
            1,
            "a second 600 KiB of params does not fit in 1 MiB"
        );
        let late = entry(&trace, false);
        assert_eq!(Arc::strong_count(&late), 1, "and nothing after it is kept");
        let (records, dropped) = trace.take(RunEnd::Settled);
        let kept: Vec<u64> = records.iter().map(|record| record.call_id).collect();
        assert_eq!(kept, [1]);
        assert_eq!(dropped.map(|dropped| dropped.count), Some(2));
    }

    /// Thousands of dropped calls that have ended leave one delivery behind, not one cell each,
    /// and an `Unknown` among them still decides the summary.
    #[test]
    fn dropped_calls_that_ended_fold_into_one_worst_delivery() {
        let trace = Trace::default();
        for call_id in 1..=6000 {
            let entry = entry(&trace, true);
            if call_id == 1001 {
                let cell = entry.delivery.as_ref().expect("an input call");
                drop(cell.begin().expect("fresh"));
            }
            entry.finish(CallResult::Ok);
            trace.finished(&entry);
        }

        assert_eq!(
            trace.dropped_cells_held(),
            1,
            "only the newest dropped call's cell, held when it arrived"
        );
        let (records, dropped) = trace.take(RunEnd::Settled);
        assert_eq!(records.len(), 1000);
        assert_eq!(
            dropped,
            Some(DroppedCalls {
                count: 5000,
                delivery: Some(Delivery::Unknown),
            })
        );
    }

    #[test]
    fn recorded_call_logs_obey_line_and_byte_caps_with_one_omission_note() {
        for (count, width) in [(250, 8), (40, 1024)] {
            let (tap, recording) = testing::tap();
            tap.secrets().register("hunter22");
            let trace = Trace::new(tap);
            let entry = entry(&trace, false);
            entry.update_log(|log| {
                for no in 0..count {
                    log.push(format!("{no} hunter22 {}", "🦀".repeat(width)));
                }
            });
            let reply = entry.finish(CallResult::Ok);
            assert_eq!(reply.len(), count, "the Playwright-visible log stays whole");
            assert!(reply[0].contains("hunter22"));
            entry.publish();
            let (records, _) = trace.take(RunEnd::Settled);
            let (note, kept) = records[0].log.split_last().unwrap();
            assert!(
                note.contains("neo-runtime dropped the last"),
                "missing omission note"
            );
            assert!(kept.len() <= 200);
            assert!(
                kept.join("\n").len() <= 16 * 1024,
                "record log exceeds 16 KiB"
            );
            assert!(!records[0].log.join("\n").contains("hunter22"));
            assert!(note.contains(&(count - kept.len()).to_string()));
            let events = recording.events.lock().unwrap().clone();
            let recorded = events
                .iter()
                .find_map(|event| match event {
                    RunEvent::AfterCall { record, .. } => Some(record),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                recorded.log, records[0].log,
                "outcome and trace share the bounded record"
            );
        }
    }

    #[test]
    fn a_call_log_past_its_cap_folds_what_it_kept_then_notes_the_rest() {
        let run = scope(Duration::from_secs(30));
        let progress = progress(0.0, &run, &CancellationToken::new());
        progress.name_locator("#a");
        progress.name_locator("getByRole('button')");
        for line in 0..1005 {
            progress.log(format!("  attempt {line}"));
        }
        let log = progress.entry.finish(CallResult::Ok);
        assert_eq!(log.len(), 1002, "the header, 1000 kept lines and the note");
        assert_eq!(log[0], "  - waiting for getByRole('button')");
        assert_eq!(log[1000], "    - attempt 999");
        assert_eq!(
            log[1001],
            "  - neo-runtime dropped the last 5 call log lines: a call keeps at most 1000 lines"
        );
    }
}
