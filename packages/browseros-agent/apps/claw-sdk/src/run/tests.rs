//! The run lifecycle end to end: the run service over the real dispatcher and runtime on the
//! fake browser, with the trace store read back. Covers `requestId` admission (replay, join,
//! duplicates, per-task scope, an unreadable store), orphan grace, teardown, the panic
//! boundary, masking and the task's page default across a restart.

use std::sync::Arc;
use std::time::Duration;
use std::{fs, io};

use browseros_core::PageId;
use browseros_core::sensitive_fields::{JournalEntry, SensitiveKind};
use browseros_playwright::{PlaywrightHooks, TypedBy};
use browseros_trace::{
    Delivery, EpochMillis, ErrorOwner, OperationId, Outcome, RequestId, RunEnded, RunId, RunQuery,
    RunStarted, RunStatus, RunSummary, SecretValue, TailQuery, TaskId, TraceRecordEvent,
    WriterConfig,
};
use serde_json::{Value, json};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::{task, time};

use super::request_ledger::{Admission, LedgerLimits};
use super::run_reply::{ReplyDelivery, ReplyEffects, ReplyStatus};
use super::{
    ReplyError, RequestLedger, RunHandle, RunMeta, RunReply, RunRequest, RunService, RunTimeout,
};
use crate::browser::PageRef;
use crate::config::Defaults;
use crate::dispatch::{ErrorCode, OperationError};
use crate::test_rig::{TestError, TestRig, on_neo_runtime, within};

/// Long enough that only the tests about orphans ever see it run out.
const GRACE: Duration = Duration::from_secs(30);

fn meta(request: Option<&str>, task: Option<&str>) -> Result<RunMeta, TestError> {
    Ok(RunMeta {
        request_id: request.map(RequestId::parse).transpose()?,
        task_id: task.map(TaskId::parse).transpose()?,
        host_data: None,
    })
}

fn request(code: &str, meta: RunMeta) -> Result<RunRequest, TestError> {
    Ok(RunRequest {
        code: code.to_owned(),
        timeout: RunTimeout::from_millis(None)?,
        meta,
    })
}

/// One keystroke as the seal's journal reports it: the field's whole value.
fn keystroke(value: &str) -> JournalEntry {
    JournalEntry {
        kind: SensitiveKind::Password,
        value: value.to_owned().into(),
    }
}

/// What the seal's journal reports as `by` types `value` into a field of `page`, a key at a
/// time: the field's whole value after each key.
fn type_into(rig: &TestRig, page: &PageRef, value: &str, by: TypedBy) {
    for end in 1..=value.len() {
        let typed = value.get(..end).unwrap_or(value);
        rig.hooks
            .field_entered(page.id.clone(), keystroke(typed), by);
    }
}

/// The read finding 5's runs made: the first open tab's title, returned.
const READ_TITLE: &str = "return await __neo.invoke('test.title', { page: context.pages()[0] });";

