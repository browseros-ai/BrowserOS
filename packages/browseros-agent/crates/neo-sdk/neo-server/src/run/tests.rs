//! The run lifecycle end to end: the run service over the real dispatcher and runtime on the
//! fake browser, with the trace store read back. Covers `requestId` admission (replay, join,
//! duplicates, per-task scope, an unreadable store), orphan grace, teardown, the panic
//! boundary, the door's text clips, and the task's page: its own tab, kept by its own acts
//! only, and across a restart.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use browseros_core::TabId;
use neo_trace::{
    BlobId, Delivery, EpochMillis, ErrorOwner, OperationId, Outcome, RequestId, RunEnded, RunId,
    RunQuery, RunStarted, RunStatus, RunSummary, TaskId, WriterConfig,
};
use serde_json::{Value, json};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use tokio::runtime::Handle;
use tokio::{fs, task, time};

use super::request_ledger::{Admission, LedgerLimits};
use super::run_reply::{ReplyDelivery, ReplyEffects, ReplyStatus};
use super::{
    ReplyError, RequestLedger, RunHandle, RunMeta, RunReply, RunRequest, RunService, RunTimeout,
};
use crate::browser::TaskPage;
use crate::config::Defaults;
use crate::dispatch::{ErrorCode, OperationError, TEXT_CHARS, TaskKind};
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
        tasks: Vec::new(),
        id: handle.id(),
        status: ReplyStatus::Ok,
        effects: ReplyEffects::None,
        value: json!(1),
        error: None,
        logs: Vec::new(),
        logs_dropped: 0,
        unknown: Vec::new(),
        not_awaited: Vec::new(),
        tips: Vec::new(),
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
        // when they start, and the trace's `task_tabs` reads the start records.
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

/// zuzu gap G3: a reply too large to carry its bytes names them, and they are on disk beside
/// the trace by the time the caller has the reply.
#[test]
fn a_reply_over_the_limit_names_blobs_already_kept_with_the_run() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = "return { shot: new Uint8Array(300 * 1024).fill(9) };";
        let reply = rig
            .run_service(GRACE)
            .run(request(code, meta(None, None)?)?)
            .await;
        assert_eq!(reply.status, ReplyStatus::Ok, "{reply:?}");
        assert_eq!(
            reply.value,
            json!({ "shot": { "$neo": "blob", "id": "b0", "size": 300 * 1024 } })
        );
        let kept = rig
            .store
            .outputs()
            .open_blob(reply.id, BlobId::new(0))
            .await?
            .ok_or("the blob is kept before the reply")?;
        assert_eq!(kept.size, 300 * 1024);
        Ok(())
    })
}

/// A client publication can fail after native input completed. Local error fields are not
/// trusted op failures, but recorded writes still make the entire run unsafe to repeat.
#[test]
fn a_local_observation_failure_after_a_completed_write_is_not_retryable() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = r"
            await context.newPage();
            throw Object.assign(new Error('Navigation events were lost before click completion'), {
                delivery: 'unknown', retryable: true, owner: 'neo', operationId: 'forged'
            });
        ";
        let reply = rig
            .run_service(GRACE)
            .run(request(code, meta(None, None)?)?)
            .await;
        assert_eq!(reply.effects, ReplyEffects::Done);
        let error = error_of(&reply)?;
        assert_eq!(error.code, "script_error");
        assert!(
            !error.retryable,
            "a local error never makes a completed write safe to replay"
        );
        assert_eq!(
            error.operation_id, None,
            "the realm cannot forge trusted operation provenance"
        );
        assert_eq!(error.delivery, None);
        Ok(())
    })
}

