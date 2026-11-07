//! What a run tells its trace sinks: every run asks each installed sink, before anything can
//! fail, whether it listens, and a sink that does hears the run's events in the order
//! `RunEvent` promises, `RunSettled` last. Records leave no secret behind, while the script
//! sees what Playwright would show it. Against the fake browser in `support/fake_browser.rs`;
//! no browser needed.

#[path = "support/fake_browser.rs"]
#[expect(dead_code, reason = "these tests read no page state")]
mod fake_browser;

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, MutexGuard, Once};
use std::time::{Duration, Instant};

use fake_browser::{FakeBrowser, PAGE, Replies, Reply};
use neo_runtime::{
    CallResult, Delivery, EndCause, GetterField, GetterResult, IdKey, PageId, RunEvent, RunId,
    RunOrigin, RunRequest, RunSink, RunStarted, RunStatus, Runtime, SessionKey, TraceSink,
};
use neo_runtime::{RunEnd, RunOutcome};
use serde_json::json;
use tokio::time;
use tokio_util::sync::CancellationToken;
use tracing::field::{Field, Visit};
use tracing::{Event, Metadata, Subscriber};
use tracing::{span, subscriber};

/// How long a test waits for a run to settle after its outcome: writers may hold its state a
/// while longer.
const SETTLE: Duration = Duration::from_secs(10);

/// One thing a recording sink saw: an event, or a run's sink being dropped.
#[derive(Debug, Clone)]
enum Seen {
    Started(RunStarted),
    Event(RunEvent),
    SinkDropped(RunId),
}

/// Records every run it is asked about and, when it listens, everything its runs' sinks see,
/// in order across runs. One that does not listen skips each run; both are asked all the same.
struct RecordingSink {
    listens: bool,
    runs: Mutex<Vec<RunStarted>>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

/// One run's sink: it queues what it is handed and notes its own drop, which the runtime does
/// right after `RunSettled`.
struct RunRecorder {
    run: RunId,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl RunSink for RunRecorder {
    fn emit(&self, event: &RunEvent) {
        lock(&self.seen).push(Seen::Event(event.clone()));
    }
}

impl Drop for RunRecorder {
    fn drop(&mut self) {
        lock(&self.seen).push(Seen::SinkDropped(self.run));
    }
}

impl RecordingSink {
    fn new(listens: bool) -> Arc<Self> {
        Arc::new(Self {
            listens,
            runs: Mutex::default(),
            seen: Arc::default(),
        })
    }

    fn runs(&self) -> MutexGuard<'_, Vec<RunStarted>> {
        lock(&self.runs)
    }

    fn run_ids(&self) -> Vec<RunId> {
        self.runs().iter().map(|started| started.run.id).collect()
    }

    /// Everything seen of `run` so far, in order.
    fn seen_of(&self, run: RunId) -> Vec<Seen> {
        lock(&self.seen)
            .iter()
            .filter(|seen| match seen {
                Seen::Started(started) => started.run.id == run,
                Seen::Event(event) => run_of(event) == run,
                Seen::SinkDropped(dropped) => *dropped == run,
            })
            .cloned()
            .collect()
    }