/// Fails when a host-facing read of run `run` holds `value`: the run with its op rows
/// (`GET /v1/runs/{id}`), each op (`GET /v1/operations/{op}`), the trace tail (`GET /v1/trace`),
/// the row each live event names (`/v1/events` builds its payload from it), and the trace file
/// with its WAL. `events` is subscribed before the run.
async fn assert_no_record_keeps(
    rig: &TestRig,
    run: RunId,
    value: &str,
    mut events: broadcast::Receiver<TraceRecordEvent>,
) -> Result<(), TestError> {
    rig.sink.flush().await?;
    let detail = rig.store.run(run).await?.ok_or("the run is recorded")?;
    assert!(
        !format!("{detail:?}").contains(value),
        "GET /v1/runs/{run} keeps `{value}`: {detail:?}"
    );
    for op in &detail.operations {
        let row = rig.store.operation(op.id).await?;
        assert!(
            !format!("{row:?}").contains(value),
            "GET /v1/operations/{} keeps `{value}`: {row:?}",
            op.id
        );
    }
    let tail = rig.store.trace_tail(TailQuery::default()).await?;
    assert!(
        !format!("{tail:?}").contains(value),
        "GET /v1/trace keeps `{value}`: {tail:?}"
    );
    let mut published = 0;
    loop {
        let row = match events.try_recv() {
            Ok(
                TraceRecordEvent::OperationStarted { id, .. }
                | TraceRecordEvent::OperationEnded { id, .. },
            ) => format!("{:?}", rig.store.operation(id).await?),
            Ok(
                TraceRecordEvent::RunStarted { run, .. } | TraceRecordEvent::RunEnded { run, .. },
            ) => {
                format!("{:?}", rig.store.run_summary(run).await?)
            }
            Ok(TraceRecordEvent::TabPrivate { .. } | TraceRecordEvent::RunComplete { .. }) => {
                continue;
            }
            Err(TryRecvError::Empty) => break,
            Err(error) => return Err(error.into()),
        };
        published += 1;
        assert!(
            !row.contains(value),
            "a live event's payload keeps `{value}`: {row}"
        );
    }
    assert!(published > 0, "the run's records were published live");
    let db = rig.dir.path().join("trace.db");
    let mut stored = fs::read(&db)?;
    match fs::read(db.with_extension("db-wal")) {
        Ok(wal) => stored.extend(wal),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    assert!(
        !stored
            .windows(value.len())
            .any(|bytes| bytes == value.as_bytes()),
        "trace.db or its WAL keeps `{value}`"
    );
    Ok(())
}

/// `text` as the process's secret set masks it now.
fn masked(rig: &TestRig, text: &str) -> String {
    let mut text = text.to_owned();
    rig.secrets.mask_str(&mut text);
    text
}

fn error_of(reply: &RunReply) -> Result<&ReplyError, TestError> {
    reply
        .error
        .as_ref()
        .ok_or_else(|| format!("expected an error, got {reply:?}").into())
}

/// The runs recorded for `request`, once everything sent so far is committed.
async fn runs_of(rig: &TestRig, request: &str) -> Result<Vec<RunSummary>, TestError> {
    rig.sink.flush().await?;
    let query = RunQuery {
        request_id: Some(RequestId::parse(request)?),
        ..RunQuery::default()
    };
    Ok(rig.store.runs(query).await?.items)
}

/// Waits until the run of task `task` has the status `status` in the trace.
async fn until_status(
    rig: &TestRig,
    task: &str,
    status: RunStatus,
) -> Result<RunSummary, TestError> {
    let query = RunQuery {
        task_id: Some(TaskId::parse(task)?),
        ..RunQuery::default()
    };
    within(Duration::from_secs(10), async {
        loop {
            rig.sink.flush().await?;
            if let Some(run) = rig
                .store
                .runs(query.clone())
                .await?
                .items
                .into_iter()
                .next()
                && run.status == status
            {
                return Ok::<_, TestError>(run);
            }
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?
}

/// Runs `sql` on the rig's trace file over a connection of its own.
async fn execute_sql(rig: &TestRig, sql: &'static str) -> Result<(), TestError> {
    let path = rig.dir.path().join("trace.db");
    let runtime = Handle::current();
    // On a blocking thread, as the rig opens the store: sqlx's futures defeat the `Send` proof
    // inside a spawned test body.
    task::spawn_blocking(move || {
        runtime.block_on(async {
            let pool =
                SqlitePool::connect_with(SqliteConnectOptions::new().filename(&path)).await?;
            sqlx::query(sql).execute(&pool).await?;
            pool.close().await;
            Ok::<_, sqlx::Error>(())
        })
    })
    .await??;
    Ok(())
}

/// Makes every commit that carries the start record of a run whose request id begins with
/// `lost-` fail, as a full disk would: the writer rolls the batch back and its records are lost.
async fn lose_lost_starts(rig: &TestRig) -> Result<(), TestError> {
    execute_sql(
        rig,
        "CREATE TRIGGER lose_start BEFORE INSERT ON runs WHEN NEW.request_id LIKE 'lost-%' \
         BEGIN SELECT RAISE(ABORT, 'the disk is full'); END",
    )
    .await
}

/// A replay window the ledger tests can wait out.
const SHORT_WINDOW: Duration = Duration::from_millis(200);

/// Admits one call of `request` on `ledger` alone, as the run service would.
async fn admit(ledger: &Arc<RequestLedger>, request: &str) -> Result<Admission, TestError> {
    let handle = RunHandle::new(RunId::new(), Duration::ZERO);
    Ok(ledger.admit(&meta(Some(request), None)?, &handle).await)
}

/// Admits `request` as a new run and ends it the way the run service does: start and end
/// records to the writer, then the reply through the ticket.
async fn run_once(
    rig: &TestRig,
    ledger: &Arc<RequestLedger>,
    request: &str,
) -> Result<Arc<RunReply>, TestError> {
    let handle = RunHandle::new(RunId::new(), Duration::ZERO);
    let request_id = RequestId::parse(request)?;
    let Admission::Fresh(ticket) = ledger.admit(&meta(Some(request), None)?, &handle).await else {
        return Err(format!("{request} is new, so it runs").into());
    };
    rig.sink.run_started(RunStarted {
        id: handle.id(),
        code: "return 1;".to_owned(),
        request_id: Some(request_id.clone()),
        task_id: None,
        host_data: None,
        also_mask: None,
        at: EpochMillis::now(),
    });
    let reply = Arc::new(RunReply {
        id: handle.id(),
        status: ReplyStatus::Ok,
        effects: ReplyEffects::None,
        value: json!(1),
        error: None,
        logs: Vec::new(),
        logs_dropped: 0,
        unknown: Vec::new(),
        not_awaited: Vec::new(),
    });
    let end = reply.trace_end(Some(&request_id));
    rig.sink.run_ended(RunEnded {
        id: handle.id(),
        status: end.status,
        error: end.error,
        result: end.result,
        replay: end.replay,
        also_mask: None,
        at: EpochMillis::now(),
    });
    ticket.finish(Arc::clone(&reply));
    Ok(reply)
}

/// Starts `request` as a caller that can be made to leave, as a dropped connection would.
fn caller(service: &Arc<RunService>, request: RunRequest) -> task::JoinHandle<Arc<RunReply>> {
    let service = Arc::clone(service);
    tokio::spawn(async move { service.run(request).await })
}

#[test]
fn a_repeated_request_id_replays_the_first_reply_and_never_runs_twice() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let code = "return Math.random();";
        let first = service
            .run(request(code, meta(Some("call-1"), None)?)?)
            .await;
        assert_eq!(first.status, ReplyStatus::Ok);
        let repeat = service
            .run(request(code, meta(Some("call-1"), None)?)?)
            .await;
        assert_eq!(
            *repeat, *first,
            "the repeat is the first reply, id and value included"
        );

        // A restart follows SIGTERM, which flushes the writer.
        rig.sink.flush().await?;
        let restarted = rig.restarted_service(GRACE)?;
        let after_restart = restarted
            .run(request(code, meta(Some("call-1"), None)?)?)
            .await;
        assert_eq!(
            *after_restart, *first,
            "after a restart the store replays it, byte for byte"
        );
        assert_eq!(runs_of(&rig, "call-1").await?.len(), 1, "it ran once");
        Ok(())
    })
}

#[test]
fn a_repeat_while_running_joins_the_run() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let code =
            "await new Promise((resolve) => setTimeout(resolve, 300)); return Math.random();";
        let first = caller(&service, request(code, meta(Some("call-2"), None)?)?);
        time::sleep(Duration::from_millis(50)).await;
        let repeat = service
            .run(request(code, meta(Some("call-2"), None)?)?)
            .await;
        let first = first.await?;
        assert_eq!(
            *repeat, *first,
            "the repeat waited for the first run's reply"
        );
        assert_eq!(runs_of(&rig, "call-2").await?.len(), 1, "it ran once");
        Ok(())
    })
}