#[test]
fn unhosted_runs_have_fresh_unowned_defaults_and_replay_without_tasks() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let code = r"
            await __neo.invoke('test.touch', { page });
            const opened = await context.newPage();
            return { tab: page.tabId, owner: page.taskId, opened: opened.tabId, openedOwner: opened.taskId };
        ";
        let previous = service.run(request(code, meta(None, None)?)?).await;
        let (first, second) = tokio::join!(
            service.run(request(code, meta(Some("plain-one"), None)?)?),
            service.run(request(code, meta(Some("plain-two"), None)?)?),
        );
        let mut tabs = HashSet::new();
        for reply in [&previous, &first, &second] {
            assert_eq!(reply.status, ReplyStatus::Ok, "{reply:?}");
            assert_eq!(reply.value["owner"], Value::Null);
            assert_eq!(reply.value["openedOwner"], Value::Null);
            assert!(
                reply.tasks.is_empty(),
                "plain Playwright never starts a task"
            );
            for key in ["tab", "opened"] {
                assert!(
                    tabs.insert(reply.value[key].as_i64().ok_or("tab id")?),
                    "every open is fresh"
                );
            }
        }
        assert_eq!(rig.fake.tabs().len(), 6, "earlier runs' tabs remain open");
        rig.sink.flush().await?;
        let replay = rig
            .restarted_service(GRACE)?
            .run(request(code, meta(Some("plain-one"), None)?)?)
            .await;
        assert_eq!(*replay, *first, "replay never opens another tab");
        assert_eq!(rig.fake.tabs().len(), 6);
        Ok(())
    })
}

#[test]
fn parallel_scripts_attach_their_default_and_new_pages_to_their_own_tasks() -> Result<(), TestError>
{
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let code = r"
            const task = await neo.createTask('parallel');
            await __neo.invoke('test.touch', { page });
            const opened = await context.newPage();
            return { id: task.id, tab: page.tabId, owner: page.taskId,
                opened: opened.tabId, openedOwner: opened.taskId,
                pages: task.pages().map(p => p.tabId) };
        ";
        let (first, second) = tokio::join!(
            service.run(request(code, meta(None, None)?)?),
            service.run(request(code, meta(None, None)?)?),
        );
        let mut tabs = HashSet::new();
        for reply in [&first, &second] {
            assert_eq!(reply.status, ReplyStatus::Ok, "{reply:?}");
            assert_eq!(reply.value["owner"], reply.value["id"]);
            assert_eq!(reply.value["openedOwner"], reply.value["id"]);
            assert_eq!(
                reply.value["pages"],
                json!([reply.value["tab"], reply.value["opened"]])
            );
            assert_eq!(reply.tasks.len(), 1);
            assert_eq!(reply.tasks[0].tabs.len(), 2);
            for key in ["tab", "opened"] {
                assert!(
                    tabs.insert(reply.value[key].as_i64().ok_or("tab id")?),
                    "tasks never share tabs"
                );
            }
        }
        assert_ne!(first.value["id"], second.value["id"]);
        Ok(())
    })
}

#[test]
fn closing_a_script_pick_restores_the_host_task_and_known_defaults_stay_fixed()
-> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let first = service
            .run(request(
                r"
            const task = await neo.createTask('temporary');
            await task.close();
            await __neo.invoke('test.touch', { page });
            const opened = await context.newPage();
            return { tab: page.tabId, owner: page.taskId, openedOwner: opened.taskId };
        ",
                meta(None, Some("host-task"))?,
            )?)
            .await;
        assert_eq!(first.status, ReplyStatus::Ok, "{first:?}");
        assert_eq!(first.value["owner"], "host-task");
        assert_eq!(first.value["openedOwner"], "host-task");
        assert_eq!(first.tasks[0].kind, TaskKind::Closed);
        let TaskPage::Tab(host_tab) = rig.tabs.task_page(&TaskId::parse("host-task")?) else {
            return Err("host tab".into());
        };
        let second = service
            .run(request(
                r"
            const task = await neo.createTask('another');
            await __neo.invoke('test.touch', { page });
            const opened = await context.newPage();
            return { tab: page.tabId, owner: page.taskId, id: task.id, openedOwner: opened.taskId };
        ",
                meta(None, Some("host-task"))?,
            )?)
            .await;
        assert_eq!(second.status, ReplyStatus::Ok, "{second:?}");
        assert_eq!(second.value["tab"], json!(host_tab.tab));
        assert_eq!(second.value["owner"], "host-task");
        assert_eq!(second.value["openedOwner"], second.value["id"]);
        Ok(())
    })
}

