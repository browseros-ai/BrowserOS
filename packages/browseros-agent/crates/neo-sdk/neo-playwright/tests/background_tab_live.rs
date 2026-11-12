//! The product fact behind SS-054 and LH-001, stated as a test: an agent tab opened where the
//! product opens it (`PagePlacement::BackgroundTab`, a background tab of the owner's window)
//! answers mouse input and screenshots at full speed.
//!
//! **It does not on Linux, so there this test is expected to fail.** Focus emulation keeps the
//! background tab "visible", so its renderer submits frames that no window draws, and after
//! more than three of them viz sends that tab one `BeginFrame` a second (`ThrottleUndrawnFrames`
//! in `CompositorFrameSinkSupport::ShouldSendBeginFrame`). A mouse move is rAF-aligned input,
//! acked at the next frame: ~1 s. A screenshot waits for a fresh frame: ~1.2 s headed, possibly
//! never headless. The SDK image hides the input half by accident (its trace camera's per-op
//! captures count as draws), not the screenshots: measured there at ~1.2 s each from the second
//! in a row, 33-82 ms once the same tab is made active. macOS draws background tabs and passes.
//! Evidence: claw-sdk `lead/verify/ss054/NOTES.md`; the browser-side fix it needs:
//! `lead/verify/ss054/CHROMIUM-PROPOSAL.md`. When this passes on Linux, take the mark off.
//!
//! Needs BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p neo-playwright --test background_tab_live -- --ignored`.

#[path = "support/fixture_server.rs"]
#[expect(dead_code, reason = "the nested fixture serves other suites")]
mod fixture_server;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "other live suites use the shared action and policy helpers"
)]
mod live_run;
#[path = "support/locator.rs"]
#[expect(dead_code, reason = "the text, frame and ref parts serve other suites")]
mod locator;

use std::collections::HashMap;
use std::fmt::Debug;
use std::time::{Duration, Instant};

use fixture_server::{FixtureServer, Page};
use live_run::{LiveRun, TestPage, TestResult, brand};
use locator::{css, locator};
use serde_json::{Value, json};

/// Playwright's `input/handle-locator.html` without its interstitial: a place to hover, and a
/// button whose `:hover` style makes every round submit a frame, which is what starts viz's
/// count of undrawn frames.
const PAGE: &str = r"<!DOCTYPE html>
<style>#target:hover { background: yellow; }</style>
<div><button id=target onclick='window.clicked = (window.clicked ?? 0) + 1'>Click me</button></div>
<div id=aside>A place on the side to hover</div>";

/// Hover/click rounds: the throttle starts a few frames in (round 3 to 9 on Linux, measured),
/// so twelve rounds reach it with room to spare.
const ROUNDS: usize = 12;
/// Screenshots in a row: the first is answered from the frame at hand; the next ones wait for
/// fresh frames.
const SHOTS: usize = 3;
/// Above the slowest fast action or capture measured on any platform (macOS: 157 ms) and well
/// below the throttled ones (Linux: 990 ms and up).
const FAST: Duration = Duration::from_millis(500);

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
#[cfg_attr(
    target_os = "linux",
    should_panic(expected = "a background agent tab got slow")
)]
async fn a_background_agent_tab_stays_fast_on_linux() {
    match measure().await {
        Ok(slow) => assert!(
            slow.is_empty(),
            "a background agent tab got slow: {} of {} calls over {FAST:?}: {slow:#?}",
            slow.len(),
            ROUNDS * 2 + SHOTS,
        ),
        Err(error) => panic!("the measurement did not run: {error}"),
    }
}

/// Every hover, click and screenshot on one background agent tab that took longer than
/// [`FAST`] or failed.
async fn measure() -> TestResult<Vec<String>> {
    let run = LiveRun::connect().await?;
    let pages = HashMap::from([("/page.html".to_owned(), Page::html(PAGE))]);
    let server = FixtureServer::start(pages).await?;
    // `context.newPage` on `Playwright::new`: the product's background-tab placement.
    let page = run.new_page().await?;
    let result = async {
        run.invoke(
            "page.goto",
            json!({ "call": { "timeoutMs": 30_000 }, "page": brand(&page),
                    "url": server.url("127.0.0.1", "/page.html"), "waitUntil": "load" }),
        )
        .await?;
        let mut slow = Vec::new();
        for round in 1..=ROUNDS {
            for (op, selector) in [("locator.hover", "#aside"), ("locator.click", "#target")] {
                let input = json!({ "call": { "api": op, "timeoutMs": 10_000 },
                                    "target": at(&page, selector), "strict": "strict" });
                let started = Instant::now();
                let outcome = run.invoke(op, input).await;
                note(
                    &mut slow,
                    &format!("round {round} {op} {selector}"),
                    started,
                    outcome,
                );
            }
        }
        for shot in 1..=SHOTS {
            let input = json!({ "call": { "timeoutMs": 10_000 }, "page": brand(&page) });
            let started = Instant::now();
            let outcome = run.invoke("page.screenshot", input).await;
            note(&mut slow, &format!("screenshot {shot}"), started, outcome);
        }
        Ok(slow)
    }
    .await;
    run.close_page(&page).await;
    result
}

fn at(page: &TestPage, selector: &str) -> Value {
    locator(page, &[css(selector)], &format!("locator('{selector}')"))
}

fn note<E: Debug>(slow: &mut Vec<String>, call: &str, started: Instant, outcome: Result<Value, E>) {
    let took = started.elapsed();
    match outcome {
        Err(error) => slow.push(format!("{call}: failed after {took:?}: {error:?}")),
        Ok(_) if took > FAST => slow.push(format!("{call}: {took:?}")),
        Ok(_) => {}
    }
}
