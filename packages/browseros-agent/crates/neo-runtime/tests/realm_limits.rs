//! The realm's limits and conversions, through `Runtime::run`, against the fake browser in
//! `support/fake_browser.rs`. No browser needed.

#[path = "support/fake_browser.rs"]
mod fake_browser;

use std::time::{Duration, Instant};

use fake_browser::{FakeBrowser, PAGE_URL, Replies, Reply, run_request};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::time;
use tokio_util::sync::CancellationToken;

/// A script that catches whatever its first `page.url()` throws and returns normally.
const CATCHES_GETTER_ERROR: &str =
    "try { page.url() } catch (error) { return `caught ${error.name}` }";

async fn run(code: &str, timeout: Duration) -> RunOutcome {
    let browser = FakeBrowser::start(Replies::default()).await;
    browser
        .runtime()
        .await
        .run(run_request(code, timeout, CancellationToken::new()))
        .await
}

/// A fake browser that answers the run's history read at start and never the script's
/// `page.url()` after it.
async fn stalled_getter() -> FakeBrowser {
    FakeBrowser::start(Replies {
        history: Reply {
            answers: 1,
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await
}

/// Whether the run stopped inside the script's `page.url()`: the history read at start was
/// answered and the script's read after it never was.
fn stopped_inside_the_getter(browser: &FakeBrowser) -> bool {
    let answered: Vec<bool> = browser
        .exchanges()
        .iter()
        .filter(|exchange| exchange.method() == "Page.getNavigationHistory")
        .map(|exchange| exchange.reply.is_some())
        .collect();
    answered == [true, false]
}

#[tokio::test(flavor = "multi_thread")]
async fn an_infinite_loop_ends_as_a_timeout() {
    let started = Instant::now();
    let outcome = run("while (true) {}", Duration::from_millis(300)).await;

    assert_eq!(outcome.status, RunStatus::Timeout, "{outcome:?}");
    let error = outcome
        .error
        .expect("a timeout carries Playwright's TimeoutError");
    assert_eq!(error.name, "TimeoutError", "{error:?}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the interrupt handler stops running JS at the deadline, took {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_timer_past_the_deadline_ends_as_a_timeout() {
    let started = Instant::now();
    let code = "await new Promise((resolve) => setTimeout(resolve, 60_000)); return 'woke'";
    let outcome = run(code, Duration::from_millis(300)).await;

    assert_eq!(outcome.status, RunStatus::Timeout, "{outcome:?}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a script parked on a timer still ends at the deadline, took {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_ends_the_run_as_cancelled() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let cancel = CancellationToken::new();
    let request = run_request(
        "await new Promise((resolve) => setTimeout(resolve, 60_000))",
        Duration::from_secs(30),
        cancel.clone(),
    );
    let canceller = tokio::spawn(async move {
        time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
    });
    let outcome = runtime.run(request).await;
    canceller.await.unwrap();

    assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_script_that_catches_the_deadline_still_ends_as_a_timeout() {
    let browser = stalled_getter().await;
    let outcome = browser
        .runtime()
        .await
        .run(run_request(
            CATCHES_GETTER_ERROR,
            Duration::from_millis(300),
            CancellationToken::new(),
        ))
        .await;

    assert!(
        stopped_inside_the_getter(&browser),
        "{:?}",
        browser.exchanges()
    );
    assert_eq!(
        outcome.status,
        RunStatus::Timeout,
        "the getter's TimeoutError is catchable, but the run still ran out of time: {outcome:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_script_that_catches_cancellation_still_ends_as_cancelled() {
    let browser = stalled_getter().await;
    let runtime = browser.runtime().await;
    let cancel = CancellationToken::new();
    let request = run_request(
        CATCHES_GETTER_ERROR,
        Duration::from_secs(30),
        cancel.clone(),
    );
    let canceller = tokio::spawn(async move {
        time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();
    });
    let outcome = runtime.run(request).await;
    canceller.await.unwrap();

    assert!(
        stopped_inside_the_getter(&browser),
        "{:?}",
        browser.exchanges()
    );
    assert_eq!(
        outcome.status,
        RunStatus::Cancelled,
        "the getter's cancellation error is catchable, but the caller still cancelled: {outcome:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_syntax_error_is_an_error_named_syntax_error() {
    let outcome = run("return {", Duration::from_secs(5)).await;

    assert_eq!(outcome.status, RunStatus::Error, "{outcome:?}");
    let error = outcome.error.expect("a failed run says why");
    assert_eq!(error.name, "SyntaxError", "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thrown_error_keeps_its_name_and_the_logs_before_it() {
    let code = "console.log('before', { step: 1 }); null.boom";
    let outcome = run(code, Duration::from_secs(5)).await;

    assert_eq!(outcome.status, RunStatus::Error, "{outcome:?}");
    assert_eq!(outcome.logs, ["before {\"step\":1}"], "{outcome:?}");
    let error = outcome.error.expect("a failed run says why");
    assert_eq!(error.name, "TypeError", "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn console_lines_past_the_line_cap_are_counted_not_kept() {
    let code = "for (let line = 0; line < 1005; line++) console.log(`line ${line}`); return 'done'";
    let outcome = run(code, Duration::from_secs(5)).await;

    assert_eq!(outcome.status, RunStatus::Ok, "{:?}", outcome.error);
    assert_eq!(outcome.value, Some(json!("done")));
    let (note, kept) = outcome.logs.split_last().expect("the run logged");
    assert_eq!(kept.len(), 1000, "the first 1000 lines are kept");
    assert_eq!(kept.last().map(String::as_str), Some("line 999"));
    assert_eq!(
        note,
        "neo-runtime dropped the last 5 console lines: a run keeps at most 1000 lines"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn console_text_past_the_byte_cap_is_counted_not_kept() {
    // Two 400 kB lines fit in 1 MiB and a third would not. Logging stops there, so the short
    // line after the big ones is dropped too.
    let code = "const big = 'x'.repeat(400_000);
for (let line = 0; line < 5; line++) console.log(big);
console.log('short');
return 'done'";
    let outcome = run(code, Duration::from_secs(5)).await;

    assert_eq!(outcome.status, RunStatus::Ok, "{:?}", outcome.error);
    assert_eq!(outcome.value, Some(json!("done")));
    let (note, kept) = outcome.logs.split_last().expect("the run logged");
    assert_eq!(kept.len(), 2, "two big lines fit");
    assert!(
        kept.iter().map(String::len).sum::<usize>() <= 1024 * 1024,
        "the kept text fits in 1 MiB"
    );
    assert_eq!(
        note,
        "neo-runtime dropped the last 4 console lines: a run keeps at most 1 MiB of console text"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn return_values_convert_to_json() {
    let cases = [
        (
            "return { a: 1, b: [true, null] }",
            Some(json!({ "a": 1, "b": [true, null] })),
        ),
        (
            "return [1, 'two', { three: 3 }]",
            Some(json!([1, "two", { "three": 3 }])),
        ),
        ("return 'text'", Some(json!("text"))),
        ("return 4.5", Some(json!(4.5))),
        ("return null", Some(Value::Null)),
        ("return undefined", None),
        ("await Promise.resolve()", None),
    ];
    for (code, expected) in cases {
        let outcome = run(code, Duration::from_secs(5)).await;
        assert_eq!(outcome.status, RunStatus::Ok, "{code}: {outcome:?}");
        assert_eq!(outcome.value, expected, "{code}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_page_getters_read_the_browser() {
    let code = "return { url: page.url(), closed: page.isClosed() }";
    let outcome = run(code, Duration::from_secs(5)).await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value,
        Some(json!({ "url": PAGE_URL, "closed": false })),
        "{outcome:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_script_cannot_read_a_page_outside_its_run() {
    let code = "__neo_runtime.get(JSON.stringify({ guid: 'page@another-tab', field: 'url' }))";
    let outcome = run(code, Duration::from_secs(5)).await;

    assert_eq!(outcome.status, RunStatus::Error, "{outcome:?}");
    let error = outcome.error.expect("a failed run says why");
    assert!(
        error.message.contains("page@another-tab"),
        "the bridge names the guid it refused: {error:?}"
    );
}