    /// Waits until `run`'s sink was dropped, then returns its events after checking the order
    /// every run keeps (`checked`).
    async fn settled(&self, run: RunId) -> Vec<RunEvent> {
        let give_up_at = Instant::now() + SETTLE;
        loop {
            let seen = self.seen_of(run);
            if seen.iter().any(|seen| matches!(seen, Seen::SinkDropped(_))) {
                return checked(&seen);
            }
            assert!(
                Instant::now() < give_up_at,
                "run {run} never settled: {seen:#?}"
            );
            time::sleep(Duration::from_millis(10)).await;
        }
    }
}

impl TraceSink for RecordingSink {
    fn run(&self, started: &RunStarted) -> Option<Arc<dyn RunSink>> {
        self.runs().push(started.clone());
        if !self.listens {
            return None;
        }
        lock(&self.seen).push(Seen::Started(started.clone()));
        let recorder: Arc<dyn RunSink> = Arc::new(RunRecorder {
            run: started.run.id,
            seen: Arc::clone(&self.seen),
        });
        Some(recorder)
    }
}

/// The run an event belongs to.
fn run_of(event: &RunEvent) -> RunId {
    match event {
        RunEvent::BeforeCall { run, .. }
        | RunEvent::InputSent { run, .. }
        | RunEvent::AfterCall { run, .. }
        | RunEvent::DeliverySettled { run, .. }
        | RunEvent::PageOpened { run, .. }
        | RunEvent::PageClosed { run, .. }
        | RunEvent::Getter { run, .. }
        | RunEvent::RunEnded { run, .. }
        | RunEvent::RunSettled { run } => *run,
    }
}

/// A run's events, after asserting what `RunEvent` promises for one run: exactly one
/// `RunEnded`; `RunSettled` last, its sink dropped right after; and for every call number in
/// `1..=calls_made`, exactly one `AfterCall`, with its `BeforeCall` before every other event of
/// the call and at most one `DeliverySettled`, after its `AfterCall`.
fn checked(seen: &[Seen]) -> Vec<RunEvent> {
    let [
        Seen::Started(_),
        events @ ..,
        Seen::Event(RunEvent::RunSettled { .. }),
        Seen::SinkDropped(_),
    ] = seen
    else {
        panic!("RunSettled last, then the sink is dropped: {seen:#?}");
    };
    let events: Vec<RunEvent> = events
        .iter()
        .map(|seen| match seen {
            Seen::Event(event) => event.clone(),
            Seen::Started(_) | Seen::SinkDropped(_) => {
                panic!("the sink was dropped before it settled: {seen:#?}")
            }
        })
        .collect();
    let ended: Vec<&RunEvent> = events
        .iter()
        .filter(|event| matches!(event, RunEvent::RunEnded { .. }))
        .collect();
    let [RunEvent::RunEnded { calls_made, .. }] = ended.as_slice() else {
        panic!("exactly one RunEnded: {events:#?}");
    };
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, RunEvent::RunSettled { .. })),
        "RunSettled only once: {events:#?}"
    );
    for call in 1..=*calls_made {
        let of_call: Vec<&RunEvent> = events
            .iter()
            .filter(|event| call_of(event) == Some(call))
            .collect();
        let after = of_call
            .iter()
            .filter(|event| matches!(event, RunEvent::AfterCall { .. }))
            .count();
        assert_eq!(after, 1, "call {call} ended exactly once: {of_call:#?}");
        assert!(
            matches!(of_call.first(), Some(RunEvent::BeforeCall { .. })),
            "call {call} starts with BeforeCall: {of_call:#?}"
        );
        assert_eq!(
            of_call
                .iter()
                .filter(|event| matches!(event, RunEvent::BeforeCall { .. }))
                .count(),
            1,
            "exactly one start per call"
        );
        assert!(
            of_call
                .iter()
                .filter(|event| matches!(event, RunEvent::InputSent { .. }))
                .count()
                <= 1,
            "at most one input-sent fact per call"
        );
        let settled: Vec<usize> = of_call
            .iter()
            .enumerate()
            .filter(|(_, event)| matches!(event, RunEvent::DeliverySettled { .. }))
            .map(|(index, _)| index)
            .collect();
        let after_at = of_call
            .iter()
            .position(|event| matches!(event, RunEvent::AfterCall { .. }));
        assert!(
            settled.len() <= 1,
            "at most one DeliverySettled: {of_call:#?}"
        );
        assert!(
            settled.iter().all(|index| Some(*index) > after_at),
            "DeliverySettled comes after AfterCall: {of_call:#?}"
        );
    }
    let numbered = events.iter().filter_map(call_of).max().unwrap_or(0);
    assert!(
        numbered <= *calls_made,
        "no call past calls_made ({calls_made}): {events:#?}"
    );
    events
}

/// The call number an event is about, if any.
fn call_of(event: &RunEvent) -> Option<u64> {
    match event {
        RunEvent::BeforeCall { call, .. }
        | RunEvent::InputSent { call, .. }
        | RunEvent::AfterCall { call, .. }
        | RunEvent::DeliverySettled { call, .. } => Some(*call),
        RunEvent::PageOpened { .. }
        | RunEvent::PageClosed { .. }
        | RunEvent::Getter { .. }
        | RunEvent::RunEnded { .. }
        | RunEvent::RunSettled { .. } => None,
    }
}

fn lock<T>(shared: &Mutex<T>) -> MutexGuard<'_, T> {
    shared
        .lock()
        .unwrap_or_else(|error| panic!("a sink panicked while recording: {error}"))
}

async fn runtime_with(browser: &FakeBrowser, sinks: &[&Arc<RecordingSink>]) -> Runtime {
    let sinks = sinks
        .iter()
        .map(|sink| Arc::clone(*sink) as Arc<dyn TraceSink>);
    browser.runtime().await.with_sinks(sinks)
}

fn request(code: &str) -> RunRequest {
    fake_browser::run_request(code, Duration::from_secs(5), CancellationToken::new())
}

/// The run's one `RunEnded`.
fn run_ended(events: &[RunEvent]) -> &RunEvent {
    events
        .iter()
        .find(|event| matches!(event, RunEvent::RunEnded { .. }))
        .unwrap_or_else(|| panic!("the run ended: {events:#?}"))
}

/// Every `tracing` event logged on any thread of this test binary, as text, so a test can check
/// that no log line holds a secret.
static LOGGED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Installs the subscriber behind [`LOGGED`], once per test binary.
fn capture_logs() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        subscriber::set_global_default(Capture)
            .unwrap_or_else(|error| panic!("install the log capture: {error}"));
    });
}

