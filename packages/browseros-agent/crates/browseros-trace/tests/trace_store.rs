//! The trace store and writer through the crate's public API, on a real SQLite file: what a
//! run's records look like after a flush, what a crash leaves behind, idempotent replay,
//! redaction, backpressure, retention and durable privacy.

use browseros_trace::{
    Delivery, Effect, EpochMillis, ErrorOwner, ErrorRecord, JsonPointer, OperationEnded,
    OperationId, OperationKind, OperationStarted, Outcome, PageKey, PageState, Privacy,
    PrivateReason, RedactionLimits, RedactionPolicy, Redactor, RetentionPolicy, RunEffects,
    RunEnded, RunId, RunQuery, RunStarted, RunStatus, SecretScope, SecretSet, SecretSetConfig,
    SecretValue, SessionId, SessionQuery, SessionStarted, SessionState, TargetSensitivity,
    TraceRecordEvent, TraceSink, TraceStore, WriterConfig, spawn_writer,
};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
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

fn at(millis: i64) -> EpochMillis {
    EpochMillis::from_millis(1_800_000_000_000 + millis)
}

fn start_run(sink: &TraceSink, session: SessionId, code: &str, key: Option<&str>) -> RunId {
    let run = RunId::new();
    sink.run_started(RunStarted {
        id: run,
        session,
        code: code.to_string(),
        host_ref: Some("tool-call-1".into()),
        host: Some(json!({"ownerRun": "r1"})),
        idempotency_key: key.map(str::to_string),
        at: at(0),
    });
    run
}

struct Op<'a> {
    id: OperationId,
    parent: Option<OperationId>,
    name: &'a str,
    kind: OperationKind,
    effect: Effect,
    args: Value,
    policy: RedactionPolicy,
}

fn op_started(sink: &TraceSink, session: SessionId, op: Op<'_>) {
    sink.op_started(OperationStarted {
        id: op.id,
        parent: op.parent,
        session,
        name: op.name.to_string(),
        kind: op.kind,
        effect: op.effect,
        page: Some(3),
        args: op.args,
        policy: Arc::new(op.policy),
        at: at(i64::from(op.id.seq) * 10),
    });
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
            tab_id: Some(77),
            url: Some("https://x.example/done".into()),
            title: Some("Done".into()),
        }),
        target,
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
        at: at(1_000),
    });
}

fn simple(id: OperationId, name: &str, effect: Effect) -> Op<'_> {
    Op {
        id,
        parent: None,
        name,
        kind: OperationKind::Builtin,
        effect,
        args: json!({"page": {"$neo": "page", "id": 3}}),
        policy: RedactionPolicy::default(),
    }
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

