//! The public run outcome waits for emulation cleanup, even when CDP cleanup is delayed.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "the browser harness serves other integration tests"
)]
mod browser;
#[path = "support/emulation_gate.rs"]
mod emulation_gate;

use std::time::Duration;

use browser::{Fixture, Tab};
use emulation_gate::{Gate, Hold};
use neo_runtime::{PageId, RunRequest, RunStatus, Runtime};
use serde_json::json;
use tokio::time;
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "multi_thread")]
async fn outcome_waits_for_emulation_restore() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/emulation").await;
    let width = tab.evaluate("innerWidth").await;
    let mut gate = Gate::start(tab.cdp.clone(), Hold::Cleanup).await;
    let runtime = Runtime::connect(gate.cdp.clone()).await.expect("runtime");
    let mut run = tokio::spawn({
        let runtime = runtime.clone();
        let page = PageId::new(tab.target_id.clone());
        async move {
            runtime.run(RunRequest {
                code: "await page.setViewportSize({width:333,height:222}); await page.emulateMedia({media:'print'}); return page.viewportSize()".into(),
                page, timeout: Duration::from_secs(15), cancel: CancellationToken::new(), artifacts: None,
            }).await
        }
    });
    let held = time::timeout(Duration::from_secs(15), gate.held.recv())
        .await
        .expect("cleanup reached relay")
        .expect("cleanup command");
    let during = tab.evaluate("innerWidth").await;
    assert_eq!(
        during,
        json!(333),
        "the relay holds cleanup before the browser sees it"
    );
    assert_eq!(
        tab.evaluate("matchMedia('print').matches").await,
        json!(true)
    );
    let early = time::timeout(Duration::from_millis(200), &mut run)
        .await
        .ok();
    gate.release.cancel();
    assert!(
        early.is_none(),
        "outcome returned while {held} was held; viewport was {during}"
    );
    let outcome = time::timeout(Duration::from_secs(15), run)
        .await
        .expect("run returned")
        .expect("run task");
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!({"width":333,"height":222})));
    assert_eq!(
        tab.evaluate("innerWidth").await,
        width,
        "restored before outcome"
    );
    assert_eq!(
        tab.evaluate("matchMedia('print').matches").await,
        json!(false)
    );
    runtime.drain().await;
    gate.cdp.disconnect().await;
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cleanup_outliving_its_budget_is_retained_for_drain() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/emulation").await;
    let width = tab.evaluate("innerWidth").await;
    let mut gate = Gate::start(tab.cdp.clone(), Hold::Cleanup).await;
    let runtime = Runtime::connect(gate.cdp.clone()).await.expect("runtime");
    let run = tokio::spawn({
        let runtime = runtime.clone();
        let page = PageId::new(tab.target_id.clone());
        async move {
            runtime.run(RunRequest {
                code: "await page.setViewportSize({width:333,height:222}); await page.emulateMedia({media:'print'})".into(),
                page, timeout: Duration::from_secs(15), cancel: CancellationToken::new(), artifacts: None,
            }).await
        }
    });
    time::timeout(Duration::from_secs(15), gate.held.recv())
        .await
        .expect("cleanup started")
        .expect("cleanup command");
    let outcome = time::timeout(Duration::from_secs(15), run)
        .await
        .expect("cleanup has a finite budget")
        .expect("run task");
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        tab.evaluate("innerWidth").await,
        json!(333),
        "cleanup is still held"
    );
    gate.release.cancel();
    time::timeout(Duration::from_secs(15), runtime.drain())
        .await
        .expect("retained cleanup finishes");
    assert_eq!(
        tab.evaluate("innerWidth").await,
        width,
        "drain awaited the late restore"
    );
    assert_eq!(
        tab.evaluate("matchMedia('print').matches").await,
        json!(false)
    );
    gate.cdp.disconnect().await;
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_restores_a_viewport_whose_acknowledgement_is_late() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/emulation").await;
    let width = tab.evaluate("innerWidth").await;
    let mut gate = Gate::start(tab.cdp.clone(), Hold::ViewportReply).await;
    let runtime = Runtime::connect(gate.cdp.clone()).await.expect("runtime");
    let cancel = CancellationToken::new();
    let run = tokio::spawn({
        let runtime = runtime.clone();
        let cancel = cancel.clone();
        let page = PageId::new(tab.target_id.clone());
        async move {
            runtime
                .run(RunRequest {
                    code: "await page.setViewportSize({width:333,height:222})".into(),
                    page,
                    timeout: Duration::from_secs(15),
                    cancel,
                    artifacts: None,
                })
                .await
        }
    });
    time::timeout(Duration::from_secs(15), gate.held.recv())
        .await
        .expect("viewport applied")
        .expect("viewport reply");
    assert_eq!(
        tab.evaluate("innerWidth").await,
        json!(333),
        "the unacknowledged write took effect"
    );
    cancel.cancel();
    let outcome = time::timeout(Duration::from_secs(15), run)
        .await
        .expect("cancelled run returns")
        .expect("run task");
    assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:?}");
    assert_eq!(
        tab.evaluate("innerWidth").await,
        width,
        "a lost acknowledgement still owes a restore"
    );
    gate.release.cancel();
    time::timeout(Duration::from_secs(15), runtime.drain())
        .await
        .expect("drain");
    assert_eq!(
        tab.evaluate("innerWidth").await,
        width,
        "no late resync re-emulates the viewport"
    );
    gate.cdp.disconnect().await;
    tab.close().await;
}
