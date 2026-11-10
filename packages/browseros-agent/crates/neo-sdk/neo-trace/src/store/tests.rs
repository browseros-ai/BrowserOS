//! Neo: Storage lifecycle tests arrange durable state through the real writer queue.
//! These cover crash recovery and age-based cascade deletion without running a browser.

use std::time::{Duration, Instant as WallInstant};

use neo_runtime::{CallRecord, CallResult, Delivery};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde_json::json;
use tempfile::TempDir;
use tokio::time::{self, Instant as TokioInstant};
use ulid::Ulid;

use super::enums::RunState;
use super::writer::Command;
use super::{TraceStore, calls, runs};
use crate::RunId;

fn begin(store: &TraceStore, started_at: i64) -> RunId {
    let id = RunId(Ulid::new());
    store.0.queue.push(Command::Start(runs::Model {
        id: id.to_string(),
        started_at,
        ended_at: None,
        status: RunState::Running,
        page: Some("test-page".to_owned()),
        code: "await page.click('#go')".to_owned(),
        error: None,
        call_count: 0,
        calls_dropped: 0,
        dropped_delivery: None,
    }));
    id
}

fn call() -> CallRecord {
    CallRecord {
        call_id: 1,
        guid: "frame@test-page".to_owned(),
        method: "click".to_owned(),
        title: Some("Click".to_owned()),
        params: json!({"selector":"#go"}),
        location: None,
        started_at_ms: 1,
        duration: Duration::from_millis(5),
        result: CallResult::Ok,
        delivery: Some(Delivery::Unknown),
        log: vec!["clicked".to_owned()],
    }
}

async fn wait_for_call(store: &TraceStore, id: RunId) {
    time::timeout(Duration::from_secs(5), async {
        loop {
            if store
                .run(id)
                .await
                .expect("read trace")
                .is_some_and(|detail| detail.calls.len() == 1)
            {
                break;
            }
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("writer commits the call");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reopening_marks_running_as_interrupted_and_keeps_live_calls_unfinished() {
    let temporary = TempDir::new().expect("temporary state");
    let path = temporary.path().join("traces.db");
    let store = TraceStore::open(&path).await.expect("open");
    let id = begin(&store, super::now_ms());
    store.start_call(
        id,
        neo_runtime::CallStarted {
            call_id: 1,
            guid: "frame@test-page".to_owned(),
            method: "click".to_owned(),
            title: Some("Click".to_owned()),
            params: json!({"selector":"#go"}),
            location: None,
            started_at_ms: 1,
        },
    );
    wait_for_call(&store, id).await;
    store.close().await;
    let reopened = TraceStore::open(&path)
        .await
        .expect("recover interrupted run");
    let detail = reopened.run(id).await.expect("read").expect("run exists");
    assert_eq!(detail.run.status, "interrupted");
    assert!(detail.run.ended_at.is_some());
    assert!(
        detail.calls[0].result.is_none(),
        "crash recovery does not invent a call result"
    );
    reopened.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_prunes_runs_older_than_seven_days_and_cascades_to_calls() {
    let temporary = TempDir::new().expect("temporary state");
    let path = temporary.path().join("traces.db");
    let store = TraceStore::open(&path).await.expect("open");
    let old = begin(&store, super::now_ms() - 8 * 24 * 60 * 60 * 1000);
    let fresh = begin(&store, super::now_ms());
    store.finish_call(old, call());
    store.finish_call(fresh, call());
    wait_for_call(&store, old).await;
    wait_for_call(&store, fresh).await;
    store.close().await;
    let reopened = TraceStore::open(&path).await.expect("prune on open");
    assert!(reopened.run(old).await.expect("old query").is_none());
    assert!(reopened.run(fresh).await.expect("fresh query").is_some());
    let connection = reopened.connection().await.expect("connection");
    assert_eq!(
        calls::Entity::find()
            .filter(calls::Column::RunId.eq(old.to_string()))
            .count(&connection)
            .await
            .expect("count orphan calls"),
        0
    );
    assert_eq!(
        calls::Entity::find()
            .count(&connection)
            .await
            .expect("count fresh calls"),
        1
    );
    reopened.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finished_without_started_upserts_and_later_delivery_changes_only_delivery() {
    let temporary = TempDir::new().expect("temporary state");
    let store = TraceStore::open(&temporary.path().join("traces.db"))
        .await
        .expect("open");
    let id = begin(&store, super::now_ms());
    // Missing Started is a supported queue-overflow case; Finished owns the whole row.
    store.finish_call(id, call());
    wait_for_call(&store, id).await;
    let before = store.run(id).await.expect("read").expect("run");
    assert_eq!(before.calls[0].delivery.as_deref(), Some("unknown"));
    store.update_delivery(id, 1, Some(Delivery::Done));
    let after = time::timeout(Duration::from_secs(5), async {
        loop {
            let detail = store.run(id).await.expect("read").expect("run");
            if detail.calls[0].delivery.as_deref() == Some("done") {
                break detail;
            }
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("delivery patch commits");
    let mut expected = before.calls[0].clone();
    expected.delivery = Some("done".to_owned());
    assert_eq!(
        after.calls[0], expected,
        "delivery patches preserve the finished row"
    );
    store.close().await;
}

#[expect(
    clippy::print_stderr,
    reason = "retention phase evidence distinguishes SQLite waits from the controlled timer advance"
)]
fn retention_phase(phase: &str, wall_start: WallInstant, clock_start: TokioInstant) {
    eprintln!(
        "trace-retention phase={phase} wall_us={} tokio_us={}",
        wall_start.elapsed().as_micros(),
        clock_start.elapsed().as_micros()
    );
}

#[tokio::test]
async fn the_writer_prunes_expired_runs_on_its_hourly_tick() {
    let wall_start = WallInstant::now();
    let clock_start = TokioInstant::now();
    let phase = |name| retention_phase(name, wall_start, clock_start);
    let temporary = TempDir::new().expect("temporary state");
    // The blocking writer inhibits paused-clock auto-advance until close. SQLite
    // setup and visibility polling need real time; pause only for the hourly jump.
    phase("open-before");
    let store = TraceStore::open(&temporary.path().join("traces.db"))
        .await
        .expect("open");
    phase("open-after");
    let old = begin(&store, super::now_ms() - 8 * 24 * 60 * 60 * 1000);
    let fresh = begin(&store, super::now_ms());
    store.finish_call(old, call());
    store.finish_call(fresh, call());
    phase("old-visible-before");
    wait_for_call(&store, old).await;
    phase("old-visible-after");
    phase("fresh-visible-before");
    wait_for_call(&store, fresh).await;
    phase("fresh-visible-after");
    phase("hour-advance-before");
    time::pause();
    time::advance(Duration::from_hours(1)).await;
    time::resume();
    phase("hour-advance-after");
    phase("prune-observation-before");
    time::timeout(Duration::from_secs(5), async {
        loop {
            if store.run(old).await.expect("expired run query").is_none() {
                break;
            }
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the writer's scheduled retention sweep runs");
    phase("prune-observed");
    assert!(store.run(fresh).await.expect("fresh run query").is_some());
    let connection = store.connection().await.expect("connection");
    assert_eq!(
        calls::Entity::find()
            .filter(calls::Column::RunId.eq(old.to_string()))
            .count(&connection)
            .await
            .expect("expired child count"),
        0
    );
    phase("close-before");
    store.close().await;
    phase("close-after");
}
