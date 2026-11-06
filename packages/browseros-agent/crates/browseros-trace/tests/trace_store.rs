//! The trace store and writer through the crate's public API, on a real SQLite file: what a
//! run's records look like after a flush, what a crash leaves behind, idempotent replay,
//! listing by task and tab, the trace tail, redaction, backpressure, retention, live events and
//! durable per-tab privacy.

use browseros_core::TabId;
use browseros_trace::{
    Delivery, Effect, EpochMillis, ErrorOwner, ErrorRecord, JsonPointer, OperationEnded,
    OperationId, OperationKind, OperationStarted, Outcome, PageState, Privacy, PrivacyRecord,
    PrivacyStore, PrivateReason, RedactionLimits, RedactionPolicy, Redactor, RequestId,
    RetentionPolicy, RunEffects, RunEnded, RunId, RunQuery, RunStarted, RunStatus, SecretSet,
    SecretSetConfig, SecretValue, TailQuery, TargetSensitivity, TaskAct, TaskId, TraceError,
    TraceRecordEvent, TraceSink, TraceStore, WriterConfig, spawn_writer,
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use std::{fs, io};
use tempfile::TempDir;

struct Harness {
    dir: TempDir,
    store: TraceStore,
    sink: TraceSink,
    secrets: Arc<SecretSet>,
}

async fn harness(config: WriterConfig) -> Harness {
    let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
    let store = open(&dir).await;
    let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
    let sink = spawn_writer(
        &store,
        Redactor::new(secrets.clone(), RedactionLimits::default()),
        config,
        None,
    );
    Harness {
        dir,
        store,
        sink,
        secrets,
    }
}

async fn open(dir: &TempDir) -> TraceStore {
    TraceStore::open(&dir.path().join("claw-sdk/trace.db"))
        .await
        .unwrap_or_else(|error| panic!("open: {error}"))
}

fn writer(store: &TraceStore) -> TraceSink {
    spawn_writer(
        store,
        Redactor::new(
            Arc::new(SecretSet::new(SecretSetConfig::default())),
            RedactionLimits::default(),
        ),
        WriterConfig::default(),
        None,
    )
}

fn at(millis: i64) -> EpochMillis {
    EpochMillis::from_millis(1_800_000_000_000 + millis)
}

fn task(name: &str) -> TaskId {
    TaskId::parse(name).unwrap_or_else(|error| panic!("{error}"))
}

fn request(name: &str) -> RequestId {
    RequestId::parse(name).unwrap_or_else(|error| panic!("{error}"))
}

/// A run's start: its code, the host ids it carries, and when.
#[derive(Clone, Copy)]
struct Start<'a> {
    code: &'a str,
    request: Option<&'a str>,
    task: Option<&'a str>,
    at: i64,
}

impl Default for Start<'_> {
    fn default() -> Self {
        Self {
            code: "x",
            request: None,
            task: None,
            at: 0,
        }
    }
}

fn start_run(sink: &TraceSink, start: Start<'_>) -> RunId {
    let run = RunId::new();
    sink.run_started(RunStarted {
        id: run,
        code: start.code.to_string(),
        request_id: start.request.map(request),
        task_id: start.task.map(task),
        host_data: Some(json!({"ownerRun": "r1"})),
        also_mask: None,
        at: at(start.at),
    });
    run
}

/// A run of `name`'s task, started at `millis`.
fn start_in(sink: &TraceSink, name: Option<&str>, millis: i64) -> RunId {
    start_run(
        sink,
        Start {
            task: name,
            at: millis,
            ..Start::default()
        },
    )
}

struct Op<'a> {
    id: OperationId,
    parent: Option<OperationId>,
    name: &'a str,
    kind: OperationKind,
    effect: Effect,
    tab: Option<TabId>,
    args: Value,
    policy: RedactionPolicy,
    at: EpochMillis,
}

fn op_started(sink: &TraceSink, op: Op<'_>) {
    sink.op_started(started(op));
}

fn op_ended(
    sink: &TraceSink,
    id: OperationId,
    delivery: Delivery,
    result: Option<Value>,
    target: TargetSensitivity,
) {
    sink.op_ended(OperationEnded {
        id,
        outcome: Outcome::Ok,
        delivery,
        sent_at: Some(at(i64::from(id.seq) * 10 + 1)),
        result,
        error: None,
        page_after: Some(PageState {
            page_id: Some(3),
            tab_id: Some(TabId(77)),
            url: Some("https://x.example/done".into()),
            title: Some("Done".into()),
        }),
        target,
        also_mask: None,
        at: at(i64::from(id.seq) * 10 + 5),
    });
}

fn end_run(sink: &TraceSink, run: RunId, status: RunStatus, error: Option<ErrorRecord>) {
    sink.run_ended(RunEnded {
        id: run,
        status,
        error,
        result: Some(json!({"title": "Done"})),
        replay: Some(json!({"content": [{"type": "text", "text": "Done"}]})),
        also_mask: None,
        at: at(1_000),
    });
}

/// An operation on tab 3, admitted at `seq * 10`.
fn simple(id: OperationId, name: &str, effect: Effect) -> Op<'_> {
    Op {
        id,
        parent: None,
        name,
        kind: OperationKind::Builtin,
        effect,
        tab: Some(TabId(3)),
        args: json!({"page": {"$neo": "page", "id": 3}}),
        policy: RedactionPolicy::default(),
        at: at(i64::from(id.seq) * 10),
    }
}

/// A read of `tab` admitted at `millis` and answered 5 ms later.
fn read_on(sink: &TraceSink, id: OperationId, tab: Option<TabId>, millis: i64) {
    op_started(
        sink,
        Op {
            tab,
            at: at(millis),
            ..simple(id, "page.title", Effect::Read)
        },
    );
    sink.op_ended(answered(id, millis, None, None));
}

/// The end of `id`, admitted at `millis` and answered 5 ms later on `page_after`.
fn answered(
    id: OperationId,
    millis: i64,
    sent_at: Option<EpochMillis>,
    page_after: Option<PageState>,
) -> OperationEnded {
    OperationEnded {
        id,
        outcome: Outcome::Ok,
        delivery: Delivery::Done,
        sent_at,
        result: None,
        error: None,
        page_after,
        target: TargetSensitivity::Ordinary,
        also_mask: None,
        at: at(millis + 5),
    }
}

/// A `newPage` admitted at `millis` with no target, which ended 5 ms later on the page of `tab`.
fn new_page(sink: &TraceSink, id: OperationId, tab: TabId, millis: i64) {
    op_started(
        sink,
        Op {
            tab: None,
            at: at(millis),
            ..simple(id, "context.newPage", Effect::Write)
        },
    );
    let page = PageState {
        page_id: Some(4),
        tab_id: Some(tab),
        url: Some("about:blank".into()),
        title: None,
    };
    sink.op_ended(answered(id, millis, Some(at(millis + 1)), Some(page)));
}

/// What `task_last_act` reads for a task that last acted on `tab` at `millis`.
fn act(tab: i64, millis: i64) -> Option<TaskAct> {
    Some(TaskAct {
        tab: TabId(tab),
        at: at(millis),
    })
}

fn fill_policy() -> RedactionPolicy {
    RedactionPolicy {
        secret: Vec::new(),
        sensitive: JsonPointer::parse("/value").into_iter().collect(),
    }
}

async fn flushed(h: &Harness) {
    h.sink
        .flush()
        .await
        .unwrap_or_else(|error| panic!("flush: {error}"));
}

async fn run_ids(store: &TraceStore, query: RunQuery) -> Vec<RunId> {
    store
        .runs(query)
        .await
        .unwrap_or_else(|error| panic!("runs: {error}"))
        .items
        .iter()
        .map(|run| run.id)
        .collect()
}