#[test]
fn after_the_replay_window_a_repeat_is_a_duplicate_naming_the_first_run() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::with_writer(WriterConfig {
            replay_window: Duration::ZERO,
            ..WriterConfig::default()
        })
        .await?;
        let first = rig
            .run_service(GRACE)
            .run(request("return 1;", meta(Some("call-3"), None)?)?)
            .await;
        rig.sink.flush().await?;
        let repeat = rig
            .restarted_service(GRACE)?
            .run(request("return 1;", meta(Some("call-3"), None)?)?)
            .await;
        assert_ne!(
            repeat.id, first.id,
            "the refusal is this call's own, so it never reads as the first run's reply"
        );
        assert_eq!(repeat.effects, ReplyEffects::None, "this call sent nothing");
        let error = error_of(&repeat)?;
        assert_eq!(
            (error.owner, error.code.as_str(), error.delivery),
            (
                ErrorOwner::Host,
                "duplicate_request",
                Some(ReplyDelivery::NotSent)
            )
        );
        assert!(
            error.message.contains(&first.id.to_string()),
            "it names the run that already happened: {}",
            error.message
        );
        assert!(
            !error.retryable,
            "the first run happened; repeating it is not safe"
        );
        assert_eq!(
            runs_of(&rig, "call-3").await?.len(),
            1,
            "the refusal is on the record without the request id, which stays the first run's"
        );
        Ok(())
    })
}

#[test]
fn replay_is_scoped_by_task_when_the_call_has_one() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let code = "return Math.random();";
        let first = service
            .run(request(code, meta(Some("call-4"), Some("task-a"))?)?)
            .await;
        let other_task = service
            .run(request(code, meta(Some("call-4"), Some("task-b"))?)?)
            .await;
        assert_ne!(
            other_task.id, first.id,
            "another task's request id is its own"
        );
        let any_task = service
            .run(request(code, meta(Some("call-4"), None)?)?)
            .await;
        assert_eq!(
            any_task.id, first.id,
            "without a task the earliest run with the id answers"
        );
        assert_eq!(runs_of(&rig, "call-4").await?.len(), 2);
        Ok(())
    })
}

#[test]
fn returning_cuts_pending_ops_and_the_reply_says_which() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        // The pause lets the hang's task start and send before the script returns.
        let code = "__neo.invoke('test.hang', {});\nawait new Promise((resolve) => setTimeout(resolve, 50));\nreturn 1;";
        let reply = rig
            .run_service(GRACE)
            .run(request(code, meta(None, None)?)?)
            .await;
        assert_eq!(reply.status, ReplyStatus::Ok);
        assert_eq!(reply.value, json!(1));
        let cut = reply.not_awaited.first().ok_or("the hang was cut short")?;
        assert_eq!(cut.name, "test.hang");
        assert!(
            cut.site
                .as_deref()
                .is_some_and(|site| site.starts_with("run:1:")),
            "named with the line that started it: {:?}",
            cut.site
        );
        assert_eq!(
            reply.effects,
            ReplyEffects::Unknown,
            "the cut write was sent, so the run may have changed the page"
        );
        assert_eq!(
            reply.unknown.first().map(|note| note.operation_id),
            Some(cut.operation_id)
        );
        Ok(())
    })
}

#[test]
fn a_pending_event_pull_ends_quietly_when_the_script_returns() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        // Started and never awaited, as the prelude's event pump is once a listener exists. The
        // pause lets its task start before the script returns.
        let code = "__neo.invoke('test.pull', {});\nawait new Promise((resolve) => setTimeout(resolve, 50));\nreturn 1;";
        let reply = rig
            .run_service(GRACE)
            .run(request(code, meta(None, None)?)?)
            .await;
        assert_eq!((reply.status, &reply.value), (ReplyStatus::Ok, &json!(1)));
        assert!(
            reply.not_awaited.is_empty(),
            "the run's end is how a pull ends, so the reply names no op the script left behind: {:?}",
            reply.not_awaited
        );
        assert_eq!(reply.effects, ReplyEffects::None, "a pull changes nothing");
        rig.sink.flush().await?;
        let pull = rig
            .store
            .operation(OperationId::new(reply.id, 1))
            .await?
            .ok_or("the pull is the run's first op, on the record")?;
        assert_eq!(pull.name, "test.pull");
        assert_eq!(
            (pull.outcome, pull.delivery, pull.error),
            (Some(Outcome::Cancelled), Some(Delivery::NotSent), None),
            "cancelled and never sent, with no error on the record: nothing failed"
        );
        Ok(())
    })
}

#[test]
fn a_caller_leaving_stops_a_run_without_a_request_id_at_once() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let code = "await new Promise((resolve) => setTimeout(resolve, 600000));";
        let leaving = caller(&service, request(code, meta(None, Some("task-left"))?)?);
        until_status(&rig, "task-left", RunStatus::Running).await?;
        leaving.abort();
        let ended = until_status(&rig, "task-left", RunStatus::Cancelled).await?;
        assert_eq!(
            ended.error.map(|error| (error.owner, error.code)),
            Some((ErrorOwner::Host, "cancelled".to_owned())),
            "nobody can collect a run without a requestId, so it stops at once"
        );
        Ok(())
    })
}

