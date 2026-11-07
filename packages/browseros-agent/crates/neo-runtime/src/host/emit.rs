//! The run path's one handle on the host side. `Runtime::run` makes a [`RunEmitter`] for each
//! run, and every host event the run causes enters the tap through it.
//!
//! ```text
//!  Runtime::run ── run_started ──────────▶ RunStarted   before the tab check: a run whose tab
//!       │                                               is already closed is still a run
//!       ├─ Trace::push ──────────────────▶ CallStarted  inside the native call
//!       ├─ Trace::finished ──────────────▶ CallEnded    the call replied
//!       ├─ Trace::take ──────────────────▶ CallEnded    the run ended under it: as its record
//!       │                                               reads, else `Unfinished`
//!       └─ RunEnding::ended / its drop ──▶ RunEnded     every return, and a dropped future,
//!                                                       which first ends every open call
//! ```
//!
//! Exactly once each. The emitter keeps the run's open calls (a `CallStarted` and no
//! `CallEnded` yet) apart from the trace's records, so a call whose record the caps dropped
//! still ends on the host side; whoever removes a call from that list sends its `CallEnded`.
//! And nothing after `RunEnded`: the state moves from running to ended once, under the lock held
//! for every push, so a call task that outlives its run is silent. That is what lets a `RunPass`
//! dropped after the run put `PassReleased` behind every event the run emitted.

use std::mem;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::host::Host;
use crate::host::events::HostEvent;
use crate::host::ids::{EpochMs, SessionKey};
use crate::host::tap::Tap;
use crate::run::{CallRecord, CallResult, PageId, RunEnd, RunId, RunStatus};

/// One run's emitter. Cheap to clone. Without a host side, or for a run with no session, it
/// holds nothing and every method returns at once, so such a run does no host work at all.
#[derive(Clone, Default)]
pub(crate) struct RunEmitter(Option<Arc<Emission>>);

struct Emission {
    tap: Arc<Tap>,
    session: SessionKey,
    run: RunId,
    /// Held across each push, so ending a call claims it and queues its `CallEnded` in one step,
    /// and `RunEnded` goes into the tap once with nothing after it.
    state: Mutex<State>,
}

enum State {
    /// The calls with a `CallStarted` and no `CallEnded` yet, in arrival order. A run has at
    /// most 64 calls in flight (`dispatch.rs`), so this stays short.
    Running(Vec<OpenCall>),
    /// `RunEnded` is in the tap; the run emits nothing more.
    Ended,
}

struct OpenCall {
    call_id: u64,
    /// What the call's `CallEnded` names: its target, or the page a context-level `newPage`
    /// created.
    page: Option<PageId>,
}

/// The `RunEnded` a started run owes the host side. [`Self::ended`] sends it with the run's
/// status; dropping the guard unsent (the caller dropped the run's future) sends it as
/// `Cancelled`.
#[must_use = "dropping it ends the run on the host side as cancelled"]
pub(crate) struct RunEnding(RunEmitter);

impl RunEmitter {
    /// The emitter for run `run` of `session`, a no-op unless both a host and a session exist.
    pub(crate) fn new(host: Option<&Host>, session: Option<&SessionKey>, run: &RunId) -> Self {
        let (Some(host), Some(session)) = (host, session) else {
            return Self::default();
        };
        Self(Some(Arc::new(Emission {
            tap: Arc::clone(host.tap()),
            session: session.clone(),
            run: run.clone(),
            state: Mutex::new(State::Running(Vec::new())),
        })))
    }

    /// Puts `RunStarted` for a run starting on `page` in the tap, and returns the `RunEnded` the
    /// run now owes. `Runtime::run` calls it first, before anything that can end the run.
    pub(crate) fn run_started(&self, page: &PageId) -> RunEnding {
        if let Some(emission) = &self.0 {
            emission.push(HostEvent::RunStarted {
                session: emission.session.clone(),
                run: emission.run.clone(),
                page: page.clone(),
                at: stamp(EpochMs::now()),
            });
        }
        RunEnding(self.clone())
    }

