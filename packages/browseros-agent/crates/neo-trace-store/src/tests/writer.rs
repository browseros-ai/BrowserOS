//! Neo: FIFO visibility, pressure, database outages, and bounded shutdown through the public store.
//! Pause is a writer-only test seam; producers always execute the production synchronous path.

use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use neo_runtime::{CallId, RunEvent, SessionKey};
use tokio::sync::oneshot;
use tokio::{task, time};

use super::support::{after, before, ended, getter, scratch, started};
use crate::ingest::Command;
use crate::{
    AfterCall, CallQuery, CallState, RunQuery, RunState, SettledBy, StoreConfig, Timeline,
    TraceStore, Unfinished,
};

async fn pause(store: &TraceStore) -> oneshot::Sender<()> {
    let (entered_tx, entered_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = oneshot::channel();
    assert!(
        store
            .inner
            .queue
            .control(Command::Pause(entered_tx, resume_rx)),
        "pause admitted"
    );
    entered_rx.await.unwrap();
    resume_tx
}

#[tokio::test]
async fn a_plain_thread_finishes_emit_with_a_paused_writer_and_exhausted_budgets() {
    let (_dir, path) = scratch();
    let mut config = StoreConfig::new(path);
    config.queue_bytes = 12 << 10;
    config.reserve_bytes = 24 << 10;
    let store = TraceStore::open(config).await.unwrap();
    let resume = pause(&store).await;
    let start = started("pressure");
    let run = start.run.id;
    let sink = store.sink();
    let handle = thread::spawn(move || {
        let capture = sink.run(&start).unwrap();
        for no in 1..=2000 {
            capture.emit(&before(run, no));
            capture.emit(&after(run, no));
        }
        capture.emit(&ended(run, 2000));
        capture.emit(&RunEvent::RunSettled { run });
    });
    // Joining while the writer is paused proves emit never needs a runtime or writer progress.
    handle.join().unwrap();
    let health = store.health();
    assert!(health.events_dropped > 0, "pressure is counted");
    assert!(
        health.queued_bytes <= 36 << 10,
        "byte budget bounds queued payloads"
    );
    assert_eq!(
        health.settles_lost, 1,
        "a lost settlement is visible to the host"
    );
    resume.send(()).unwrap();
    store.flushed().await;
    let rows = store
        .calls(
            &run,
            CallQuery {
                after: None,
                limit: 10_000,
            },
        )
        .await
        .unwrap()
        .items;
    assert!(
        rows.len() as u64 + health.events_dropped >= 2000,
        "each number has a row or a counted loss"
    );
    assert_eq!(store.health().queued_bytes, 0, "commit releases permits");
}

#[tokio::test]
async fn flush_cannot_overtake_events_and_watch_moves_after_rows_are_readable() {
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(path)).await.unwrap();
    let mut watch = store.watch(&SessionKey::new("fifo"));
    let initial = watch.borrow().version;
    let resume = pause(&store).await;
    let start = started("fifo");
    let run = start.run.id;
    let capture = store.sink().run(&start).unwrap();
    capture.emit(&after(run, 1));
    let waiting = store.clone();
    let flush = task::spawn(async move {
        waiting.flushed().await;
    });
    task::yield_now().await;
    assert!(!flush.is_finished(), "fence is held by earlier writes");
    assert_eq!(
        watch.borrow().version,
        initial,
        "uncommitted events do not move heads"
    );
    assert!(
        store.run(&run).await.unwrap().is_none(),
        "paused writer has no durable rows"
    );
    resume.send(()).unwrap();
    watch.changed().await.unwrap();
    assert!(
        store.run(&run).await.unwrap().is_some(),
        "head implies readable run"
    );
    assert_eq!(
        store
            .calls(
                &run,
                CallQuery {
                    after: None,
                    limit: 10
                }
            )
            .await
            .unwrap()
            .items
            .len(),
        1,
        "head implies readable call"
    );
    flush.await.unwrap();
    capture.emit(&ended(run, 1));
    capture.emit(&RunEvent::RunSettled { run });
    store.flushed().await;
    assert_eq!(watch.borrow().unsettled_runs, 0, "settlement is committed");
}