#[test]
fn failed_scripts_keep_created_and_fetched_tasks_in_first_touch_order() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let code = "const first = await __neo.invoke('neo.createTask', { name: ' first ' });\nconst second = await __neo.invoke('neo.createTask', { name: 'second' });\nawait __neo.invoke('neo.task', { id: first.id });\nthrow new Error('later failure');";
        let reply = rig
            .run_service(GRACE)
            .run(request(code, meta(None, None)?)?)
            .await;
        assert_eq!(reply.status, ReplyStatus::Error);
        assert_eq!(reply.tasks.len(), 2);
        assert_eq!(reply.tasks[0].name.as_deref(), Some("first"));
        assert_eq!(reply.tasks[1].name.as_deref(), Some("second"));
        assert_eq!(reply.tasks[0].kind, TaskKind::Created);
        assert!(reply.tasks.iter().all(|task| task.tabs.is_empty()));
        let fetched = rig.run_service(GRACE).run(request(
            &format!("await __neo.invoke('neo.task', {{ id: '{}' }}); throw new Error('failed again');", reply.tasks[0].id),
            meta(None, None)?,
        )?).await;
        assert_eq!(fetched.tasks[0].kind, TaskKind::Fetched);
        assert_eq!(fetched.tasks[0].id, reply.tasks[0].id);
        Ok(())
    })
}

#[test]
fn host_defaults_keep_their_own_tab_and_only_report_explicit_fetches() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let first = service
            .run(request(
                "return (await __neo.invoke('test.touch', { page })).tab;",
                meta(None, Some("host-task"))?,
            )?)
            .await;
        assert_eq!(first.status, ReplyStatus::Ok, "{first:?}");
        assert!(
            first.tasks.is_empty(),
            "host-only task ids stay out of recovery notes"
        );
        let tab = TabId(first.value.as_i64().ok_or("tab id")?);
        assert_eq!(rig.tabs.owner(tab), Some(TaskId::parse("host-task")?));
        let again = service
            .run(request(
                "return page.toJSON().tab;",
                meta(None, Some("host-task"))?,
            )?)
            .await;
        assert_eq!(again.value, first.value);
        let fetched = service
            .run(request(
                "return await __neo.invoke('neo.task', { id: 'host-task' });",
                meta(None, None)?,
            )?)
            .await;
        assert_eq!(fetched.tasks[0].kind, TaskKind::Fetched);
        assert_eq!(
            fetched.value["pages"],
            json!([{ "$neo": "page", "tab": tab }])
        );
        let other = rig.owner_tab("https://other.example/").await?;
        let touched = service.run(request(
            &format!("return await __neo.invoke('test.touch', {{ page: {{ '$neo': 'page', tab: {} }} }});", other.tab.0),
            meta(None, Some("host-task"))?,
        )?).await;
        assert_eq!(touched.status, ReplyStatus::Ok);
        let last = service
            .run(request(
                "return page.toJSON().tab;",
                meta(None, Some("host-task"))?,
            )?)
            .await;
        assert_eq!(
            last.value, first.value,
            "a script's act on a user's tab never makes it the task's page: that is its own tab"
        );
        assert_eq!(
            rig.tabs.owner(other.tab),
            None,
            "touching never takes a user's tab"
        );
        Ok(())
    })
}

#[test]
fn task_notes_read_urls_at_reply_time() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let code = "await neo.createTask(); await context.newPage(); await __neo.invoke('test.gate', {}); await __neo.invoke('test.gate', {});";
        let request = request(code, meta(None, None)?)?;
        let pending = tokio::spawn(async move { service.run(request).await });
        within(Duration::from_secs(5), rig.gate.wait()).await?;
        let tab = *rig.fake.tabs().first().ok_or("opened tab")?;
        assert!(
            rig.fake
                .navigate_tab(tab, "https://changed.example/", "Changed")
        );
        within(Duration::from_secs(5), rig.gate.wait()).await?;
        let reply = within(Duration::from_secs(5), pending).await??;
        assert_eq!(reply.tasks[0].tabs[0].url, "https://changed.example/");
        Ok(())
    })
}

#[test]
fn task_pages_keep_opening_order_across_context_and_task_openers() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let reply = rig
            .run_service(GRACE)
            .run(request(
                r"
            const task = await neo.createTask();
            const first = await context.newPage();
            const second = await task.newPage();
            const third = await context.newPage();
            const fourth = await task.newPage();
            return {
                expected: [first.tabId, second.tabId, third.tabId, fourth.tabId],
                actual: task.pages().map(p => p.tabId),
                owners: task.pages().map(p => p.taskId),
                id: task.id
            };
        ",
                meta(None, None)?,
            )?)
            .await;
        assert_eq!(reply.status, ReplyStatus::Ok, "{reply:?}");
        assert_eq!(reply.value["actual"], reply.value["expected"]);
        assert_eq!(reply.value["owners"], json!(vec![&reply.value["id"]; 4]));
        Ok(())
    })
}