#[tokio::test]
async fn a_run_reads_back_as_a_tree_with_owners_and_deliveries() {
    let h = harness(WriterConfig::default()).await;
    let run = start_run(
        &h.sink,
        Start {
            code: "await page.goto(url)",
            request: Some("tool-call-1"),
            task: Some("task-1"),
            ..Start::default()
        },
    );
    let goto = OperationId::new(run, 1);
    let fill = OperationId::new(run, 2);
    let host_call = OperationId::new(run, 3);
    let protected = OperationId::new(run, 4);
    op_started(&h.sink, simple(goto, "page.goto", Effect::Write));
    op_ended(
        &h.sink,
        goto,
        Delivery::Done,
        None,
        TargetSensitivity::Ordinary,
    );
    op_started(
        &h.sink,
        Op {
            kind: OperationKind::Defined,
            ..simple(fill, "zuzu.auth.fill", Effect::Write)
        },
    );
    op_started(
        &h.sink,
        Op {
            parent: Some(fill),
            kind: OperationKind::HostCall,
            tab: None,
            ..simple(host_call, "host.call", Effect::Write)
        },
    );
    op_started(
        &h.sink,
        Op {
            parent: Some(host_call),
            kind: OperationKind::Protected,
            ..simple(protected, "host.protectedFill", Effect::Write)
        },
    );
    op_ended(
        &h.sink,
        protected,
        Delivery::Done,
        Some(json!({"status": "filled"})),
        TargetSensitivity::Ordinary,
    );
    op_ended(
        &h.sink,
        host_call,
        Delivery::Done,
        None,
        TargetSensitivity::Ordinary,
    );
    op_ended(
        &h.sink,
        fill,
        Delivery::Done,
        Some(json!({"status": "filled"})),
        TargetSensitivity::Ordinary,
    );
    end_run(&h.sink, run, RunStatus::Ok, None);
    flushed(&h).await;

    let detail = h.store.run(run).await.ok().flatten();
    let Some(detail) = detail else {
        panic!("run not found");
    };
    assert_eq!(detail.summary.status, RunStatus::Ok);
    assert_eq!(detail.summary.effects, Some(RunEffects::Done));
    assert_eq!(detail.summary.operation_count, 4);
    assert_eq!(detail.summary.request_id, Some(request("tool-call-1")));
    assert_eq!(detail.summary.task_id, Some(task("task-1")));
    assert_eq!(detail.summary.tabs, vec![TabId(3)]);
    assert_eq!(detail.host_data, Some(json!({"ownerRun": "r1"})));
    let ops = &detail.operations;
    assert_eq!(
        ops.iter().map(|op| op.id.seq).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(ops[2].parent, Some(fill));
    assert_eq!(ops[3].parent, Some(host_call));
    assert_eq!(ops[3].kind, OperationKind::Protected);
    assert!(ops.iter().all(|op| op.delivery == Some(Delivery::Done)));
    assert_eq!(ops[0].tab, Some(TabId(3)), "the target tab");
    assert_eq!(ops[2].tab, None, "a host call targets no tab");
    assert_eq!(ops[0].page.url.as_deref(), Some("https://x.example/done"));
    assert_eq!(ops[0].page.tab_id, Some(TabId(77)), "the page after it");
}

#[tokio::test]
async fn effects_roll_up_from_write_operations() {
    let h = harness(WriterConfig::default()).await;
    let reads = start_in(&h.sink, None, 0);
    let read = OperationId::new(reads, 1);
    op_started(&h.sink, simple(read, "locator.textContent", Effect::Read));
    op_ended(
        &h.sink,
        read,
        Delivery::Done,
        Some(json!("text")),
        TargetSensitivity::Ordinary,
    );
    end_run(&h.sink, reads, RunStatus::Ok, None);

    let unsure = start_in(&h.sink, None, 0);
    let click = OperationId::new(unsure, 1);
    op_started(&h.sink, simple(click, "locator.click", Effect::Write));
    op_ended(
        &h.sink,
        click,
        Delivery::Unknown,
        None,
        TargetSensitivity::Ordinary,
    );
    end_run(
        &h.sink,
        unsure,
        RunStatus::Timeout,
        Some(ErrorRecord {
            owner: ErrorOwner::Neo,
            code: "run_timeout".into(),
            message: "the run hit its 120 s budget".into(),
            host_code: None,
            retryable: false,
            operation_id: Some(click),
        }),
    );
    flushed(&h).await;

    let reads = h
        .store
        .run(reads)
        .await
        .ok()
        .flatten()
        .map(|run| run.summary);
    assert_eq!(reads.and_then(|run| run.effects), Some(RunEffects::None));
    let unsure = h
        .store
        .run(unsure)
        .await
        .ok()
        .flatten()
        .map(|run| run.summary);
    let Some(unsure) = unsure else {
        panic!("run not found");
    };
    assert_eq!(unsure.effects, Some(RunEffects::Unknown));
    assert_eq!(unsure.status, RunStatus::Timeout);
    assert_eq!(
        unsure.error.map(|error| error.operation_id),
        Some(Some(click))
    );
}

#[tokio::test]
async fn a_crash_leaves_runs_interrupted_and_open_writes_unknown() {
    let h = harness(WriterConfig::default()).await;
    let run = start_in(&h.sink, Some("task-1"), 0);
    let read = OperationId::new(run, 1);
    let submit = OperationId::new(run, 2);
    op_started(&h.sink, simple(read, "page.title", Effect::Read));
    op_started(&h.sink, simple(submit, "locator.click", Effect::Write));
    flushed(&h).await;
    let running = h.store.run(run).await.ok().flatten();
    assert_eq!(
        running.and_then(|run| run.code),
        None,
        "code reaches disk only at the run's end"
    );
    let Harness { dir, sink, .. } = h;
    drop(sink);

    let reopened = open(&dir).await;
    let detail = reopened.run(run).await.ok().flatten();
    let Some(detail) = detail else {
        panic!("run not found");
    };
    assert_eq!(detail.code_withheld.as_deref(), Some("unavailable"));
    assert_eq!(detail.summary.status, RunStatus::Interrupted);
    assert_eq!(detail.summary.effects, Some(RunEffects::Unknown));
    assert_eq!(detail.summary.task_id, Some(task("task-1")));
    assert_eq!(detail.operations[0].delivery, Some(Delivery::NotSent));
    assert_eq!(detail.operations[1].delivery, Some(Delivery::Unknown));
    assert_eq!(detail.operations[1].outcome, Some(Outcome::Cancelled));
}

#[tokio::test]
async fn interrupted_runs_expire_like_finished_ones() {
    let h = harness(WriterConfig::default()).await;
    let run = start_in(&h.sink, None, 0);
    flushed(&h).await;
    let Harness { dir, sink, .. } = h;
    drop(sink);

    let reopened = open(&dir).await;
    let eight_days = 8 * 24 * 60 * 60 * 1000;
    let swept = writer(&reopened)
        .sweep(RetentionPolicy::default(), at(eight_days))
        .await
        .unwrap_or_else(|error| panic!("sweep: {error}"));
    assert_eq!(swept.runs, 1);
    assert!(reopened.run(run).await.ok().flatten().is_none());
}

/// A reader that must rule out an operation in flight (the takeover lock's settle) learns
/// that the trace dropped some, and the doubt ends once the run's end is recorded.
#[tokio::test]
async fn the_sink_says_when_a_running_run_lost_operation_events() {
    let h = harness(WriterConfig {
        max_backlog: 0,
        ..WriterConfig::default()
    })
    .await;
    assert!(!h.sink.lost_operation_events());
    let run = start_in(&h.sink, None, 0);
    let click = OperationId::new(run, 1);
    op_started(&h.sink, simple(click, "locator.click", Effect::Write));
    assert!(h.sink.lost_operation_events(), "the start was dropped");
    end_run(&h.sink, run, RunStatus::Ok, None);
    h.sink
        .flush()
        .await
        .unwrap_or_else(|error| panic!("flush: {error}"));
    assert!(!h.sink.lost_operation_events(), "the run ended");
}

#[tokio::test]
async fn dropped_events_still_count_when_the_last_sink_is_gone() {
    let h = harness(WriterConfig {
        max_backlog: 0,
        ..WriterConfig::default()
    })
    .await;
    let run = start_in(&h.sink, None, 0);
    let click = OperationId::new(run, 1);
    op_started(&h.sink, simple(click, "locator.click", Effect::Write));
    op_ended(
        &h.sink,
        click,
        Delivery::Done,
        None,
        TargetSensitivity::Ordinary,
    );
    end_run(&h.sink, run, RunStatus::Ok, None);
    let Harness { store, sink, .. } = h;
    // The run's end is still queued when the last sink goes; the writer drains it.
    drop(sink);
    let mut summary = None;
    for _ in 0..200 {
        summary = store
            .run(run)
            .await
            .ok()
            .flatten()
            .map(|run| run.summary)
            .filter(|run| run.status == RunStatus::Ok);
        if summary.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let Some(summary) = summary else {
        panic!("the writer never wrote the run's end");
    };
    assert_eq!(summary.trace_gaps, 2);
    assert_eq!(summary.effects, Some(RunEffects::Unknown));
}

#[tokio::test]
async fn two_runs_racing_with_one_request_id_lose_no_records() {
    // Admission refuses a repeated request id, but two runs can race past it before either
    // row is written. The store must keep both rather than fail the writer's batch.
    let h = harness(WriterConfig::default()).await;
    let dup = Start {
        request: Some("dup-1"),
        task: Some("task-1"),
        ..Start::default()
    };
    let first = start_run(&h.sink, dup);
    let second = start_run(&h.sink, Start { at: 5, ..dup });
    end_run(&h.sink, first, RunStatus::Ok, None);
    end_run(&h.sink, second, RunStatus::Ok, None);
    flushed(&h).await;
    assert!(h.store.run(first).await.ok().flatten().is_some());
    assert!(h.store.run(second).await.ok().flatten().is_some());
    let found = h
        .store
        .run_by_request(&request("dup-1"), Some(&task("task-1")))
        .await
        .ok()
        .flatten();
    assert_eq!(
        found.map(|run| run.id),
        Some(first),
        "the earliest run answers the lookup"
    );
}

#[tokio::test]
async fn a_request_id_finds_the_run_and_its_replay_expires() {
    let h = harness(WriterConfig {
        replay_window: Duration::from_millis(100),
        ..WriterConfig::default()
    })
    .await;
    let run = start_run(
        &h.sink,
        Start {
            request: Some("key-1"),
            ..Start::default()
        },
    );
    end_run(&h.sink, run, RunStatus::Ok, None);
    flushed(&h).await;

    let found = h
        .store
        .run_by_request(&request("key-1"), None)
        .await
        .ok()
        .flatten();
    assert_eq!(found.map(|run| run.id), Some(run));
    let replay = h.store.replay(run, at(1_050)).await.ok().flatten();
    assert_eq!(
        replay,
        Some(json!({"content": [{"type": "text", "text": "Done"}]}))
    );
    assert_eq!(h.store.replay(run, at(1_100)).await.ok().flatten(), None);

    let swept = h.sink.sweep(RetentionPolicy::default(), at(2_000)).await;
    assert_eq!(swept.ok().map(|swept| swept.replays), Some(1));
}

#[tokio::test]
async fn request_ids_dedupe_per_task_when_one_is_given_else_globally() {
    let h = harness(WriterConfig::default()).await;
    let start = |request: &'static str, task: Option<&'static str>, at: i64| Start {
        request: Some(request),
        task,
        at,
        ..Start::default()
    };
    let in_checkout = start_run(&h.sink, start("call-7", Some("checkout"), 0));
    let in_search = start_run(&h.sink, start("call-7", Some("search"), 10));
    let untasked = start_run(&h.sink, start("call-8", None, 20));
    flushed(&h).await;

    let lookup = |request_id: &'static str, task_id: Option<&'static str>| {
        let store = h.store.clone();
        async move {
            let task_id = task_id.map(task);
            store
                .run_by_request(&request(request_id), task_id.as_ref())
                .await
                .unwrap_or_else(|error| panic!("lookup: {error}"))
                .map(|run| run.id)
        }
    };
    assert_eq!(lookup("call-7", Some("checkout")).await, Some(in_checkout));
    assert_eq!(lookup("call-7", Some("search")).await, Some(in_search));
    assert_eq!(lookup("call-7", Some("other")).await, None, "per task");
    assert_eq!(
        lookup("call-7", None).await,
        Some(in_checkout),
        "without a task, the earliest run with the id, whatever its task"
    );
    assert_eq!(lookup("call-8", None).await, Some(untasked));
    assert_eq!(
        lookup("call-8", Some("checkout")).await,
        None,
        "a run with no task is not in any task"
    );
}

#[tokio::test]
async fn sensitive_targets_redact_their_input_and_withhold_the_code() {
    let h = harness(WriterConfig::default()).await;
    let run = start_run(
        &h.sink,
        Start {
            code: "await email.fill('me@x.example'); await pw.fill('hunter22')",
            ..Start::default()
        },
    );
    let email = OperationId::new(run, 1);
    let password = OperationId::new(run, 2);
    for (id, value) in [(email, "me@x.example"), (password, "hunter22")] {
        op_started(
            &h.sink,
            Op {
                args: json!({"locator": {"$neo": "locator", "page": 3, "selector": "getByLabel('x')"}, "value": value}),
                policy: fill_policy(),
                ..simple(id, "locator.fill", Effect::Write)
            },
        );
    }
    flushed(&h).await;
    let open = h.store.run(run).await.ok().flatten();
    assert_eq!(
        open.map(|run| run.operations[0].args.clone()),
        Some(None),
        "while open, the input is not on disk: its end stores it"
    );

    op_ended(
        &h.sink,
        email,
        Delivery::Done,
        None,
        TargetSensitivity::Ordinary,
    );
    op_ended(
        &h.sink,
        password,
        Delivery::Done,
        None,
        TargetSensitivity::Sensitive,
    );
    end_run(&h.sink, run, RunStatus::Ok, None);
    flushed(&h).await;
    let Some(done) = h.store.run(run).await.ok().flatten() else {
        panic!("run not found");
    };
    assert_eq!(
        done.operations[0]
            .args
            .as_ref()
            .map(|args| args["value"].clone()),
        Some(json!("me@x.example"))
    );
    assert_eq!(
        done.operations[1]
            .args
            .as_ref()
            .map(|args| args["value"].clone()),
        Some(json!("[redacted]"))
    );
    assert_eq!(
        done.operations[1]
            .args
            .as_ref()
            .map(|args| args["locator"]["selector"].clone()),
        Some(json!("getByLabel('x')"))
    );
    assert_eq!(done.code, None);
    assert_eq!(done.code_withheld.as_deref(), Some("sensitive_input"));
}

#[tokio::test]
async fn registered_secrets_never_reach_the_file() {
    let h = harness(WriterConfig::default()).await;
    h.secrets
        .register(SecretValue::new("123-456-otp".into()), None);
    let run = start_run(
        &h.sink,
        Start {
            code: "return '123-456-otp'",
            ..Start::default()
        },
    );
    let read = OperationId::new(run, 1);
    op_started(&h.sink, simple(read, "locator.textContent", Effect::Read));
    op_ended(
        &h.sink,
        read,
        Delivery::Done,
        Some(json!({"text": "your code is 123-456-otp"})),
        TargetSensitivity::Ordinary,
    );
    end_run(&h.sink, run, RunStatus::Ok, None);
    flushed(&h).await;
    let Some(done) = h.store.run(run).await.ok().flatten() else {
        panic!("run not found");
    };
    assert_eq!(done.code.as_deref(), Some("return '[private]'"));
    assert_eq!(
        done.operations[0].result,
        Some(json!({"text": "your code is [private]"}))
    );

    let bytes = std::fs::read(h.dir.path().join("claw-sdk/trace.db")).unwrap_or_default();
    let wal = std::fs::read(h.dir.path().join("claw-sdk/trace.db-wal")).unwrap_or_default();
    let needle = b"123-456-otp";
    for file in [&bytes, &wal] {
        assert!(!file.windows(needle.len()).any(|window| window == needle));
    }
}

/// Host data whose compact JSON, the size the app checks, is exactly `bytes` long. It is one
/// string, so a cut per string would show as much as a cut of the whole.
fn host_data_of(bytes: usize) -> Value {
    let empty = json!({"blob": ""}).to_string().len();
    let data = json!({"blob": "x".repeat(bytes - empty)});
    assert_eq!(data.to_string().len(), bytes);
    data
}

#[tokio::test]
async fn host_data_of_4_kib_is_stored_whole_and_a_larger_one_is_cut()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let h = harness(WriterConfig::default()).await;
    let runs = [4096, 4097].map(|bytes| {
        let run = RunId::new();
        h.sink.run_started(RunStarted {
            id: run,
            code: "x".to_string(),
            request_id: None,
            task_id: None,
            host_data: Some(host_data_of(bytes)),
            also_mask: None,
            at: at(0),
        });
        // Host data reaches disk with the run's end.
        end_run(&h.sink, run, RunStatus::Ok, None);
        run
    });
    flushed(&h).await;
    let mut stored = Vec::new();
    for run in runs {
        let detail = h.store.run(run).await?.ok_or("run not found")?;
        stored.push(detail.host_data);
    }

    assert_eq!(stored[0], Some(host_data_of(4096)), "D14 allows 4 KiB");
    let cut = stored[1]
        .as_ref()
        .and_then(|data| data["blob"].as_str())
        .unwrap_or_default();
    assert!(
        cut.starts_with("xxx") && cut.ends_with(" bytes]") && cut.contains("…[+"),
        "{cut}"
    );
    assert!(
        stored[1]
            .as_ref()
            .is_some_and(|data| data.to_string().len() <= 4096),
        "{stored:?}"
    );
    Ok(())
}

async fn notes_of(
    store: &TraceStore,
    id: OperationId,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let row = store
        .operation(id)
        .await?
        .ok_or_else(|| format!("{id} not found"))?;
    Ok(row.notes)
}

#[tokio::test]
async fn an_operations_notes_reach_its_record_with_its_end_masked_bounded_and_in_order()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let h = harness(WriterConfig::default()).await;
    h.secrets
        .register(SecretValue::new("hunter2-swordfish".into()), None);
    let limits = RedactionLimits::default();
    let run = start_in(&h.sink, None, 0);
    let click = OperationId::new(run, 1);
    let title = OperationId::new(run, 2);
    op_started(&h.sink, simple(click, "locator.click", Effect::Write));
    op_started(&h.sink, simple(title, "page.title", Effect::Read));
    h.sink.op_notes(
        click,
        vec![
            "waiting for getByRole('button')".to_string(),
            "typed hunter2-swordfish into the field".to_string(),
            "x".repeat(5 * limits.note),
        ],
    );
    // A second call adds to the first, past the lines an operation keeps: 70 lines in all.
    h.sink.op_notes(
        click,
        (3..limits.note_lines + 6)
            .map(|step| format!("step {step}"))
            .collect(),
    );
    flushed(&h).await;
    assert_eq!(
        notes_of(&h.store, click).await?,
        Vec::<String>::new(),
        "stored with the end"
    );

    op_ended(
        &h.sink,
        click,
        Delivery::Done,
        None,
        TargetSensitivity::Ordinary,
    );
    op_ended(
        &h.sink,
        title,
        Delivery::Done,
        None,
        TargetSensitivity::Ordinary,
    );
    h.sink.op_notes(click, vec!["after the end".to_string()]);
    flushed(&h).await;

    let notes = notes_of(&h.store, click).await?;
    assert_eq!(notes.len(), limits.note_lines + 1, "{notes:?}");
    assert_eq!(notes[0], "waiting for getByRole('button')");
    assert_eq!(notes[1], "typed [private] into the field");
    assert!(
        notes[2].starts_with("xxx") && notes[2].contains("…[+") && notes[2].len() <= limits.note,
        "{} bytes: {}",
        notes[2].len(),
        notes[2]
    );
    let steps: Vec<String> = (3..limits.note_lines)
        .map(|step| format!("step {step}"))
        .collect();
    assert_eq!(notes[3..limits.note_lines], steps[..]);
    assert_eq!(notes[limits.note_lines], "…[+6 lines]");
    assert!(
        !notes.iter().any(|line| line == "after the end"),
        "notes never change once the operation has ended"
    );
    assert_eq!(notes_of(&h.store, title).await?, Vec::<String>::new());
    let detail = h.store.run(run).await?.ok_or("run not found")?;
    let in_run: Vec<Vec<String>> = detail
        .operations
        .iter()
        .map(|op| op.notes.clone())
        .collect();
    assert_eq!(in_run, [notes, Vec::new()]);

    // The database must be read; only the WAL may be absent (after a checkpoint). A read that
    // fails must fail the test, or the scan would pass without looking at the file.
    let bytes = fs::read(h.dir.path().join("claw-sdk/trace.db"))?;
    let wal = match fs::read(h.dir.path().join("claw-sdk/trace.db-wal")) {
        Ok(wal) => wal,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let needle = b"hunter2-swordfish";
    for file in [&bytes, &wal] {
        assert!(!file.windows(needle.len()).any(|window| window == needle));
    }
    Ok(())
}

#[tokio::test]
async fn over_the_backlog_operation_events_drop_but_lifecycle_events_do_not() {
    let h = harness(WriterConfig {
        max_backlog: 0,
        ..WriterConfig::default()
    })
    .await;
    let run = start_in(&h.sink, None, 0);
    let click = OperationId::new(run, 1);
    op_started(&h.sink, simple(click, "locator.click", Effect::Write));
    h.sink
        .op_notes(click, vec!["attempting click action".to_string()]);
    op_ended(
        &h.sink,
        click,
        Delivery::Done,
        None,
        TargetSensitivity::Ordinary,
    );
    end_run(&h.sink, run, RunStatus::Ok, None);
    flushed(&h).await;
    let Some(summary) = h.store.run(run).await.ok().flatten().map(|run| run.summary) else {
        panic!("the run's lifecycle must survive");
    };
    assert_eq!(summary.status, RunStatus::Ok);
    assert_eq!(
        summary.trace_gaps, 2,
        "the start and the end; a dropped note hides no write"
    );
    assert_eq!(summary.operation_count, 0);
    assert_eq!(
        summary.effects,
        Some(RunEffects::Unknown),
        "a gap may have been a write"
    );
}

#[tokio::test]
async fn retention_deletes_old_runs_and_keeps_private_tabs() {
    let h = harness(WriterConfig::default()).await;
    let privacy = Privacy::new(
        h.secrets.clone(),
        Arc::new(h.sink.clone()),
        Default::default(),
    );
    let old = start_in(&h.sink, Some("task-1"), 0);
    read_on(&h.sink, OperationId::new(old, 1), Some(TabId(3)), 10);
    h.sink.run_ended(RunEnded {
        id: old,
        status: RunStatus::Ok,
        error: None,
        result: None,
        replay: None,
        also_mask: None,
        at: at(20),
    });
    assert!(
        privacy
            .make_private(TabId(3), PrivateReason::SecretInput, at(15))
            .await
            .is_ok()
    );
    let eight_days = 8 * 24 * 60 * 60 * 1000;
    let recent = start_in(&h.sink, Some("task-1"), eight_days);
    flushed(&h).await;

    let swept = h
        .sink
        .sweep(RetentionPolicy::default(), at(eight_days + 1))
        .await
        .unwrap_or_else(|error| panic!("sweep: {error}"));
    assert_eq!((swept.runs, swept.operations), (1, 1));
    assert!(h.store.run(old).await.ok().flatten().is_none());
    assert_eq!(
        run_ids(
            &h.store,
            RunQuery {
                tab: Some(TabId(3)),
                ..RunQuery::default()
            }
        )
        .await,
        Vec::<RunId>::new(),
        "a swept run is no longer listed under its tab"
    );
    assert!(
        h.store.run(recent).await.ok().flatten().is_some(),
        "a running run is kept"
    );
    let private = h.store.private_tabs().await.unwrap_or_default();
    assert!(
        private.contains_key(&TabId(3)),
        "privacy outlives the runs on its tab"
    );
}

#[tokio::test]
async fn a_private_tab_is_announced_and_its_flag_survives_a_reopen() {
    let h = harness(WriterConfig::default()).await;
    let mut events = h.sink.subscribe();
    let privacy = Privacy::new(
        h.secrets.clone(),
        Arc::new(h.sink.clone()),
        Default::default(),
    );
    let tab = TabId(9);
    assert!(
        privacy
            .make_private(tab, PrivateReason::Takeover, at(5))
            .await
            .is_ok()
    );
    assert_eq!(
        events.recv().await.ok(),
        Some(TraceRecordEvent::TabPrivate {
            tab,
            reason: PrivateReason::Takeover,
            at: at(5),
        })
    );
    let Harness { dir, sink, .. } = h;
    drop(privacy);
    drop(sink);

    let reopened = open(&dir).await;
    let loaded = reopened.private_tabs().await.unwrap_or_default();
    assert_eq!(
        loaded.get(&tab).map(|record| record.reason),
        Some(PrivateReason::Takeover)
    );
    let privacy = Privacy::new(
        Arc::new(SecretSet::new(SecretSetConfig::default())),
        Arc::new(writer(&reopened)),
        loaded,
    );
    assert!(privacy.capture(tab).is_none());
    assert!(privacy.capture(TabId(10)).is_some());
}

#[tokio::test]
async fn closed_and_vanished_tabs_lose_their_flags_durably() {
    let h = harness(WriterConfig::default()).await;
    let privacy = Privacy::new(
        h.secrets.clone(),
        Arc::new(h.sink.clone()),
        Default::default(),
    );
    for tab in [TabId(1), TabId(2), TabId(3)] {
        assert!(
            privacy
                .make_private(tab, PrivateReason::Host, at(1))
                .await
                .is_ok()
        );
    }
    privacy.tab_closed(TabId(1)).await;
    // The browser restarted: only tab 3 is open, and tab 2's id may come back as a new tab.
    privacy.retain_tabs(&[TabId(3)]).await;
    let Harness { dir, sink, .. } = h;
    drop(privacy);
    drop(sink);

    let reopened = open(&dir).await;
    let mut tabs = reopened
        .private_tabs()
        .await
        .unwrap_or_default()
        .into_keys()
        .collect::<Vec<_>>();
    tabs.sort_unstable();
    assert_eq!(tabs, vec![TabId(3)]);
}

/// Which of a tab's two privacy writes a [`Slow`] store is slow around.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PrivacyWrite {
    /// The commit of its flag.
    Commit,
    /// The delete of its flag.
    Forget,
}