    /// `Trace::push`: call `call_id` arrived at `at`, aimed at `page`. Opens the call and puts
    /// `CallStarted` in the tap.
    pub(crate) fn call_started(
        &self,
        call_id: u64,
        page: Option<&PageId>,
        snapshot: bool,
        at: SystemTime,
    ) {
        self.with_open(|emission, open| {
            open.push(OpenCall {
                call_id,
                page: page.cloned(),
            });
            emission.push(HostEvent::CallStarted {
                session: emission.session.clone(),
                run: emission.run.clone(),
                call_id,
                page: page.cloned(),
                snapshot,
                at: stamp(EpochMs::from_system_time(at)),
            });
        });
    }

    /// `Trace::finished`: call `call_id` replied with `result`. Puts its `CallEnded` in the tap,
    /// unless the run's end already ended it.
    pub(crate) fn call_ended(&self, call_id: u64, result: &CallResult) {
        self.with_open(|emission, open| {
            if let Some(index) = open.iter().position(|call| call.call_id == call_id) {
                emission.push(emission.call_ended(open.remove(index), result.clone()));
            }
        });
    }

    /// `Trace::take`: ends every call still open the way `records` (the run's kept records)
    /// read it, and as `Unfinished { reason }` when the caps dropped its record.
    pub(crate) fn unfinished(&self, reason: RunEnd, records: &[CallRecord]) {
        self.with_open(|emission, open| {
            for call in open.drain(..) {
                let result = records
                    .iter()
                    .find(|record| record.call_id == call.call_id)
                    .map_or(CallResult::Unfinished { reason }, |record| {
                        record.result.clone()
                    });
                emission.push(emission.call_ended(call, result));
            }
        });
    }

    /// Ends every call still open as `Unfinished` (the run's future was dropped, or `take`
    /// never ran), then puts `RunEnded` in the tap. Does nothing once the run has ended.
    fn run_ended(&self, status: RunStatus) {
        let Some(emission) = &self.0 else {
            return;
        };
        let Ok(mut state) = emission.state.lock() else {
            return;
        };
        let State::Running(open) = mem::replace(&mut *state, State::Ended) else {
            return;
        };
        let reason = status.run_end();
        for call in open {
            emission.push(emission.call_ended(call, CallResult::Unfinished { reason }));
        }
        emission.push(HostEvent::RunEnded {
            session: emission.session.clone(),
            run: emission.run.clone(),
            status,
            at: stamp(EpochMs::now()),
        });
    }

    /// Runs `step` on the open calls under the state lock, unless the run has ended.
    fn with_open(&self, step: impl FnOnce(&Emission, &mut Vec<OpenCall>)) {
        let Some(emission) = &self.0 else {
            return;
        };
        let Ok(mut state) = emission.state.lock() else {
            return;
        };
        if let State::Running(open) = &mut *state {
            step(emission, open);
        }
    }
}

impl RunEnding {
    pub(crate) fn ended(self, status: RunStatus) {
        self.0.run_ended(status);
    }
}

impl Drop for RunEnding {
    fn drop(&mut self) {
        // After `ended` the run is already over on the host side, and this does nothing.
        self.0.run_ended(RunStatus::Cancelled);
    }
}

impl Emission {
    fn push(&self, event: HostEvent) {
        // The tap counts what it refuses; a run never waits on, or fails for, the host side.
        let _pushed = self.tap.try_push(event);
    }

    fn call_ended(&self, call: OpenCall, result: CallResult) -> HostEvent {
        HostEvent::CallEnded {
            session: self.session.clone(),
            run: self.run.clone(),
            call_id: call.call_id,
            page: call.page,
            result,
        }
    }
}