struct Capture;

impl Subscriber for Capture {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }

    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}

    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut text = String::new();
        event.record(&mut Fields(&mut text));
        lock(&LOGGED).push(text);
    }

    fn enter(&self, _: &span::Id) {}

    fn exit(&self, _: &span::Id) {}
}

struct Fields<'t>(&'t mut String);

impl Visit for Fields<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let _ = write!(self.0, "{}={value:?} ", field.name());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_sink_hears_of_each_run_once_under_its_outcomes_id() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let listening = RecordingSink::new(true);
    let skipping = RecordingSink::new(false);
    let runtime = runtime_with(&browser, &[&listening, &skipping]).await;

    let finished = runtime.run(request("return page.url()")).await;
    let closed_tab = runtime
        .run(RunRequest {
            page: PageId::new("closed-tab".to_string()),
            ..request("return 1")
        })
        .await;

    assert_eq!(finished.status, RunStatus::Ok, "{finished:?}");
    assert_eq!(
        closed_tab.error.as_ref().map(|error| error.name.as_str()),
        Some("TargetClosedError"),
        "the run ended before its script: {closed_tab:?}"
    );
    assert_ne!(
        finished.run_id, closed_tab.run_id,
        "each run has its own id"
    );
    let outcomes = [finished.run_id, closed_tab.run_id];
    assert_eq!(
        listening.run_ids(),
        outcomes,
        "a listening sink heard of each run once, under its outcome's id"
    );
    assert_eq!(
        skipping.run_ids(),
        outcomes,
        "a sink that skips runs was still asked about each one"
    );
    assert!(
        lock(&skipping.seen).is_empty(),
        "a sink that skips a run hears none of its events"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_tells_its_sinks_what_it_is_within_the_origin_caps() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let sink = RecordingSink::new(true);
    let runtime = runtime_with(&browser, &[&sink]).await;
    let key = |n: usize| IdKey::new(&format!("host.id-{n:02}")).unwrap();
    let ids: BTreeMap<IdKey, String> = (0..20).map(|n| (key(n), "v".repeat(300))).collect();
    let code = "return 1";

    let outcome = runtime
        .run(RunRequest {
            origin: RunOrigin {
                session: Some(SessionKey::new("task-42")),
                client: Some("é".repeat(200)),
                ids,
            },
            ..request(code)
        })
        .await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    let runs = sink.runs();
    let [started] = runs.as_slice() else {
        panic!("one run, announced once: {runs:?}");
    };
    assert_eq!(started.run.id, outcome.run_id, "under the outcome's id");
    assert_eq!(
        started.run.session,
        Some(SessionKey::new("task-42")),
        "the host's session"
    );
    assert_eq!(
        started.page,
        PageId::new(PAGE.to_string()),
        "the tab the run starts from"
    );
    assert_eq!(started.timeout_ms, 5000, "the run's budget");
    assert_eq!(
        started.code_bytes,
        u64::try_from(code.len()).unwrap(),
        "the script's length"
    );
    assert_eq!(
        started.ids.keys().cloned().collect::<Vec<_>>(),
        (0..16).map(key).collect::<Vec<_>>(),
        "the first 16 ids in key order"
    );
    assert!(
        started.ids.values().all(|value| *value == "v".repeat(256)),
        "each id's value keeps its first 256 bytes: {:?}",
        started.ids
    );
    assert_eq!(
        started.client,
        Some("é".repeat(128)),
        "the client keeps its first 256 bytes, whole characters only"
    );
}

/// On tokio's current-thread runtime a getter could not block, so the run refuses to start,
/// before its deadline or its tab exist. Its sinks heard of it all the same, and of its end.
#[tokio::test]
async fn a_run_refused_before_its_tab_opens_was_still_announced() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let sink = RecordingSink::new(true);
    let runtime = runtime_with(&browser, &[&sink]).await;

    let outcome = runtime.run(request("return 1")).await;

    assert_eq!(outcome.status, RunStatus::Error, "{outcome:?}");
    assert_eq!(
        sink.run_ids(),
        [outcome.run_id],
        "announced once, under the outcome's id"
    );
    let events = sink.settled(outcome.run_id).await;
    assert!(
        matches!(
            events.as_slice(),
            [RunEvent::RunEnded {
                status: RunStatus::Error,
                cause: EndCause::Unsupported,
                ..
            }]
        ),
        "only its end, as unsupported: {events:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_reports_its_end_with_its_console_and_settles_last() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let sink = RecordingSink::new(true);
    let runtime = runtime_with(&browser, &[&sink]).await;
    let code = "console.log('one'); console.log('two', 2); return { done: true }";

    let outcome = runtime.run(request(code)).await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.logs,
        ["one", "two 2"],
        "the outcome keeps the console"
    );
    let events = sink.settled(outcome.run_id).await;
    let RunEvent::RunEnded {
        status,
        cause,
        error,
        value,
        value_truncated,
        logs,
        script,
        calls_made,
        unreadable_calls,
        ..
    } = run_ended(&events)
    else {
        panic!("a run reports its end: {events:#?}");
    };
    assert_eq!(
        (*status, *cause, error, *calls_made, *unreadable_calls),
        (RunStatus::Ok, EndCause::Returned, &None, 0, 0)
    );
    assert_eq!(
        value.as_deref(),
        Some(&json!({ "done": true })),
        "the value returned"
    );
    assert!(!value_truncated, "the value was kept");
    assert_eq!(
        logs.iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["one", "two 2"],
        "the console, line by line"
    );
    assert!(
        logs.windows(2)
            .all(|pair| pair[0].offset_us <= pair[1].offset_us),
        "each line is timed in order: {logs:?}"
    );
    assert_eq!(script.as_deref(), Some(code), "the script that ran");
}