/// Long enough for a racing call on another thread to ask for its own write.
const PAUSE: Duration = Duration::from_millis(100);

/// The writer as a `PrivacyStore` that pauses around its `slow` write, once in the call before
/// it calls the writer and once before the caller waits on the write. A racing call on the same
/// tab runs inside the first pause if `Privacy` calls the store after releasing its lock, and
/// inside the second if the writer queues a write only when it is awaited.
struct Slow {
    sink: TraceSink,
    slow: PrivacyWrite,
}

impl Slow {
    fn pause_in_call(&self, write: PrivacyWrite) {
        if write == self.slow {
            std::thread::sleep(PAUSE);
        }
    }

    async fn pause_before_wait(&self, write: PrivacyWrite) {
        if write == self.slow {
            tokio::time::sleep(PAUSE).await;
        }
    }
}

impl PrivacyStore for Slow {
    fn persist_private(
        &self,
        tab: TabId,
        record: PrivacyRecord,
    ) -> BoxFuture<'_, Result<(), TraceError>> {
        self.pause_in_call(PrivacyWrite::Commit);
        let written = self.sink.persist_private(tab, record);
        Box::pin(async move {
            self.pause_before_wait(PrivacyWrite::Commit).await;
            written.await
        })
    }

    fn forget_private(&self, tab: TabId) -> BoxFuture<'_, Result<(), TraceError>> {
        self.pause_in_call(PrivacyWrite::Forget);
        let written = self.sink.forget_private(tab);
        Box::pin(async move {
            self.pause_before_wait(PrivacyWrite::Forget).await;
            written.await
        })
    }
}

