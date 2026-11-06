//! One browser call's progress, Playwright's `server/progress.ts` for Neo: the call's own
//! deadline, the run ending under it, the tab closing, its call log and its delivery cell.
//!
//! A call's durable state lives in a [`CallEntry`] that the run's [`Trace`] holds, never in the
//! realm and never only in the call's task. Three things can end a call early and none may lose
//! its record: the run deadline drops the realm, the script returning drops it too, and the
//! end-of-run wait can leave a task running past the run. So the record is derived from the
//! entry when the run reads it ([`Trace::take`]), never assembled by whoever ended the call.
//!
//! ```text
//!   dispatch ──creates──▶ CallEntry ◀──holds── Trace (run)          read once, after `finish`'s wait
//!       │                    ▲   ▲
//!       └──▶ Progress ───────┘   └── DeliveryCell ◀── InputTicket / AckedInput (input writer)
//!              race(work): run deadline or cancel │ run ended │ tab closed │ own deadline
//! ```

use std::future;
use std::future::Future;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tokio::time;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::info;

use crate::deadline::{Deadline, Stop};
use crate::run::{CallLocation, CallRecord, CallResult, Delivery, DroppedCalls, RunEnd};

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

/// The wire error names Neo replies with. The SDK rebuilds `TimeoutError` under that name and
/// `TargetClosedError` with name `Error`, as Playwright's client does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorName {
    Error,
    TimeoutError,
    TargetClosedError,
}

impl ErrorName {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::TimeoutError => "TimeoutError",
            Self::TargetClosedError => "TargetClosedError",
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
    /// `ended` fired because the dispatcher dropped: read as [`RunEnd::Cancelled`].
    end_reason: Arc<OnceLock<RunEnd>>,
    /// The run's call tasks, and every task they start. `Dispatcher::finish` waits on it,
    /// bounded, so records read final deliveries.
    pub(crate) tasks: TaskTracker,
}

impl RunScope {
    pub(crate) fn new(deadline: Deadline, tasks: TaskTracker) -> Self {
        Self {
            deadline,
            ended: CancellationToken::new(),
            end_reason: Arc::default(),
            tasks,
        }
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
    /// Playwright's backoff caps; `0.0` means none.
    timeout_ms: f64,
    /// When this call times out on its own. `None` for no timeout, and also when its timeout
    /// reaches the run's deadline: then only the run can stop it, so the two deadlines never
    /// fire together and the outcome is always the run's `Timeout`.
    own_deadline: Option<Instant>,
    run: RunScope,
    /// Cancelled when the tab closes.
    page_closed: CancellationToken,
}

impl Progress {
    pub(crate) fn new(
        entry: Arc<CallEntry>,
        timeout_ms: f64,
        run: RunScope,
        page_closed: CancellationToken,
    ) -> Self {
        let own_deadline = Duration::try_from_secs_f64(timeout_ms / 1000.0)
            .ok()
            .filter(|timeout| !timeout.is_zero())
            .and_then(|timeout| Instant::now().checked_add(timeout))
            .filter(|at| *at < run.deadline.at());
        Self {
            entry,
            timeout_ms,
            own_deadline,
            run,
            page_closed,
        }
    }
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Frame.click's handler is the first caller of a call's race, waits and log"
    )
)]
impl Progress {
    /// Runs `work` until it finishes or the call must stop, whichever is first. The `biased`
    /// order is the precedence: the run's deadline or cancellation, then the run ending
    /// otherwise, then the tab closing (`TargetClosedError`), then the call's own deadline
    /// (`TimeoutError`). Every `work` passed in must be cancel-safe: dropping it may lose only
    /// its answer, never leave the page half-changed.
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
            () = self.page_closed.cancelled() => Err(PlaywrightError::target_closed().into()),
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
        if let Some(stop) = self.run.deadline.stop() {
            return Err(CallError::RunEnded(run_end(stop)));
        }
        if self.run.ended.is_cancelled() {
            return Err(CallError::RunEnded(self.run.end_reason()));
        }
        if self.page_closed.is_cancelled() {
            return Err(PlaywrightError::target_closed().into());
        }
        if self.own_deadline.is_some_and(|at| Instant::now() >= at) {
            return Err(PlaywrightError::timeout(self.timeout_ms).into());
        }
        Ok(())
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
}

