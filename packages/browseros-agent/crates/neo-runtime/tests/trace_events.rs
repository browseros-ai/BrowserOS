//! The trace sinks a host installs on a `Runtime`: every run asks each one, before anything can
//! fail, whether it listens, and the outcome carries the id the sinks heard the run under.
//! Against the fake browser in `support/fake_browser.rs`. No browser needed.

#[path = "support/fake_browser.rs"]
#[expect(dead_code, reason = "these tests read no page state")]
mod fake_browser;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use fake_browser::{FakeBrowser, PAGE, Replies};
use neo_runtime::{
    IdKey, PageId, RunEvent, RunId, RunOrigin, RunRequest, RunSink, RunStarted, RunStatus, Runtime,
    SessionKey, TraceSink,
};
use tokio_util::sync::CancellationToken;

/// Records every run it is asked about. One that `listens` returns a run sink, one that does not
/// skips the run; both are asked all the same.
struct RecordingSink {
    listens: bool,
    runs: Mutex<Vec<RunStarted>>,
}

impl RecordingSink {
    fn new(listens: bool) -> Arc<Self> {
        Arc::new(Self {
            listens,
            runs: Mutex::default(),
        })
    }

    fn runs(&self) -> MutexGuard<'_, Vec<RunStarted>> {
        self.runs
            .lock()
            .unwrap_or_else(|error| panic!("a sink panicked while recording: {error}"))
    }

    fn run_ids(&self) -> Vec<RunId> {
        self.runs().iter().map(|started| started.run.id).collect()
    }
}

impl TraceSink for RecordingSink {
    fn run(&self, started: &RunStarted) -> Option<Arc<dyn RunSink>> {
        self.runs().push(started.clone());
        let ignore: Arc<dyn RunSink> = Arc::new(Ignore);
        self.listens.then_some(ignore)
    }
}

struct Ignore;

impl RunSink for Ignore {
    fn emit(&self, _event: &RunEvent) {}
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

#[tokio::test(flavor = "multi_thread")]
async fn every_sink_hears_of_each_run_once_under_its_outcomes_id() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let listening = RecordingSink::new(true);
    let skipping = RecordingSink::new(false);
    let runtime = runtime_with(&browser, &[&listening, &skipping]).await;

    let finished = runtime.run(request("return page.url()")).await;
    let closed_tab = runtime
        .run(RunRequest {
            origin: Default::default(),
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
/// before its deadline or its tab exist. Its sinks heard of it all the same.
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
}