fn slow_privacy(h: &Harness, slow: PrivacyWrite) -> Arc<Privacy> {
    let store = Slow {
        sink: h.sink.clone(),
        slow,
    };
    Privacy::new(h.secrets.clone(), Arc::new(store), Default::default())
}

/// Waits until `Privacy::is_private` reports `private` for `tab`, which a call on another thread
/// changes.
async fn until_private(privacy: &Privacy, tab: TabId, private: bool) {
    let changed = tokio::time::timeout(Duration::from_secs(5), async {
        while privacy.is_private(tab).is_some() != private {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        changed.is_ok(),
        "the racing call sets tab {}'s flag to private = {private} within seconds",
        tab.0
    );
}

/// Flushes the writer, then reports whether `tab`'s flag is on disk, where a restart would load
/// it.
async fn flag_on_disk(h: &Harness, tab: TabId) -> bool {
    flushed(h).await;
    h.store
        .private_tabs()
        .await
        .unwrap_or_else(|error| panic!("private tabs: {error}"))
        .contains_key(&tab)
}

/// The tab closes on another thread while `make_private` is between setting the flag and
/// committing it. A delete that reaches the disk before the flag leaves the flag there, and a
/// restart makes the next tab with this id private.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tab_that_closes_while_it_goes_private_leaves_no_flag_on_disk()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let h = harness(WriterConfig::default()).await;
    let privacy = slow_privacy(&h, PrivacyWrite::Commit);
    let tab = TabId(4);
    let made = tokio::spawn({
        let privacy = Arc::clone(&privacy);
        async move { privacy.make_private(tab, PrivateReason::Host, at(1)).await }
    });
    until_private(&privacy, tab, true).await;
    privacy.tab_closed(tab).await;
    made.await??;
    assert!(
        privacy.is_private(tab).is_none(),
        "the close forgot the flag"
    );
    assert!(
        !flag_on_disk(&h, tab).await,
        "the delete reached the disk after the flag it deletes"
    );
    Ok(())
}

/// The reverse race. The tab goes private again on another thread while its close is between
/// forgetting the flag and deleting it. A delete that reaches the disk after the new flag erases
/// it, and a restart captures a tab that memory keeps private.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tab_that_goes_private_while_its_flag_is_forgotten_keeps_it_on_disk()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let h = harness(WriterConfig::default()).await;
    let privacy = slow_privacy(&h, PrivacyWrite::Forget);
    let tab = TabId(4);
    privacy
        .make_private(tab, PrivateReason::Host, at(1))
        .await?;
    let closed = tokio::spawn({
        let privacy = Arc::clone(&privacy);
        async move { privacy.tab_closed(tab).await }
    });
    until_private(&privacy, tab, false).await;
    privacy
        .make_private(tab, PrivateReason::Host, at(2))
        .await?;
    closed.await?;
    assert!(
        privacy.is_private(tab).is_some(),
        "the flag set after the close holds"
    );
    assert!(
        flag_on_disk(&h, tab).await,
        "the new flag reached the disk after the delete it raced"
    );
    Ok(())
}

