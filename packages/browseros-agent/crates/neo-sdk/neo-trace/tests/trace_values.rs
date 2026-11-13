//! Recording policy through the public writer and a real SQLite file. Raw database and WAL
//! scans catch values that a read model might hide after they have already reached disk.

use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;

use browseros_core::TabId;
use neo_trace::{
    Delivery, Effect, EpochMillis, ErrorOwner, ErrorRecord, OperationEnded, OperationId,
    OperationKind, OperationRow, OperationSite, OperationStarted, Outcome, PageState, RecordPolicy,
    RedactionLimits, Redactor, RequestId, RunDetail, RunEffects, RunEnded, RunId, RunQuery,
    RunStarted, RunStatus, TailQuery, TaskId, TraceSink, TraceStore, Values, WriterConfig,
    spawn_writer,
};
use serde_json::{Value, json};
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

const TASK: &str = "trace-values-task";
const CODE: &str = "trace-values-code-7mQ2";
const ARGS: &str = "trace-values-args-4xP8";
const OP_RESULT: &str = "trace-values-operation-result-9nD3";
const RUN_RESULT: &str = "trace-values-run-result-2vJ6";
const NOTES: &str = "trace-values-notes-5rK1";
const OP_ERROR: &str = "trace-values-operation-error-8bL4";
const RUN_ERROR: &str = "trace-values-run-error-3hF9";
const HOST_DATA: &str = "trace-values-host-data-6wC7";
const REPLAY: &str = "trace-values-replay-1zS5";
const CANARIES: [&str; 9] = [
    CODE, ARGS, OP_RESULT, RUN_RESULT, NOTES, OP_ERROR, RUN_ERROR, HOST_DATA, REPLAY,
];

/// Each test owns its writer and files. Flush is the persistence boundary before either
/// reading records or inspecting SQLite's raw bytes, so a passing scan cannot race a write.
struct Harness {
    dir: TempDir,
    store: TraceStore,
    sink: TraceSink,
}

impl Harness {
    async fn new(values: Values) -> TestResult<Self> {
        let dir = tempfile::tempdir()?;
        let store = TraceStore::open(&dir.path().join("trace.db")).await?;
        let sink = spawn_writer(
            &store,
            Redactor::new(RedactionLimits::default()),
            WriterConfig::default(),
            None,
        );
        set_values(&sink, values);
        Ok(Self { dir, store, sink })
    }

    fn database(&self) -> PathBuf {
        self.dir.path().join("trace.db")
    }