#[test]
fn a_request_id_run_outlives_its_caller_and_a_retry_joins_it() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let code = "await new Promise((resolve) => setTimeout(resolve, 500)); return 42;";
        let leaving = caller(
            &service,
            request(code, meta(Some("call-5"), Some("task-5"))?)?,
        );
        until_status(&rig, "task-5", RunStatus::Running).await?;
        leaving.abort();
        let retry = service
            .run(request(code, meta(Some("call-5"), Some("task-5"))?)?)
            .await;
        assert_eq!(
            retry.status,
            ReplyStatus::Ok,
            "the transport blip cost nothing"
        );
        assert_eq!(retry.value, json!(42));
        assert_eq!(
            runs_of(&rig, "call-5").await?.len(),
            1,
            "the retry joined; it did not run again"
        );
        Ok(())
    })
}

#[test]
fn an_orphaned_run_stops_after_the_grace_and_a_retry_replays_its_end() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let grace = Duration::from_millis(300);
        let service = rig.run_service(grace);
        let code = "await new Promise((resolve) => setTimeout(resolve, 600000));";
        let leaving = caller(
            &service,
            request(code, meta(Some("call-6"), Some("task-6"))?)?,
        );
        until_status(&rig, "task-6", RunStatus::Running).await?;
        leaving.abort();
        let ended = until_status(&rig, "task-6", RunStatus::Cancelled).await?;
        let retry = service
            .run(request(code, meta(Some("call-6"), Some("task-6"))?)?)
            .await;
        assert_eq!(
            retry.id, ended.id,
            "a retry after the stop gets the stopped run, not a new one"
        );
        assert_eq!(retry.status, ReplyStatus::Cancelled);
        Ok(())
    })
}

#[test]
fn an_unreadable_trace_refuses_a_request_id_rather_than_risk_a_second_run() -> Result<(), TestError>
{
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        execute_sql(&rig, "DROP TABLE runs").await?;
        let reply = rig
            .run_service(GRACE)
            .run(request("return 1;", meta(Some("call-7"), None)?)?)
            .await;
        let error = error_of(&reply)?;
        assert_eq!(
            (error.owner, error.code.as_str()),
            (ErrorOwner::Neo, "trace_unavailable")
        );
        assert!(
            error.retryable,
            "nothing ran, so the host may retry once the store is back"
        );
        Ok(())
    })
}

#[test]
fn a_request_the_trace_lost_is_refused_after_the_window_never_run_again() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        lose_lost_starts(&rig).await?;
        let limits = LedgerLimits {
            replay_window: SHORT_WINDOW,
            ..LedgerLimits::default()
        };
        let ledger = RequestLedger::new(rig.store.clone(), rig.sink.clone(), limits);
        let first = run_once(&rig, &ledger, "lost-1").await?;
        // Whichever flush runs first is told of the loss; the store shows what landed.
        let _lost = rig.sink.flush().await;
        assert_eq!(
            rig.store.run(first.id).await?.map(|run| run.summary.id),
            None,
            "the writer lost the run's records"
        );
        match admit(&ledger, "lost-1").await? {
            Admission::Replay(reply) => assert_eq!(reply.id, first.id),
            _ => return Err("within the window a repeat replays the first reply".into()),
        }
        time::sleep(SHORT_WINDOW * 2).await;
        match admit(&ledger, "lost-1").await? {
            Admission::Duplicate(run) => assert_eq!(run, first.id, "the refusal names the run"),
            _ => {
                return Err(
                    "memory is the only proof the request ran, so a late repeat is refused".into(),
                );
            }
        }
        Ok(())
    })
}

#[test]
fn once_too_many_requests_are_unrecorded_a_new_request_id_is_refused() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        lose_lost_starts(&rig).await?;
        let limits = LedgerLimits {
            replay_window: SHORT_WINDOW,
            max_unrecorded: 1,
        };
        let ledger = RequestLedger::new(rig.store.clone(), rig.sink.clone(), limits);
        run_once(&rig, &ledger, "kept-1").await?;
        time::sleep(SHORT_WINDOW * 2).await;
        run_once(&rig, &ledger, "kept-2").await.map_err(|error| {
            format!("a request the trace stored leaves memory and holds no place: {error}")
        })?;
        run_once(&rig, &ledger, "lost-2").await?;
        time::sleep(SHORT_WINDOW * 2).await;
        match admit(&ledger, "fresh-1").await? {
            Admission::Unavailable(error) => {
                assert_eq!(error.code(), ErrorCode::TRACE_UNAVAILABLE);
            }
            _ => {
                return Err(
                    "memory is full of unrecorded requests, so a new one is refused".into(),
                );
            }
        }
        assert!(
            matches!(admit(&ledger, "lost-2").await?, Admission::Duplicate(_)),
            "the unrecorded request is still refused"
        );
        Ok(())
    })
}

#[test]
fn a_repeat_that_races_the_first_runs_commit_joins_it() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let ledger =
            RequestLedger::new(rig.store.clone(), rig.sink.clone(), LedgerLimits::default());
        // The repeat misses memory and is held before its store lookup; meanwhile the first
        // call is admitted and its running row commits.
        let (held, resume) = ledger.pause_next_store_lookup();
        let racing = Arc::clone(&ledger);
        let repeat = tokio::spawn(async move { admit(&racing, "race-1").await });
        held.await?;
        let handle = RunHandle::new(RunId::new(), Duration::ZERO);
        let Admission::Fresh(ticket) = ledger.admit(&meta(Some("race-1"), None)?, &handle).await
        else {
            return Err("the first call to reach the ledger's claim runs".into());
        };
        rig.sink.run_started(RunStarted {
            id: handle.id(),
            code: "return 1;".to_owned(),
            request_id: Some(RequestId::parse("race-1")?),
            task_id: None,
            host_data: None,
            also_mask: None,
            at: EpochMillis::now(),
        });
        rig.sink.flush().await?;
        resume.send(()).map_err(|()| "the repeat is held")?;
        let Admission::Join(waiter) = repeat.await?? else {
            return Err("while the first run is running, the repeat waits for it".into());
        };
        ticket.finish(Arc::new(RunReply::crashed(handle.id())));
        assert_eq!(
            waiter.reply().await.id,
            handle.id(),
            "the repeat gets the first run's reply"
        );
        Ok(())
    })
}

