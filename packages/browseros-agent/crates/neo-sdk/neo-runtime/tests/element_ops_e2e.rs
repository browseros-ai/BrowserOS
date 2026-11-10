//! B2's behavior in real Neo runs on a throwaway browser profile: the highlight overlays a run
//! removes when it ends, and the exact box of an element inside a bordered out-of-process frame
//! (Playwright's border-box placement, pinned in numbers). Everything Neo shares with Playwright
//! is also compared against it in neo-quickjs-injected's parity tests; `dispatchEvent`'s
//! `eventInit` is Playwright's own now (`dispatchEvent.test.mjs`).
#![expect(
    clippy::disallowed_methods,
    reason = "the test drives an independent CDP session as the browser oracle"
)]

#[expect(
    dead_code,
    reason = "the shared fixture also provides navigation helpers"
)]
#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::RunStatus;
use serde_json::{Value, json};
use tokio::time::{self, Instant};

/// How long a page fact may take to settle before the test fails.
const SETTLE: Duration = Duration::from_secs(15);
/// How often a page fact is read again while it settles.
const POLL: Duration = Duration::from_millis(50);

const OVERLAYS: &str = r#"<!doctype html><html><body>
<button id='target' type='button'>Target</button>
<iframe id='child' srcdoc="<button id='inner' type='button'>Inner</button>"></iframe>
</body></html>"#;

/// The script highlights in both documents, empties the main document's overlay, then waits for
/// the test to add `#done`, so the test reads the overlays while the run is still going.
const HIGHLIGHTS: &str = r"await page.locator('#target').highlight()
await page.frameLocator('#child').locator('#inner').highlight()
await page.locator('#target').hideHighlight()
await page.locator('#done').waitFor({ timeout: 20000 })
return 'done'";

const MAIN_OVERLAYS: &str =
    "document.documentElement.querySelectorAll(':scope > x-pw-glass').length";
const CHILD_OVERLAYS: &str = "document.querySelector('#child').contentDocument\
    .documentElement.querySelectorAll(':scope > x-pw-glass').length";

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_run_removes_its_highlight_overlays_when_it_ends() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![("/overlays.html".into(), OVERLAYS.into())],
            ..Fixture::default()
        },
        "/overlays.html",
    )
    .await;
    let run = tab.start_run(HIGHLIGHTS.to_string()).await;
    // During the run: the child shows its highlight; the main document keeps its emptied
    // overlay, as Playwright's does.
    settled(&tab, CHILD_OVERLAYS, &json!(1)).await;
    settled(&tab, MAIN_OVERLAYS, &json!(1)).await;
    tab.evaluate(
        "document.body.append(Object.assign(document.createElement('p'), { id: 'done', textContent: 'done' }))",
    )
    .await;
    let outcome = run
        .await
        .unwrap_or_else(|error| panic!("run task: {error}"));
    let after = [
        tab.evaluate(MAIN_OVERLAYS).await,
        tab.evaluate(CHILD_OVERLAYS).await,
    ];
    // A later run finds nothing of the first one's to clean.
    let second = tab
        .start_run("return (await page.locator('#target').count())".to_string())
        .await
        .await
        .unwrap_or_else(|error| panic!("second run task: {error}"));
    let after_second = [
        tab.evaluate(MAIN_OVERLAYS).await,
        tab.evaluate(CHILD_OVERLAYS).await,
    ];
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    assert_eq!(outcome.value, Some(json!("done")));
    assert_eq!(
        after,
        [json!(0), json!(0)],
        "the run's end removed both overlays, emptied and shown"
    );
    assert_eq!(second.status, RunStatus::Ok, "{second:#?}");
    assert_eq!(after_second, [json!(0), json!(0)]);
}

/// A cross-site, so out-of-process, frame with a 7px border and 5px padding, and a box at
/// (11, 13) inside it.
const BORDERED: &str = r"<!doctype html><html><body style='margin:0'>
<iframe id='cross' title='Cross' style='position:absolute; left:40px; top:30px; width:300px; height:150px; border:7px solid #888; padding:5px'></iframe>
<script>document.getElementById('cross').src = 'http://localhost:' + location.port + '/cross.html'</script>
</body></html>";

const CROSS: &str = r"<!doctype html><html><body style='margin:0'>
<div id='inner' style='position:absolute; left:11px; top:13px; width:20px; height:10px'></div>
</body></html>";

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_box_in_a_bordered_out_of_process_frame_is_placed_by_its_iframes_border_box() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![
                ("/bordered.html".into(), BORDERED.into()),
                ("/cross.html".into(), CROSS.into()),
            ],
            ..Fixture::default()
        },
        "/bordered.html",
    )
    .await;
    let outcome = tab
        .start_run(
            "return await page.frameLocator('#cross').locator('#inner').boundingBox()".to_string(),
        )
        .await
        .await
        .unwrap_or_else(|error| panic!("run task: {error}"));
    let targets = tab
        .cdp
        .send("Target.getTargets", json!({}), None)
        .await
        .unwrap_or_else(|error| panic!("Target.getTargets: {error}"));
    tab.close().await;

    let out_of_process = targets["targetInfos"].as_array().is_some_and(|targets| {
        targets.iter().any(|target| {
            target["type"] == "iframe"
                && target["url"]
                    .as_str()
                    .is_some_and(|url| url.starts_with("http://localhost:"))
        })
    });
    assert!(out_of_process, "the frame runs out of process: {targets}");
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    // Playwright's `getBoundingBox`: the iframe's border-box origin (40, 30) plus the box's own
    // (11, 13), not where the frame's viewport starts (40 + 7 + 5, 30 + 7 + 5), which input
    // aims by.
    assert_eq!(
        outcome.value,
        Some(json!({ "x": 51, "y": 43, "width": 20, "height": 10 }))
    );
}

/// Reads `expression` in the tab until it is `expected`, or fails after [`SETTLE`].
async fn settled(tab: &Tab, expression: &str, expected: &Value) {
    let deadline = Instant::now() + SETTLE;
    loop {
        let value = tab.evaluate(expression).await;
        if &value == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{expression} stayed {value}, not {expected}"
        );
        time::sleep(POLL).await;
    }
}