/// The commit is already queued when the tab closes, so the delete comes after it and wins.
#[tokio::test]
async fn a_flag_already_queued_when_its_tab_closes_is_deleted_on_disk()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use futures_util::FutureExt;

    let h = harness(WriterConfig::default()).await;
    let privacy = Privacy::new(
        h.secrets.clone(),
        Arc::new(h.sink.clone()),
        Default::default(),
    );
    let tab = TabId(4);
    let made = privacy.make_private(tab, PrivateReason::Host, at(1));
    tokio::pin!(made);
    assert!(
        made.as_mut().now_or_never().is_none(),
        "the commit is queued and waits on the writer"
    );
    privacy.tab_closed(tab).await;
    made.await?;
    assert!(privacy.is_private(tab).is_none());
    assert!(
        !flag_on_disk(&h, tab).await,
        "the delete was queued after the commit, so it wins"
    );
    Ok(())
}

/// A commit's answer says only that the flag it wrote is on disk. Here the answer is read after
/// the tab closed and went private again, while the new flag's own commit is still queued.
#[tokio::test]
async fn make_private_answers_only_once_its_own_flag_is_on_disk()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use futures_util::FutureExt;

    let h = harness(WriterConfig::default()).await;
    let privacy = Privacy::new(
        h.secrets.clone(),
        Arc::new(h.sink.clone()),
        Default::default(),
    );
    let tab = TabId(4);
    let first = privacy.make_private(tab, PrivateReason::Host, at(1));
    tokio::pin!(first);
    assert!(first.as_mut().now_or_never().is_none());
    // Commits the first flag, whose answer `first` has not read yet.
    flushed(&h).await;
    privacy.tab_closed(tab).await;
    let second = privacy.make_private(tab, PrivateReason::Host, at(2));
    tokio::pin!(second);
    assert!(second.as_mut().now_or_never().is_none());
    first.await?;
    // Nothing has yielded to the writer since the second commit was queued.
    let third = privacy
        .make_private(tab, PrivateReason::Host, at(3))
        .now_or_never();
    assert!(
        third.is_none(),
        "make_private answers once the flag is on disk, and its commit is still queued"
    );
    second.await?;
    assert!(flag_on_disk(&h, tab).await);
    Ok(())
}