#[test]
fn a_panicking_run_task_still_answers_records_its_end_and_the_next_run_works()
-> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        service.crash_next_run();
        let crashed = service
            .run(request("return 1;", meta(Some("call-8"), None)?)?)
            .await;
        let error = error_of(&crashed)?;
        assert_eq!(
            (error.owner, error.code.as_str()),
            (ErrorOwner::Neo, "internal_error")
        );
        assert_eq!(
            crashed.effects,
            ReplyEffects::Unknown,
            "its ops may have sent anything"
        );
        let recorded = runs_of(&rig, "call-8").await?;
        assert_eq!(
            recorded.first().map(|run| (run.id, run.status)),
            Some((crashed.id, RunStatus::Error)),
            "the guard recorded the run's end while the task unwound"
        );
        let repeat = service
            .run(request("return 1;", meta(Some("call-8"), None)?)?)
            .await;
        assert_eq!(
            *repeat, *crashed,
            "a crashed request answers with its crash, never a new run"
        );
        let next = service
            .run(request("return 2;", meta(Some("call-9"), None)?)?)
            .await;
        assert_eq!((next.status, &next.value), (ReplyStatus::Ok, &json!(2)));
        Ok(())
    })
}

#[test]
fn a_new_task_gets_a_fresh_tab_and_a_known_task_keeps_its_tab_across_a_restart()
-> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let owner = rig.owner_tab("https://owner.example/").await?;
        // Two ops: the first one on a fresh page opens it, so only later ones record its tab
        // when they start, and the trace's `task_last_tab` reads the start records.
        let touch = "await __neo.invoke('test.touch', { page });\nreturn (await __neo.invoke('test.touch', { page })).tab;";
        let first = rig
            .run_service(GRACE)
            .run(request(touch, meta(None, Some("task-t"))?)?)
            .await;
        let tab = first.value.as_i64().ok_or("a tab id")?;
        assert_ne!(
            tab, owner.tab.0,
            "a new task never inherits a tab it did not use"
        );
        rig.sink.flush().await?;

        let restarted = rig.restarted_service(GRACE)?;
        let page_tab = "return page.toJSON().tab;";
        let again = restarted
            .run(request(page_tab, meta(None, Some("task-t"))?)?)
            .await;
        assert_eq!(
            again.value,
            json!(tab),
            "the trace remembers the task's tab across a restart"
        );
        let other = restarted
            .run(request(
                "return page.toJSON().id;",
                meta(None, Some("task-u"))?,
            )?)
            .await;
        assert_eq!(
            other.value,
            Value::Null,
            "another new task starts with no tab yet"
        );
        Ok(())
    })
}

#[test]
fn context_pages_lists_a_tab_opened_before_the_run_in_its_first_statement() -> Result<(), TestError>
{
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let _registration = rig
            .secrets
            .register(SecretValue::new("hunter22".to_owned()), None);
        let owner = rig
            .owner_tab("https://owner.example/?code=hunter22")
            .await?;
        // Reversed, so the reply's own mask could not hide a secret the script was handed.
        let code = "return context.pages().map((tab) => ({ brand: tab.toJSON(), url: [...tab.url()].reverse().join('') }));";
        let reply = rig
            .run_service(GRACE)
            .run(request(code, meta(None, None)?)?)
            .await;
        assert_eq!(reply.status, ReplyStatus::Ok, "{reply:?}");
        let masked: String = "https://owner.example/?code=[private]"
            .chars()
            .rev()
            .collect();
        assert_eq!(
            reply.value,
            json!([{
                "brand": { "$neo": "page", "id": owner.id.0, "tab": owner.tab.0 },
                "url": masked,
            }]),
            "every open tab before any op, its URL masked before the script sees it"
        );
        Ok(())
    })
}

#[test]
fn without_the_browser_a_call_is_refused_and_its_request_id_stays_free() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        rig.fake.drop_connection();
        let refused = service
            .run(request("return 1;", meta(Some("call-10"), None)?)?)
            .await;
        let error = error_of(&refused)?;
        assert_eq!(
            (error.owner, error.code.as_str()),
            (ErrorOwner::Neo, "browser_unavailable")
        );
        rig.fake.reconnect();
        let retry = service
            .run(request("return 1;", meta(Some("call-10"), None)?)?)
            .await;
        assert_eq!(
            retry.status,
            ReplyStatus::Ok,
            "nothing ran, so the retry runs"
        );
        assert_ne!(retry.id, refused.id);
        Ok(())
    })
}

#[test]
fn known_secrets_are_masked_in_the_reply() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let _registration = rig
            .secrets
            .register(SecretValue::new("hunter22".to_owned()), None);
        let code = "console.log('typed hunter22'); return { said: 'hunter22' };";
        let reply = rig
            .run_service(GRACE)
            .run(request(code, meta(None, None)?)?)
            .await;
        assert_eq!(reply.value, json!({ "said": "[private]" }));
        assert_eq!(
            reply.logs.first().map(|line| line.text.as_str()),
            Some("typed [private]")
        );
        let thrown = rig
            .run_service(GRACE)
            .run(request("throw new Error('hunter22');", meta(None, None)?)?)
            .await;
        assert!(
            error_of(&thrown)?.message.contains("[private]"),
            "and in the error"
        );
        Ok(())
    })
}

#[test]
fn a_value_typed_during_the_run_is_masked_in_its_reply_before_its_field_goes_quiet()
-> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        for value in ["hunt", "hunte", "hunter", "hunter2"] {
            rig.typed.note(PageId(1), None, keystroke(value));
        }
        let code = "return 'hunte or hunter2?';";
        let reply = rig
            .run_service(GRACE)
            .run(request(code, meta(None, None)?)?)
            .await;
        assert_eq!(
            reply.value,
            json!("hunte or [private]?"),
            "the reply is masked with the field's value before it goes quiet, and never its prefix"
        );
        Ok(())
    })
}

