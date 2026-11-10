//! Neo: Behavioral persistence checks use the production runtime's existing observer stream.
//! `SQLite` reads are the oracle; the browser double controls cancellation and input delivery.

#[path = "../../neo-runtime/tests/support/fake_browser.rs"]
#[expect(
    dead_code,
    reason = "persistence exercises only part of the shared browser fixture"
)]
mod fake_browser;

use std::time::Duration;

use fake_browser::{FakeBrowser, Replies, Reply};
use neo_runtime::{RunOutcome, RunStatus};
use neo_trace::{RunDetail, RunId, TraceStore};
use tempfile::TempDir;
use tokio::fs;
use tokio::runtime::Builder;
use tokio::time;
use tokio_util::sync::CancellationToken;

#[expect(
    clippy::expect_used,
    reason = "a failed database read or bounded commit wait fails the test"
)]
async fn complete(store: &TraceStore, id: RunId) -> RunDetail {
    time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(detail) = store.run(id).await.expect("read persisted run")
                && detail.run.ended_at.is_some()
            {
                return detail;
            }
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the queued final transaction commits")
}

async fn execute(store: &TraceStore, code: &str) -> (RunId, RunOutcome) {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let result = neo_trace::run(
        &runtime,
        fake_browser::run_request(code, Duration::from_secs(20), CancellationToken::new()),
        Some(store),
    )
    .await;
    runtime.drain().await;
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_creates_the_schema_and_reopening_preserves_the_same_rows() {
    let directory = TempDir::new().expect("temporary state");
    let path = directory.path().join("nested/traces.db");
    let store = TraceStore::open(&path)
        .await
        .expect("create database and migration");
    assert!(
        path.is_file(),
        "opening a missing path creates its SQLite file"
    );
    assert!(store.runs(10).await.expect("runs schema").is_empty());
    let (id, outcome) = execute(&store, "return 42").await;
    assert_eq!(outcome.status, RunStatus::Ok);
    let before = complete(&store, id).await;
    store.close().await;
    let reopened = TraceStore::open(&path).await.expect("idempotent migration");
    assert_eq!(reopened.run(id).await.expect("query"), Some(before));
    let list = reopened.runs(1).await.expect("list without code");
    assert_eq!(list.len(), 1);
    assert!(
        serde_json::to_value(&list[0])
            .expect("serialize summary")
            .get("code")
            .is_none()
    );
    reopened.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn call_errors_keep_the_exact_error_and_call_log() {
    let directory = TempDir::new().expect("temporary state");
    let store = TraceStore::open(&directory.path().join("traces.db"))
        .await
        .expect("open");
    let code = "await page.click('#missing', {timeout: 20})";
    let (id, outcome) = execute(&store, code).await;
    assert_eq!(outcome.status, RunStatus::Error, "{outcome:?}");
    let detail = complete(&store, id).await;
    assert_eq!(detail.code, code);
    assert_eq!(detail.run.status, "error");
    assert_eq!(detail.calls.len(), 1);
    assert_eq!(detail.calls[0].result.as_deref(), Some("error"));
    assert!(
        detail.calls[0]
            .error
            .as_deref()
            .expect("call error")
            .starts_with("TimeoutError: ")
    );
    assert_eq!(detail.calls[0].log, Some(outcome.calls[0].log.join("\n")));
    assert!(!detail.calls[0].log.as_deref().expect("call log").is_empty());
    store.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timed_out_cancelled_and_settled_runs_keep_unfinished_reasons() {
    let directory = TempDir::new().expect("temporary state");
    let store = TraceStore::open(&directory.path().join("traces.db"))
        .await
        .expect("open");
    for (expected, code, timeout) in [
        (
            "timeout",
            "await page.click('#missing')",
            Duration::from_secs(1),
        ),
        (
            "cancelled",
            "await page.click('#missing')",
            Duration::from_secs(20),
        ),
        (
            "settled",
            "page.click('#missing'); return 1",
            Duration::from_secs(20),
        ),
    ] {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = browser.runtime().await;
        let cancel = CancellationToken::new();
        let cancellation = async {
            if expected == "cancelled" {
                loop {
                    let rows = store.runs(1).await.expect("live run list");
                    if let Some(row) = rows.first()
                        && row.ended_at.is_none()
                        && let Some(run) = store
                            .run(row.id.parse().expect("run ULID"))
                            .await
                            .expect("live run")
                        && !run.calls.is_empty()
                    {
                        break;
                    }
                    time::sleep(Duration::from_millis(5)).await;
                }
                cancel.cancel();
            }
        };
        let (result, ()) = tokio::join!(
            neo_trace::run(
                &runtime,
                fake_browser::run_request(code, timeout, cancel.clone()),
                Some(&store)
            ),
            cancellation,
        );
        let (id, outcome) = result;
        assert_eq!(
            outcome.status,
            match expected {
                "timeout" => RunStatus::Timeout,
                "cancelled" => RunStatus::Cancelled,
                _ => RunStatus::Ok,
            },
            "{outcome:?}"
        );
        let detail = complete(&store, id).await;
        assert_eq!(
            detail.run.status,
            if expected == "settled" {
                "ok"
            } else {
                expected
            }
        );
        assert_eq!(detail.calls.len(), 1);
        assert_eq!(detail.calls[0].result.as_deref(), Some("unfinished"));
        assert_eq!(detail.calls[0].unfinished_reason.as_deref(), Some(expected));
        runtime.drain().await;
    }
    store.close().await;
}

#[test]
fn a_live_row_commits_before_a_single_worker_stalled_getter_is_released() {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let directory = TempDir::new().expect("temporary state");
        let store = TraceStore::open(&directory.path().join("traces.db"))
            .await
            .expect("open");
        let browser = FakeBrowser::start(Replies {
            history: Reply {
                answers: 1,
                ..Reply::default()
            },
            ..Replies::default()
        })
        .await;
        let neo = browser.runtime().await;
        let cancel = CancellationToken::new();
        let request = fake_browser::run_request(
            "const pending = page.waitForTimeout(10000); page.url(); await pending;",
            Duration::from_secs(20),
            cancel.clone(),
        );
        let recording = store.clone();
        let pending =
            tokio::spawn(async move { neo_trace::run(&neo, request, Some(&recording)).await });
        let observed = time::timeout(Duration::from_secs(5), async {
            loop {
                let blocked = browser
                    .exchanges()
                    .iter()
                    .filter(|exchange| exchange.method() == "Page.getNavigationHistory")
                    .count()
                    == 2;
                let rows = store.runs(1).await.expect("read live rows");
                if blocked
                    && let Some(row) = rows.first()
                    && let Some(detail) = store
                        .run(row.id.parse().expect("ULID"))
                        .await
                        .expect("live detail")
                    && !detail.calls.is_empty()
                {
                    return detail;
                }
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        let still_running = !pending.is_finished();
        // Always release the controlled stall, even when the old behavior would fail.
        cancel.cancel();
        let (id, outcome) = pending.await.expect("run task");
        let live = observed.expect("a live row is readable before releasing the getter");
        assert!(
            still_running,
            "the row was committed during the controlled stall"
        );
        assert_eq!(live.run.status, "running");
        assert!(live.run.ended_at.is_none());
        assert!(live.calls[0].result.is_none());
        assert!(live.calls[0].duration_ms.is_none());
        assert_eq!(outcome.status, RunStatus::Cancelled);
        assert_eq!(complete(&store, id).await.run.status, "cancelled");
        store.close().await;
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_late_input_acknowledgement_is_reconciled_after_call_end() {
    let directory = TempDir::new().expect("temporary state");
    let store = TraceStore::open(&directory.path().join("traces.db"))
        .await
        .expect("open");
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let recording = store.clone();
    let pending = tokio::spawn(async move {
        neo_trace::run(&runtime, fake_browser::run_request(
            "page.click('#go', { delay: 60000 }); await new Promise(resolve => setTimeout(resolve, 300)); return 1;",
            Duration::from_secs(20), CancellationToken::new()), Some(&recording)).await
    });
    let (id, outcome) = pending.await.expect("run task");
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.calls.len(), 1);
    assert_eq!(outcome.calls[0].delivery, Some(neo_runtime::Delivery::Done));
    let detail = complete(&store, id).await;
    assert_eq!(detail.calls[0].delivery.as_deref(), Some("done"));
    assert_eq!(detail.calls[0].result.as_deref(), Some("unfinished"));
    assert_eq!(
        detail.calls[0].unfinished_reason.as_deref(),
        Some("settled")
    );
    store.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_runtime_record_limit_is_reported_without_replaying_dropped_calls() {
    let directory = TempDir::new().expect("temporary state");
    let store = TraceStore::open(&directory.path().join("traces.db"))
        .await
        .expect("open");
    let (id, outcome) = execute(&store, "for (let i = 0; i < 1001; i++) await page.title()").await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.calls_dropped.map(|dropped| dropped.count), Some(1));
    let detail = complete(&store, id).await;
    assert_eq!(detail.run.call_count, 1000);
    assert_eq!(detail.run.calls_dropped, 1);
    assert_eq!(detail.calls.len(), 1000);
    assert_eq!(detail.calls[0].seq, 1);
    assert_eq!(detail.calls[999].seq, 1000);
    store.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finished_call_evicted_by_the_byte_budget_is_removed_before_the_run_ends() {
    let directory = TempDir::new().expect("temporary state");
    let store = TraceStore::open(&directory.path().join("traces.db"))
        .await
        .expect("open");
    let code = format!(
        "await page.click('{}', {{timeout: 1}}).catch(() => {{}})",
        "x".repeat(600 * 1024)
    );
    let (id, outcome) = execute(&store, &code).await;
    assert_eq!(outcome.status, RunStatus::Ok, "{:?}", outcome.error);
    assert!(
        outcome.calls.is_empty(),
        "the completed record exceeds the runtime byte budget"
    );
    assert_eq!(outcome.calls_dropped.map(|dropped| dropped.count), Some(1));
    let detail = complete(&store, id).await;
    assert_eq!(
        detail.code, code,
        "raw run code is independent of call-record eviction"
    );
    assert!(
        detail.calls.is_empty(),
        "a live Finished row must not survive authoritative eviction"
    );
    assert_eq!(detail.run.call_count, 0);
    assert_eq!(detail.run.calls_dropped, 1);
    store.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_store_or_unwritable_path_does_not_change_a_run_outcome() {
    let directory = TempDir::new().expect("temporary state");
    let store = TraceStore::open(&directory.path().join("traces.db"))
        .await
        .expect("open");
    store.close().await;
    let (id, outcome) = execute(&store, "return 42").await;
    assert_eq!(outcome.status, RunStatus::Ok);
    assert_eq!(outcome.value, Some(serde_json::json!(42)));
    assert!(
        store.run(id).await.is_err(),
        "closing closes the shared database pool"
    );
    let blocked = directory.path().join("regular-file");
    fs::write(&blocked, b"not a directory")
        .await
        .expect("block parent");
    assert!(TraceStore::open(&blocked.join("traces.db")).await.is_err());
    let failing =
        TraceStore::open_lazy(blocked.join("traces.db")).expect("create nonblocking queue");
    let (_, outcome) = execute(&failing, "return 42").await;
    assert_eq!(outcome.status, RunStatus::Ok);
    assert_eq!(outcome.value, Some(serde_json::json!(42)));
    failing.close().await;
}
