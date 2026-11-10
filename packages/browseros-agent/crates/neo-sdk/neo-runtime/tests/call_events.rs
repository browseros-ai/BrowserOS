//! Neo: The live call stream agrees with the outcome without changing the runtime's
//! admission, cancellation or record budgets. The fake browser drives the production run.

#[path = "support/fake_browser.rs"]
#[expect(
    dead_code,
    reason = "this suite exercises call observation, not every browser fixture"
)]
mod fake_browser;

use std::collections::BTreeMap;
use std::time::Duration;

use fake_browser::{FakeBrowser, Replies};
use neo_runtime::{CallEvent, CallRecord, CallResult, RunEnd, RunOutcome, RunStatus};
use tokio::sync::mpsc;
use tokio::time;
use tokio_util::sync::CancellationToken;

async fn observed(code: &str, timeout: Duration) -> (RunOutcome, Vec<CallEvent>) {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let outcome = runtime
        .run_observed(
            fake_browser::run_request(code, timeout, CancellationToken::new()),
            tx,
        )
        .await;
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    assert_eq!(
        time::timeout(Duration::from_millis(100), rx.recv()).await,
        Ok(None),
        "the trace releases the sender when the outcome returns"
    );
    (outcome, events)
}

fn paired(events: &[CallEvent]) -> Vec<CallRecord> {
    let mut started = BTreeMap::new();
    let mut finished = BTreeMap::new();
    let mut last = 0;
    for event in events {
        match event {
            CallEvent::Started(call) => {
                assert!(call.call_id > last, "Started must arrive in call order");
                last = call.call_id;
                assert!(
                    started.insert(call.call_id, call).is_none(),
                    "one Started per call"
                );
            }
            CallEvent::Finished(call) => {
                let Some(start) = started.get(&call.call_id) else {
                    panic!("Started must precede Finished for call {}", call.call_id);
                };
                assert_eq!(
                    (
                        &start.guid,
                        &start.method,
                        &start.params,
                        start.location,
                        start.started_at_ms
                    ),
                    (
                        &call.guid,
                        &call.method,
                        &call.params,
                        call.location,
                        call.started_at_ms
                    ),
                    "the completion preserves the call's identity, parameters and source location"
                );
                assert!(
                    finished.insert(call.call_id, call.clone()).is_none(),
                    "one Finished per call"
                );
            }
        }
    }
    assert_eq!(started.len(), finished.len(), "every Started is paired");
    finished.into_values().collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn live_pairs_equal_the_outcome_without_eviction() {
    let code = "await page.locator('#go').click();\nawait page.title();\npage.click('#missing', {timeout: 20}).catch(() => {});\nawait page.click('#also-missing', {timeout: 60}).catch(() => {});";
    let (outcome, events) = observed(code, Duration::from_secs(10)).await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(paired(&events), outcome.calls);
    assert_eq!(outcome.calls.len(), 4);
    assert_eq!(
        outcome.calls[0].location.as_ref().map(|at| at.line),
        Some(1)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unfinished_calls_pair_on_settlement_and_timeout() {
    for (code, timeout, status, reason) in [
        (
            "page.click('#missing'); return 1",
            Duration::from_secs(5),
            RunStatus::Ok,
            RunEnd::Settled,
        ),
        (
            "await page.click('#missing')",
            Duration::from_millis(300),
            RunStatus::Timeout,
            RunEnd::Timeout,
        ),
    ] {
        let (outcome, events) = observed(code, timeout).await;
        assert_eq!(outcome.status, status, "{outcome:?}");
        assert_eq!(paired(&events), outcome.calls);
        assert_eq!(outcome.calls[0].result, CallResult::Unfinished { reason });
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_record_cap_keeps_one_thousand_pairs() {
    let (outcome, events) = observed(
        "for (let i=0; i<1001; i++) await page.click('#missing', {timeout: 1}).catch(() => {})",
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(paired(&events), outcome.calls);
    assert_eq!(outcome.calls.len(), 1000);
    assert_eq!(outcome.calls_dropped.map(|dropped| dropped.count), Some(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_call_still_has_a_pair() {
    let (outcome, events) = observed(
        "for (let i=0; i<64; i++) page.click('#missing', {timeout: 0}).catch(() => {}); try { await page.click('#go') } catch(e) { return e.message }",
        Duration::from_secs(10),
    ).await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(paired(&events), outcome.calls);
    assert_eq!(outcome.calls.len(), 65);
    assert!(
        matches!(&outcome.calls[64].result, CallResult::Error { message, .. } if message.contains("64 browser calls")),
        "{outcome:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_evicted_call_is_logged_but_not_kept_in_the_outcome() {
    let selector = "x".repeat(600 * 1024);
    let code = format!("await page.click('{selector}', {{timeout: 1}}).catch(() => {{}})");
    let (outcome, events) = observed(&code, Duration::from_secs(10)).await;
    assert_eq!(outcome.status, RunStatus::Ok, "{:?}", outcome.error);
    assert_eq!(
        paired(&events).len(),
        1,
        "the admitted call retains both events"
    );
    assert!(
        outcome.calls.is_empty(),
        "the huge call log evicts the record"
    );
    assert_eq!(outcome.calls_dropped.map(|dropped| dropped.count), Some(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_preserves_live_pairing_and_does_not_wait_for_channel_closure() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let cancel = CancellationToken::new();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let execution = runtime.run_observed(
        fake_browser::run_request(
            "await page.click('#missing')",
            Duration::from_secs(10),
            cancel.clone(),
        ),
        tx,
    );
    tokio::pin!(execution);
    let first = tokio::select! {
        event = rx.recv() => event.expect("a live call event"),
        outcome = &mut execution => panic!("run ended before live event: {outcome:?}"),
    };
    assert!(
        matches!(first, CallEvent::Started(_)),
        "start is observable during execution"
    );
    cancel.cancel();
    let outcome = time::timeout(Duration::from_secs(5), execution)
        .await
        .expect("cancellation completes");
    let mut events = vec![first];
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    assert_eq!(outcome.status, RunStatus::Cancelled);
    assert_eq!(paired(&events), outcome.calls);
    assert_eq!(rx.recv().await, None);
}