/// Value omission changes persistence only: a caller still receives and can fetch its result.
#[test]
fn values_off_preserves_the_reply_blobs_and_memory_replay_but_not_restart_replay()
-> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        rig.settings.patch(br#"{"trace":{"values":"off"}}"#).await?;
        let service = rig.run_service(GRACE);
        let code = "return { shot: new Uint8Array(300 * 1024).fill(9) };";
        let first = service
            .run(request(code, meta(Some("off-reply"), None)?)?)
            .await;
        assert_eq!(first.status, ReplyStatus::Ok);
        let blob = rig
            .store
            .outputs()
            .open_blob(first.id, BlobId::new(0))
            .await?
            .ok_or("reply blob")?;
        assert_eq!(blob.size, 300 * 1024);
        let sidecar = rig
            .dir
            .path()
            .join(format!("outputs/{}/blob-expiry", first.id));
        let until: i64 = fs::read_to_string(&sidecar).await?.parse()?;
        let now = EpochMillis::now().millis();
        assert!(
            until > now && until <= now + 600_000,
            "expiry is the absolute ten-minute reply window"
        );
        rig.sink.flush().await?;
        let row = rig.store.run(first.id).await?.ok_or("run")?;
        assert_eq!(row.summary.values.as_deref(), Some("off"));
        assert!(row.code.is_none() && row.result.is_none());
        assert!(
            rig.store
                .replay(first.id, EpochMillis::now())
                .await?
                .is_none()
        );
        let repeat = service
            .run(request(
                "throw new Error('ran twice')",
                meta(Some("off-reply"), None)?,
            )?)
            .await;
        assert_eq!(
            *repeat, *first,
            "same-process memory replays the complete reply"
        );
        let after_restart = rig
            .restarted_service(GRACE)?
            .run(request(code, meta(Some("off-reply"), None)?)?)
            .await;
        assert_eq!(error_of(&after_restart)?.code, "duplicate_request");
        assert_eq!(
            runs_of(&rig, "off-reply")
                .await?
                .iter()
                .filter(|run| run.status == RunStatus::Ok)
                .count(),
            1
        );
        assert!(
            rig.store
                .outputs()
                .open_blob(first.id, BlobId::new(0))
                .await?
                .is_some()
        );
        Ok(())
    })
}

/// An off record remains the request-id authority after its in-memory reply expires.
#[test]
fn values_off_does_not_execute_a_request_again_after_the_memory_window() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        rig.settings.patch(br#"{"trace":{"values":"off"}}"#).await?;
        let ledger = RequestLedger::new(
            rig.store.clone(),
            rig.sink.clone(),
            LedgerLimits {
                replay_window: SHORT_WINDOW,
                ..LedgerLimits::default()
            },
        );
        let first = run_once(&rig, &ledger, "off-expired").await?;
        rig.sink.flush().await?;
        assert!(
            rig.store
                .replay(first.id, EpochMillis::now())
                .await?
                .is_none()
        );
        time::sleep(SHORT_WINDOW + Duration::from_millis(20)).await;
        assert!(
            matches!(admit(&ledger, "off-expired").await?, Admission::Duplicate(run) if run == first.id)
        );
        assert_eq!(runs_of(&rig, "off-expired").await?.len(), 1);
        Ok(())
    })
}

/// The default debug-note path must not grow a stored payload in either recording mode.
#[test]
fn operation_notes_remain_unstored_through_the_real_dispatcher() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        for mode in ["on", "off"] {
            rig.settings
                .patch(json!({"trace":{"values":mode}}).to_string().as_bytes())
                .await?;
            let reply = service
                .run(request(
                    "return await __neo.invoke('test.echo', { note: 'dispatch-note-canary' });",
                    RunMeta::default(),
                )?)
                .await;
            assert_eq!(reply.status, ReplyStatus::Ok);
            rig.sink.flush().await?;
            let op = rig
                .store
                .operation(OperationId::new(reply.id, 1))
                .await?
                .ok_or("echo operation")?;
            assert!(
                op.notes.is_empty(),
                "call-log lines stay on the debug channel in both modes"
            );
        }
        Ok(())
    })
}

