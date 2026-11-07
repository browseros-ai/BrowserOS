//! Neo: shared assertions at the Runtime/store boundary. Only public store views are used;
//! closing before reopening proves runtime settlement reached disk rather than a read cache.

#![cfg(test)]

use std::path::Path;
use std::time::Duration;

use neo_runtime::{CallResult, RunEnd, RunId, RunOutcome, RunStatus, Runtime};
use neo_trace_store::{
    CallQuery, CallState, CallTrace, Loss, OverCap, RunState, RunTrace, SettledBy, StoreConfig,
    StoreHealth, TraceStore, Unfinished,
};
use tokio::time;

/// Neo: test cleanup is bounded independently of run and call deadlines.
pub const BUDGET: Duration = Duration::from_secs(10);

pub fn assert_clean(health: &StoreHealth) {
    assert!(health.closed, "close completed: {health:?}");
    assert_eq!(health.queued_events, 0);
    assert_eq!(health.queued_bytes, 0);
    assert_eq!(health.unflushed_events, 0);
    assert_eq!(health.unflushed_bytes, 0);
    assert_eq!(health.events_stubbed, 0);
    assert_eq!(health.events_dropped, 0);
    assert_eq!(health.settles_lost, 0);
    assert_eq!(health.writes_dropped, 0);
    assert_eq!(health.artifacts_dropped, 0);
    assert_eq!(health.failing_since_ms, None);
}

pub async fn reopen(runtime: &Runtime, traces: TraceStore, path: &Path) -> TraceStore {
    // Outcomes can precede writer acknowledgements and dropped-future cleanup. Draining the
    // runtime first is essential: closing ingest first could discard those final events.
    time::timeout(BUDGET, runtime.drain()).await.unwrap();
    assert_clean(&traces.close(BUDGET).await);
    drop(traces);
    TraceStore::open(StoreConfig::new(path)).await.unwrap()
}

pub async fn complete(traces: &TraceStore, id: RunId) -> (RunTrace, Vec<CallTrace>) {
    let run = traces.run(&id).await.unwrap().unwrap();
    assert!(run.summary.ended_at_ms.is_some());
    assert_ne!(run.summary.state, RunState::Running);
    assert_eq!(run.summary.settled.unwrap().by, SettledBy::Runtime);
    assert_eq!(run.summary.loss, Loss::default());
    assert_eq!(run.summary.over_cap, OverCap::default());
    assert_eq!(run.summary.unreadable_calls, 0);
    let calls = traces
        .calls(
            &id,
            CallQuery {
                after: None,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert!(!calls.more);
    assert_eq!(calls.items.len() as u64, run.summary.calls_made);
    assert_eq!(
        calls
            .items
            .iter()
            .map(|call| call.id.no)
            .collect::<Vec<_>>(),
        (1..=run.summary.calls_made).collect::<Vec<_>>()
    );
    for call in &calls.items {
        assert_eq!(call.id.run, id);
        assert!(call.settled);
        assert!(!matches!(call.state, CallState::Running | CallState::Lost));
    }
    (run, calls.items)
}

pub fn matches_outcome(run: &RunTrace, calls: &[CallTrace], outcome: &RunOutcome) {
    let state = match outcome.status {
        RunStatus::Ok => RunState::Ok,
        RunStatus::Error => RunState::Error,
        RunStatus::Timeout => RunState::Timeout,
        RunStatus::Cancelled => RunState::Cancelled,
    };
    assert_eq!(run.summary.id, outcome.run_id);
    assert_eq!(run.summary.state, state);
    assert_eq!(calls.len(), outcome.calls.len());
    for (row, record) in calls.iter().zip(&outcome.calls) {
        assert_eq!(row.id.no, record.call_id);
        assert_eq!(row.method, record.method);
        assert_eq!(row.object, record.guid);
        assert_eq!(row.params, record.params);
        assert_eq!(row.delivery, record.delivery);
        assert_eq!(row.timeout_ms, record.timeout_ms);
        match &record.result {
            CallResult::Ok => assert_eq!(row.state, CallState::Ok),
            CallResult::Error { name, .. } => {
                assert_eq!(row.state, CallState::Error);
                assert_eq!(row.error.as_ref().map(|error| &error.name), Some(name));
            }
            CallResult::Unfinished { reason } => {
                assert_eq!(row.state, CallState::Unfinished);
                let reason = match reason {
                    RunEnd::Settled => Unfinished::Settled,
                    RunEnd::Timeout => Unfinished::Timeout,
                    RunEnd::Cancelled => Unfinished::Cancelled,
                };
                assert_eq!(row.unfinished, Some(reason));
            }
        }
    }
}