#[test]
fn a_run_that_ends_while_another_field_is_typed_registers_none_of_it() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        // The owner is typing a password into another tab while an unrelated run goes on.
        for value in ["hunt", "hunte"] {
            rig.typed.note(PageId(1), None, keystroke(value));
        }
        let reply = rig
            .run_service(GRACE)
            .run(request("return 'it shows hunte';", meta(None, None)?)?)
            .await;
        assert_eq!(
            reply.value,
            json!("it shows [private]"),
            "the reply is masked with what is typed so far"
        );
        assert_eq!(
            masked(&rig, "hunte"),
            "hunte",
            "the run's end registered nothing: `hunte` may be a prefix"
        );
        for value in ["hunter", "hunter2"] {
            rig.typed.note(PageId(1), None, keystroke(value));
        }
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(250)).await;
        assert_eq!(
            masked(&rig, "hunte or hunter2"),
            "hunte or [private]",
            "the field went quiet: its final value is the secret, and its prefix never was"
        );
        Ok(())
    })
}

#[test]
fn a_prefix_read_mid_typing_stays_masked_in_the_reply_after_a_later_key_supersedes_it()
-> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        // The page echoes its password field into its title. The owner has typed `hunte` so far.
        let page = rig
            .owner_tab_titled("https://b.test/journal", "hunte")
            .await?;
        type_into(&rig, &page, "hunte", TypedBy::NotAgent);
        // The script reads the echo, then meets the test at the gate twice: once so the test
        // knows the read is done, and once after the test typed on.
        let code = "const echo = await __neo.invoke('test.title', { page: context.pages()[0] });\n\
                    await __neo.invoke('test.gate', {});\n\
                    await __neo.invoke('test.gate', {});\n\
                    return echo;";
        let events = rig.sink.subscribe();
        let run = caller(&rig.run_service(GRACE), request(code, meta(None, None)?)?);
        within(Duration::from_secs(10), rig.gate.wait()).await?;
        for value in ["hunter", "hunter2"] {
            rig.hooks
                .field_entered(page.id.clone(), keystroke(value), TypedBy::NotAgent);
        }
        within(Duration::from_secs(10), rig.gate.wait()).await?;
        let reply = run.await?;
        assert_eq!(
            reply.value,
            json!("[private]"),
            "`hunte` was held while the run read it, though `hunter2` superseded it by the end"
        );
        assert_no_record_keeps(&rig, reply.id, "hunte", events).await?;
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(250)).await;
        assert_eq!(
            masked(&rig, "hunte or hunter2"),
            "hunte or [private]",
            "the field went quiet: its final value is the secret, and the prefix the run masked never was"
        );
        Ok(())
    })
}

/// Finding 5, its first shape: a value still in its quiet period, which the page echoes and the
/// next run reads back. The agent's own `keyboard.type` (`keys`), the owner on a raw control
/// display (`control`) and the page's own input events (`page_echoes`, journaled) all reach the
/// journal the same way.
#[test]
fn an_echo_of_a_value_still_in_its_quiet_period_is_masked_in_every_record_as_in_the_reply()
-> Result<(), TestError> {
    on_neo_runtime(async {
        for (typist, value, by) in [
            ("the agent", "agent-typed-7Qx", TypedBy::Agent),
            ("the owner", "owner-typed-7Qx", TypedBy::NotAgent),
            ("the page", "page-typed-7Qx", TypedBy::NotAgent),
        ] {
            let rig = TestRig::new().await?;
            let page = rig
                .owner_tab_titled("https://b.test/typed", &format!("typed: {value}"))
                .await?;
            type_into(&rig, &page, value, by);
            let events = rig.sink.subscribe();
            let reply = rig
                .run_service(GRACE)
                .run(request(READ_TITLE, meta(None, None)?)?)
                .await;
            assert_eq!(
                reply.value,
                json!("typed: [private]"),
                "{typist}: the reply is masked with the value still held"
            );
            assert_eq!(
                masked(&rig, value),
                value,
                "{typist}: the value is still in its quiet period, and no record registered it"
            );
            assert_no_record_keeps(&rig, reply.id, value, events).await?;
        }
        Ok(())
    })
}

/// Finding 5, its second shape: `page.title()` on a page whose title echoes its password field.
/// The read's own sweep holds the password and masks it out of the read's result, so the reply
/// is masked; the op's page fact, which the door reads after the op, is a record of it too.
#[test]
fn a_page_fact_that_echoes_what_the_reads_own_sweep_found_is_masked_in_every_record()
-> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let value = "swept-pw-7Qx";
        let page = rig
            .owner_tab_titled("https://a.test/login", &format!("Welcome {value}"))
            .await?;
        rig.fields.fill(&page, value);
        let events = rig.sink.subscribe();
        let reply = rig
            .run_service(GRACE)
            .run(request(READ_TITLE, meta(None, None)?)?)
            .await;
        assert_eq!(
            reply.value,
            json!("Welcome [private]"),
            "the read masked what it swept out of its own result"
        );
        assert_eq!(
            masked(&rig, value),
            value,
            "the sweep holds the value, and no record registered it"
        );
        assert_no_record_keeps(&rig, reply.id, value, events).await?;
        Ok(())
    })
}