/// A separate process installs the real subscriber once; both its stderr and daily file must
/// suppress off-mode selectors and typed values while keeping the on-mode control visible.
#[test]
fn values_off_suppresses_real_call_log_values_at_debug() -> Result<(), TestError> {
    use std::process::Command;
    use std::{env, fs};

    let state = tempfile::tempdir()?;
    fs::write(state.path().join("launch.json"), "{}")?;
    let output = Command::new(env::current_exe()?)
        .args(["--ignored", "--exact", "run::tests::trace_values_log_probe", "--nocapture"])
        .env_clear().env("BROWSERCLAW_DIR", state.path()).env("HOME", state.path())
        .env("CLAW_LOG", "neo_server=debug,neo_playwright=debug,neo_trace=debug,neo_http=debug,neo_takeover=debug")
        .output()?;
    assert!(
        output.status.success(),
        "logging child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr)?;
    let mut logged = String::new();
    for entry in fs::read_dir(state.path().join("logs"))? {
        let path = entry?.path();
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("claw-sdk.log"))
        {
            logged.push_str(&fs::read_to_string(path)?);
        }
    }
    for text in [stderr, logged] {
        assert!(
            text.contains("selector-on-canary") && text.contains("filled-on-canary"),
            "on must keep its existing debug notes: {text}"
        );
        assert!(
            !text.contains("selector-off-canary") && !text.contains("filled-off-canary"),
            "off must suppress call-log values at the source: {text}"
        );
    }
    Ok(())
}

#[test]
#[ignore = "installs process-global logging in the subprocess above"]
fn trace_values_log_probe() -> Result<(), TestError> {
    use crate::{AppConfig, Logging};
    use std::env;
    use std::path::Path;

    let state = env::var_os("BROWSERCLAW_DIR").ok_or("child state")?;
    let logging = Logging::install(&AppConfig::load(&Path::new(&state).join("launch.json"))?)?;
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        for mode in ["on", "off"] {
            rig.settings
                .patch(json!({"trace":{"values":mode}}).to_string().as_bytes())
                .await?;
            let code = format!(
                "return await __neo.invoke('test.echo', {{note: \"selector-{mode}-canary filled-{mode}-canary\"}});"
            );
            let reply = service.run(request(&code, RunMeta::default())?).await;
            assert_eq!(reply.status, ReplyStatus::Ok);
        }
        rig.sink.flush().await?;
        Ok(())
    })?;
    drop(logging);
    Ok(())
}

// A run's built-in `page` is its task's own tab, the one its own script last acted on. These
// replay what broke it for the first host: `page.waitForTimeout` ends with the prelude reading
// the frames of every tab the run lists (FR-1), and the door counted those reads as the task
// acting on each tab, so the task's next run started on whichever tab answered last: another
// task's, or the browser's New Tab.

/// A task's first act opens its tab; the run returns that tab.
const OPEN_OWN_TAB: &str = "return (await __neo.invoke('test.touch', { page })).tab;";

/// What the prelude sends as `waitForTimeout` ends: a read of every listed tab, each with the
/// explicit empty call site that marks the prelude's own work (bootstrap.js `invoke`). The run's
/// own tab is read first, so a door that counted these reads would end on another tab.
const NEO_READS_EVERY_TAB: &str = r"
    await __neo.invoke('test.touch', { page }, { site: '' });
    for (const other of context.pages()) {
      if (other !== page) await __neo.invoke('test.touch', { page: other }, { site: '' });
    }
    return 1;";

/// The same reads in a loop that only the run's deadline ends, as a wait loop does when its
/// time runs out (the first host's card entry).
const NEO_READS_UNTIL_THE_DEADLINE: &str = r"
    for (;;) {
      await __neo.invoke('test.touch', { page }, { site: '' });
      for (const other of context.pages()) {
        if (other !== page) await __neo.invoke('test.touch', { page: other }, { site: '' });
      }
    }";