#[tokio::test]
async fn runs_page_newest_first_and_filter_by_request_id() {
    let h = harness(WriterConfig::default()).await;
    let mut ids = Vec::new();
    for (offset, request_id) in [(0, "a"), (10, "b"), (20, "a")] {
        ids.push(start_run(
            &h.sink,
            Start {
                request: Some(request_id),
                task: Some("task-1"),
                at: offset,
                ..Start::default()
            },
        ));
    }
    flushed(&h).await;

    let first = h
        .store
        .runs(RunQuery {
            task_id: Some(task("task-1")),
            limit: 2,
            ..RunQuery::default()
        })
        .await
        .unwrap_or_else(|error| panic!("runs: {error}"));
    assert_eq!(
        first.items.iter().map(|run| run.id).collect::<Vec<_>>(),
        vec![ids[2], ids[1]]
    );
    let second = h
        .store
        .runs(RunQuery {
            task_id: Some(task("task-1")),
            limit: 2,
            cursor: first.next_cursor,
            ..RunQuery::default()
        })
        .await
        .unwrap_or_else(|error| panic!("runs: {error}"));
    assert_eq!(
        second.items.iter().map(|run| run.id).collect::<Vec<_>>(),
        vec![ids[0]]
    );
    assert_eq!(second.next_cursor, None);

    let by_request = run_ids(
        &h.store,
        RunQuery {
            request_id: Some(request("a")),
            ..RunQuery::default()
        },
    )
    .await;
    assert_eq!(by_request, vec![ids[2], ids[0]]);
    assert!(
        h.store
            .runs(RunQuery {
                cursor: Some("garbage".into()),
                ..RunQuery::default()
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn runs_are_listed_by_task_and_by_tab() {
    let h = harness(WriterConfig::default()).await;
    let first = start_in(&h.sink, Some("checkout"), 0);
    read_on(&h.sink, OperationId::new(first, 1), Some(TabId(5)), 0);
    let second = start_in(&h.sink, Some("checkout"), 10);
    read_on(&h.sink, OperationId::new(second, 1), Some(TabId(6)), 10);
    read_on(&h.sink, OperationId::new(second, 2), Some(TabId(5)), 11);
    let other = start_in(&h.sink, Some("search"), 20);
    read_on(&h.sink, OperationId::new(other, 1), Some(TabId(5)), 20);
    let no_tab = start_in(&h.sink, None, 30);
    read_on(&h.sink, OperationId::new(no_tab, 1), None, 30);
    flushed(&h).await;

    let query = |task_id: Option<&str>, tab: Option<i64>| RunQuery {
        task_id: task_id.map(task),
        tab: tab.map(TabId),
        ..RunQuery::default()
    };
    assert_eq!(
        run_ids(&h.store, query(Some("checkout"), None)).await,
        vec![second, first]
    );
    assert_eq!(
        run_ids(&h.store, query(None, Some(5))).await,
        vec![other, second, first]
    );
    assert_eq!(
        run_ids(&h.store, query(Some("checkout"), Some(6))).await,
        vec![second],
        "the filters combine with AND"
    );
    assert_eq!(
        run_ids(&h.store, query(Some("search"), Some(6))).await,
        Vec::<RunId>::new()
    );
    let summary = h
        .store
        .run(second)
        .await
        .ok()
        .flatten()
        .map(|run| run.summary);
    assert_eq!(
        summary.map(|run| run.tabs),
        Some(vec![TabId(5), TabId(6)]),
        "a run's tabs, ascending"
    );
    let summary = h
        .store
        .run(no_tab)
        .await
        .ok()
        .flatten()
        .map(|run| run.summary);
    assert_eq!(summary.map(|run| run.tabs), Some(Vec::new()));
}

#[tokio::test]
async fn a_trace_tail_keeps_the_latest_matching_operations_oldest_first() {
    let h = harness(WriterConfig::default()).await;
    let early = start_in(&h.sink, Some("checkout"), 0);
    let e1 = OperationId::new(early, 1);
    let e2 = OperationId::new(early, 2);
    read_on(&h.sink, e1, Some(TabId(5)), 10);
    read_on(&h.sink, e2, Some(TabId(6)), 20);
    let other = start_in(&h.sink, Some("search"), 25);
    let o1 = OperationId::new(other, 1);
    read_on(&h.sink, o1, Some(TabId(5)), 30);
    let late = start_in(&h.sink, Some("checkout"), 35);
    let l1 = OperationId::new(late, 1);
    let l2 = OperationId::new(late, 2);
    read_on(&h.sink, l1, Some(TabId(5)), 40);
    read_on(&h.sink, l2, Some(TabId(5)), 50);
    flushed(&h).await;

    let tail = |task_id: Option<&'static str>, tab: Option<i64>, tail: u32| {
        let store = h.store.clone();
        async move {
            store
                .trace_tail(TailQuery {
                    task_id: task_id.map(task),
                    tab: tab.map(TabId),
                    tail,
                })
                .await
                .unwrap_or_else(|error| panic!("trace_tail: {error}"))
        }
    };
    let ops = |trace: &browseros_trace::TraceTail| {
        trace.operations.iter().map(|op| op.id).collect::<Vec<_>>()
    };
    let runs = |trace: &browseros_trace::TraceTail| {
        trace.runs.iter().map(|run| run.id).collect::<Vec<_>>()
    };

    let last_two = tail(Some("checkout"), None, 2).await;
    assert_eq!(ops(&last_two), vec![l1, l2]);
    assert!(last_two.truncated);
    assert_eq!(runs(&last_two), vec![late]);

    let whole_task = tail(Some("checkout"), None, 10).await;
    assert_eq!(ops(&whole_task), vec![e1, e2, l1, l2]);
    assert!(!whole_task.truncated);
    assert_eq!(runs(&whole_task), vec![early, late]);

    let on_tab = tail(None, Some(5), 10).await;
    assert_eq!(ops(&on_tab), vec![e1, o1, l1, l2]);
    assert_eq!(runs(&on_tab), vec![early, other, late]);

    let both = tail(Some("checkout"), Some(5), 2).await;
    assert_eq!(ops(&both), vec![l1, l2]);
    assert!(both.truncated, "e1 matches both filters too");

    let everything = tail(None, None, 0).await;
    assert_eq!(
        ops(&everything),
        vec![e1, e2, o1, l1, l2],
        "zero means the default"
    );
    assert!(!everything.truncated);
}

#[tokio::test]
async fn a_tasks_last_tab_is_its_latest_operation_with_a_target_and_survives_a_reopen() {
    let h = harness(WriterConfig::default()).await;
    let first = start_in(&h.sink, Some("checkout"), 0);
    read_on(&h.sink, OperationId::new(first, 1), Some(TabId(5)), 10);
    let second = start_in(&h.sink, Some("checkout"), 15);
    read_on(&h.sink, OperationId::new(second, 1), Some(TabId(6)), 20);
    read_on(&h.sink, OperationId::new(second, 2), None, 30);
    let other = start_in(&h.sink, Some("search"), 35);
    read_on(&h.sink, OperationId::new(other, 1), Some(TabId(9)), 40);
    flushed(&h).await;

    let last_tab = |store: TraceStore, name: &'static str| async move {
        store
            .task_last_tab(&task(name))
            .await
            .unwrap_or_else(|error| panic!("task_last_tab: {error}"))
    };
    assert_eq!(
        last_tab(h.store.clone(), "checkout").await,
        Some(TabId(6)),
        "an operation with no target is skipped"
    );
    assert_eq!(last_tab(h.store.clone(), "search").await, Some(TabId(9)));
    assert_eq!(last_tab(h.store.clone(), "unknown").await, None);

    let Harness { dir, sink, .. } = h;
    drop(sink);
    let reopened = open(&dir).await;
    assert_eq!(last_tab(reopened, "checkout").await, Some(TabId(6)));
}

/// A `newPage` has no target when it starts; its tab is known only from the page it ended on.
/// A task whose latest step opened a tab resumes in that tab, and a target still beats the page
/// an operation ended on.
#[tokio::test]
async fn a_tasks_last_tab_falls_back_to_the_tab_an_operation_ended_on() {
    let h = harness(WriterConfig::default()).await;
    let run = start_in(&h.sink, Some("checkout"), 0);
    read_on(&h.sink, OperationId::new(run, 1), Some(TabId(5)), 10);
    new_page(&h.sink, OperationId::new(run, 2), TabId(8), 20);
    read_on(&h.sink, OperationId::new(run, 3), None, 30);
    flushed(&h).await;
    let last_tab = || async {
        h.store
            .task_last_tab(&task("checkout"))
            .await
            .unwrap_or_else(|error| panic!("task_last_tab: {error}"))
    };
    assert_eq!(
        last_tab().await,
        Some(TabId(8)),
        "the opened tab, and an operation with neither tab is skipped"
    );

    // An operation with a target counts with it, not with the page it ended on (tab 77).
    let later = OperationId::new(run, 4);
    op_started(&h.sink, simple(later, "page.click", Effect::Write));
    op_ended(
        &h.sink,
        later,
        Delivery::Done,
        None,
        TargetSensitivity::Ordinary,
    );
    flushed(&h).await;
    assert_eq!(last_tab().await, Some(TabId(3)));
}

/// When a task last acted on its last tab, for a follow view that lost the task's live events:
/// a target counts from its operation's start, and the page an operation ended on from its end.
#[tokio::test]
async fn a_tasks_last_act_is_its_last_tab_and_when_the_task_acted_on_it() {
    let h = harness(WriterConfig::default()).await;
    let run = start_in(&h.sink, Some("checkout"), 0);
    let last_act = || async {
        h.store
            .task_last_act(&task("checkout"))
            .await
            .unwrap_or_else(|error| panic!("task_last_act: {error}"))
    };
    read_on(&h.sink, OperationId::new(run, 1), Some(TabId(5)), 10);
    flushed(&h).await;
    assert_eq!(last_act().await, act(5, 10), "its start, not its end at 15");

    new_page(&h.sink, OperationId::new(run, 2), TabId(8), 20);
    flushed(&h).await;
    assert_eq!(last_act().await, act(8, 25), "its end, not its start at 20");
}

#[tokio::test]
async fn events_name_the_task_and_the_target_tab() {
    let h = harness(WriterConfig::default()).await;
    let mut events = h.sink.subscribe();
    let tasked = start_in(&h.sink, Some("checkout"), 0);
    let click = OperationId::new(tasked, 1);
    op_started(
        &h.sink,
        Op {
            tab: Some(TabId(4)),
            ..simple(click, "locator.click", Effect::Write)
        },
    );
    op_ended(
        &h.sink,
        click,
        Delivery::Done,
        None,
        TargetSensitivity::Ordinary,
    );
    end_run(&h.sink, tasked, RunStatus::Ok, None);
    let untasked = start_in(&h.sink, None, 0);
    let read = OperationId::new(untasked, 1);
    op_started(
        &h.sink,
        Op {
            tab: None,
            ..simple(read, "neo.solveCaptcha", Effect::Read)
        },
    );
    flushed(&h).await;

    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    let checkout = Some(task("checkout"));
    assert_eq!(
        received,
        vec![
            TraceRecordEvent::RunStarted {
                run: tasked,
                task: checkout.clone(),
                at: at(0),
            },
            TraceRecordEvent::OperationStarted {
                id: click,
                task: checkout.clone(),
                tab: Some(TabId(4)),
                effect: Effect::Write,
                at: at(10),
            },
            TraceRecordEvent::OperationEnded {
                id: click,
                task: checkout.clone(),
                tab: Some(TabId(4)),
                outcome: Outcome::Ok,
                delivery: Delivery::Done,
                at: at(15),
            },
            TraceRecordEvent::RunEnded {
                run: tasked,
                task: checkout,
                status: RunStatus::Ok,
                effects: RunEffects::Done,
                at: at(1_000),
            },
            TraceRecordEvent::RunComplete { run: tasked },
            TraceRecordEvent::RunStarted {
                run: untasked,
                task: None,
                at: at(0),
            },
            TraceRecordEvent::OperationStarted {
                id: read,
                task: None,
                tab: None,
                effect: Effect::Read,
                at: at(10),
            },
        ]
    );
}

#[tokio::test]
async fn a_run_summary_is_the_run_without_its_operations() {
    let h = harness(WriterConfig::default()).await;
    let run = start_run(
        &h.sink,
        Start {
            request: Some("call-9"),
            task: Some("task-9"),
            ..Start::default()
        },
    );
    let goto = OperationId::new(run, 1);
    op_started(
        &h.sink,
        Op {
            tab: Some(TabId(4)),
            ..simple(goto, "page.goto", Effect::Write)
        },
    );
    op_started(
        &h.sink,
        simple(OperationId::new(run, 2), "page.title", Effect::Read),
    );
    flushed(&h).await;

    let running = h.store.run_summary(run).await.ok().flatten();
    let Some(running) = running else {
        panic!("run not found");
    };
    assert_eq!(running.status, RunStatus::Running);
    assert_eq!(running.effects, None);
    assert_eq!(running.tabs, vec![TabId(3), TabId(4)]);
    assert_eq!(running.operation_count, 2);
    assert_eq!(running.task_id, Some(task("task-9")));
    assert_eq!(running.request_id, Some(request("call-9")));

    op_ended(
        &h.sink,
        goto,
        Delivery::Unknown,
        None,
        TargetSensitivity::Ordinary,
    );
    end_run(&h.sink, run, RunStatus::Ok, None);
    flushed(&h).await;
    let ended = h.store.run_summary(run).await.ok().flatten();
    assert_eq!(
        ended.as_ref().map(|run| (run.status, run.effects)),
        Some((RunStatus::Ok, Some(RunEffects::Unknown)))
    );
    let detail = h.store.run(run).await.ok().flatten();
    assert_eq!(
        ended,
        detail.map(|detail| detail.summary),
        "the same summary as the full read"
    );
    assert!(
        h.store
            .run_summary(RunId::new())
            .await
            .is_ok_and(|summary| summary.is_none())
    );
}

#[tokio::test]
async fn open_writes_count_a_tabs_unfinished_writes_in_running_runs() {
    let h = harness(WriterConfig::default()).await;
    let open_writes = |tab: i64| {
        let store = h.store.clone();
        async move {
            store
                .open_writes(TabId(tab))
                .await
                .unwrap_or_else(|error| panic!("open_writes: {error}"))
        }
    };
    let run = start_in(&h.sink, None, 0);
    let click = OperationId::new(run, 1);
    let fill = OperationId::new(run, 2);
    let goto = OperationId::new(run, 4);
    op_started(&h.sink, simple(click, "locator.click", Effect::Write));
    op_started(&h.sink, simple(fill, "locator.fill", Effect::Write));
    op_ended(
        &h.sink,
        fill,
        Delivery::Done,
        None,
        TargetSensitivity::Ordinary,
    );
    op_started(
        &h.sink,
        simple(OperationId::new(run, 3), "page.title", Effect::Read),
    );
    op_started(
        &h.sink,
        Op {
            tab: Some(TabId(4)),
            ..simple(goto, "page.goto", Effect::Write)
        },
    );
    flushed(&h).await;
    assert_eq!(
        open_writes(3).await,
        1,
        "the click; not the ended fill or the read"
    );
    assert_eq!(open_writes(4).await, 1);
    assert_eq!(open_writes(5).await, 0);

    op_ended(
        &h.sink,
        click,
        Delivery::Done,
        None,
        TargetSensitivity::Ordinary,
    );
    flushed(&h).await;
    assert_eq!(open_writes(3).await, 0);

    end_run(&h.sink, run, RunStatus::Ok, None);
    flushed(&h).await;
    assert_eq!(open_writes(4).await, 0, "the run's end closed the goto");

    op_started(
        &h.sink,
        simple(OperationId::new(run, 5), "locator.click", Effect::Write),
    );
    flushed(&h).await;
    assert_eq!(
        open_writes(3).await,
        0,
        "a write that started after its run ended is not in flight"
    );
}

#[tokio::test]
async fn a_restart_leaves_no_write_in_flight() {
    let h = harness(WriterConfig::default()).await;
    let run = start_in(&h.sink, None, 0);
    op_started(
        &h.sink,
        simple(OperationId::new(run, 1), "locator.click", Effect::Write),
    );
    flushed(&h).await;
    assert!(
        h.store
            .open_writes(TabId(3))
            .await
            .is_ok_and(|open| open == 1)
    );
    let Harness { dir, sink, .. } = h;
    drop(sink);

    let reopened = open(&dir).await;
    assert!(
        reopened
            .open_writes(TabId(3))
            .await
            .is_ok_and(|open| open == 0)
    );
}

/// Recording must stay off the agent's path: the dispatcher's side of one operation is two
/// channel sends. Measured at about 200 ns per operation in release (sdk-traces design D4); the
/// bound here is loose enough for debug builds and only catches a blocking regression.
#[tokio::test]
async fn recording_an_operation_never_waits_on_the_writer() {
    let h = harness(WriterConfig::default()).await;
    let run = start_in(&h.sink, Some("bench"), 0);
    // Under the backlog cap (two events per operation), so nothing is dropped.
    let samples = 2_000u32;
    let started = std::time::Instant::now();
    for seq in 1..=samples {
        let id = OperationId::new(run, seq);
        op_started(&h.sink, simple(id, "locator.click", Effect::Write));
        op_ended(
            &h.sink,
            id,
            Delivery::Done,
            None,
            TargetSensitivity::Ordinary,
        );
    }
    let per_op = started.elapsed() / samples;
    assert!(
        per_op < Duration::from_micros(50),
        "{per_op:?} per operation"
    );
    flushed(&h).await;
    let count = h
        .store
        .run(run)
        .await
        .ok()
        .flatten()
        .map(|run| run.summary.operation_count);
    assert_eq!(count, Some(u64::from(samples)));
}

/// A value claw-sdk still holds unregistered (decision 32), which no process set knows.
const HELD: &str = "held-7Qx-swordfish";

/// A set of its own, as claw-sdk passes its held values: the writer masks a record with it and
/// the process set never learns them.
fn also_mask(values: &[&str]) -> Arc<SecretSet> {
    let set = SecretSet::new(SecretSetConfig::default());
    for value in values {
        let _registration = set.register(SecretValue::new((*value).to_string()), None);
    }
    Arc::new(set)
}

/// The store file and its WAL, each read whole. Only the WAL may be absent (after a
/// checkpoint): a read that fails must fail the test, or a scan would pass without looking.
fn store_files(h: &Harness) -> Result<[Vec<u8>; 2], Box<dyn std::error::Error + Send + Sync>> {
    let bytes = fs::read(h.dir.path().join("claw-sdk/trace.db"))?;
    let wal = match fs::read(h.dir.path().join("claw-sdk/trace.db-wal")) {
        Ok(wal) => wal,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    Ok([bytes, wal])
}

fn holds(file: &[u8], needle: &str) -> bool {
    file.windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

fn failure(message: String) -> ErrorRecord {
    ErrorRecord {
        owner: ErrorOwner::Website,
        code: "assertion_failed".into(),
        message,
        host_code: None,
        retryable: false,
        operation_id: None,
    }
}

#[tokio::test]
async fn a_records_also_mask_masks_every_text_it_stores_and_registers_nothing()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let h = harness(WriterConfig::default()).await;
    let held = also_mask(&[HELD]);
    let run = RunId::new();
    h.sink.run_started(RunStarted {
        id: run,
        code: format!("return '{HELD}'"),
        request_id: None,
        task_id: None,
        host_data: Some(json!({"note": format!("for {HELD}")})),
        also_mask: Some(Arc::clone(&held)),
        at: at(0),
    });
    let read = OperationId::new(run, 1);
    h.sink.op_started(OperationStarted {
        args: json!({"expected": format!("Welcome {HELD}")}),
        also_mask: Some(Arc::clone(&held)),
        ..started(simple(read, "page.title", Effect::Read))
    });
    flushed(&h).await;
    let admitted = h
        .store
        .operation(read)
        .await?
        .ok_or("operation not found")?;
    assert_eq!(admitted.args, None, "the start row stores no input");

    h.sink
        .op_notes(read, vec![format!("read {HELD} from the title")]);
    h.sink.op_ended(OperationEnded {
        result: Some(json!({"title": format!("Welcome {HELD}")})),
        page_after: Some(PageState {
            page_id: Some(3),
            tab_id: Some(TabId(3)),
            url: Some(format!("https://b.test/welcome?name={HELD}")),
            title: Some(format!("Welcome {HELD}")),
        }),
        also_mask: Some(Arc::clone(&held)),
        ..ended(read)
    });
    // An end with no set of its own still masks with what its start was given: a value held as
    // the operation began may be in what it returned.
    let check = OperationId::new(run, 2);
    h.sink.op_started(OperationStarted {
        also_mask: Some(Arc::clone(&held)),
        ..started(simple(check, "expect.toHaveTitle", Effect::Read))
    });
    h.sink.op_ended(OperationEnded {
        outcome: Outcome::Error,
        error: Some(failure(format!("expected title {HELD}"))),
        ..ended(check)
    });
    h.sink.run_ended(RunEnded {
        id: run,
        status: RunStatus::Error,
        error: Some(failure(format!("Error: expected title {HELD}"))),
        result: Some(json!({"echo": HELD})),
        replay: None,
        also_mask: Some(Arc::clone(&held)),
        at: at(1_000),
    });
    flushed(&h).await;

    let done = h.store.run(run).await?.ok_or("run not found")?;
    assert_eq!(done.code.as_deref(), Some("return '[private]'"));
    assert_eq!(done.host_data, Some(json!({"note": "for [private]"})));
    assert_eq!(done.result, Some(json!({"echo": "[private]"})));
    assert_eq!(
        done.summary.error.map(|error| error.message).as_deref(),
        Some("Error: expected title [private]")
    );
    let [title, expect] = &done.operations[..] else {
        panic!("two operations: {:?}", done.operations);
    };
    assert_eq!(title.args, Some(json!({"expected": "Welcome [private]"})));
    assert_eq!(title.notes, ["read [private] from the title"]);
    assert_eq!(title.result, Some(json!({"title": "Welcome [private]"})));
    assert_eq!(
        title.page.url.as_deref(),
        Some("https://b.test/welcome?name=[private]")
    );
    assert_eq!(title.page.title.as_deref(), Some("Welcome [private]"));
    assert_eq!(
        expect.error.as_ref().map(|error| error.message.as_str()),
        Some("expected title [private]")
    );
    for file in store_files(&h)? {
        assert!(!holds(&file, HELD), "the file keeps the held value");
    }

    assert_eq!(h.secrets.count(), 0, "a record's set registers nothing");
    let later = start_run(
        &h.sink,
        Start {
            code: HELD,
            ..Start::default()
        },
    );
    end_run(&h.sink, later, RunStatus::Ok, None);
    flushed(&h).await;
    let later = h.store.run(later).await?.ok_or("run not found")?;
    assert_eq!(
        later.code.as_deref(),
        Some(HELD),
        "a record without the set keeps what the process set does not know"
    );
    Ok(())
}

/// The prefix of [`HELD`] the owner had typed, or left after a backspace, at one end of a record.
const PREFIX: &str = "held-7Qx";

#[tokio::test]
async fn a_held_value_that_grew_or_shrank_between_an_ops_start_and_end_is_masked_whole()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let h = harness(WriterConfig::default()).await;
    let run = start_in(&h.sink, None, 0);
    // Typed on as the op ran, then backspaced as the next one ran: the start's set and the
    // end's each hold the value or its prefix, so whichever set masks first must not leave the
    // other's tail behind.
    let grew = OperationId::new(run, 1);
    let shrank = OperationId::new(run, 2);
    for (op, at_start, at_end) in [(grew, PREFIX, HELD), (shrank, HELD, PREFIX)] {
        h.sink.op_started(OperationStarted {
            args: json!({"expected": HELD}),
            also_mask: Some(also_mask(&[at_start])),
            ..started(simple(op, "locator.textContent", Effect::Read))
        });
        h.sink.op_ended(OperationEnded {
            result: Some(json!({"text": HELD})),
            also_mask: Some(also_mask(&[at_end])),
            ..ended(op)
        });
    }
    flushed(&h).await;
    for op in [grew, shrank] {
        let row = h.store.operation(op).await?.ok_or("operation not found")?;
        assert_eq!(row.args, Some(json!({"expected": "[private]"})), "{op}");
        assert_eq!(row.result, Some(json!({"text": "[private]"})), "{op}");
    }
    Ok(())
}

#[tokio::test]
async fn a_held_value_that_grew_or_shrank_during_a_run_is_masked_whole_in_its_code()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let h = harness(WriterConfig::default()).await;
    let mut runs = Vec::new();
    for (at_start, at_end) in [(PREFIX, HELD), (HELD, PREFIX)] {
        let run = RunId::new();
        h.sink.run_started(RunStarted {
            id: run,
            code: format!("return '{HELD}'"),
            request_id: None,
            task_id: None,
            host_data: None,
            also_mask: Some(also_mask(&[at_start])),
            at: at(0),
        });
        h.sink.run_ended(RunEnded {
            id: run,
            status: RunStatus::Ok,
            error: None,
            result: None,
            replay: None,
            also_mask: Some(also_mask(&[at_end])),
            at: at(1_000),
        });
        runs.push(run);
    }
    flushed(&h).await;
    for run in runs {
        let done = h.store.run(run).await?.ok_or("run not found")?;
        assert_eq!(done.code.as_deref(), Some("return '[private]'"), "{run}");
    }
    Ok(())
}

#[tokio::test]
async fn a_held_value_whose_prefix_the_process_set_knows_is_masked_whole()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let h = harness(WriterConfig::default()).await;
    // The owner paused long enough for the prefix to be registered, then typed on. The record's
    // set holds the value only as typed, not as the process set's mask leaves it.
    let _registration = h
        .secrets
        .register(SecretValue::new(PREFIX.to_string()), None);
    let run = start_in(&h.sink, None, 0);
    let read = OperationId::new(run, 1);
    op_started(&h.sink, simple(read, "locator.textContent", Effect::Read));
    h.sink.op_ended(OperationEnded {
        result: Some(json!({"text": HELD})),
        also_mask: Some(also_mask(&[HELD])),
        ..ended(read)
    });
    flushed(&h).await;
    let row = h
        .store
        .operation(read)
        .await?
        .ok_or("operation not found")?;
    assert_eq!(row.result, Some(json!({"text": "[private]"})));
    Ok(())
}

