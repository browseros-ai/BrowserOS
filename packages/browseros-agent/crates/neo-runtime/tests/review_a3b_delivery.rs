//! A source acknowledgement arriving after a drag timeout cannot complete the whole drag.
#[expect(dead_code, reason = "shared fake covers other suites")]
#[path = "support/fake_browser.rs"]
mod fake_browser;

use std::time::Duration;

use fake_browser::{FakeBrowser, Replies};
use neo_runtime::{Delivery, RunStatus};
use tokio_util::sync::CancellationToken;

async fn check(delay: Option<Duration>) {
    let browser = FakeBrowser::start(Replies {
        mouse_press_delay: delay,
        ..Replies::default()
    })
    .await;
    let runtime = browser.runtime().await;
    let outcome = runtime.run(fake_browser::run_request(
        "try { await page.dragAndDrop('#go','#missing',{timeout:500}) } catch (_) {} await new Promise(r => setTimeout(r,1200)); return 'settled'",
        Duration::from_secs(5), CancellationToken::new(),
    )).await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert!(
        browser.exchanges().iter().any(|exchange| {
            exchange.method() == "Input.dispatchMouseEvent"
                && exchange.command["params"]["type"] == "mousePressed"
        }),
        "the source was pressed"
    );
    let call = outcome
        .calls
        .iter()
        .find(|call| call.method == "dragAndDrop")
        .unwrap_or_else(|| panic!("drag call"));
    assert_eq!(
        call.delivery,
        Some(Delivery::Unknown),
        "no target half ran; delay={delay:?}: {call:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_a3b_missing_target_stays_unknown() {
    check(None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_a3b_late_source_ack_stays_unknown() {
    check(Some(Duration::from_millis(800))).await;
}