#[tokio::test(flavor = "multi_thread")]
async fn early_exits_report_only_how_the_run_ended() {
    let held_attach = FakeBrowser::start(Replies::default()).await;
    let sink = RecordingSink::new(true);
    let runtime = runtime_with(&held_attach, &[&sink]).await;

    let closed_tab = runtime
        .run(RunRequest {
            page: PageId::new("closed-tab".to_string()),
            ..request("return 1")
        })
        .await;
    held_attach.page.lock().unwrap().tab_info_delay = Some(Duration::from_secs(1));
    let cancel = CancellationToken::new();
    let cancelled = {
        let cancelling = cancel.clone();
        tokio::spawn(async move {
            time::sleep(Duration::from_millis(200)).await;
            cancelling.cancel();
        });
        runtime
            .run(fake_browser::run_request(
                "return 1",
                Duration::from_secs(5),
                cancel,
            ))
            .await
    };

    for (outcome, status, cause) in [
        (&closed_tab, RunStatus::Error, EndCause::TabUnavailable),
        (&cancelled, RunStatus::Cancelled, EndCause::Cancelled),
    ] {
        assert_eq!(outcome.status, status, "{outcome:?}");
        let events = sink.settled(outcome.run_id).await;
        match events.as_slice() {
            [
                RunEvent::RunEnded {
                    status: reported,
                    cause: reported_cause,
                    calls_made: 0,
                    ..
                },
            ] => assert_eq!((*reported, *reported_cause), (status, cause)),
            _ => panic!("only its end, {cause:?}: {events:#?}"),
        }
    }
}

/// A host that drops `run`'s future gets no outcome, but its sinks hear the run end, as
/// cancelled by a drop, with the console lines written so far.
#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_run_still_ends_and_settles() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let sink = RecordingSink::new(true);
    let runtime = runtime_with(&browser, &[&sink]).await;
    let forever = "console.log('before'); await new Promise(() => {})";

    let timed_out = time::timeout(Duration::from_millis(500), runtime.run(request(forever))).await;
    assert!(timed_out.is_err(), "the host gave up on the run");
    let aborted = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.run(request(forever)).await })
    };
    time::sleep(Duration::from_millis(500)).await;
    aborted.abort();
    assert!(aborted.await.is_err_and(|error| error.is_cancelled()));

    let runs = sink.run_ids();
    assert_eq!(runs.len(), 2, "both runs were announced");
    for run in runs {
        let events = sink.settled(run).await;
        match run_ended(&events) {
            RunEvent::RunEnded {
                status: RunStatus::Cancelled,
                cause: EndCause::Dropped,
                logs,
                ..
            } => assert_eq!(
                logs.iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>(),
                ["before"],
                "the lines written before the drop"
            ),
            ended => panic!("ended by the drop: {ended:#?}"),
        }
    }
}

/// A message the door cannot read has no id, so it is no call: the run counts it.
#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_message_is_counted_and_is_no_call() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let sink = RecordingSink::new(true);
    let runtime = runtime_with(&browser, &[&sink]).await;

    let outcome = runtime
        .run(request(
            "try { await __neo_runtime.call('not json') } catch (error) { return error.message }",
        ))
        .await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert!(outcome.calls.is_empty(), "{outcome:?}");
    let events = sink.settled(outcome.run_id).await;
    assert!(
        matches!(
            run_ended(&events),
            RunEvent::RunEnded {
                calls_made: 0,
                unreadable_calls: 1,
                ..
            }
        ),
        "counted, with no call event: {events:#?}"
    );
}