/// A held value long enough that a cut placed near it falls inside it.
const LONG_HELD: &str = "held-7Qx-swordfish-and-a-long-passphrase-tail";

/// Where a clip of a text with `limit` bytes cuts: it keeps room for its `…[+N bytes]` marker.
const MARKER_ROOM: usize = 24;

#[tokio::test]
async fn a_held_value_the_size_cut_falls_on_is_masked_before_the_cut()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let h = harness(WriterConfig::default()).await;
    let limits = RedactionLimits::default();
    let held = also_mask(&[LONG_HELD]);
    let head = &LONG_HELD[..8];
    // The held value starts 16 bytes before where `cut` says the clip falls, and text follows
    // it. A cut made before the mask keeps the value's head; one made after keeps `[private`.
    let across = |cut: usize, fill: char| {
        format!(
            "{}{LONG_HELD}{}",
            fill.to_string().repeat(cut - 16),
            "z".repeat(100)
        )
    };
    // A JSON string's room is the string limit; host data's is its whole budget, less the
    // object around its one string.
    let json_cut = limits.string - MARKER_ROOM;
    let host_cut = limits.host_data - json!({"blob": ""}).to_string().len() - MARKER_ROOM;
    let run = RunId::new();
    h.sink.run_started(RunStarted {
        id: run,
        code: across(limits.code - MARKER_ROOM, 'c'),
        request_id: None,
        task_id: None,
        host_data: Some(json!({"blob": across(host_cut, 'h')})),
        also_mask: Some(Arc::clone(&held)),
        at: at(0),
    });
    let read = OperationId::new(run, 1);
    h.sink.op_started(OperationStarted {
        also_mask: Some(Arc::clone(&held)),
        ..started(simple(read, "page.title", Effect::Read))
    });
    h.sink
        .op_notes(read, vec![across(limits.note - MARKER_ROOM, 'n')]);
    h.sink.op_ended(OperationEnded {
        result: Some(json!({"text": across(json_cut, 'r')})),
        page_after: Some(PageState {
            page_id: Some(3),
            tab_id: Some(TabId(3)),
            url: None,
            title: Some(across(limits.text - MARKER_ROOM, 't')),
        }),
        also_mask: Some(Arc::clone(&held)),
        ..ended(read)
    });
    h.sink.run_ended(RunEnded {
        id: run,
        status: RunStatus::Ok,
        error: None,
        result: Some(json!({"text": across(json_cut, 'e')})),
        replay: None,
        also_mask: Some(Arc::clone(&held)),
        at: at(1_000),
    });
    flushed(&h).await;

    let done = h.store.run(run).await?.ok_or("run not found")?;
    let title = &done.operations[0];
    assert!(
        done.code_truncated && done.result_truncated && title.result_truncated,
        "every text was cut"
    );
    let stored = [
        done.code.clone().unwrap_or_default(),
        done.host_data
            .map(|data| data.to_string())
            .unwrap_or_default(),
        done.result
            .map(|result| result.to_string())
            .unwrap_or_default(),
        title.notes.join("\n"),
        title
            .result
            .as_ref()
            .map(Value::to_string)
            .unwrap_or_default(),
        title.page.title.clone().unwrap_or_default(),
    ];
    for text in &stored {
        assert!(text.contains("…[+"), "not cut: …{}", tail_of(text));
        assert!(
            !text.contains(head),
            "the cut kept the held value's head: …{}",
            tail_of(text)
        );
    }
    for file in store_files(&h)? {
        assert!(!holds(&file, head), "the file keeps the held value's head");
    }
    Ok(())
}