/// Each start record is masked with what is held as it is sent: here a value the owner typed a
/// moment before, in the run's host data and in the input of an op still running. Until that op
/// ends, its start row is all a host reads of it.
#[test]
fn a_value_held_as_a_run_starts_is_in_none_of_its_start_records() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let value = "owner-typed-7Qx";
        let page = rig
            .owner_tab_titled("https://b.test/typed", &format!("typed: {value}"))
            .await?;
        type_into(&rig, &page, value, TypedBy::NotAgent);
        let events = rig.sink.subscribe();
        let code = "const title = await __neo.invoke('test.title', { page: context.pages()[0] });\n\
                    await __neo.invoke('test.gate', { title });\n\
                    return 1;";
        let meta = RunMeta::parse(Some(&json!({ "hostData": { "echo": value } })))
            .map_err(|error| error.to_string())?;
        let run = caller(&rig.run_service(GRACE), request(code, meta)?);
        let running = within(Duration::from_secs(10), async {
            loop {
                rig.sink.flush().await?;
                let tail = rig.store.trace_tail(TailQuery::default()).await?;
                if let Some(op) = tail
                    .operations
                    .into_iter()
                    .find(|op| op.name == "test.gate")
                {
                    return Ok::<_, TestError>(op);
                }
                time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await??;
        assert_eq!(running.outcome, None, "the op waits at the gate");
        assert!(
            !format!("{running:?}").contains(value),
            "a running op's start row keeps `{value}`: {running:?}"
        );
        within(Duration::from_secs(10), rig.gate.wait()).await?;
        let reply = run.await?;
        assert_eq!(reply.status, ReplyStatus::Ok, "{reply:?}");
        assert_no_record_keeps(&rig, reply.id, value, events).await?;
        Ok(())
    })
}

/// The end record is masked with every value held since the run began, as the reply is: here a
/// password the run's own read swept, which its code names too. Nothing held it as the run
/// started, and a run's code reaches the store only with its end, so only the end's set can mask
/// it.
#[test]
fn a_value_a_runs_own_read_swept_is_masked_in_its_stored_code() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let value = "swept-pw-7Qx";
        let page = rig
            .owner_tab_titled("https://a.test/login", "Sign in")
            .await?;
        rig.fields.fill(&page, value);
        let events = rig.sink.subscribe();
        let code = format!(
            "await __neo.invoke('test.title', {{ page: context.pages()[0] }});\nreturn '{value}'.length;"
        );
        let reply = rig
            .run_service(GRACE)
            .run(request(&code, meta(None, None)?)?)
            .await;
        assert_eq!(reply.value, json!(value.len()), "{reply:?}");
        assert_no_record_keeps(&rig, reply.id, value, events).await?;
        Ok(())
    })
}

/// Past `MAX_SUPERSEDED`, a value a running run read is forgotten, so nothing can mask it out of
/// what the run sends from then on. That output is withheld, the reply's value and the run's end
/// record included, and the value is never registered: it may be a prefix.
#[test]
fn a_run_that_loses_a_value_it_read_past_the_cap_withholds_its_output() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        // The page echoes its password field into its title. The owner has typed `hunte` so far.
        let page = rig
            .owner_tab_titled("https://b.test/journal", "hunte")
            .await?;
        type_into(&rig, &page, "hunte", TypedBy::NotAgent);
        let code = "const echo = await __neo.invoke('test.title', { page: context.pages()[0] });\n\
                    await __neo.invoke('test.gate', {});\n\
                    await __neo.invoke('test.gate', {});\n\
                    return echo;";
        let events = rig.sink.subscribe();
        let run = caller(&rig.run_service(GRACE), request(code, meta(None, None)?)?);
        within(Duration::from_secs(10), rig.gate.wait()).await?;
        // The owner types on, which supersedes `hunte`. Then another page raises input events on
        // its own field, each one superseding the last, until `hunte` is the one past the cap.
        rig.hooks
            .field_entered(page.id.clone(), keystroke("hunter"), TypedBy::NotAgent);
        for pair in 0..=Defaults::MAX_SUPERSEDED {
            for value in [format!("v{pair:03}a"), format!("v{pair:03}ab")] {
                rig.typed.note(PageId(999), None, keystroke(&value));
            }
        }
        within(Duration::from_secs(10), rig.gate.wait()).await?;
        let reply = run.await?;
        let shown = serde_json::to_string(&*reply)?;
        assert!(!shown.contains("hunte"), "the reply keeps `hunte`: {shown}");
        let error = error_of(&reply)?;
        assert_eq!(
            (error.owner, error.code.as_str(), &reply.value),
            (ErrorOwner::Neo, "output_withheld", &Value::Null),
            "the reply says why its value is withheld: {shown}"
        );
        assert_no_record_keeps(&rig, reply.id, "hunte", events).await?;
        assert_eq!(
            masked(&rig, "hunte"),
            "hunte",
            "a value forgotten past the cap is never registered: it may be a prefix"
        );
        Ok(())
    })
}

/// Two password fields on one page, the second's value a prefix of the first's. The first is
/// held, so the read's sweep leaves the second out of what it holds: it cannot tell it from a
/// prefix the first field was typed past. The run's reply and records are masked with it all the
/// same, and it is never registered.
#[test]
fn a_swept_value_a_held_one_extends_masks_the_run_and_is_never_registered() -> Result<(), TestError>
{
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let (first, second) = ("separatePWX", "separatePW");
        // The page echoes its second field into its URL and its title.
        let page = rig
            .owner_tab_titled(
                &format!("https://a.test/login?echo={second}"),
                &format!("Welcome {second}"),
            )
            .await?;
        type_into(&rig, &page, first, TypedBy::NotAgent);
        rig.fields.fill(&page, first);
        rig.fields.fill(&page, second);
        let events = rig.sink.subscribe();
        let code = "await __neo.invoke('test.title', { page: context.pages()[0] });\n\
                    return context.pages()[0].url();";
        let reply = rig
            .run_service(GRACE)
            .run(request(code, meta(None, None)?)?)
            .await;
        assert_eq!(
            reply.value,
            json!("https://a.test/login?echo=[private]"),
            "the page fact the script returned is masked with what the read swept: {reply:?}"
        );
        assert_no_record_keeps(&rig, reply.id, second, events).await?;
        time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(250)).await;
        assert_eq!(
            masked(&rig, &format!("{second} or {first}")),
            format!("{second} or [private]"),
            "only the value the first field was typed to is registered: the second may be its prefix"
        );
        Ok(())
    })
}