/// A wall-clock reading as host events carry it. `None` is a clock set before the epoch, which
/// reads as the epoch's first millisecond: a wrong time beats a missing event.
fn stamp(time: Option<EpochMs>) -> EpochMs {
    time.unwrap_or(EpochMs::MIN)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant, UNIX_EPOCH};

    use serde_json::{Value, json};
    use tokio::time;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::fake_browser::{Exchange, FakeBrowser, PAGE, Replies};
    use crate::host::HostOptions;
    use crate::host::tap::{Refused, TAP_QUEUE};
    use crate::progress::{CallEntry, CallStart, Trace};
    use crate::run::{RunOutcome, RunRequest};
    use crate::runtime::{Runtime, RuntimeOptions};

    const SESSION: &str = "7f3a9c21";

    /// How long a test waits for what a run leaves running in the background.
    const SETTLE: Duration = Duration::from_secs(5);

    /// A click that presses, then holds the button for 5 s: mid-call for as long as a test needs.
    const LONG_CLICK: &str = "await page.click('#go', { delay: 5000 })";

    /// Two calls: `Frame.title`, which Playwright's spec does not flag `snapshot`, then
    /// `Frame.click`, which it does.
    const TWO_CALLS: &str = "await page.title(); await page.click('#go'); return page.url()";

    async fn hosted(browser: &FakeBrowser) -> Runtime {
        let options = RuntimeOptions {
            host: Some(HostOptions::new(9222, 1)),
        };
        Runtime::connect_with(browser.cdp().await, options)
            .await
            .unwrap()
    }

    fn request(code: &str) -> RunRequest {
        RunRequest {
            code: code.to_string(),
            page: PageId::new(PAGE.to_string()),
            timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(),
            session: Some(SessionKey::new(SESSION)),
            artifacts: None,
        }
    }

    fn tap(runtime: &Runtime) -> &Tap {
        runtime.host().unwrap().tap()
    }

    /// The tap's events so far with their times cleared, so tests compare what happened.
    fn events(tap: &Tap) -> Vec<HostEvent> {
        tap.drain().into_iter().map(untimed).collect()
    }

    fn untimed(event: HostEvent) -> HostEvent {
        let at = EpochMs::MIN;
        match event {
            HostEvent::RunStarted {
                session, run, page, ..
            } => HostEvent::RunStarted {
                session,
                run,
                page,
                at,
            },
            HostEvent::CallStarted {
                session,
                run,
                call_id,
                page,
                snapshot,
                ..
            } => HostEvent::CallStarted {
                session,
                run,
                call_id,
                page,
                snapshot,
                at,
            },
            HostEvent::RunEnded {
                session,
                run,
                status,
                ..
            } => HostEvent::RunEnded {
                session,
                run,
                status,
                at,
            },
            other => other,
        }
    }

    fn time_of(event: &HostEvent) -> Option<u64> {
        match event {
            HostEvent::RunStarted { at, .. }
            | HostEvent::CallStarted { at, .. }
            | HostEvent::RunEnded { at, .. } => Some(at.get()),
            _ => None,
        }
    }

    fn started(run: &RunId, page: &str) -> HostEvent {
        HostEvent::RunStarted {
            session: SessionKey::new(SESSION),
            run: run.clone(),
            page: PageId::new(page.to_string()),
            at: EpochMs::MIN,
        }
    }

    /// A call on the run's page.
    fn call_started(run: &RunId, call_id: u64, snapshot: bool) -> HostEvent {
        HostEvent::CallStarted {
            session: SessionKey::new(SESSION),
            run: run.clone(),
            call_id,
            page: Some(PageId::new(PAGE.to_string())),
            snapshot,
            at: EpochMs::MIN,
        }
    }

    fn call_ended(run: &RunId, call_id: u64, result: CallResult) -> HostEvent {
        HostEvent::CallEnded {
            session: SessionKey::new(SESSION),
            run: run.clone(),
            call_id,
            page: Some(PageId::new(PAGE.to_string())),
            result,
        }
    }

    fn ended(run: &RunId, status: RunStatus) -> HostEvent {
        HostEvent::RunEnded {
            session: SessionKey::new(SESSION),
            run: run.clone(),
            status,
            at: EpochMs::MIN,
        }
    }

    fn released() -> HostEvent {
        HostEvent::PassReleased {
            session: SessionKey::new(SESSION),
        }
    }

    /// The run a dropped future was running, which no outcome names: the one `RunStarted` names.
    fn run_of(events: &[HostEvent]) -> RunId {
        match events.first() {
            Some(HostEvent::RunStarted { run, .. }) => run.clone(),
            _ => panic!("the first event starts the run: {events:#?}"),
        }
    }

    fn millis_since_epoch() -> u64 {
        let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        u64::try_from(since.as_millis()).unwrap()
    }

    /// Waits until `done` holds for the fake's exchanges.
    async fn until(browser: &FakeBrowser, done: impl Fn(&[Exchange]) -> bool) {
        let give_up = Instant::now() + SETTLE;
        loop {
            let exchanges = browser.exchanges();
            if done(&exchanges) {
                return;
            }
            assert!(Instant::now() < give_up, "never happened: {exchanges:#?}");
            time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Waits until every CDP session the fake handed out was asked to detach. By then every
    /// task a run left behind has ended, since each holds its tab's session, so the run can emit
    /// nothing more.
    async fn detached(browser: &FakeBrowser) {
        until(browser, |exchanges| {
            let count = |method: &str| exchanges.iter().filter(|e| e.method() == method).count();
            let attached = count("Target.attachToTarget");
            attached > 0 && attached == count("Target.detachFromTarget")
        })
        .await;
    }

    /// The CDP methods the fake received, sorted: what was sent, whatever the background order.
    fn methods(browser: &FakeBrowser) -> Vec<String> {
        let mut methods: Vec<String> = browser
            .exchanges()
            .iter()
            .map(|exchange| exchange.method().to_string())
            .collect();
        methods.sort();
        methods
    }

    /// `outcome` without what differs between two runs of one script: its id, now `run_id`,
    /// and its calls' times.
    fn comparable(outcome: &RunOutcome, run_id: &RunId) -> RunOutcome {
        let mut outcome = outcome.clone();
        outcome.run_id = run_id.clone();
        for call in &mut outcome.calls {
            call.started_at_ms = 0;
            call.duration = Duration::ZERO;
        }
        outcome
    }

    /// A trace whose calls report to a fresh host, as a hosted run's trace does.
    fn traced() -> (Host, Trace) {
        let host = Host::new(HostOptions::new(9222, 1));
        let emitter = RunEmitter::new(Some(&host), Some(&SessionKey::new(SESSION)), &RunId::mint());
        (host, Trace::new(emitter))
    }

    fn click(call_id: u64) -> Arc<CallEntry> {
        Arc::new(CallEntry::new(CallStart {
            call_id,
            guid: format!("frame@{PAGE}"),
            method: "click".to_string(),
            title: Some("Click"),
            params: Value::Null,
            location: None,
            input: true,
            page: Some(PageId::new(PAGE.to_string())),
            snapshot: true,
        }))
    }

    /// Each call's `CallEnded` results, by call id, in tap order.
    fn call_ends(events: &[HostEvent]) -> Vec<(u64, CallResult)> {
        events
            .iter()
            .filter_map(|event| match event {
                HostEvent::CallEnded {
                    call_id, result, ..
                } => Some((*call_id, result.clone())),
                _ => None,
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_reports_each_call_once_in_order() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&browser).await;
        let outcome = runtime.run(request(TWO_CALLS)).await;

        assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
        let run = &outcome.run_id;
        assert_eq!(
            events(tap(&runtime)),
            [
                started(run, PAGE),
                call_started(run, 1, false),
                call_ended(run, 1, CallResult::Ok),
                call_started(run, 2, true),
                call_ended(run, 2, CallResult::Ok),
                ended(run, RunStatus::Ok),
            ]
        );
    }

    /// Host times come from the wall clock within the run, in order, and a call's is its
    /// record's own `startedAtMs`, so host views and records sort calls alike.
    #[tokio::test(flavor = "multi_thread")]
    async fn host_times_are_the_run_clock_and_the_records_times() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&browser).await;
        let before = millis_since_epoch();
        let outcome = runtime.run(request(TWO_CALLS)).await;
        let after = millis_since_epoch();

        let times: Vec<u64> = tap(&runtime).drain().iter().filter_map(time_of).collect();
        let recorded: Vec<u64> = outcome
            .calls
            .iter()
            .map(|call| call.started_at_ms)
            .collect();
        assert_eq!(
            times.len(),
            4,
            "the run's start, two calls, its end: {times:?}"
        );
        assert!(
            times.is_sorted() && before <= times[0] && times[3] <= after,
            "{times:?} rise within [{before}, {after}]"
        );
        assert_eq!(times[1..3], recorded[..]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_whose_tab_is_closed_still_starts_and_ends() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&browser).await;
        let mut closed = request("return 1");
        closed.page = PageId::new("closed-tab".to_string());
        let outcome = runtime.run(closed).await;

        assert_eq!(outcome.status, RunStatus::Error, "{outcome:?}");
        assert_eq!(
            events(tap(&runtime)),
            [
                started(&outcome.run_id, "closed-tab"),
                ended(&outcome.run_id, RunStatus::Error)
            ]
        );
    }

    /// The script returns while its click is mid-press. The run's end cuts the click off: it
    /// ends once, `Unfinished` as its record reads, and its task ending later adds nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_call_the_run_cuts_off_ends_once_as_unfinished() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&browser).await;
        let outcome = runtime
            .run(request(
                "page.click('#go', { delay: 5000 })
                 await new Promise((resolve) => setTimeout(resolve, 300))
                 return 'returned'",
            ))
            .await;
        detached(&browser).await;

        let cut_off = CallResult::Unfinished {
            reason: RunEnd::Settled,
        };
        assert_eq!(outcome.value, Some(json!("returned")), "{outcome:?}");
        assert_eq!(outcome.calls[0].result, cut_off, "{outcome:?}");
        let run = &outcome.run_id;
        assert_eq!(
            events(tap(&runtime)),
            [
                started(run, PAGE),
                call_started(run, 1, true),
                call_ended(run, 1, cut_off),
                ended(run, RunStatus::Ok),
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pass_dropped_after_its_run_lands_behind_every_event_of_the_run() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&browser).await;
        let pass = runtime
            .host()
            .unwrap()
            .issue_pass(SessionKey::new(SESSION), None);
        let mut labeled = request(TWO_CALLS);
        labeled.session = Some(pass.session().clone());
        let outcome = runtime.run(labeled).await;
        drop(pass);

        let events = events(tap(&runtime));
        assert_eq!(events.len(), 7, "{events:#?}");
        assert_eq!(events[5], ended(&outcome.run_id, RunStatus::Ok));
        assert_eq!(events[6], released());
    }

    /// The run tool drops a run's future mid-click and then its pass. The click and the run end
    /// on the host side once each, as cancelled, the release lands behind them, and nothing
    /// follows once the click's own task has ended too.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dropped_run_ends_its_calls_and_itself_once_as_cancelled() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&browser).await;
        let pass = runtime
            .host()
            .unwrap()
            .issue_pass(SessionKey::new(SESSION), None);
        let run = tokio::spawn({
            let runtime = runtime.clone();
            async move { runtime.run(request(LONG_CLICK)).await }
        });
        until(&browser, |exchanges| {
            exchanges
                .iter()
                .any(|exchange| exchange.command["params"]["type"] == "mousePressed")
        })
        .await;
        run.abort();
        assert!(run.await.is_err_and(|error| error.is_cancelled()));
        drop(pass);
        detached(&browser).await;

        let events = events(tap(&runtime));
        let run = run_of(&events);
        let cancelled = CallResult::Unfinished {
            reason: RunEnd::Cancelled,
        };
        assert_eq!(
            events,
            [
                started(&run, PAGE),
                call_started(&run, 1, true),
                call_ended(&run, 1, cancelled),
                ended(&run, RunStatus::Cancelled),
                released(),
            ]
        );
    }

    #[test]
    fn a_call_that_ends_after_the_records_were_taken_ends_once() {
        let (host, trace) = traced();
        let entry = click(1);
        trace.push(Arc::clone(&entry));
        let (records, _) = trace.take(RunEnd::Settled);
        entry.finish(CallResult::Ok);
        trace.finished(&entry);

        let cut_off = CallResult::Unfinished {
            reason: RunEnd::Settled,
        };
        assert_eq!(records[0].result, cut_off);
        assert_eq!(call_ends(&host.tap().drain()), [(1, cut_off)]);
    }

    /// The run's future is dropped with a call open: the run ends the call and itself, and a
    /// call ending or starting later adds nothing, so a pass released next lands last.
    #[test]
    fn nothing_follows_the_end_of_a_run() {
        let host = Host::new(HostOptions::new(9222, 1));
        let run = RunId::mint();
        let emitter = RunEmitter::new(Some(&host), Some(&SessionKey::new(SESSION)), &run);
        let ending = emitter.run_started(&PageId::new(PAGE.to_string()));
        let trace = Trace::new(emitter);
        let entry = click(1);
        trace.push(Arc::clone(&entry));
        drop(ending);
        entry.finish(CallResult::Ok);
        trace.finished(&entry);
        trace.push(click(2));

        let cancelled = CallResult::Unfinished {
            reason: RunEnd::Cancelled,
        };
        assert_eq!(
            events(host.tap()),
            [
                started(&run, PAGE),
                call_started(&run, 1, true),
                call_ended(&run, 1, cancelled),
                ended(&run, RunStatus::Cancelled),
            ]
        );
    }

    /// The call's task ends it just as the run takes its records: the record reads `Ok`, and
    /// so does the host side, whichever of the two reports it first.
    #[test]
    fn a_call_that_ended_as_the_records_were_taken_reads_the_same_on_both_sides() {
        let (host, trace) = traced();
        let entry = click(1);
        trace.push(Arc::clone(&entry));
        entry.finish(CallResult::Ok);
        let (records, _) = trace.take(RunEnd::Settled);
        trace.finished(&entry);

        assert_eq!(records[0].result, CallResult::Ok);
        assert_eq!(call_ends(&host.tap().drain()), [(1, CallResult::Ok)]);
    }

    /// The record caps keep the first 1000 calls; the host side still sees every call start and
    /// end once, the last one cut off by the run's end with no record to read.
    #[test]
    fn calls_past_the_record_caps_still_start_and_end_once_each() {
        let (host, trace) = traced();
        for call_id in 1..=1002 {
            let entry = click(call_id);
            trace.push(Arc::clone(&entry));
            if call_id < 1002 {
                entry.finish(CallResult::Ok);
                trace.finished(&entry);
            }
        }
        let (records, dropped) = trace.take(RunEnd::Settled);

        assert_eq!(records.len(), 1000);
        assert_eq!(dropped.map(|dropped| dropped.count), Some(2));
        let events = host.tap().drain();
        let starts = events
            .iter()
            .filter(|event| matches!(event, HostEvent::CallStarted { .. }))
            .count();
        assert_eq!(starts, 1002);
        let expected: Vec<(u64, CallResult)> = (1..=1001)
            .map(|call_id| (call_id, CallResult::Ok))
            .chain([(
                1002,
                CallResult::Unfinished {
                    reason: RunEnd::Settled,
                },
            )])
            .collect();
        assert_eq!(call_ends(&events), expected);
    }

    /// A ledger that never drains: with the per-call band full, runs go on exactly as without a
    /// host, while their lifecycle events still enter the band. Past the band, a run's lost
    /// start and end mark its session damaged, and only that session.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_full_tap_never_changes_a_run() {
        let plain_browser = FakeBrowser::start(Replies::default()).await;
        let plain = plain_browser.runtime().await.run(request(TWO_CALLS)).await;
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&browser).await;
        let tap = tap(&runtime);
        let stale = HostEvent::CallStarted {
            session: SessionKey::new("stale"),
            run: RunId::mint(),
            call_id: 1,
            page: None,
            snapshot: false,
            at: EpochMs::MIN,
        };
        for _ in 0..TAP_QUEUE {
            let _queued = tap.try_push(stale.clone());
        }
        let full = runtime
            .run(RunRequest {
                session: Some(SessionKey::new("full")),
                ..request(TWO_CALLS)
            })
            .await;
        // A band of one run's four slots, two of them now taken by `full`.
        for _ in 0..2 {
            let _queued = tap.try_push(HostEvent::PassReleased {
                session: SessionKey::new("stale"),
            });
        }
        let past_band = runtime
            .run(RunRequest {
                session: Some(SessionKey::new("past-band")),
                ..request(TWO_CALLS)
            })
            .await;

        assert_eq!(plain.status, RunStatus::Ok, "{plain:?}");
        let plain_run = &plain.run_id;
        let plain = comparable(&plain, plain_run);
        assert_eq!(comparable(&full, plain_run), plain);
        assert_eq!(comparable(&past_band, plain_run), plain);
        assert_eq!(
            tap.refused(),
            Refused {
                calls: 8,
                lifecycle: 2
            },
            "each run's four call events, and the second run's start and end"
        );
        assert_eq!(tap.take_damaged(), [SessionKey::new("past-band")]);
        let queued: Vec<HostEvent> = tap.drain().into_iter().map(untimed).collect();
        assert_eq!(queued.len(), TAP_QUEUE + 4);
        assert_eq!(
            queued[TAP_QUEUE..],
            [
                HostEvent::RunStarted {
                    session: SessionKey::new("full"),
                    run: full.run_id.clone(),
                    page: PageId::new(PAGE.to_string()),
                    at: EpochMs::MIN,
                },
                HostEvent::RunEnded {
                    session: SessionKey::new("full"),
                    run: full.run_id.clone(),
                    status: RunStatus::Ok,
                    at: EpochMs::MIN,
                },
                HostEvent::PassReleased {
                    session: SessionKey::new("stale"),
                },
                HostEvent::PassReleased {
                    session: SessionKey::new("stale"),
                },
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_without_a_session_tells_the_host_nothing() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&browser).await;
        let outcome = runtime
            .run(RunRequest {
                session: None,
                ..request(TWO_CALLS)
            })
            .await;

        assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
        assert!(tap(&runtime).drain().is_empty());
    }

    /// The host side sends no CDP command of its own: a tracked run on a runtime with a host
    /// sends exactly what the same run sends on a runtime without one.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_host_side_adds_no_cdp_traffic_to_a_run() {
        let plain = FakeBrowser::start(Replies::default()).await;
        let untracked = plain.runtime().await.run(request(TWO_CALLS)).await;
        let tracked_browser = FakeBrowser::start(Replies::default()).await;
        let runtime = hosted(&tracked_browser).await;
        let tracked = runtime.run(request(TWO_CALLS)).await;

        assert_eq!(untracked.status, RunStatus::Ok, "{untracked:?}");
        assert_eq!(tracked.status, RunStatus::Ok, "{tracked:?}");
        assert_eq!(events(tap(&runtime)).len(), 6, "the tracked run reported");
        detached(&plain).await;
        detached(&tracked_browser).await;
        assert_eq!(methods(&tracked_browser), methods(&plain));
    }
}