/// The last 40 characters, enough to show what a cut kept.
fn tail_of(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    chars[chars.len().saturating_sub(40)..].iter().collect()
}

/// `op`'s start, as [`op_started`] sends it, with no set of its own.
fn started(op: Op<'_>) -> OperationStarted {
    OperationStarted {
        id: op.id,
        parent: op.parent,
        name: op.name.to_string(),
        kind: op.kind,
        effect: op.effect,
        tab: op.tab,
        args: op.args,
        policy: Arc::new(op.policy),
        also_mask: None,
        at: op.at,
    }
}

/// An end that returned nothing, with no set of its own.
fn ended(id: OperationId) -> OperationEnded {
    OperationEnded {
        id,
        outcome: Outcome::Ok,
        delivery: Delivery::Done,
        sent_at: None,
        result: None,
        error: None,
        page_after: None,
        target: TargetSensitivity::Ordinary,
        also_mask: None,
        at: at(i64::from(id.seq) * 10 + 5),
    }
}

/// What a run's open operation and the run itself have on disk while they are open: neither its
/// input nor its host data, until its end masks them with what the end's own set knows.
#[tokio::test]
async fn a_start_keeps_its_input_and_host_data_off_disk_until_its_end_masks_them()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let h = harness(WriterConfig::default()).await;
    // A value only the end's own set knows: the op's own sweep found it, so the start's set
    // (and the process set) did not have it yet.
    let found = "swept-9Qp-cardnumber";
    let run = RunId::new();
    h.sink.run_started(RunStarted {
        id: run,
        code: "x".to_string(),
        request_id: None,
        task_id: None,
        host_data: Some(json!({"note": format!("for {found}")})),
        also_mask: None,
        at: at(0),
    });
    let fill = OperationId::new(run, 1);
    h.sink.op_started(OperationStarted {
        args: json!({"value": format!("copied {found}")}),
        ..started(simple(fill, "locator.fill", Effect::Write))
    });
    flushed(&h).await;

    let open = h.store.run(run).await?.ok_or("run not found")?;
    assert_eq!(open.host_data, None, "an open run stores no host data");
    let admitted = h
        .store
        .operation(fill)
        .await?
        .ok_or("operation not found")?;
    assert_eq!(admitted.args, None, "an open operation stores no input");
    for file in store_files(&h)? {
        assert!(
            !holds(&file, found),
            "the open records keep the value on disk"
        );
    }

    let held = also_mask(&[found]);
    h.sink.op_ended(OperationEnded {
        also_mask: Some(Arc::clone(&held)),
        ..ended(fill)
    });
    h.sink.run_ended(RunEnded {
        id: run,
        status: RunStatus::Ok,
        error: None,
        result: None,
        replay: None,
        also_mask: Some(held),
        at: at(1_000),
    });
    flushed(&h).await;
    let done = h.store.run(run).await?.ok_or("run not found")?;
    assert_eq!(done.host_data, Some(json!({"note": "for [private]"})));
    assert_eq!(
        done.operations[0].args,
        Some(json!({"value": "copied [private]"}))
    );
    for file in store_files(&h)? {
        assert!(!holds(&file, found), "the file keeps the value");
    }
    Ok(())
}

/// The name of operation `call` of task `task`, as each read of the store gives it: the
/// operation's row, its run's operations, and the trace's tail.
async fn names_read(
    h: &Harness,
    call: OperationId,
    task_name: &str,
) -> Result<[String; 3], Box<dyn std::error::Error + Send + Sync>> {
    let row = h.store.operation(call).await?.ok_or("no operation")?;
    let run = h.store.run(call.run).await?.ok_or("no run")?;
    let query = TailQuery {
        task_id: Some(task(task_name)),
        tab: None,
        tail: 10,
    };
    let tail = h.store.trace_tail(query).await?;
    let only = |names: Vec<&str>| names.join(",");
    Ok([
        row.name,
        only(run.operations.iter().map(|op| op.name.as_str()).collect()),
        only(tail.operations.iter().map(|op| op.name.as_str()).collect()),
    ])
}

/// A script can call any name, and an unknown method is recorded under the name it was called
/// by, which may hold a value its run still holds or one the process set knows. A running
/// operation is listed by its name, so the name is stored as the operation starts, masked with
/// what the start knows.
#[tokio::test]
async fn an_operations_name_is_masked_with_what_its_start_knows_before_it_is_stored()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let h = harness(WriterConfig::default()).await;
    let known = "known-4Rt-apikey";
    let _registration = h
        .secrets
        .register(SecretValue::new(known.to_string()), None);
    let run = start_in(&h.sink, Some("checkout"), 0);
    let call = OperationId::new(run, 1);
    let name = format!("vault.{HELD}.{known}");
    h.sink.op_started(OperationStarted {
        also_mask: Some(also_mask(&[HELD])),
        ..started(simple(call, &name, Effect::Write))
    });
    flushed(&h).await;

    let masked = "vault.[private].[private]".to_string();
    assert_eq!(
        names_read(&h, call, "checkout").await?,
        [masked.clone(), masked.clone(), masked],
        "the operation's row (/v1/operations, /v1/events), its run while it runs, the trace"
    );
    for file in store_files(&h)? {
        assert!(!holds(&file, HELD), "the file keeps the held value");
        assert!(!holds(&file, known), "the file keeps the registered value");
    }
    assert_eq!(h.secrets.count(), 1, "the start's set registers nothing");
    Ok(())
}