/// The script itself acts on every other tab it can reach through `context.pages()`.
const SCRIPT_ACTS_ON_OTHER_TABS: &str = r"
    for (const other of context.pages()) {
      if (other !== page) await __neo.invoke('test.touch', { page: other });
    }
    return 1;";

async fn run_in(
    service: &Arc<RunService>,
    task: &str,
    code: &str,
) -> Result<Arc<RunReply>, TestError> {
    Ok(service.run(request(code, meta(None, Some(task))?)?).await)
}

/// The tab the task's next run starts on, or null when it has none yet.
async fn next_page(service: &Arc<RunService>, task: &str) -> Result<Value, TestError> {
    let reply = run_in(service, task, "return page.tabId;").await?;
    assert_eq!(reply.status, ReplyStatus::Ok, "{reply:?}");
    Ok(reply.value.clone())
}

#[test]
fn neos_own_reads_of_other_tabs_leave_a_task_on_its_own_tab() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let new_tab = rig.owner_tab("chrome://newtab/").await?;
        let mine = run_in(&service, "task-mine", OPEN_OWN_TAB)
            .await?
            .value
            .clone();
        let theirs = run_in(&service, "task-theirs", OPEN_OWN_TAB)
            .await?
            .value
            .clone();
        assert_ne!(mine, theirs, "each task opened its own tab");

        let waited = run_in(&service, "task-mine", NEO_READS_EVERY_TAB).await?;
        assert_eq!(waited.status, ReplyStatus::Ok, "{waited:?}");
        assert_eq!(
            next_page(&service, "task-mine").await?,
            mine,
            "a run that only waited leaves its task on its own tab, not {theirs} or the New Tab {}",
            new_tab.tab.0
        );

        let looped = service
            .run(RunRequest {
                code: NEO_READS_UNTIL_THE_DEADLINE.to_owned(),
                timeout: RunTimeout::from_millis(Some(300))?,
                meta: meta(None, Some("task-mine"))?,
            })
            .await;
        assert_eq!(looped.status, ReplyStatus::Timeout, "{looped:?}");
        assert_eq!(
            next_page(&service, "task-mine").await?,
            mine,
            "nor one whose wait loop ran out of time"
        );
        assert_eq!(
            next_page(&service, "task-theirs").await?,
            theirs,
            "and the other task keeps its own tab"
        );
        Ok(())
    })
}

#[test]
fn a_script_acting_on_other_tabs_does_not_move_its_own_page() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        rig.owner_tab("chrome://newtab/").await?;
        let mine = run_in(&service, "task-mine", OPEN_OWN_TAB)
            .await?
            .value
            .clone();
        let theirs = run_in(&service, "task-theirs", OPEN_OWN_TAB)
            .await?
            .value
            .clone();

        let acted = run_in(&service, "task-mine", SCRIPT_ACTS_ON_OTHER_TABS).await?;
        assert_eq!(acted.status, ReplyStatus::Ok, "{acted:?}");
        assert_eq!(
            next_page(&service, "task-mine").await?,
            mine,
            "its own tab, though its script last acted on another task's tab and the New Tab"
        );
        assert_eq!(next_page(&service, "task-theirs").await?, theirs);
        Ok(())
    })
}

#[test]
fn a_task_whose_tab_closed_gets_a_new_tab_not_a_neighbours() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let new_tab = rig.owner_tab("chrome://newtab/").await?;
        let mine = run_in(&service, "task-mine", OPEN_OWN_TAB)
            .await?
            .value
            .clone();
        let theirs = run_in(&service, "task-theirs", OPEN_OWN_TAB)
            .await?
            .value
            .clone();
        let waited = run_in(&service, "task-mine", NEO_READS_EVERY_TAB).await?;
        assert_eq!(waited.status, ReplyStatus::Ok, "{waited:?}");

        let tab = TabId(mine.as_i64().ok_or("a tab id")?);
        assert!(rig.fake.close_tab(tab), "the owner closes the task's tab");
        within(Duration::from_secs(5), async {
            while rig.tabs.is_open(tab) {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?;

        assert_eq!(
            next_page(&service, "task-mine").await?,
            Value::Null,
            "no tab yet: not {theirs} or the New Tab {}",
            new_tab.tab.0
        );
        let fresh = run_in(&service, "task-mine", OPEN_OWN_TAB)
            .await?
            .value
            .clone();
        for neighbour in [&mine, &theirs, &json!(new_tab.tab.0)] {
            assert_ne!(&fresh, neighbour, "its first act opens a new tab");
        }
        Ok(())
    })
}