/// Full Playwright parity for what the script sees, and no secret in any record: the script
/// fills a value, the fill fails, and the error the script catches shows the value as
/// Playwright's would, while no event, no outcome record and no log line holds it.
#[tokio::test(flavor = "multi_thread")]
async fn a_filled_secret_reaches_the_script_and_no_record() {
    capture_logs();
    let browser = FakeBrowser::start(Replies {
        fill: Reply {
            delay: Duration::from_millis(600),
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let sink = RecordingSink::new(true);
    let runtime = runtime_with(&browser, &[&sink]).await;
    let secret = "hunter2-correct-horse";
    let code = format!(
        "const secret = '{secret}'
try {{ await page.fill('#go', secret, {{ timeout: 300 }}) }} catch (error) {{ return error.message.includes(secret) }}
return 'filled'"
    );

    let outcome = runtime.run(request(&code)).await;

    assert_eq!(
        outcome.value,
        Some(json!(true)),
        "the script saw its value in the error, as Playwright shows it: {outcome:?}"
    );
    let [fill] = outcome.calls.as_slice() else {
        panic!("one call: {outcome:?}");
    };
    assert!(matches!(&fill.result, CallResult::Error { name, .. } if name == "TimeoutError"));
    assert_eq!(
        fill.params["value"],
        format!("[redacted {}]", secret.len()),
        "the record keeps only the value's length"
    );
    let records = serde_json::to_string(&outcome.calls).unwrap();
    assert!(!records.contains(secret), "no outcome record: {records}");
    let events = sink.settled(outcome.run_id).await;
    for event in &events {
        let text = serde_json::to_string(event).unwrap();
        assert!(!text.contains(secret), "no event: {text}");
    }
    let logged = lock(&LOGGED).clone();
    assert!(
        logged.iter().any(|line| line.contains("browser call")),
        "the run logged its call: {logged:#?}"
    );
    assert!(
        logged.iter().all(|line| !line.contains(secret)),
        "no log line: {logged:#?}"
    );
}

#[expect(
    clippy::large_types_passed_by_value,
    reason = "each test owns a copy of the fake browser's reply schedule"
)]
async fn case(code: &str, replies: Replies, timeout: Duration) -> (RunOutcome, Vec<RunEvent>) {
    let browser = FakeBrowser::start(replies).await;
    let sink = RecordingSink::new(true);
    let runtime = runtime_with(&browser, &[&sink]).await;
    let outcome = runtime
        .run(RunRequest {
            timeout,
            ..request(code)
        })
        .await;
    let events = sink.settled(outcome.run_id).await;
    for record in &outcome.calls {
        let emitted = events.iter().find_map(|event| match event {
            RunEvent::AfterCall {
                call,
                record: emitted,
                ..
            } if *call == record.call_id => Some(emitted.as_ref()),
            _ => None,
        });
        let mut snapshot = record.clone();
        // Only delivery may advance between publication and the outcome's drain.
        snapshot.delivery = emitted.and_then(|record| record.delivery);
        assert_eq!(emitted, Some(&snapshot), "the sealed record agrees");
    }
    (outcome, events)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_successful_call_lands_once() {
    let (outcome, _) = case(
        "await page.click('#go'); return 7",
        Replies::default(),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(outcome.value, Some(json!(7)));
    assert_eq!(outcome.calls[0].result, CallResult::Ok);
    assert_eq!(outcome.calls[0].delivery, Some(Delivery::Done));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_handler_error_lands_once() {
    let (outcome, events) = case(
        "await page.fill('#go', 'text')",
        Replies::default(),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Error);
    assert!(matches!(&outcome.calls[0].result, CallResult::Error { name, .. } if name == "Error"));
    assert!(matches!(
        run_ended(&events),
        RunEvent::RunEnded {
            cause: EndCause::Threw,
            ..
        }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_calls_own_timeout_lands_once() {
    let (outcome, _) = case(
        "try { await page.click('#absent', {timeout: 100}) } catch(e) { return e.name }",
        Replies::default(),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(outcome.value, Some(json!("TimeoutError")));
    assert!(
        matches!(&outcome.calls[0].result, CallResult::Error { name, .. } if name == "TimeoutError")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_timeout_lands_every_call_once() {
    let (outcome, events) = case(
        "await page.click('#absent')",
        Replies::default(),
        Duration::from_millis(300),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Timeout);
    assert_eq!(outcome.calls.len(), 1);
    assert!(matches!(
        &outcome.calls[0].result,
        CallResult::Unfinished {
            reason: RunEnd::Timeout
        }
    ));
    assert!(matches!(
        run_ended(&events),
        RunEvent::RunEnded {
            cause: EndCause::Deadline,
            ..
        }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_lands_every_call_once() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let sink = RecordingSink::new(true);
    let runtime = runtime_with(&browser, &[&sink]).await;
    let cancel = CancellationToken::new();
    let pending = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            runtime
                .run(fake_browser::run_request(
                    "await page.click('#absent')",
                    Duration::from_secs(5),
                    cancel,
                ))
                .await
        }
    });
    wait_for_call(&sink, 1).await;
    cancel.cancel();
    let outcome = pending.await.unwrap();
    let events = sink.settled(outcome.run_id).await;
    assert_eq!(outcome.status, RunStatus::Cancelled);
    assert_eq!(outcome.calls.len(), 1);
    assert!(matches!(
        run_ended(&events),
        RunEvent::RunEnded {
            cause: EndCause::Cancelled,
            ..
        }
    ));
}

async fn wait_for_call(sink: &RecordingSink, call: u64) {
    time::timeout(SETTLE, async {
        loop {
            if lock(&sink.seen).iter().any(|seen| matches!(seen, Seen::Event(RunEvent::BeforeCall { call: n, .. }) if *n == call)) { return; }
            time::sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap_or_else(|error| panic!("the call never reached the door: {error}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_left_unawaited_lands_once() {
    let (outcome, _) = case(
        "page.click('#absent').catch(() => {}); return 8",
        Replies::default(),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(outcome.value, Some(json!(8)));
    assert_eq!(outcome.calls.len(), 1);
    assert!(matches!(
        &outcome.calls[0].result,
        CallResult::Unfinished {
            reason: RunEnd::Settled
        }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn calls_past_the_outcome_cap_still_emit() {
    let (outcome, events) = case("for(let i=0; i<1002; i++) { try { await page.click('#go', {modifiers:['Shift']}) } catch(e) {} }", Replies::default(), Duration::from_secs(10)).await;
    assert_eq!(outcome.status, RunStatus::Ok);
    assert_eq!(outcome.calls.len(), 1000);
    assert_eq!(outcome.calls_dropped.as_ref().map(|d| d.count), Some(2));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::AfterCall { .. }))
            .count(),
        1002
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_65th_in_flight_call_is_recorded_as_refused() {
    let (outcome, events) = case("const calls = Array.from({length:65}, () => page.click('#absent').catch(e => e.name)); return await calls[64]", Replies::default(), Duration::from_secs(5)).await;
    assert_eq!(outcome.value, Some(json!("Error")));
    assert_eq!(outcome.calls.len(), 65);
    assert!(matches!(&outcome.calls[64].result, CallResult::Error { name, .. } if name == "Error"));
    assert!(matches!(
        run_ended(&events),
        RunEvent::RunEnded { calls_made: 65, .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_guid_is_recorded() {
    let (outcome, _) = case("return JSON.parse(await __neo_runtime.call(JSON.stringify({id:1,guid:'frame@missing',method:'click',params:{value:'hidden-string'}}))).error.error.name", Replies::default(), Duration::from_secs(5)).await;
    assert_eq!(outcome.value, Some(json!("TargetClosedError")));
    assert_eq!(outcome.calls[0].params, json!({"value":"[redacted 13]"}));
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_params_are_recorded() {
    let (outcome, _) = case("return JSON.parse(await __neo_runtime.call(JSON.stringify({id:1,guid:'frame@fake-page',method:'click',params:{selector:5}}))).error.error.name", Replies::default(), Duration::from_secs(5)).await;
    assert_eq!(outcome.value, Some(json!("Error")));
    assert!(matches!(&outcome.calls[0].result, CallResult::Error { name, .. } if name == "Error"));
}

#[tokio::test(flavor = "multi_thread")]
async fn dropped_futures_land_in_flight_calls_for_timeout_and_abort() {
    for abort in [false, true] {
        let browser = FakeBrowser::start(Replies::default()).await;
        let sink = RecordingSink::new(true);
        let runtime = runtime_with(&browser, &[&sink]).await;
        let work = async move {
            runtime
                .run(request(
                    "page.click('#absent').catch(() => {}); await new Promise(() => {})",
                ))
                .await
        };
        if abort {
            let pending = tokio::spawn(work);
            wait_for_call(&sink, 1).await;
            pending.abort();
            assert!(pending.await.is_err_and(|error| error.is_cancelled()));
        } else {
            let expired = time::timeout(Duration::from_millis(500), work).await;
            assert!(expired.is_err());
        }
        let run = sink.run_ids()[0];
        let events = sink.settled(run).await;
        assert!(matches!(
            run_ended(&events),
            RunEvent::RunEnded {
                cause: EndCause::Dropped,
                calls_made: 1,
                ..
            }
        ));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn late_delivery_corrects_unknown_after_after_call() {
    let (outcome, events) = case(
        "try { await page.click('#go', {timeout:100}) } catch(e) { return e.name }",
        Replies {
            mouse: Reply {
                delay: Duration::from_millis(1500),
                ..Reply::default()
            },
            ..Replies::default()
        },
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(outcome.value, Some(json!("TimeoutError")));
    assert_eq!(outcome.calls[0].delivery, Some(Delivery::Unknown));
    assert!(events.iter().any(|event| matches!(
        event,
        RunEvent::DeliverySettled {
            call: 1,
            delivery: Delivery::Done,
            ..
        }
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn writer_acknowledged_during_drain_refreshes_outcome_but_not_after_call() {
    let (outcome, events) = case(
        "try { await page.click('#go', {timeout:100}) } catch(e) { return e.name }",
        Replies {
            mouse: Reply {
                delay: Duration::from_millis(700),
                ..Reply::default()
            },
            ..Replies::default()
        },
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(outcome.value, Some(json!("TimeoutError")));
    assert_eq!(outcome.calls[0].delivery, Some(Delivery::Done));
    let deliveries: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            RunEvent::AfterCall {
                call: 1, record, ..
            } => record.delivery,
            RunEvent::DeliverySettled {
                call: 1, delivery, ..
            } => Some(*delivery),
            _ => None,
        })
        .collect();
    assert_eq!(deliveries, [Delivery::Unknown, Delivery::Done]);
}

#[tokio::test(flavor = "multi_thread")]
async fn pages_and_getters_report_identities_without_page_content() {
    let browser = FakeBrowser::start(Replies::default()).await;
    browser.page.lock().unwrap().tabs.push(json!({"targetId":"listed", "tabId":31, "windowId":1, "liveBrowserContextId":"profile", "url":"https://a.test/?token=private"}));
    let sink = RecordingSink::new(true);
    let runtime = runtime_with(&browser, &[&sink]).await;
    let outcome = runtime.run(request("const before = context.pages(); const p = await context.newPage(); p.url(); p.isClosed(); await p.close(); await p.close(); return before.length")).await;
    assert_eq!(outcome.value, Some(json!(2)));
    let events = sink.settled(outcome.run_id).await;
    let opened: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            RunEvent::PageOpened {
                page,
                created,
                by_call,
                ..
            } => Some((page.target.as_str(), page.tab_id, *created, *by_call)),
            _ => None,
        })
        .collect();
    assert_eq!(
        opened,
        [
            (PAGE, Some(11), false, None),
            ("listed", Some(31), false, None),
            ("new-page-1", Some(12), true, Some(1))
        ]
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, RunEvent::PageClosed { .. }))
            .count(),
        1
    );
    let getters: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            RunEvent::Getter {
                no, field, result, ..
            } => Some((*no, *field, result)),
            _ => None,
        })
        .collect();
    assert_eq!(
        getters
            .iter()
            .map(|(no, field, _)| (*no, *field))
            .collect::<Vec<_>>(),
        [
            (1, GetterField::Pages),
            (2, GetterField::Initializer),
            (3, GetterField::Url),
            (4, GetterField::IsClosed)
        ]
    );
    assert_eq!(
        getters[0].2,
        &GetterResult::Ok {
            summary: json!(["page@fake-page", "page@listed"])
        }
    );
    assert_eq!(
        getters[1].2,
        &GetterResult::Ok {
            summary: json!("page@new-page-1")
        }
    );
    assert_eq!(
        getters[3].2,
        &GetterResult::Ok {
            summary: json!(false)
        }
    );
    for event in events {
        if let RunEvent::BeforeCall { call, page, .. } = event {
            assert_eq!(page.map(|p| p.tab_id), (call != 1).then_some(Some(12)));
        }
    }
}

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "the trace smoke test uses only the headless browser fixture"
)]
mod browser;

#[tokio::test(flavor = "multi_thread")]
async fn real_browser_records_operations_and_getters_without_the_password() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = browser::Tab::open(binary, browser::Fixture {
        pages: vec![("/trace".to_string(), "<input id='password' type='password'><button id='go' onclick='window.clicked=true'>Go</button>".to_string())],
        ..browser::Fixture::default()
    }, "/start").await;
    let sink = RecordingSink::new(true);
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap()
        .with_sinks([Arc::clone(&sink) as Arc<dyn TraceSink>]);
    let secret = "trace-password-do-not-record";
    let url = format!("{}/trace", tab.origin);
    let outcome = runtime.run(RunRequest {
        page: PageId::new(tab.target_id.clone()),
        code: format!("await page.goto({}); await page.fill('#password', {secret:?}); await page.click('#go'); const url = page.url(); const created = await context.newPage(); await created.close(); return url;", json!(url)),
        timeout: Duration::from_secs(30),
        ..request("")
    }).await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!(url)));
    assert_eq!(tab.evaluate("window.clicked").await, json!(true));
    let events = sink.settled(outcome.run_id).await;
    assert_eq!(outcome.calls.len(), 5);
    for record in &outcome.calls {
        assert!(events.iter().any(|event| matches!(event, RunEvent::AfterCall { record: emitted, .. } if emitted.as_ref() == record)));
    }
    let opened: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            RunEvent::PageOpened {
                page,
                created,
                by_call,
                ..
            } => Some((page, *created, *by_call)),
            _ => None,
        })
        .collect();
    assert_eq!(opened.len(), 2);
    assert!(opened.iter().all(|(page, _, _)| page.tab_id.is_some()));
    assert_eq!((opened[1].1, opened[1].2), (true, Some(4)));
    assert!(events.iter().any(|event| matches!(event, RunEvent::Getter { field: GetterField::Url, result: GetterResult::Ok { summary }, .. } if summary == &json!(url))));
    assert!(!serde_json::to_string(&events).unwrap().contains(secret));
    assert!(
        !serde_json::to_string(&outcome.calls)
            .unwrap()
            .contains(secret)
    );
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn trial_input_emits_no_input_sent_or_delivery_correction() {
    let (outcome, events) = case(
        "await page.click('#go', {trial:true})",
        Replies::default(),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(outcome.calls[0].delivery, Some(Delivery::NotSent));
    assert!(!events.iter().any(|event| matches!(
        event,
        RunEvent::InputSent { .. } | RunEvent::DeliverySettled { .. }
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn getter_urls_are_redacted_and_getter_failures_are_recorded() {
    let browser = FakeBrowser::start(Replies::default()).await;
    browser.page.lock().unwrap().tabs[0]["url"] =
        json!("https://user:pw@a.test/path?token=hidden#private");
    let sink = RecordingSink::new(true);
    let runtime = runtime_with(&browser, &[&sink]).await;
    let outcome = runtime.run(request("const url = page.url(); try { __neo_runtime.get(JSON.stringify({guid:'page@missing',field:'url'})) } catch(e) {} return url")).await;
    assert_eq!(
        outcome.value,
        Some(json!("https://user:pw@a.test/path?token=hidden#private"))
    );
    let events = sink.settled(outcome.run_id).await;
    let getters: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            RunEvent::Getter { no, result, .. } => Some((*no, result)),
            _ => None,
        })
        .collect();
    assert_eq!(
        getters[0],
        (
            1,
            &GetterResult::Ok {
                summary: json!("https://a.test/path?token=[redacted]")
            }
        )
    );
    assert!(matches!(getters[1], (2, GetterResult::Error { name, .. }) if name == "Error"));
    for text in ["user:pw", "hidden", "private"] {
        assert!(!serde_json::to_string(&events).unwrap().contains(text));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn run_end_caps_its_copies_and_keeps_script_output_whole() {
    let code = format!(
        "const text = 'x'.repeat(70*1024); console.log(text); return text; /*{}*/",
        "comment ".repeat(10_000)
    );
    let (outcome, events) = case(&code, Replies::default(), Duration::from_secs(5)).await;
    assert_eq!(outcome.value, Some(json!("x".repeat(70 * 1024))));
    assert_eq!(outcome.logs, ["x".repeat(70 * 1024)]);
    let RunEvent::RunEnded {
        value,
        value_truncated,
        logs,
        script,
        ..
    } = run_ended(&events)
    else {
        panic!("run end");
    };
    assert_eq!(value, &None);
    assert!(value_truncated);
    assert_eq!(logs[0].text.len(), 8 * 1024);
    assert_eq!(script.as_deref().unwrap().len(), 64 * 1024);

    let (outcome, events) = case(
        "throw new Error('x'.repeat(70*1024))",
        Replies::default(),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Error);
    let RunEvent::RunEnded {
        error: Some(error), ..
    } = run_ended(&events)
    else {
        panic!("run error");
    };
    assert_eq!(error.name, "Error");
    assert!(error.message.len() <= 64 * 1024);
}

/// Platform helpers share the getter door even though they never attach another CDP session.
#[tokio::test(flavor = "multi_thread")]
async fn platform_getters_are_recorded_with_their_single_bridge_vocabulary() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let sink = RecordingSink::new(true);
    let runtime = runtime_with(&browser, &[&sink]).await;
    let outcome = runtime
        .run(request(
            r#"
        const get = request => JSON.parse(__neo_runtime.get(JSON.stringify(request)));
        const matched = get({field:'matchesURL',url:'https://a.test/',matcher:{glob:'**'}});
        get({field:'devices'});
        get({field:'parseURL',url:'https://a.test/?token=private-data'});
        get({field:'setURL',href:'https://a.test/',part:'pathname',value:'/next'});
        get({field:'randomValues',length:1});
        return matched;
    "#,
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!(true)));
    let events = sink.settled(outcome.run_id).await;
    let fields: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            RunEvent::Getter { field, .. } => Some(*field),
            _ => None,
        })
        .collect();
    assert_eq!(
        fields,
        [
            GetterField::MatchesUrl,
            GetterField::Devices,
            GetterField::ParseUrl,
            GetterField::SetUrl,
            GetterField::RandomValues
        ]
    );
    assert_eq!(outcome.calls.len(), 0);
}