/// The delivery state of one input call, shared by the call's task, the input writer and the
/// trace entry, so the record reads the truth however any of them ends. Transitions happen only
/// through [`InputTicket`] and [`AckedInput`]:
///
/// ```text
///  NotSent ──begin()──▶ Unknown ──acked()──▶ Done ──blocked()──▶ NotSent
///     ▲                    │                       (verified: Neo's interceptor stopped the
///     └──── unsent() ──────┘                        events before the page saw them)
///  (nothing was written)   ticket dropped: stays Unknown     AckedInput dropped: stays Done
/// ```
///
/// `begin` succeeds only from `NotSent`, so after input whose fate is unknown or done no code
/// path can write it again.
#[derive(Debug, Default)]
pub(crate) struct DeliveryCell(AtomicU8);

const NOT_SENT: u8 = 0;
const UNKNOWN: u8 = 1;
const DONE: u8 = 2;

/// Proof that this attempt may write input: the cell read `NotSent` and now reads `Unknown`.
/// It moves into whatever writes the input, which reports through it.
#[must_use = "dropping the ticket leaves the call's delivery Unknown"]
#[derive(Debug)]
pub(crate) struct InputTicket(Arc<DeliveryCell>);

/// The browser acknowledged every input event of the attempt: the cell reads `Done`.
#[must_use = "a blocked verdict must call `blocked`"]
#[derive(Debug)]
pub(crate) struct AckedInput(Arc<DeliveryCell>);

/// `begin` refused: earlier input of this call may have reached the page.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InputAlreadySent;

impl DeliveryCell {
    /// The delivery this cell proves now.
    pub(crate) fn delivery(&self) -> Delivery {
        match self.0.load(Ordering::Acquire) {
            NOT_SENT => Delivery::NotSent,
            DONE => Delivery::Done,
            _ => Delivery::Unknown,
        }
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Frame.click is the first call that writes input")
)]
impl DeliveryCell {
    /// Called just before an attempt hands its first input event to the writer.
    pub(crate) fn begin(self: &Arc<Self>) -> Result<InputTicket, InputAlreadySent> {
        self.0
            .compare_exchange(NOT_SENT, UNKNOWN, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| InputTicket(Arc::clone(self)))
            .map_err(|_| InputAlreadySent)
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Frame.click is the first call that writes input")
)]
impl InputTicket {
    /// The writer proved nothing was written (browseros-cdp refuses a send at a stale epoch
    /// before writing it): back to `NotSent`.
    pub(crate) fn unsent(self) {
        self.0.0.store(NOT_SENT, Ordering::Release);
    }

    /// Every event was acknowledged.
    pub(crate) fn acked(self) -> AckedInput {
        self.0.0.store(DONE, Ordering::Release);
        AckedInput(self.0)
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Frame.click is the first call that writes input")
)]
impl AckedInput {
    /// Neo's hit-target interceptor stopped the events before the page's handlers saw them (a
    /// trial, or a press that landed on another element): back to `NotSent`.
    pub(crate) fn blocked(self) {
        self.0.0.store(NOT_SENT, Ordering::Release);
    }

    /// The events reached the page: stays `Done`.
    pub(crate) fn delivered(self) {
        drop(self);
    }
}

// The trace caps. They mirror `bridge::Logs`, for its reason: records live in Rust, outside the
// realm's memory limit, so a script looping on browser calls could otherwise hold any amount of
// the server's memory. Like `Logs`, the first entries are kept and the rest only counted.

/// The most raw log lines one call keeps. Far above any golden (the 1 s `covered` click logs
/// 33); a call with no timeout retrying for the whole run would otherwise grow without bound.
const MAX_CALL_LOG_LINES: usize = 1000;

/// The most call records a run keeps: the first calls to arrive.
const MAX_CALL_RECORDS: usize = 1000;

/// The most record bytes (params plus folded log) a run keeps. A record is charged when its
/// call finishes, or at [`Trace::take`] for one still running, because only then is its log
/// final.
const MAX_TRACE_BYTES: usize = 1024 * 1024;

