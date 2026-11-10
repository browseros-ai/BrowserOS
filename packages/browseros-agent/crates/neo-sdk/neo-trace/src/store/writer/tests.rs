//! Neo: Exercise queue-loss isolation and initialization rollback through the real SQL writer.
//! Faults interrupt actual `SeaORM` transactions; no replacement SQL model implements the writes.

use sea_orm::{ConnectionTrait, DbBackend, EntityTrait, Statement};
use tempfile::TempDir;
use tokio::sync::mpsc::UnboundedReceiver;
use ulid::Ulid;

use super::{Command, SqliteConnectOptions, SqlitePoolOptions, SqlxSqliteConnector};
use crate::RunId;
use crate::store::enums::{CallState, DeliveryState, RunState};
use crate::store::queue::{Message, Queue, Queued};
use crate::store::{self, TraceStore, calls, runs};

fn start(id: RunId) -> Command {
    Command::Start(runs::Model {
        id: id.to_string(),
        started_at: store::now_ms(),
        ended_at: None,
        status: RunState::Running,
        page: None,
        code: "await page.click('#go')".to_owned(),
        error: None,
        call_count: 0,
        calls_dropped: 0,
        dropped_delivery: None,
    })
}

fn call(id: RunId, seq: i64) -> Command {
    Command::Call {
        row: calls::Model {
            run_id: id.to_string(),
            seq,
            guid: "frame@test-page".to_owned(),
            method: "click".to_owned(),
            title: None,
            params: "{\"selector\":\"#go\"}".to_owned(),
            line: None,
            col: None,
            started_at: store::now_ms(),
            duration_ms: Some(5),
            result: Some(CallState::Ok),
            error: None,
            unfinished_reason: None,
            delivery: Some(DeliveryState::Unknown),
            log: None,
        },
        finished: true,
    }
}

fn end(id: RunId) -> Command {
    Command::End {
        run_id: id.to_string(),
        ended_at: store::now_ms(),
        status: RunState::Ok,
        error: None,
        call_count: 1,
        calls_dropped: 1,
        dropped_delivery: None,
        retained_calls: Some(vec![1]),
    }
}

fn drain(receiver: &mut UnboundedReceiver<Message>) -> Vec<Queued> {
    let mut batch = Vec::new();
    while let Ok(message) = receiver.try_recv() {
        let Message::Write(queued) = message else {
            panic!("test queue must remain open");
        };
        batch.push(*queued);
    }
    batch
}

enum MissingDependency {
    RunForCall,
    RunForEnd,
    CallForDelivery,
}

async fn assert_queue_loss_isolated(missing: MissingDependency) {
    let temporary = TempDir::new().expect("temporary state");
    let path = temporary.path().join("traces.db");
    let database = super::open(&path).await.expect("open database");
    let healthy = RunId(Ulid::new());
    let orphan = RunId(Ulid::new());
    let (queue, mut receiver) = Queue::with_limit(4096);
    queue.push(start(healthy));
    super::write(&database, drain(&mut receiver))
        .await
        .expect("commit the healthy parent before capacity loss");

    // Fill the actual admission budget, then lose two dependencies independently.
    // Draining the holding commands releases capacity before later traffic arrives.
    for _ in 0..100 {
        queue.push(Command::Delivery {
            run_id: "budget-holder".to_owned(),
            seq: 1,
            delivery: None,
        });
    }
    queue.push(start(orphan));
    queue.push(call(healthy, 2));
    let holding = drain(&mut receiver);
    assert!(!holding.is_empty());
    assert!(
        holding.iter().all(|queued| matches!(
            &queued.command,
            Command::Delivery { run_id, .. } if run_id == "budget-holder"
        )),
        "the saturated queue rejects both dependencies"
    );
    drop(holding);

    queue.push(call(healthy, 1));
    queue.push(end(healthy));
    queue.push(match missing {
        MissingDependency::RunForCall => call(orphan, 1),
        MissingDependency::RunForEnd => end(orphan),
        MissingDependency::CallForDelivery => Command::Delivery {
            run_id: healthy.to_string(),
            seq: 2,
            delivery: Some(DeliveryState::Done),
        },
    });
    let batch = drain(&mut receiver);
    assert_eq!(batch.len(), 3, "all post-overflow commands are admitted");
    super::write(&database, batch)
        .await
        .expect("missing dependencies must not roll back healthy accepted writes");
    assert!(
        runs::Entity::find_by_id(orphan.to_string())
            .one(&database)
            .await
            .expect("query missing parent")
            .is_none()
    );
    database.close().await.expect("close writer database");

    let reopened = TraceStore::open(&path).await.expect("reopen");
    let detail = reopened
        .run(healthy)
        .await
        .expect("read")
        .expect("healthy run");
    assert_eq!(
        detail.run.status, "ok",
        "a committed terminal run is not recovered as interrupted"
    );
    assert_eq!(detail.run.call_count, 1);
    assert_eq!(detail.calls.len(), 1);
    assert_eq!(detail.calls[0].seq, 1);
    assert_eq!(detail.calls[0].result.as_deref(), Some("ok"));
    assert_eq!(detail.calls[0].delivery.as_deref(), Some("unknown"));
    reopened.close().await;
}

