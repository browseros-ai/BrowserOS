//! Neo: Retention operates on bounded child-row chunks, not a bound per run in the backlog.

use std::time::Duration;

use neo_runtime::RunId;
use sea_orm::{ConnectionTrait, DbBackend, EntityTrait, Statement};

use super::support::{count, exec, inspect, scratch};
use crate::db::entities::trace_runs;
use crate::db::migration::{Migrator, run_pending};
use crate::{Retention, retention};

#[tokio::test]
async fn ten_thousand_runs_and_a_large_run_are_pruned_in_row_bounded_turns() {
    let (_dir, path) = scratch();
    let db = inspect(&path).await;
    run_pending::<Migrator>(&db, None).await.unwrap();
    // Recursive CTEs seed a backlog cheaply; the production sweep must not bind 10,000 ids.
    exec(&db, "INSERT INTO trace_sessions (session_key, runs, last_activity_ms) VALUES ('old', 10000, 1); ").await;
    for n in 1..=10_000 {
        let run = RunId::floor(n);
        db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT INTO trace_runs (run_id, session_key, started_at_ms, status, settled_at_ms, settled_by) VALUES (?, 'old', 1, 'ok', 1, 'runtime')", [run.to_string().into()])).await.unwrap();
    }
    exec(
        &db,
        &format!(
            "UPDATE trace_sessions SET first_run_id='{}' WHERE session_key='old'",
            RunId::floor(1)
        ),
    )
    .await;
    let large = RunId::floor(1).to_string();
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<50000) INSERT INTO trace_calls (run_id, call_no, object, method, started_at_ms, offset_us, state, detail) SELECT ?, x, 'frame@p', 'click', 1, x, 'ok', 'stub' FROM n", [large.into()])).await.unwrap();
    let live = RunId::floor(99_999);
    db.execute(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "INSERT INTO trace_runs (run_id, session_key, started_at_ms) VALUES (?, 'live', 99999)",
        [live.to_string().into()],
    ))
    .await
    .unwrap();
    let policy = Retention {
        max_age: Duration::from_secs(10),
        max_bytes: u64::MAX,
    };
    let mut previous = 50_000;
    for _ in 0..20 {
        let more = retention::step(&db, policy, 110_000).await.unwrap();
        let current = count(&db, "SELECT count(*) FROM trace_calls").await;
        assert!(
            previous - current <= 10_000,
            "at most 10,000 children per transaction"
        );
        previous = current;
        if !more {
            break;
        }
    }
    assert_eq!(previous, 0, "large run drained in several turns");
    assert_eq!(
        count(&db, "SELECT count(*) FROM trace_runs").await,
        1,
        "unsettled run before cutoff stays"
    );
    assert!(
        trace_runs::Entity::find_by_id(live.to_string())
            .one(&db)
            .await
            .unwrap()
            .is_some(),
        "recent unsettled run retained"
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM trace_sessions WHERE session_key='old'"
        )
        .await,
        0,
        "session removed with final run"
    );
    while retention::step(&db, policy, 86_600_000).await.unwrap() {}
    assert_eq!(
        count(&db, "SELECT count(*) FROM trace_runs").await,
        0,
        "day-past-cutoff unsettled run can leave"
    );
}

#[tokio::test]
async fn size_cap_removes_the_oldest_settled_run_first() {
    let (_dir, path) = scratch();
    let db = inspect(&path).await;
    run_pending::<Migrator>(&db, None).await.unwrap();
    let old = RunId::floor(1);
    let new = RunId::floor(2);
    for run in [old, new] {
        db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT INTO trace_runs (run_id, session_key, started_at_ms, settled_at_ms) VALUES (?, 's', 1, 1)", [run.to_string().into()])).await.unwrap();
    }
    assert!(
        retention::step(
            &db,
            Retention {
                max_age: Duration::MAX,
                max_bytes: 0
            },
            100
        )
        .await
        .unwrap(),
        "size requests cleanup"
    );
    assert!(
        trace_runs::Entity::find_by_id(old.to_string())
            .one(&db)
            .await
            .unwrap()
            .is_none(),
        "oldest left first"
    );
    assert!(
        trace_runs::Entity::find_by_id(new.to_string())
            .one(&db)
            .await
            .unwrap()
            .is_some(),
        "newest survives first turn"
    );
}
