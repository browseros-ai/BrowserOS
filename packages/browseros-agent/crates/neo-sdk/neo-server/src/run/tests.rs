//! The run lifecycle end to end: the run service over the real dispatcher and runtime on the
//! fake browser, with the trace store read back. Covers `requestId` admission (replay, join,
//! duplicates, per-task scope, an unreadable store), orphan grace, teardown, the panic
//! boundary, the door's text clips and the task's page default across a restart.

use std::sync::Arc;
use std::time::Duration;

use neo_trace::{
    Delivery, EpochMillis, ErrorOwner, OperationId, Outcome, RequestId, RunEnded, RunId, RunQuery,
    RunStarted, RunStatus, RunSummary, TaskId, WriterConfig,
};
use serde_json::{Value, json};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use tokio::runtime::Handle;
use tokio::{task, time};

use super::request_ledger::{Admission, LedgerLimits};
use super::run_reply::{ReplyDelivery, ReplyEffects, ReplyStatus};
use super::{
    ReplyError, RequestLedger, RunHandle, RunMeta, RunReply, RunRequest, RunService, RunTimeout,
};
use crate::config::Defaults;
use crate::dispatch::{ErrorCode, OperationError, TEXT_CHARS};
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
        let owner = rig.owner_tab("https://owner.example/?code=7Qx").await?;
        let code =
            "return context.pages().map((tab) => ({ brand: tab.toJSON(), url: tab.url() }));";
        let reply = rig
            .run_service(GRACE)
            .run(request(code, meta(None, None)?)?)
            .await;
        assert_eq!(reply.status, ReplyStatus::Ok, "{reply:?}");
        assert_eq!(
            reply.value,
            json!([{
                "brand": { "$neo": "page", "tab": owner.tab.0 },
                "url": "https://owner.example/?code=7Qx",
            }]),
            "every open tab before any op, its URL as the browser reports it"
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

/// A script chooses the texts in two of the door's own errors: what it threw, and an unknown
/// method's name. The door clips each one (`dispatch::clip`), so a script cannot grow the run's
/// error, or the trace's op name, without bound.
#[test]
fn a_scripts_error_texts_are_clipped_at_the_doors_limits() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let thrown = service
            .run(request(
                "throw new Error('x'.repeat(5000));",
                meta(None, None)?,
            )?)
            .await;
        let message = &error_of(&thrown)?.message;
        let kept = format!("{}…", "x".repeat(TEXT_CHARS));
        assert!(
            message.contains(&kept) && !message.contains(&format!("{}x", "x".repeat(TEXT_CHARS))),
            "a thrown message keeps TEXT_CHARS characters, the cut marked: {}",
            message
                .get(message.len().saturating_sub(32)..)
                .unwrap_or_default()
        );

        let unknown = service
            .run(request(
                "await __neo.invoke('y'.repeat(200), {});",
                meta(None, None)?,
            )?)
            .await;
        let error = error_of(&unknown)?;
        assert_eq!(error.code, "method_unavailable");
        assert!(
            error.message.contains(&format!("`{}…`", "y".repeat(128))),
            "an unknown name keeps 128 characters in its refusal: {}",
            error.message
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