#[tokio::test]
async fn a_run_reads_back_as_a_tree_with_owners_and_deliveries() {
    let h = harness(WriterConfig::default()).await;
    let session = SessionId::new();
    h.sink.session_started(SessionStarted {
        id: session,
        client: Some("zuzu".into()),
        at: at(0),
    });
    let run = start_run(&h.sink, session, "await page.goto(url)", None);
    let goto = OperationId::new(run, 1);
    let fill = OperationId::new(run, 2);
    let host_call = OperationId::new(run, 3);
    let protected = OperationId::new(run, 4);
    op_started(&h.sink, session, simple(goto, "page.goto", Effect::Write));
    op_ended(
        &h.sink,
        goto,
        Delivery::Done,
        None,
        TargetSensitivity::Ordinary,
    );
    op_started(
        &h.sink,
        session,
        Op {
            kind: OperationKind::Defined,
            ..simple(fill, "zuzu.auth.fill", Effect::Write)
        },
    );
    op_started(
        &h.sink,
        session,
        Op {
            parent: Some(fill),
            kind: OperationKind::HostCall,
            ..simple(host_call, "host.call", Effect::Write)
        },
    );
    op_started(
        &h.sink,
        session,
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
    assert_eq!(detail.summary.host_ref.as_deref(), Some("tool-call-1"));
    assert_eq!(detail.host, Some(json!({"ownerRun": "r1"})));
    let ops = &detail.operations;
    assert_eq!(
        ops.iter().map(|op| op.id.seq).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(ops[2].parent, Some(fill));
    assert_eq!(ops[3].parent, Some(host_call));
    assert_eq!(ops[3].kind, OperationKind::Protected);
    assert!(ops.iter().all(|op| op.delivery == Some(Delivery::Done)));
    assert_eq!(ops[0].page.url.as_deref(), Some("https://x.example/done"));
    assert_eq!(ops[0].page.tab_id, Some(77));

    let trace = h.store.session_trace(session, Some(2)).await.ok().flatten();
    let Some(trace) = trace else {
        panic!("session not found");
    };
    assert!(trace.truncated);
    assert_eq!(trace.operations.len(), 2);
    assert_eq!(trace.summary.client.as_deref(), Some("zuzu"));
    assert_eq!(trace.summary.run_count, 1);
}

#[tokio::test]
async fn effects_roll_up_from_write_operations() {
    let h = harness(WriterConfig::default()).await;
    let session = SessionId::new();
    let reads = start_run(&h.sink, session, "read", None);
    let read = OperationId::new(reads, 1);
    op_started(
        &h.sink,
        session,
        simple(read, "locator.textContent", Effect::Read),
    );
    op_ended(
        &h.sink,
        read,
        Delivery::Done,
        Some(json!("text")),
        TargetSensitivity::Ordinary,
    );
    end_run(&h.sink, reads, RunStatus::Ok, None);

    let unsure = start_run(&h.sink, session, "click", None);
    let click = OperationId::new(unsure, 1);
    op_started(
        &h.sink,
        session,
        simple(click, "locator.click", Effect::Write),
    );
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
    let session = SessionId::new();
    let run = start_run(&h.sink, session, "submit", None);
    let read = OperationId::new(run, 1);
    let submit = OperationId::new(run, 2);
    op_started(&h.sink, session, simple(read, "page.title", Effect::Read));
    op_started(
        &h.sink,
        session,
        simple(submit, "locator.click", Effect::Write),
    );
    flushed(&h).await;
    let Harness { dir, sink, .. } = h;
    drop(sink);

    let reopened = open(&dir).await;
    let detail = reopened.run(run).await.ok().flatten();
    let Some(detail) = detail else {
        panic!("run not found");
    };
    assert_eq!(detail.summary.status, RunStatus::Interrupted);
    assert_eq!(detail.summary.effects, Some(RunEffects::Unknown));
    assert_eq!(detail.operations[0].delivery, Some(Delivery::NotSent));
    assert_eq!(detail.operations[1].delivery, Some(Delivery::Unknown));
    assert_eq!(detail.operations[1].outcome, Some(Outcome::Cancelled));
    let sessions = reopened.sessions(SessionQuery {
        state: Some(SessionState::Open),
        ..SessionQuery::default()
    });
    assert_eq!(sessions.await.ok().map(|page| page.items.len()), Some(0));
}

#[tokio::test]
async fn an_idempotency_key_finds_the_run_and_its_replay_expires() {
    let h = harness(WriterConfig {
        replay_window: Duration::from_millis(100),
        ..WriterConfig::default()
    })
    .await;
    let session = SessionId::new();
    let run = start_run(&h.sink, session, "buy", Some("key-1"));
    end_run(&h.sink, run, RunStatus::Ok, None);
    flushed(&h).await;

    let found = h.store.run_by_key(session, "key-1").await.ok().flatten();
    assert_eq!(found.map(|run| run.id), Some(run));
    let other = h
        .store
        .run_by_key(SessionId::new(), "key-1")
        .await
        .ok()
        .flatten();
    assert!(other.is_none(), "keys are per session");
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
async fn sensitive_targets_redact_their_input_and_withhold_the_code() {
    let h = harness(WriterConfig::default()).await;
    let session = SessionId::new();
    let run = start_run(
        &h.sink,
        session,
        "await email.fill('me@x.example'); await pw.fill('hunter22')",
        None,
    );
    let email = OperationId::new(run, 1);
    let password = OperationId::new(run, 2);
    for (id, value) in [(email, "me@x.example"), (password, "hunter22")] {
        op_started(
            &h.sink,
            session,
            Op {
                args: json!({"locator": {"$neo": "locator", "page": 3, "selector": "getByLabel('x')"}, "value": value}),
                policy: fill_policy(),
                ..simple(id, "locator.fill", Effect::Write)
            },
        );
    }
    flushed(&h).await;
    let open = h.store.run(run).await.ok().flatten();
    let conservative = open.map(|run| run.operations[0].args.clone());
    assert_eq!(
        conservative.flatten().map(|args| args["value"].clone()),
        Some(json!("[redacted]")),
        "while open, both kinds of path are redacted"
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
    h.secrets.register(
        SecretValue::new("123-456-otp".into()),
        SecretScope::Browser,
        None,
    );
    let session = SessionId::new();
    let run = start_run(&h.sink, session, "return '123-456-otp'", None);
    let read = OperationId::new(run, 1);
    op_started(
        &h.sink,
        session,
        simple(read, "locator.textContent", Effect::Read),
    );
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

#[tokio::test]
async fn over_the_backlog_operation_events_drop_but_lifecycle_events_do_not() {
    let h = harness(WriterConfig {
        max_backlog: 0,
        ..WriterConfig::default()
    })
    .await;
    let session = SessionId::new();
    let run = start_run(&h.sink, session, "x", None);
    let click = OperationId::new(run, 1);
    op_started(
        &h.sink,
        session,
        simple(click, "locator.click", Effect::Write),
    );
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
    assert_eq!(summary.trace_gaps, 2);
    assert_eq!(summary.operation_count, 0);
    assert_eq!(
        summary.effects,
        Some(RunEffects::Unknown),
        "a gap may have been a write"
    );
}

#[tokio::test]
async fn retention_deletes_old_runs_and_empty_sessions() {
    let h = harness(WriterConfig::default()).await;
    let old_session = SessionId::new();
    let old = RunId::new();
    h.sink.run_started(RunStarted {
        id: old,
        session: old_session,
        code: "old".into(),
        host_ref: None,
        host: None,
        idempotency_key: None,
        at: at(0),
    });
    let read = OperationId::new(old, 1);
    op_started(
        &h.sink,
        old_session,
        simple(read, "page.title", Effect::Read),
    );
    op_ended(
        &h.sink,
        read,
        Delivery::Done,
        None,
        TargetSensitivity::Ordinary,
    );
    h.sink.run_ended(RunEnded {
        id: old,
        status: RunStatus::Ok,
        error: None,
        result: None,
        replay: None,
        at: at(10),
    });
    let new_session = SessionId::new();
    let eight_days = 8 * 24 * 60 * 60 * 1000;
    let recent = RunId::new();
    h.sink.run_started(RunStarted {
        id: recent,
        session: new_session,
        code: "recent".into(),
        host_ref: None,
        host: None,
        idempotency_key: None,
        at: at(eight_days),
    });
    flushed(&h).await;

    let swept = h
        .sink
        .sweep(RetentionPolicy::default(), at(eight_days + 1))
        .await
        .unwrap_or_else(|error| panic!("sweep: {error}"));
    assert_eq!((swept.runs, swept.operations, swept.sessions), (1, 1, 1));
    assert!(h.store.run(old).await.ok().flatten().is_none());
    assert!(h.store.session(old_session).await.ok().flatten().is_none());
    assert!(
        h.store.run(recent).await.ok().flatten().is_some(),
        "a running run is kept"
    );
}

#[tokio::test]
async fn privacy_commits_through_the_writer_and_survives_a_reopen() {
    let h = harness(WriterConfig::default()).await;
    let mut events = h.sink.subscribe();
    let privacy = Privacy::new(
        h.secrets.clone(),
        Arc::new(h.sink.clone()),
        h.store.private_sessions().await.unwrap_or_default(),
    );
    let session = SessionId::new();
    let result = privacy
        .make_private(session, PrivateReason::Takeover, at(5))
        .await;
    assert!(result.is_ok());
    assert!(matches!(
        events.recv().await,
        Ok(TraceRecordEvent::SessionPrivate { session: s, .. }) if s == session
    ));
    let Harness { dir, sink, .. } = h;
    drop(privacy);
    drop(sink);

    let reopened = open(&dir).await;
    let loaded = reopened.private_sessions().await.unwrap_or_default();
    assert_eq!(
        loaded.get(&session).map(|record| record.reason),
        Some(PrivateReason::Takeover)
    );
    let privacy = Privacy::new(
        Arc::new(SecretSet::new(SecretSetConfig::default())),
        Arc::new(spawn_writer(
            &reopened,
            Redactor::new(
                Arc::new(SecretSet::new(SecretSetConfig::default())),
                RedactionLimits::default(),
            ),
            WriterConfig::default(),
        )),
        loaded,
    );
    assert!(privacy.capture(session, PageKey("t".into())).is_none());
}

#[tokio::test]
async fn runs_page_newest_first_and_filter_by_host_ref() {
    let h = harness(WriterConfig::default()).await;
    let session = SessionId::new();
    let mut ids = Vec::new();
    for (offset, host_ref) in [(0, "a"), (10, "b"), (20, "a")] {
        let id = RunId::new();
        h.sink.run_started(RunStarted {
            id,
            session,
            code: String::new(),
            host_ref: Some(host_ref.into()),
            host: None,
            idempotency_key: None,
            at: at(offset),
        });
        ids.push(id);
    }
    flushed(&h).await;

    let first = h
        .store
        .runs(RunQuery {
            session: Some(session),
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
            session: Some(session),
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

    let by_ref = h
        .store
        .runs(RunQuery {
            host_ref: Some("a".into()),
            ..RunQuery::default()
        })
        .await
        .map(|page| page.items.len());
    assert_eq!(by_ref.ok(), Some(2));
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

/// Recording must stay off the agent's path: the dispatcher's side of one operation is two
/// channel sends. Measured at about 200 ns per operation in release (sdk-traces design D4); the
/// bound here is loose enough for debug builds and only catches a blocking regression.
#[tokio::test]
async fn recording_an_operation_never_waits_on_the_writer() {
    let h = harness(WriterConfig::default()).await;
    let session = SessionId::new();
    let run = start_run(&h.sink, session, "bench", None);
    // Under the backlog cap (two events per operation), so nothing is dropped.
    let samples = 2_000u32;
    let started = std::time::Instant::now();
    for seq in 1..=samples {
        let id = OperationId::new(run, seq);
        op_started(&h.sink, session, simple(id, "locator.click", Effect::Write));
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
