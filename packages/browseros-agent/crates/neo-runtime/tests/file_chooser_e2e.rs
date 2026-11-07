//! File chooser interception on a real BrowserOS neo (`src/page_events.rs`
//! `intercept_file_chooser`): a run that listens for file choosers gets one for a click, and once
//! the run ends no session of it intercepts the page's choosers any more.
//!
//! The fixture's `Runtime` shares the test's CDP client, so the test sees every session's
//! `Page.fileChooserOpened` on it, the run's included.

#[expect(
    dead_code,
    reason = "the shared fixture also provides navigation helpers"
)]
#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use browseros_cdp::CdpEvent;
use neo_runtime::{PageId, RunOutcome, RunRequest, RunStatus, Runtime};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

const PAGE: &str = r#"<!doctype html><html><body><input type="file" id="single"></body></html>"#;

/// How long the test waits for a `Page.fileChooserOpened` that must not come.
const QUIET: Duration = Duration::from_millis(700);

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: vec![("/chooser.html".into(), PAGE.into())],
                ..Fixture::default()
            },
            "/chooser.html",
        )
        .await,
    )
}

fn value(outcome: &RunOutcome) -> &Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    outcome.value.as_ref().unwrap_or(&Value::Null)
}

/// Clicks the input with a user gesture over the test's own session.
async fn click_input(tab: &Tab) {
    tab.cdp
        .send(
            "Runtime.evaluate",
            json!({ "expression": "document.querySelector('#single').click()", "userGesture": true }),
            Some(&tab.driver),
        )
        .await
        .unwrap_or_else(|error| panic!("click the input: {error}"));
}

/// Whether `events` reports a file chooser within `within`, on any session.
async fn chooser_reported(events: &mut broadcast::Receiver<CdpEvent>, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    loop {
        match time::timeout_at(deadline, events.recv()).await {
            Err(_) => return false,
            Ok(Ok(event)) if event.method == "Page.fileChooserOpened" => return true,
            Ok(_) => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_file_chooser_interception_ends_with_the_run() {
    let Some(tab) = open().await else {
        return;
    };
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    let outcome = runtime
        .run(RunRequest {
            code: r"const [chooser] = await Promise.all([page.waitForEvent('filechooser'), page.click('#single')])
return [chooser.isMultiple(), await chooser.element().evaluate((input) => input.id)]"
                .to_string(),
            page: PageId::new(tab.target_id.clone()),
            timeout: Duration::from_secs(30),
            cancel: CancellationToken::new(),
            artifacts: None,
        })
        .await;
    assert_eq!(
        value(&outcome),
        &json!([false, "single"]),
        "the run got its chooser"
    );

    // The run's session detaches in the background: once it has, a click is intercepted by
    // nobody (headless BrowserOS then shows no dialog).
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut quiet = false;
    while !quiet && Instant::now() < deadline {
        let mut events = tab.cdp.events();
        click_input(&tab).await;
        quiet = !chooser_reported(&mut events, QUIET).await;
    }
    assert!(
        quiet,
        "no session of the run intercepts the page's file choosers"
    );

    // The click does open choosers: a session that intercepts them gets one.
    tab.cdp
        .send("Page.enable", json!({}), Some(&tab.driver))
        .await
        .unwrap_or_else(|error| panic!("Page.enable: {error}"));
    tab.cdp
        .send(
            "Page.setInterceptFileChooserDialog",
            json!({ "enabled": true }),
            Some(&tab.driver),
        )
        .await
        .unwrap_or_else(|error| panic!("intercept: {error}"));
    let mut events = tab.cdp.events();
    click_input(&tab).await;
    assert!(
        chooser_reported(&mut events, Duration::from_secs(5)).await,
        "the click opens a chooser an intercepting session sees"
    );
    tab.close().await;
}