/// The durable state of one call, shared by its `Progress` and the run's `Trace`.
pub(crate) struct CallEntry {
    call_id: u64,
    guid: String,
    method: String,
    title: Option<&'static str>,
    params: Value,
    location: Option<CallLocation>,
    started_at: SystemTime,
    started: Instant,
    log: Mutex<CallLog>,
    /// `Some` for methods Playwright's spec flags `input`.
    delivery: Option<Arc<DeliveryCell>>,
    /// Set once by whoever ends the call; absent while it runs.
    end: OnceLock<CallEnd>,
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

struct CallEnd {
    result: CallResult,
    duration: Duration,
    /// The folded log, as the reply carried it.
    log: Vec<String>,
}

/// What a call is, known when it arrives.
pub(crate) struct CallStart {
    pub(crate) call_id: u64,
    pub(crate) guid: String,
    pub(crate) method: String,
    pub(crate) title: Option<&'static str>,
    pub(crate) params: Value,
    pub(crate) location: Option<CallLocation>,
    /// Whether the method writes input, so its delivery is tracked.
    pub(crate) input: bool,
}

impl CallEntry {
    pub(crate) fn new(start: CallStart) -> Self {
        let CallStart {
            call_id,
            guid,
            method,
            title,
            params,
            location,
            input,
        } = start;
        Self {
            call_id,
            guid,
            method,
            title,
            params,
            location,
            started_at: SystemTime::now(),
            started: Instant::now(),
            log: Mutex::default(),
            delivery: input.then(Arc::default),
            end: OnceLock::new(),
        }
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

    fn record(&self, reason: RunEnd) -> CallRecord {
        let (result, duration, log) = match self.end.get() {
            Some(end) => (end.result.clone(), end.duration, end.log.clone()),
            None => (
                CallResult::Unfinished { reason },
                self.started.elapsed(),
                self.folded_log(),
            ),
        };
        let started_at_ms = self
            .started_at
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            });
        CallRecord {
            call_id: self.call_id,
            guid: self.guid.clone(),
            method: self.method.clone(),
            title: self.title.map(str::to_string),
            params: self.params.clone(),
            location: self.location,
            started_at_ms,
            duration,
            result,
            delivery: self.delivery.as_ref().map(|cell| cell.delivery()),
            log,
        }
    }

