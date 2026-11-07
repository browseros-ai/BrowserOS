//! Review probes for the real `PageSession` drag-interception cleanup path.
#[expect(dead_code, reason = "shared fake covers other suites")]
#[path = "support/fake_browser.rs"]
mod fake_browser;

use std::time::Duration;

use fake_browser::{FakeBrowser, Replies};
use tokio_util::sync::CancellationToken;

async fn check_cleanup(fail_drag_enable: bool) {
    let browser = FakeBrowser::start(Replies {
        fail_drag_enable,
        ..Replies::default()
    })
    .await;
    let runtime = browser.runtime().await;
    let outcome = runtime
        .run(fake_browser::run_request(
            "await page.mouse.move(10, 10); await page.mouse.down(); await page.mouse.move(20, 20); return 'done'",
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await;
    assert!(
        browser.exchanges().iter().any(|exchange| {
            exchange.method() == "Input.setInterceptDrags"
                && exchange.command["params"]["enabled"] == true
        }),
        "the probe reached interception: {outcome:?}"
    );
    assert!(
        !browser
            .page
            .lock()
            .unwrap_or_else(|error| panic!("page: {error}"))
            .drag_listeners,
        "drag listeners remain after the run; fail_drag_enable={fail_drag_enable}; outcome={outcome:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_a3b_successful_intercept_removes_listeners() {
    check_cleanup(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_a3b_failed_enable_removes_listeners() {
    check_cleanup(true).await;
}
