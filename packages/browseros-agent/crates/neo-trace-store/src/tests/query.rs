//! Neo: Large session reads retain every run and its host ids without exceeding SQLite's binds.

use std::time::Duration;

use neo_runtime::{IdKey, SessionKey};

use super::support::{exec, inspect, scratch};
use crate::{StoreConfig, Timeline, TraceStore};

#[tokio::test]
async fn session_over_sqlite_bind_limit_preserves_run_order_and_ids() {
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let db = inspect(&path).await;
    // Hex digits are also valid ULID digits. One CTE avoids 33,000 runtime events or inserts.
    exec(&db, "INSERT INTO trace_sessions (session_key, runs, last_activity_ms) VALUES ('large', 33000, 1)").await;
    exec(&db, "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<33000) INSERT INTO trace_runs (run_id, session_key, started_at_ms, status, settled_at_ms, settled_by) SELECT printf('%026X', x), 'large', 1, 'ok', 1, 'runtime' FROM n").await;
    exec(&db, "INSERT INTO trace_run_ids (run_id, key, value) SELECT run_id, 'zuzu.task', run_id FROM trace_runs").await;
    let timeline = store
        .session(&SessionKey::new("large"), Timeline::LastCalls(1))
        .await;
    assert!(timeline.is_ok(), "large session read failed: {timeline:?}");
    let timeline = timeline.unwrap().unwrap();
    assert_eq!(timeline.runs.len(), 33_000);
    let key = IdKey::new("zuzu.task").unwrap();
    for (index, run) in timeline.runs.iter().enumerate() {
        let expected = format!("{:026X}", index + 1);
        assert_eq!(run.id.to_string(), expected);
        assert_eq!(run.ids.get(&key), Some(&expected));
    }
    assert!(timeline.calls.is_empty());
    store.close(Duration::from_secs(5)).await;
}