/// Opens a second tab in the run's own task, acts on it and returns it.
const OPEN_SECOND_TAB: &str = r"
    const second = await context.newPage();
    return (await __neo.invoke('test.touch', { page: second })).tab;";

#[test]
fn after_a_restart_a_task_never_takes_a_tab_a_task_it_picked_opened() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let mine = run_in(&service, "task-a", OPEN_OWN_TAB)
            .await?
            .value
            .clone();
        // The host task's run picks another task; the tab it opens then is that task's.
        let picked = run_in(
            &service,
            "task-a",
            r"
            await neo.createTask('picked');
            const theirs = await context.newPage();
            await __neo.invoke('test.touch', { page: theirs });
            return { tab: theirs.tabId, owner: theirs.taskId };",
        )
        .await?;
        assert_eq!(picked.status, ReplyStatus::Ok, "{picked:?}");
        assert_ne!(picked.value["owner"], json!("task-a"), "{picked:?}");
        rig.sink.flush().await?;

        let restarted = rig.restarted_service(GRACE)?;
        assert_eq!(
            next_page(&restarted, "task-a").await?,
            mine,
            "its own tab, not {} that the task it picked opened and its script acted on last",
            picked.value["tab"]
        );
        Ok(())
    })
}

#[test]
fn after_a_restart_every_surviving_tab_of_a_task_is_its_own_again() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let first = run_in(&service, "task-a", OPEN_OWN_TAB)
            .await?
            .value
            .clone();
        let second = run_in(&service, "task-a", OPEN_SECOND_TAB)
            .await?
            .value
            .clone();
        rig.sink.flush().await?;

        let restarted = rig.restarted_service(GRACE)?;
        assert_eq!(
            next_page(&restarted, "task-a").await?,
            second,
            "its last act"
        );
        let acted = run_in(
            &restarted,
            "task-a",
            &format!(
                "const first = context.pages().find((q) => q.tabId === {first});\n\
                 await __neo.invoke('test.touch', {{ page: first }});\nreturn first.taskId;"
            ),
        )
        .await?;
        assert_eq!(acted.value, json!("task-a"), "{acted:?}");
        assert_eq!(
            next_page(&restarted, "task-a").await?,
            first,
            "an act on its other surviving tab counts: that tab is the task's again"
        );
        Ok(())
    })
}

#[test]
fn after_a_restart_a_task_never_takes_a_tab_a_task_it_named_opened() -> Result<(), TestError> {
    on_neo_runtime(async {
        let rig = TestRig::new().await?;
        let service = rig.run_service(GRACE);
        let mine = run_in(&service, "task-a", OPEN_OWN_TAB)
            .await?
            .value
            .clone();
        let created = run_in(
            &service,
            "task-other",
            "return (await neo.createTask('named')).id;",
        )
        .await?;
        let named = created.value.as_str().ok_or("a task id")?.to_owned();
        // No pick: the host task's script opens a tab in the task it names. The open is a
        // child of `task.newPage` in the host task's run, and the tab is the named task's.
        let opened = run_in(
            &service,
            "task-a",
            &format!(
                "const {{ page: theirs }} = await __neo.invoke('task.newPage', {{ task: {named:?} }});\n\
                 await __neo.invoke('test.touch', {{ page: theirs }});\n\
                 return {{ tab: theirs.tabId, owner: theirs.taskId }};"
            ),
        )
        .await?;
        assert_eq!(opened.status, ReplyStatus::Ok, "{opened:?}");
        assert_eq!(
            opened.value["owner"],
            json!(named),
            "the named task's tab: {opened:?}"
        );
        assert_eq!(
            next_page(&service, "task-a").await?,
            mine,
            "and not the host task's page"
        );
        rig.sink.flush().await?;

        let restarted = rig.restarted_service(GRACE)?;
        assert_eq!(
            next_page(&restarted, "task-a").await?,
            mine,
            "after a restart too: not {}, which the task it named opened",
            opened.value["tab"]
        );
        Ok(())
    })
}