    /// What a kept record costs against [`MAX_TRACE_BYTES`]: its params as JSON plus its
    /// folded log.
    fn record_bytes(&self) -> usize {
        let log: usize = match self.end.get() {
            Some(end) => end.log.iter().map(String::len).sum(),
            None => self.folded_log().iter().map(String::len).sum(),
        };
        self.params.to_string().len() + log
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

/// Every call of one run. The dispatcher appends; the run reads once, after `finish`'s wait.
///
/// Memory stays bounded: at most [`MAX_CALL_RECORDS`] entries are held, each entry's log is
/// capped by [`MAX_CALL_LOG_LINES`], and kept records total at most [`MAX_TRACE_BYTES`]. A call
/// past a cap still runs and replies; only its record is not kept. Its delivery cell is kept (a
/// few bytes), so [`DroppedCalls::delivery`] still reports the least safe delivery among the
/// dropped calls and dropping a record never hides a write.
#[derive(Clone, Default)]
pub(crate) struct Trace(Arc<Mutex<TraceInner>>);

#[derive(Default)]
struct TraceInner {
    /// In arrival order, each with whether its bytes were charged.
    kept: Vec<(Arc<CallEntry>, bool)>,
    kept_bytes: usize,
    /// Calls ever admitted, kept or not, so the record cap keeps the first calls.
    arrived: usize,
    /// Set by the first record that did not fit the byte budget; every later call is dropped,
    /// so the kept records stay the run's first ones.
    full: bool,
    dropped_count: u64,
    dropped_cells: Vec<Arc<DeliveryCell>>,
}

impl Trace {
    /// Registers a call that just arrived: kept while the caps allow, otherwise counted.
    pub(crate) fn push(&self, entry: Arc<CallEntry>) {
        let Ok(mut trace) = self.0.lock() else {
            return;
        };
        trace.arrived += 1;
        if trace.full || trace.arrived > MAX_CALL_RECORDS {
            trace.drop_entry(&entry);
        } else {
            trace.kept.push((entry, false));
        }
    }

    /// Charges a kept call's record bytes when it finishes. Called right after
    /// [`CallEntry::finish`].
    pub(crate) fn finished(&self, entry: &Arc<CallEntry>) {
        let Ok(mut trace) = self.0.lock() else {
            return;
        };
        if let Some(index) = trace
            .kept
            .iter()
            .position(|(kept, _)| Arc::ptr_eq(kept, entry))
        {
            trace.charge(index);
        }
    }

    /// The run's records, in arrival order, and the dropped calls. A call with no end is
    /// `Unfinished { reason }`, its duration runs to now, and its delivery is whatever its cell
    /// proves at this moment. Logs one `tracing` event per record.
    pub(crate) fn take(&self, reason: RunEnd) -> (Vec<CallRecord>, Option<DroppedCalls>) {
        let Ok(mut trace) = self.0.lock() else {
            return (Vec::new(), None);
        };
        let mut index = 0;
        while index < trace.kept.len() {
            if trace.kept[index].1 || trace.charge(index) {
                index += 1;
            }
        }
        let records: Vec<CallRecord> = trace
            .kept
            .drain(..)
            .map(|(entry, _)| entry.record(reason))
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
        let dropped = (trace.dropped_count > 0).then(|| DroppedCalls {
            count: trace.dropped_count,
            delivery: trace.dropped_cells.iter().map(|cell| cell.delivery()).max(),
        });
        (records, dropped)
    }
}

impl TraceInner {
    /// Charges `kept[index]`'s bytes. Returns whether it stays kept; past the budget it moves
    /// to the dropped count and every later call is dropped too.
    fn charge(&mut self, index: usize) -> bool {
        let bytes = self.kept[index].0.record_bytes();
        if !self.full && self.kept_bytes + bytes <= MAX_TRACE_BYTES {
            self.kept_bytes += bytes;
            self.kept[index].1 = true;
            return true;
        }
        self.full = true;
        let (entry, _) = self.kept.remove(index);
        self.drop_entry(&entry);
        false
    }

    fn drop_entry(&mut self, entry: &CallEntry) {
        self.dropped_count += 1;
        self.dropped_cells.extend(entry.delivery.clone());
    }
}

/// Playwright's `compressCallLog` (`server/callLog.ts`) with `findRepeatedSubsequences`,
/// ported line for line. Maximal repeated runs fold into `'  ' + ws + 'N × ' + first line`
/// plus continuation lines, every other line prints as `'  ' + ws + '- ' + line`, and a line's
/// leading whitespace becomes its nesting. The goldens' irregular indentation comes from this
/// function, so it is tested byte for byte against them.
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
fn is_js_whitespace(c: char) -> bool {
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
    use crate::click_goldens::{golden, raw_log};

    fn entry(call_id: u64, input: bool) -> Arc<CallEntry> {
        Arc::new(CallEntry::new(CallStart {
            call_id,
            guid: "frame@T".to_string(),
            method: "click".to_string(),
            title: Some("Click"),
            params: serde_json::json!({ "selector": "#go" }),
            location: None,
            input,
        }))
    }

    fn scope(run_timeout: Duration) -> RunScope {
        RunScope::new(
            Deadline::new(run_timeout, CancellationToken::new()),
            TaskTracker::new(),
        )
    }

    fn progress(timeout_ms: f64, run: &RunScope, page_closed: &CancellationToken) -> Progress {
        Progress::new(entry(1, true), timeout_ms, run.clone(), page_closed.clone())
    }

    #[test]
    fn compress_call_log_reproduces_every_golden_byte_for_byte() {
        for name in ["strict", "hidden", "covered", "disabled"] {
            assert_eq!(
                compress_call_log(&raw_log(name)),
                golden(name).call_log(),
                "the {name} golden"
            );
        }
    }

    #[test]
    fn compress_call_log_prints_lone_and_nested_lines() {
        assert_eq!(compress_call_log(&["waiting for x"]), ["  - waiting for x"]);
        assert_eq!(compress_call_log::<&str>(&[]), Vec::<String>::new());
        // Ten repeats widen the count prefix, and the continuation pads to match it (checked
        // against Playwright's own `compressCallLog`).
        let raw: Vec<&str> = ["a", "  b"].repeat(10);
        assert_eq!(compress_call_log(&raw), ["  10 × a", "       - b"]);
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
        let finished = entry(1, true);
        let running = entry(2, true);
        trace.push(Arc::clone(&finished));
        trace.push(Arc::clone(&running));
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
            let entry = entry(call_id, true);
            trace.push(Arc::clone(&entry));
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
        let entries: Vec<_> = (1..=3).map(|call_id| entry(call_id, false)).collect();
        for entry in &entries {
            trace.push(Arc::clone(entry));
            let line = big.clone();
            entry.update_log(|log| log.push(line));
        }
        for entry in &entries {
            entry.finish(CallResult::Ok);
            trace.finished(entry);
        }
        let late = entry(4, false);
        trace.push(Arc::clone(&late));

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
