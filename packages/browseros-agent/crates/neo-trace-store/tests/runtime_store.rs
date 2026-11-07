//! Neo: real Runtime emission through the queued SQLite sink, including exits with no
//! `RunOutcome`. Fake CDP answers isolate run lifetime and persistence from browser mechanics.

#![cfg(test)]

#[path = "../../neo-runtime/tests/support/fake_browser.rs"]
#[expect(
    dead_code,
    reason = "the store tests use only the fake's connection and default page"
)]
mod fake_browser;
mod support;

use std::time::Duration;

use fake_browser::{FakeBrowser, PAGE_URL, Replies, Reply};
use neo_runtime::{EndCause, GetterField, RunOutcome, RunStatus};
use neo_trace_store::{
    CallQuery, CallState, GetterOutcome, RunQuery, RunState, StoreConfig, TraceStore, Unfinished,
};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::time;
use tokio_util::sync::CancellationToken;

/// The run exit being tested; every case starts one call after one live URL getter.
#[derive(Clone, Copy, Debug)]
enum End {
    Ok,
    Error,
    OwnTimeout,
    RunTimeout,
    Cancel,
    Unawaited,
    Dropped,
}

impl End {
    fn expected(self) -> (RunState, EndCause, CallState, Option<Unfinished>) {
        match self {
            End::Ok => (RunState::Ok, EndCause::Returned, CallState::Ok, None),
            End::Error => (RunState::Error, EndCause::Threw, CallState::Error, None),
            End::OwnTimeout => (RunState::Ok, EndCause::Returned, CallState::Error, None),
            End::RunTimeout => (
                RunState::Timeout,
                EndCause::Deadline,
                CallState::Unfinished,
                Some(Unfinished::Timeout),
            ),
            End::Cancel => (
                RunState::Cancelled,
                EndCause::Cancelled,
                CallState::Unfinished,
                Some(Unfinished::Cancelled),
            ),
            End::Unawaited => (
                RunState::Ok,
                EndCause::Returned,
                CallState::Unfinished,
                Some(Unfinished::Settled),
            ),
            End::Dropped => (
                RunState::Cancelled,
                EndCause::Dropped,
                CallState::Unfinished,
                Some(Unfinished::Cancelled),
            ),
        }
    }
}

async fn call_started(traces: &TraceStore) {
    let admitted = time::timeout(support::BUDGET, async {
        loop {
            let runs = traces.runs(RunQuery::recent(1)).await.unwrap();
            if let Some(run) = runs.items.first() {
                let calls = traces
                    .calls(
                        &run.id,
                        CallQuery {
                            after: None,
                            limit: 1,
                        },
                    )
                    .await
                    .unwrap();
                if !calls.items.is_empty() {
                    return;
                }
            }
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(
        admitted.is_ok(),
        "call not admitted: health={:?}, runs={:?}",
        traces.health(),
        traces.runs(RunQuery::recent(2)).await
    );
}

async fn check(end: End) {
    let replies = if matches!(end, End::OwnTimeout) {
        Replies {
            mouse: Reply {
                delay: Duration::from_millis(700),
                ..Reply::default()
            },
            ..Replies::default()
        }
    } else {
        Replies::default()
    };
    let browser = FakeBrowser::start(replies).await;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("traces.sqlite");
    let traces = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let runtime = browser.runtime().await.with_sinks([traces.sink()]);
    let cancel = CancellationToken::new();
    let code = match end {
        End::Ok => "await page.click('#go')",
        End::Error => "await page.fill('#go', 'text')",
        End::OwnTimeout => {
            "try { await page.click('#go', {timeout:100}) } catch(e) { return e.name }"
        }
        End::RunTimeout | End::Cancel | End::Dropped => "await page.click('#absent')",
        End::Unawaited => "page.click('#absent').catch(() => {}); return 8",
    };
    let request = fake_browser::run_request(
        &format!("page.url(); {code}"),
        if matches!(end, End::RunTimeout) {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(30)
        },
        cancel.clone(),
    );
    // Poll the realm on its own host task, as the server does. The admission observer must
    // not share that task with a synchronous getter's block_in_place handoff.
    let (drop_tx, drop_rx) = oneshot::channel();
    let pending = tokio::spawn({
        let runtime = runtime.clone();
        async move {
            if !matches!(end, End::Dropped) {
                return Some(runtime.run(request).await);
            }
            let run = runtime.run(request);
            tokio::pin!(run);
            tokio::select! {
                result = &mut run => panic!("run ended before interruption: {result:?}"),
                signal = drop_rx => { signal.unwrap(); }
            }
            assert!(time::timeout(Duration::from_millis(10), run).await.is_err());
            None
        }
    });
    if matches!(end, End::Cancel | End::Dropped) {
        // Observe actual admission before interruption, rather than relying on startup speed.
        call_started(&traces).await;
        if matches!(end, End::Cancel) {
            cancel.cancel();
        } else {
            drop_tx.send(()).unwrap();
        }
    }
    let outcome: Option<RunOutcome> = pending.await.unwrap();
    let reopened = support::reopen(&runtime, traces, &path).await;
    let runs = reopened.runs(RunQuery::recent(2)).await.unwrap();
    assert_eq!(runs.items.len(), 1, "{end:?}");
    let (run, calls) = support::complete(&reopened, runs.items[0].id).await;
    assert_eq!(run.summary.calls_made, 1, "{end:?}");
    let (state, cause, call_state, unfinished) = end.expected();
    assert_eq!(run.summary.state, state, "{end:?}");
    assert_eq!(run.summary.end_cause, Some(cause), "{end:?}");
    assert_eq!(calls[0].state, call_state, "{end:?}");
    assert_eq!(calls[0].unfinished, unfinished, "{end:?}");
    if matches!(end, End::OwnTimeout) {
        assert_eq!(calls[0].error.as_ref().unwrap().name, "TimeoutError");
        assert_eq!(calls[0].delivery, Some(neo_runtime::Delivery::Done));
        assert_eq!(outcome.as_ref().unwrap().value, Some(json!("TimeoutError")));
    }
    if let Some(outcome) = outcome {
        support::matches_outcome(&run, &calls, &outcome);
        assert_eq!(outcome.status == RunStatus::Ok, state == RunState::Ok);
    }
    let getters = reopened.getters(&run.summary.id).await.unwrap();
    assert_eq!(getters.len(), 1);
    assert_eq!(getters[0].field, GetterField::Url);
    assert_eq!(
        getters[0].outcome,
        GetterOutcome::Ok {
            summary: json!(PAGE_URL)
        }
    );
    assert_eq!(getters[0].page.as_ref().unwrap().tab_id, Some(11));
    support::assert_clean(&reopened.close(support::BUDGET).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn ok_is_persisted() {
    check(End::Ok).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn handler_error_is_persisted() {
    check(End::Error).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn own_timeout_is_persisted() {
    check(End::OwnTimeout).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn run_timeout_is_persisted() {
    check(End::RunTimeout).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn cancellation_is_persisted() {
    check(End::Cancel).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn unawaited_call_is_persisted() {
    check(End::Unawaited).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn dropped_run_future_is_persisted() {
    check(End::Dropped).await;
}