#[tokio::test]
async fn sqlite_readonly_outage_preserves_the_queue_then_drains_after_recovery() {
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(path)).await.unwrap();
    let start = started("outage");
    let run = start.run.id;
    let capture = store.sink().run(&start).unwrap();
    store.flushed().await;
    // chmod on an already-open SQLite descriptor does not revoke write access. query_only
    // causes real SQLITE_READONLY failures on that descriptor, and is independently resettable
    // while the FIFO batch remains pinned (a control message could not pass the failed batch).
    store
        .inner
        .queue
        .state
        .test_readonly
        .store(true, Ordering::Release);
    capture.emit(&after(run, 1));
    capture.emit(&ended(run, 1));
    capture.emit(&RunEvent::RunSettled { run });
    time::timeout(Duration::from_secs(10), async {
        while store.health().failing_since_ms.is_none()
            || store
                .inner
                .queue
                .state
                .test_failures
                .load(Ordering::Acquire)
                < 6
        {
            task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let waiting = store.clone();
    let flush = task::spawn(async move {
        waiting.flushed().await;
    });
    task::yield_now().await;
    assert!(!flush.is_finished(), "failed batch retains its barrier");
    assert!(
        store.health().queued_bytes > 0,
        "failed transaction retains its permits"
    );
    store
        .inner
        .queue
        .state
        .test_readonly
        .store(false, Ordering::Release);
    time::timeout(Duration::from_secs(10), flush)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        store.health().failing_since_ms,
        None,
        "recovered writer is healthy"
    );
    assert_eq!(
        store.health().writes_dropped,
        0,
        "transient database failure loses no events"
    );
    assert_eq!(
        store.run(&run).await.unwrap().unwrap().summary.state,
        RunState::Ok,
        "run end drained"
    );
}

#[tokio::test]
async fn close_drains_and_reopen_interrupts_unfinished_runs_and_calls() {
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let start = started("restart");
    let run = start.run.id;
    let capture = store.sink().run(&start).unwrap();
    capture.emit(&before(run, 1));
    let health = store.close(Duration::from_secs(5)).await;
    assert_eq!(health.queued_events, 0, "close drained its accepted events");
    let reopened = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let summary = reopened.run(&run).await.unwrap().unwrap().summary;
    assert_eq!(summary.state, RunState::Interrupted, "old run interrupted");
    assert_eq!(
        summary.settled.unwrap().by,
        SettledBy::Restart,
        "restart settlement"
    );
    let calls = reopened
        .calls(
            &run,
            CallQuery {
                after: None,
                limit: 10,
            },
        )
        .await
        .unwrap()
        .items;
    assert_eq!(
        (calls[0].state, calls[0].unfinished),
        (CallState::Unfinished, Some(Unfinished::Interrupted)),
        "running calls interrupted"
    );
    assert!(
        reopened
            .watch(&SessionKey::new("restart"))
            .borrow()
            .last_run
            .is_some(),
        "heads initialized from durable rows"
    );
}

#[tokio::test]
async fn close_budget_cancels_a_stuck_writer_and_closes_admission() {
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let _resume = pause(&store).await;
    let start = started("close");
    store
        .sink()
        .run(&start)
        .unwrap()
        .emit(&after(start.run.id, 1));
    let health = store.close(Duration::from_millis(20)).await;
    assert!(health.closed, "budget completion seals ingest");
    assert!(
        health.unflushed_events > 0 && health.unflushed_bytes > 0,
        "budget expiry reports the undrained queue"
    );
    assert!(
        store.sink().run(&started("late")).is_none(),
        "no admission after close"
    );
    time::timeout(Duration::from_secs(5), async {
        loop {
            let result = TraceStore::open(StoreConfig::new(&path)).await;
            if matches!(result, Err(crate::StoreError::Locked { .. })) {
                task::yield_now().await;
                continue;
            }
            result.unwrap().close(Duration::from_secs(5)).await;
            break;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn artifacts_privacy_ids_and_session_erasure_share_fifo_ordering() {
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(path)).await.unwrap();
    let start = started("private");
    let run = start.run.id;
    let capture = store.sink().run(&start).unwrap();
    capture.emit(&after(run, 1));
    capture.emit(&getter(run, 1));
    let artifact = AfterCall {
        tab_id: Some(7),
        url: Some("https://example.com/".into()),
        title: Some("Example".into()),
        jpeg: Some(vec![1; 1024]),
        captured_at_ms: 10,
    };
    store.attach_after_call(CallId { run, no: 1 }, artifact.clone());
    store.flushed().await;
    assert!(
        store
            .calls(
                &run,
                CallQuery {
                    after: None,
                    limit: 10
                }
            )
            .await
            .unwrap()
            .items[0]
            .after
            .as_ref()
            .unwrap()
            .jpeg
            .is_some(),
        "capture stored"
    );
    store.withhold(&SessionKey::new("private"));
    store.attach_after_call(
        CallId { run, no: 1 },
        AfterCall {
            captured_at_ms: 11,
            ..artifact
        },
    );
    store.flushed().await;
    let timeline = store
        .session(&SessionKey::new("private"), Timeline::LastCalls(1))
        .await
        .unwrap()
        .unwrap();
    assert!(timeline.session.withheld, "privacy persisted");
    assert_eq!(
        timeline.calls[0].after.as_ref().unwrap().jpeg,
        None,
        "old and future JPEGs removed"
    );
    assert_eq!(
        store
            .runs(RunQuery {
                id: Some((neo_runtime::IdKey::new("zuzu.task").unwrap(), "t_1".into())),
                ..RunQuery::recent(10)
            })
            .await
            .unwrap()
            .items
            .len(),
        1,
        "host id index finds run"
    );
    assert_eq!(
        store
            .forget_session(&SessionKey::new("private"))
            .await
            .unwrap(),
        1,
        "one erased run"
    );
    assert!(
        !store.knows(&SessionKey::new("private")).await.unwrap(),
        "session gone"
    );
    assert!(
        store.getters(&run).await.unwrap().is_empty(),
        "child rows gone"
    );
}

#[tokio::test]
async fn an_invalid_event_is_isolated_counted_and_does_not_wedge_later_events() {
    use super::support::{exec, inspect};
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let db = inspect(&path).await;
    exec(&db, "CREATE TRIGGER reject_one_call BEFORE INSERT ON trace_calls WHEN NEW.call_no=1 BEGIN SELECT RAISE(ABORT, 'rejected'); END;").await;
    let start = started("isolation");
    let run = start.run.id;
    let capture = store.sink().run(&start).unwrap();
    capture.emit(&after(run, 1));
    capture.emit(&after(run, 2));
    capture.emit(&ended(run, 2));
    capture.emit(&RunEvent::RunSettled { run });
    time::timeout(Duration::from_secs(10), store.flushed())
        .await
        .unwrap();
    assert_eq!(store.health().writes_dropped, 1, "one isolated bad event");
    let row = store.run(&run).await.unwrap().unwrap();
    assert_eq!(
        row.summary.loss.dropped, 1,
        "writer loss is visible on its run"
    );
    assert_eq!(
        store
            .calls(
                &run,
                CallQuery {
                    after: None,
                    limit: 10
                }
            )
            .await
            .unwrap()
            .items
            .iter()
            .map(|call| call.id.no)
            .collect::<Vec<_>>(),
        [2],
        "later valid calls drain"
    );
}

#[tokio::test]
async fn reserve_stubs_preserve_call_and_getter_outcomes_and_settlement_losses() {
    let (_dir, path) = scratch();
    let mut config = StoreConfig::new(path);
    config.queue_bytes = 0;
    let store = TraceStore::open(config).await.unwrap();
    let start = started("reserve");
    let run = start.run.id;
    let capture = store.sink().run(&start).unwrap();
    capture.emit(&after(run, 1));
    capture.emit(&getter(run, 1));
    capture.emit(&ended(run, 1));
    capture.emit(&RunEvent::RunSettled { run });
    store.flushed().await;
    let row = store.run(&run).await.unwrap().unwrap();
    assert!(
        row.summary.loss.stubbed >= 3,
        "absolute reserve losses persisted at settlement"
    );
    let call = store
        .calls(
            &run,
            CallQuery {
                after: None,
                limit: 10,
            },
        )
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(
        (call.state, call.detail),
        (CallState::Ok, crate::Detail::Stub),
        "stub retains the call outcome"
    );
    assert_eq!(
        store.getters(&run).await.unwrap()[0].detail,
        crate::Detail::Stub,
        "getter stub"
    );
}

#[tokio::test]
async fn a_late_artifact_cannot_recreate_an_erased_run() {
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(path)).await.unwrap();
    let start = started("erased");
    let run = start.run.id;
    let capture = store.sink().run(&start).unwrap();
    capture.emit(&ended(run, 0));
    capture.emit(&RunEvent::RunSettled { run });
    store.flushed().await;
    store
        .forget_session(&SessionKey::new("erased"))
        .await
        .unwrap();
    store.attach_after_call(
        CallId { run, no: 1 },
        AfterCall {
            tab_id: None,
            url: None,
            title: None,
            jpeg: Some(vec![1]),
            captured_at_ms: 1,
        },
    );
    store.flushed().await;
    assert!(
        store.run(&run).await.unwrap().is_none(),
        "late capture cannot resurrect a forgotten run"
    );
}

#[tokio::test]
async fn a_pre_run_privacy_tombstone_survives_retention_and_the_first_start() {
    use super::support::inspect;
    use crate::{Retention, retention, writer};
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let key = SessionKey::new("pre-private");
    store.withhold(&key);
    store.flushed().await;
    let db = inspect(&path).await;
    // A real cleanup turn (not an empty no-op) must preserve a session tombstone with no runs.
    let old = started("old");
    let old_run = old.run.id;
    let capture = store.sink().run(&old).unwrap();
    capture.emit(&ended(old_run, 0));
    capture.emit(&RunEvent::RunSettled { run: old_run });
    store.flushed().await;
    while retention::step(
        &db,
        Retention {
            max_age: Duration::ZERO,
            max_bytes: u64::MAX,
        },
        writer::now() + 1,
    )
    .await
    .unwrap()
    {}
    let start = started("pre-private");
    let run = start.run.id;
    store.sink().run(&start).unwrap().emit(&after(run, 1));
    store.attach_after_call(
        CallId { run, no: 1 },
        AfterCall {
            tab_id: None,
            url: None,
            title: None,
            jpeg: Some(vec![1]),
            captured_at_ms: 1,
        },
    );
    store.flushed().await;
    let session = store.session(&key, Timeline::All).await.unwrap().unwrap();
    assert!(session.session.withheld, "privacy survives the first run");
    assert_eq!(
        session.calls[0].after.as_ref().unwrap().jpeg,
        None,
        "pre-run policy denies JPEG"
    );
}