    fn raw_files(&self) -> TestResult<[Vec<u8>; 2]> {
        let database = fs::read(self.database())?;
        // A checkpoint may remove the WAL. Any other read failure must fail the scan.
        let wal = match fs::read(self.dir.path().join("trace.db-wal")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        Ok([database, wal])
    }
}

fn at(millis: i64) -> EpochMillis {
    EpochMillis::from_millis(1_800_000_000_000 + millis)
}

fn set_values(sink: &TraceSink, values: Values) {
    sink.record_policy(RecordPolicy {
        values,
        ..RecordPolicy::default()
    });
}

fn start_run(
    sink: &TraceSink,
    request: &str,
    code: &str,
    host_data: Option<Value>,
) -> TestResult<RunId> {
    let id = RunId::new();
    sink.run_started(RunStarted {
        id,
        code: code.to_owned(),
        request_id: Some(RequestId::parse(request)?),
        task_id: Some(TaskId::parse(TASK)?),
        host_data,
        at: at(0),
    });
    Ok(id)
}

fn operation_started(id: OperationId, name: &str, effect: Effect, args: Value) -> OperationStarted {
    OperationStarted {
        site: Some(OperationSite { line: 3, column: 5 }),
        id,
        parent: None,
        name: name.to_owned(),
        kind: OperationKind::Builtin,
        effect,
        tab: Some(TabId(7)),
        args,
        at: at(i64::from(id.seq) * 10),
    }
}

fn operation_ended(id: OperationId) -> OperationEnded {
    OperationEnded {
        id,
        outcome: Outcome::Ok,
        delivery: Delivery::Done,
        sent_at: None,
        result: None,
        error: None,
        page_after: Some(PageState {
            tab_id: Some(TabId(7)),
            url: Some("https://checkout.example/form".to_owned()),
            title: Some("Checkout".to_owned()),
        }),
        main_world: true,
        frames_incomplete: false,
        at: at(i64::from(id.seq) * 10 + 5),
    }
}

fn run_ended(id: RunId) -> RunEnded {
    RunEnded {
        id,
        status: RunStatus::Ok,
        error: None,
        result: None,
        replay: None,
        at: at(100),
    }
}

fn error(message: &str, operation_id: Option<OperationId>) -> ErrorRecord {
    ErrorRecord {
        owner: ErrorOwner::Host,
        code: "host_error".to_owned(),
        message: message.to_owned(),
        host_code: Some("requires_owner".to_owned()),
        retryable: true,
        operation_id,
    }
}

fn replay() -> Value {
    json!({"content": [{"type": "text", "text": REPLAY}], "isError": false})
}

/// A successful run supplies result canaries; a failed run supplies error canaries. Keeping
/// those lifecycles separate exercises every payload column without inventing a result/error pair.
fn canary_runs(sink: &TraceSink) -> TestResult<[RunId; 2]> {
    let ok = start_run(sink, "request-ok", CODE, Some(json!({"note": HOST_DATA})))?;
    let read = OperationId::new(ok, 1);
    sink.op_started(operation_started(
        read,
        "page.evaluate",
        Effect::Read,
        json!({"expression": ARGS}),
    ));
    sink.op_notes(read, vec![NOTES.to_owned()]);
    sink.op_ended(OperationEnded {
        result: Some(json!({"value": OP_RESULT})),
        ..operation_ended(read)
    });
    sink.run_ended(RunEnded {
        result: Some(json!({"value": RUN_RESULT})),
        replay: Some(replay()),
        ..run_ended(ok)
    });

    let failed = start_run(
        sink,
        "request-error",
        CODE,
        Some(json!({"note": HOST_DATA})),
    )?;
    let fill = OperationId::new(failed, 1);
    sink.op_started(operation_started(
        fill,
        "locator.fill",
        Effect::Write,
        json!({"value": ARGS}),
    ));
    sink.op_notes(fill, vec![NOTES.to_owned()]);
    sink.op_ended(OperationEnded {
        outcome: Outcome::Error,
        delivery: Delivery::NotSent,
        error: Some(error(OP_ERROR, None)),
        ..operation_ended(fill)
    });
    sink.run_ended(RunEnded {
        status: RunStatus::Error,
        error: Some(error(RUN_ERROR, Some(fill))),
        ..run_ended(failed)
    });
    Ok([ok, failed])
}

fn contains(bytes: &[u8], canary: &str) -> bool {
    bytes
        .windows(canary.len())
        .any(|part| part == canary.as_bytes())
}

fn assert_run_metadata(run: &RunDetail, id: RunId, status: RunStatus, request: &str) {
    assert_eq!(run.summary.id, id);
    assert_eq!(run.summary.task_id.as_ref().map(TaskId::as_str), Some(TASK));
    assert_eq!(
        run.summary.request_id.as_ref().map(RequestId::as_str),
        Some(request)
    );
    assert_eq!(run.summary.tabs, [TabId(7)]);
    assert_eq!(run.summary.status, status);
    assert_eq!(run.summary.effects, Some(RunEffects::None));
    assert_eq!(run.summary.started_at, at(0));
    assert_eq!(run.summary.ended_at, Some(at(100)));
    assert_eq!(run.summary.operation_count, 1);
    assert_eq!(run.summary.trace_gaps, 0);
    assert!(run.summary.complete);
    assert!(run.downloads.is_empty());
}

fn assert_operation_metadata(op: &OperationRow, id: OperationId, name: &str, effect: Effect) {
    assert_eq!(op.id, id);
    assert_eq!(op.parent, None);
    assert_eq!(op.name, name);
    assert_eq!(op.site, Some(OperationSite { line: 3, column: 5 }));
    assert_eq!(op.kind, OperationKind::Builtin);
    assert_eq!(op.effect, effect);
    assert_eq!(op.tab, Some(TabId(7)));
    assert_eq!(op.page.tab_id, Some(TabId(7)));
    assert_eq!(
        op.page.url.as_deref(),
        Some("https://checkout.example/form")
    );
    assert_eq!(op.page.title.as_deref(), Some("Checkout"));
    assert_eq!(op.started_at, at(10));
    assert_eq!(op.sent_at, None);
    assert_eq!(op.ended_at, Some(at(15)));
    assert_eq!(op.main_world, Some(true));
    assert_eq!(op.screenshot, None);
}

#[tokio::test]
async fn off_never_writes_values_and_keeps_metadata_on_every_read() -> TestResult {
    let h = Harness::new(Values::Off).await?;
    let [ok, failed] = canary_runs(&h.sink)?;
    h.sink.flush().await?;

    let ok_run = h.store.run(ok).await?.ok_or("missing successful run")?;
    let failed_run = h.store.run(failed).await?.ok_or("missing failed run")?;
    assert_run_metadata(&ok_run, ok, RunStatus::Ok, "request-ok");
    assert_run_metadata(&failed_run, failed, RunStatus::Error, "request-error");
    assert_operation_metadata(
        &ok_run.operations[0],
        OperationId::new(ok, 1),
        "page.evaluate",
        Effect::Read,
    );
    assert_operation_metadata(
        &failed_run.operations[0],
        OperationId::new(failed, 1),
        "locator.fill",
        Effect::Write,
    );
    assert_eq!(ok_run.operations[0].outcome, Some(Outcome::Ok));
    assert_eq!(ok_run.operations[0].delivery, Some(Delivery::Done));
    assert_eq!(failed_run.operations[0].outcome, Some(Outcome::Error));
    assert_eq!(failed_run.operations[0].delivery, Some(Delivery::NotSent));
    assert_eq!(failed_run.operations[0].error, Some(error("", None)));
    assert_eq!(
        failed_run.summary.error,
        Some(error("", Some(OperationId::new(failed, 1))))
    );

    let tail = h.store.trace_tail(TailQuery::default()).await?;
    for run in [&ok_run, &failed_run] {
        assert_eq!(run.summary.values.as_deref(), Some("off"));
        assert_eq!(run.code, None);
        assert_eq!(
            run.code_withheld, None,
            "intentional omission is not interrupted code"
        );
        assert_eq!(run.result, None);
        assert_eq!(run.host_data, None);
        assert!(!run.code_truncated);
        assert!(!run.result_truncated);
        let op = &run.operations[0];
        assert_eq!(op.values.as_deref(), Some("off"));
        assert_eq!(op.args, None);
        assert_eq!(op.result, None);
        assert!(op.notes.is_empty());
        assert!(!op.args_truncated);
        assert!(!op.result_truncated);
        assert_eq!(h.store.operation(op.id).await?, Some(op.clone()));
        assert_eq!(
            h.store.run_summary(run.summary.id).await?,
            Some(run.summary.clone())
        );
        let listed = h
            .store
            .runs(RunQuery {
                request_id: run.summary.request_id.clone(),
                ..RunQuery::default()
            })
            .await?;
        assert_eq!(listed.items.as_slice(), std::slice::from_ref(&run.summary));
        assert_eq!(
            tail.runs
                .iter()
                .find(|summary| summary.id == run.summary.id),
            Some(&run.summary)
        );
        assert_eq!(tail.operations.iter().find(|row| row.id == op.id), Some(op));
        assert_eq!(h.store.replay(run.summary.id, at(101)).await?, None);
    }
    for canary in CANARIES {
        assert!(
            h.raw_files()?.iter().all(|file| !contains(file, canary)),
            "off persisted {canary} in trace.db or its WAL"
        );
    }

    // Reopening has no access to the writer's raw inputs or its reply memory. Request identity
    // must survive even though no persisted reply is available for replay after a restart.
    let reopened = TraceStore::open(&h.database()).await?;
    let request = RequestId::parse("request-ok")?;
    let task = TaskId::parse(TASK)?;
    assert_eq!(
        reopened.run_by_request(&request, Some(&task)).await?,
        Some(ok_run.summary.clone())
    );
    assert_eq!(reopened.replay(ok, at(101)).await?, None);
    assert_eq!(reopened.run(ok).await?, Some(ok_run));
    assert_eq!(reopened.run(failed).await?, Some(failed_run));
    Ok(())
}

#[tokio::test]
async fn on_preserves_values_and_proves_the_raw_scan_detects_every_canary() -> TestResult {
    let h = Harness::new(Values::On).await?;
    let [ok, failed] = canary_runs(&h.sink)?;
    h.sink.flush().await?;
    let ok_run = h.store.run(ok).await?.ok_or("missing successful run")?;
    let failed_run = h.store.run(failed).await?.ok_or("missing failed run")?;

    assert_eq!(ok_run.code.as_deref(), Some(CODE));
    assert_eq!(ok_run.host_data, Some(json!({"note": HOST_DATA})));
    assert_eq!(ok_run.result, Some(json!({"value": RUN_RESULT})));
    assert_eq!(ok_run.operations[0].args, Some(json!({"expression": ARGS})));
    assert_eq!(
        ok_run.operations[0].result,
        Some(json!({"value": OP_RESULT}))
    );
    assert_eq!(failed_run.code.as_deref(), Some(CODE));
    assert_eq!(failed_run.host_data, Some(json!({"note": HOST_DATA})));
    assert_eq!(failed_run.operations[0].args, Some(json!({"value": ARGS})));
    assert_eq!(failed_run.operations[0].error, Some(error(OP_ERROR, None)));
    assert_eq!(
        failed_run.summary.error,
        Some(error(RUN_ERROR, Some(OperationId::new(failed, 1))))
    );
    for run in [&ok_run, &failed_run] {
        assert_eq!(run.summary.values, None);
        assert_eq!(run.code_withheld, None);
        assert!(!run.code_truncated);
        assert!(!run.result_truncated);
        let op = &run.operations[0];
        assert_eq!(op.values, None);
        assert_eq!(op.notes, [NOTES.to_owned()]);
        assert!(!op.args_truncated);
        assert!(!op.result_truncated);
    }
    assert_eq!(h.store.replay(ok, at(101)).await?, Some(replay()));
    assert_eq!(h.store.replay(ok, at(600_100)).await?, None);
    for canary in CANARIES {
        assert!(
            h.raw_files()?.iter().any(|file| contains(file, canary)),
            "the on control did not expose {canary} to the raw-file scan"
        );
    }
    Ok(())
}

#[tokio::test]
async fn policy_follows_record_ends_in_both_directions_without_purging_earlier_values() -> TestResult
{
    let h = Harness::new(Values::On).await?;
    let omitted = [
        "mixed-off-code-4fN8",
        "mixed-off-host-7rC2",
        "mixed-off-args-1sJ9",
        "mixed-off-result-3bP6",
        "mixed-off-notes-8wL5",
        "mixed-off-run-result-2xD4",
        "mixed-off-replay-6vH1",
    ];
    let run = start_run(&h.sink, "mixed-off", omitted[0], Some(json!(omitted[1])))?;
    let before = OperationId::new(run, 1);
    h.sink.op_started(operation_started(
        before,
        "page.evaluate",
        Effect::Read,
        json!("kept-before-args"),
    ));
    h.sink.op_ended(OperationEnded {
        result: Some(json!("kept-before-result")),
        ..operation_ended(before)
    });
    let crossing = OperationId::new(run, 2);
    h.sink.op_started(operation_started(
        crossing,
        "page.evaluate",
        Effect::Read,
        json!(omitted[2]),
    ));
    h.sink.op_notes(crossing, vec![omitted[4].to_owned()]);
    set_values(&h.sink, Values::Off);
    h.sink.op_ended(OperationEnded {
        result: Some(json!(omitted[3])),
        ..operation_ended(crossing)
    });
    h.sink.run_ended(RunEnded {
        result: Some(json!(omitted[5])),
        replay: Some(json!(omitted[6])),
        ..run_ended(run)
    });

    // Inputs admitted while off remain memory-only, so switching back on before their end
    // records them under the new policy rather than permanently discarding them at admission.
    let restored = start_run(
        &h.sink,
        "mixed-on",
        "restored-code",
        Some(json!("restored-host")),
    )?;
    let after = OperationId::new(restored, 1);
    h.sink.op_started(operation_started(
        after,
        "page.evaluate",
        Effect::Read,
        json!("restored-args"),
    ));
    h.sink.op_notes(after, vec!["restored-notes".to_owned()]);
    set_values(&h.sink, Values::On);
    h.sink.op_ended(OperationEnded {
        result: Some(json!("restored-result")),
        ..operation_ended(after)
    });
    h.sink.run_ended(RunEnded {
        result: Some(json!("restored-run-result")),
        replay: Some(json!("restored-replay")),
        ..run_ended(restored)
    });
    h.sink.flush().await?;

    let mixed = h.store.run(run).await?.ok_or("missing mixed run")?;
    assert_eq!(mixed.summary.values.as_deref(), Some("off"));
    assert_eq!(mixed.code, None);
    assert_eq!(mixed.host_data, None);
    assert_eq!(mixed.result, None);
    assert_eq!(h.store.replay(run, at(101)).await?, None);
    assert_eq!(mixed.operations[0].values, None);
    assert_eq!(mixed.operations[0].args, Some(json!("kept-before-args")));
    assert_eq!(
        mixed.operations[0].result,
        Some(json!("kept-before-result"))
    );
    assert_eq!(mixed.operations[1].values.as_deref(), Some("off"));
    assert_eq!(mixed.operations[1].args, None);
    assert_eq!(mixed.operations[1].result, None);
    assert!(mixed.operations[1].notes.is_empty());

    let restored = h.store.run(restored).await?.ok_or("missing restored run")?;
    assert_eq!(restored.summary.values, None);
    assert_eq!(restored.code.as_deref(), Some("restored-code"));
    assert_eq!(restored.host_data, Some(json!("restored-host")));
    assert_eq!(restored.result, Some(json!("restored-run-result")));
    assert_eq!(restored.operations[0].values, None);
    assert_eq!(restored.operations[0].args, Some(json!("restored-args")));
    assert_eq!(
        restored.operations[0].result,
        Some(json!("restored-result"))
    );
    assert_eq!(restored.operations[0].notes, ["restored-notes".to_owned()]);
    assert_eq!(
        h.store.replay(restored.summary.id, at(101)).await?,
        Some(json!("restored-replay"))
    );
    let files = h.raw_files()?;
    assert!(
        files
            .iter()
            .any(|file| contains(file, "kept-before-result")),
        "switching off must not purge a previously recorded value"
    );
    for canary in omitted {
        assert!(
            files.iter().all(|file| !contains(file, canary)),
            "an input admitted before switching off reached disk: {canary}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn run_end_marks_pending_operations_off_without_materializing_their_inputs() -> TestResult {
    let h = Harness::new(Values::On).await?;
    let omitted = [
        "pending-code-3nV8",
        "pending-host-7xJ2",
        "pending-args-1cQ5",
        "pending-notes-4fL9",
        "pending-replay-6bD1",
    ];
    let run = start_run(&h.sink, "pending", omitted[0], Some(json!(omitted[1])))?;
    let write = OperationId::new(run, 1);
    let read = OperationId::new(run, 2);
    h.sink.op_started(operation_started(
        write,
        "locator.fill",
        Effect::Write,
        json!(omitted[2]),
    ));
    h.sink.op_notes(write, vec![omitted[3].to_owned()]);
    h.sink.op_started(OperationStarted {
        parent: Some(write),
        ..operation_started(read, "page.textContent", Effect::Read, json!(omitted[2]))
    });
    h.sink.op_notes(read, vec![omitted[3].to_owned()]);
    set_values(&h.sink, Values::Off);
    h.sink.run_ended(RunEnded {
        status: RunStatus::Cancelled,
        replay: Some(json!(omitted[4])),
        ..run_ended(run)
    });
    h.sink.flush().await?;

    let detail = h.store.run(run).await?.ok_or("missing cancelled run")?;
    assert_eq!(detail.summary.values.as_deref(), Some("off"));
    assert_eq!(detail.summary.status, RunStatus::Cancelled);
    assert_eq!(detail.summary.effects, Some(RunEffects::Unknown));
    assert_eq!(detail.summary.operation_count, 2);
    assert_eq!(detail.summary.tabs, [TabId(7)]);
    assert!(detail.summary.complete);
    assert_eq!(detail.code, None);
    assert_eq!(detail.host_data, None);
    assert_eq!(detail.result, None);
    assert_eq!(h.store.replay(run, at(101)).await?, None);
    assert_eq!(detail.operations[0].delivery, Some(Delivery::Unknown));
    assert_eq!(detail.operations[1].delivery, Some(Delivery::NotSent));
    assert_eq!(detail.operations[1].parent, Some(write));
    for op in &detail.operations {
        assert_eq!(op.values.as_deref(), Some("off"));
        assert_eq!(op.outcome, Some(Outcome::Cancelled));
        assert_eq!(op.ended_at, Some(at(100)));
        assert_eq!(op.args, None);
        assert_eq!(op.result, None);
        assert!(op.notes.is_empty());
    }
    let files = h.raw_files()?;
    for canary in omitted {
        assert!(
            files.iter().all(|file| !contains(file, canary)),
            "a pending operation's input reached disk: {canary}"
        );
    }
    Ok(())
}