#[tokio::test]
async fn queue_loss_orphan_call_preserves_healthy_accepted_writes() {
    assert_queue_loss_isolated(MissingDependency::RunForCall).await;
}

#[tokio::test]
async fn queue_loss_orphan_end_preserves_healthy_accepted_writes() {
    assert_queue_loss_isolated(MissingDependency::RunForEnd).await;
}

#[tokio::test]
async fn queue_loss_missing_delivery_preserves_healthy_accepted_writes() {
    assert_queue_loss_isolated(MissingDependency::CallForDelivery).await;
}

#[tokio::test]
async fn terminal_cleanup_and_status_roll_back_together_on_real_database_failure() {
    let temporary = TempDir::new().expect("temporary state");
    let database = super::open(&temporary.path().join("traces.db"))
        .await
        .expect("open database");
    let id = RunId(Ulid::new());
    let (queue, mut receiver) = Queue::with_limit(4096);
    queue.push(start(id));
    queue.push(call(id, 1));
    queue.push(call(id, 2));
    super::write(&database, drain(&mut receiver))
        .await
        .expect("seed live rows");
    // Reject the status update after the real End command deletes its evicted call.
    // Genuine storage errors must still roll that deletion back with the terminal status.
    database.execute(Statement::from_string(DbBackend::Sqlite,
        "CREATE TRIGGER reject_terminal BEFORE UPDATE OF status ON runs BEGIN SELECT RAISE(ABORT, 'injected terminal failure'); END;"))
        .await.expect("install temporary fault");
    queue.push(end(id));
    assert!(super::write(&database, drain(&mut receiver)).await.is_err());
    let run = runs::Entity::find_by_id(id.to_string())
        .one(&database)
        .await
        .expect("query")
        .expect("run");
    assert_eq!(run.status, RunState::Running);
    assert!(
        calls::Entity::find_by_id((id.to_string(), 2))
            .one(&database)
            .await
            .expect("query")
            .is_some()
    );
    database
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            "DROP TRIGGER reject_terminal",
        ))
        .await
        .expect("remove temporary fault");
    queue.push(end(id));
    super::write(&database, drain(&mut receiver))
        .await
        .expect("retry terminal cleanup");
    let run = runs::Entity::find_by_id(id.to_string())
        .one(&database)
        .await
        .expect("query")
        .expect("run");
    assert_eq!(run.status, RunState::Ok);
    assert!(
        calls::Entity::find_by_id((id.to_string(), 2))
            .one(&database)
            .await
            .expect("query")
            .is_none()
    );
    database.close().await.expect("close database");
}

#[tokio::test]
async fn interrupted_first_migration_can_retry_without_partial_schema() {
    let temporary = TempDir::new().expect("temporary state");
    let path = temporary.path().join("traces.db");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .foreign_keys(true),
        )
        .await
        .expect("connect before first initialization");
    let database = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
    // Interrupt the candidate's real migration at its final CREATE TABLE, after it
    // has created runs and its index. Only this test-owned fault table is synthetic.
    database
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TABLE calls (fault TEXT)",
        ))
        .await
        .expect("install temporary DDL fault");
    let error = super::initialize(&database)
        .await
        .expect_err("interrupt migration");
    assert!(
        error.to_string().contains("calls") && error.to_string().contains("already exists"),
        "{error}"
    );
    let after_error = database.query_all(Statement::from_string(DbBackend::Sqlite,
        "SELECT name FROM sqlite_master WHERE name IN ('runs', 'runs_started_at', 'seaql_migrations') ORDER BY name"))
        .await.expect("inspect interrupted initialization");
    database
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            "DROP TABLE calls",
        ))
        .await
        .expect("remove only the injected fault");
    database
        .close()
        .await
        .expect("close interrupted connection");
    let reopened = TraceStore::open(&path)
        .await
        .expect("retry interrupted first migration");
    assert!(
        after_error.is_empty(),
        "DDL and migration bookkeeping must roll back together"
    );
    assert!(
        reopened
            .runs(10)
            .await
            .expect("schema can serve reads")
            .is_empty()
    );
    reopened.close().await;
    let reopened = TraceStore::open(&path)
        .await
        .expect("completed migration reopens normally");
    reopened.close().await;
}