/// The trace keeps an op's name as the door hands it over, and a script can call any name. One
/// named after what it read, a value still held, is masked in the op's record as in the reply.
/// So is one where that value overlaps a secret the process knows: masked with one set after the
/// other, it would keep the held value's tail.
#[test]
fn an_unknown_methods_name_is_masked_in_its_record_with_what_its_run_holds() -> Result<(), TestError>
{
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let value = "owner-typed-7Qx";
        let page = rig.owner_tab_titled("https://b.test/typed", value).await?;
        type_into(&rig, &page, value, TypedBy::NotAgent);
        let _registration = rig
            .secrets
            .register(SecretValue::new("known-owner".to_owned()), None);
        let events = rig.sink.subscribe();
        let code = "const title = await __neo.invoke('test.title', { page: context.pages()[0] });\n\
                    const said = [];\n\
                    for (const name of [title, 'known-' + title]) {\n\
                      try { await __neo.invoke(name, {}); } catch (error) { said.push(error.message); }\n\
                    }\n\
                    return said;";
        let reply = rig
            .run_service(GRACE)
            .run(request(code, meta(None, None)?)?)
            .await;
        let said = serde_json::to_string(&reply.value)?;
        assert!(
            said.matches("is not a Neo method").count() == 2 && !said.contains("typed-7Qx"),
            "the reply is masked with the value still held: {reply:?}"
        );
        assert_no_record_keeps(&rig, reply.id, value, events).await?;
        let detail = rig
            .store
            .run(reply.id)
            .await?
            .ok_or("the run is recorded")?;
        let names: Vec<&str> = detail
            .operations
            .iter()
            .map(|op| op.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["test.title", "[private]", "[withheld]"],
            "a name the held value overlaps a known secret in is masked whole, never in part"
        );
        Ok(())
    })
}

#[test]
fn a_secret_across_the_console_limit_is_masked_before_the_cut() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let _registration = rig
            .secrets
            .register(SecretValue::new("hunter22".to_owned()), None);
        // One byte over the limit, so the cut falls inside the secret.
        let code = format!(
            "console.log('x'.repeat({}) + 'hunter22');",
            Defaults::MAX_LOG_BYTES - 7
        );
        let reply = rig
            .run_service(GRACE)
            .run(request(&code, meta(None, None)?)?)
            .await;
        let line = reply.logs.first().ok_or("the line is kept, cut")?;
        assert!(
            !line.text.contains("hunter2"),
            "no part of a known secret survives the cut: {}",
            line.text
                .get(line.text.len().saturating_sub(16)..)
                .unwrap_or_default()
        );
        Ok(())
    })
}

#[test]
fn a_secret_across_an_error_text_limit_is_masked_before_the_cut() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let _registration = rig
            .secrets
            .register(SecretValue::new("hunter22".to_owned()), None);
        let service = rig.run_service(GRACE);
        // A thrown message keeps 4096 characters; this cut falls inside the secret.
        let thrown = service
            .run(request(
                "throw new Error('x'.repeat(4089) + 'hunter22');",
                meta(None, None)?,
            )?)
            .await;
        let message = &error_of(&thrown)?.message;
        assert!(
            !message.contains("hunter2"),
            "no part of the secret survives the thrown message's cut: {}",
            message
                .get(message.len().saturating_sub(32)..)
                .unwrap_or_default()
        );

        // An unknown method's name keeps 128 characters, in the error and the trace.
        let unknown = service
            .run(request(
                "await __neo.invoke('x'.repeat(121) + 'hunter22', {});",
                meta(None, None)?,
            )?)
            .await;
        let error = error_of(&unknown)?;
        assert_eq!(error.code, "method_unavailable");
        assert!(
            !error.message.contains("hunter2"),
            "no part of the secret survives the name's cut: {}",
            error.message
        );
        let operation = error.operation_id.ok_or("the error names its op")?;
        rig.sink.flush().await?;
        let recorded = rig
            .store
            .operation(operation)
            .await?
            .ok_or("the op is recorded")?;
        assert!(
            !recorded.name.contains("hunter2"),
            "nor in the trace's op name: {}",
            recorded.name
        );
        Ok(())
    })
}

#[test]
fn the_deadline_times_the_run_out() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let mut short = request("await new Promise(() => {});", meta(None, None)?)?;
        short.timeout = RunTimeout::from_millis(Some(300))?;
        let started = time::Instant::now();
        let reply = rig.run_service(GRACE).run(short).await;
        assert_eq!(reply.status, ReplyStatus::Timeout);
        assert_eq!(error_of(&reply)?.code, "run_timeout");
        assert!(
            started.elapsed() < Duration::from_millis(300) + Defaults::TEARDOWN_GRACE * 2,
            "the deadline stops it, then the grace ends the script that awaits forever"
        );
        Ok(())
    })
}

#[test]
fn a_refused_call_is_recorded_without_its_request_id() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let error = OperationError::host(ErrorCode::INVALID_META, "taskId must be 1 to 128 bytes");
        let refused = service.refuse(
            "return 1;".to_owned(),
            meta(Some("call-11"), Some("task-r"))?,
            &error,
        );
        let run = until_status(&rig, "task-r", RunStatus::Error).await?;
        assert_eq!(run.id, refused.id, "the reply's id resolves in the trace");
        assert_eq!(
            run.request_id, None,
            "a recorded request id would refuse the retry for good"
        );
        let retry = service
            .run(request("return 1;", meta(Some("call-11"), None)?)?)
            .await;
        assert_eq!(retry.status, ReplyStatus::Ok, "the corrected retry runs");
        Ok(())
    })
}
