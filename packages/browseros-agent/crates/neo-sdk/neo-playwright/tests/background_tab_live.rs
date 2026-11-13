//! The browser fact behind SS-054 and LH-001, stated as a test: an agent tab left in the
//! background of the owner's window answers mouse input and screenshots at full speed. This
//! suite's host brings nothing forward (`OpEnv::bring_forward` does nothing), so it measures the
//! browser, not the SDK, which brings an agent tab forward before input or a capture
//! (neo-server's `AgentFront`).
//!
//! **It does not on Linux, so there this test is expected to fail.** Two separate slow paths:
//! - Input. Focus emulation keeps the background tab "visible", so its renderer submits frames
//!   that no window draws, and after more than three of them viz sends that tab one `BeginFrame`
//!   a second (`ThrottleUndrawnFrames` in `CompositorFrameSinkSupport::ShouldSendBeginFrame`). A
//!   mouse move is rAF-aligned input, acked at the next frame: ~1 s.
//! - Captures. A screenshot waits for a forced frame's presentation: ~1.2 s headed, possibly
//!   never headless. That wait is not the throttle (measured at 1.2 s with no `BeginFrame`
//!   throttled); its cause is not yet found.
//!
//! macOS draws background tabs and passes. Both halves must be fast for this to pass on Linux:
//! a browser patch that only lifts the throttle leaves captures slow. When it passes, take the
//! mark off. `AgentFront`'s activation is not kept for this alone: a background tab's cross-site
//! frames also take focus only from it (neo-server `tests/cross_site_focus_browseros.rs`), and
//! its TODO lists everything that must hold before it goes.
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
use std::time::{Duration, Instant};

use fixture_server::{FixtureServer, Page};
use live_run::{LiveRun, TestPage, TestResult, brand};
use locator::{css, locator};
use neo_playwright::{PlaywrightError, TimedOut};
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
/// The rounds judged: from here on a throttled tab pays ~1 s for every action.
const SETTLED_FROM: usize = 4;
/// Screenshots in a row: the first is answered from the frame at hand; the next ones wait for
/// fresh frames.
const SHOTS: usize = 3;
/// Well above a fast action or capture (macOS: 2-160 ms) and below the throttled ones (Linux:
/// 990 ms and up, or no reply). Judged on medians and on every repeat screenshot, so one call a
/// loaded machine delays does not decide it; a call that fails counts at the time it took.
const FAST: Duration = Duration::from_millis(500);

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
#[cfg_attr(
    target_os = "linux",
    should_panic(expected = "a background agent tab got slow")
)]
async fn a_background_agent_tab_stays_fast_on_linux() {
    let times = match measure().await {
        Ok(times) => times,
        Err(error) => panic!("the measurement did not run: {error}"),
    };
    // Only a Playwright timeout is the slowness measured here (a stalled screenshot); any other
    // failure means the test itself broke, which must not pass for the expected Linux failure.
    let broken: Vec<String> = times
        .iter()
        .filter(|call| call.failed.is_some() && !call.timed_out)
        .map(Call::line)
        .collect();
    assert!(
        broken.is_empty(),
        "the measurement did not run: {}",
        broken.join("\n")
    );
    let settled: Vec<Duration> = times
        .iter()
        .filter(|call| call.round >= SETTLED_FROM && call.round <= ROUNDS)
        .map(|call| call.took)
        .collect();
    let repeat_shots: Vec<Duration> = times
        .iter()
        .filter(|call| call.round > ROUNDS + 1)
        .map(|call| call.took)
        .collect();
    let actions_slow = median(&settled) > FAST;
    let shots_slow = repeat_shots.iter().all(|took| *took > FAST);
    assert!(
        !actions_slow && !shots_slow,
        "a background agent tab got slow (median action from round {SETTLED_FROM}: {:?}, \
         repeat screenshots: {repeat_shots:?}):\n{}",
        median(&settled),
        times.iter().map(Call::line).collect::<Vec<_>>().join("\n"),
    );
}

/// One timed call; screenshots are numbered as rounds after the last hover/click round.
struct Call {
    round: usize,
    what: &'static str,
    took: Duration,
    failed: Option<String>,
    /// The failure was the op's own Playwright timeout: slow, counted at the time it took.
    timed_out: bool,
}

impl Call {
    fn line(&self) -> String {
        let Self {
            round,
            what,
            took,
            failed,
            ..
        } = self;
        match failed {
            Some(error) => format!("{round:>2} {what}: failed after {took:?}: {error}"),
            None => format!("{round:>2} {what}: {took:?}"),
        }
    }
}

/// Every hover, click and screenshot on one background agent tab, timed.
async fn measure() -> TestResult<Vec<Call>> {
    let run = LiveRun::connect().await?;
    let pages = HashMap::from([("/page.html".to_owned(), Page::html(PAGE))]);
    let server = FixtureServer::start(pages).await?;
    // `context.newPage`: a background tab, as core opens every agent tab.
    let page = run.new_page().await?;
    let result = async {
        run.invoke(
            "page.goto",
            json!({ "call": { "timeoutMs": 30_000 }, "page": brand(&page),
                    "url": server.url("127.0.0.1", "/page.html"), "waitUntil": "load" }),
        )
        .await?;
        let mut times = Vec::new();
        for round in 1..=ROUNDS {
            for (op, selector) in [("locator.hover", "#aside"), ("locator.click", "#target")] {
                let input = json!({ "call": { "api": op, "timeoutMs": 10_000 },
                                    "target": at(&page, selector), "strict": "strict" });
                let started = Instant::now();
                let outcome = run.invoke(op, input).await;
                times.push(timed(round, op, started, outcome));
            }
        }
        for shot in 1..=SHOTS {
            let input = json!({ "call": { "timeoutMs": 10_000 }, "page": brand(&page) });
            let started = Instant::now();
            let outcome = run.invoke("page.screenshot", input).await;
            times.push(timed(ROUNDS + shot, "page.screenshot", started, outcome));
        }
        Ok(times)
    }
    .await;
    run.close_page(&page).await;
    result
}

fn at(page: &TestPage, selector: &str) -> Value {
    locator(page, &[css(selector)], &format!("locator('{selector}')"))
}

fn timed(
    round: usize,
    what: &'static str,
    started: Instant,
    outcome: Result<Value, PlaywrightError>,
) -> Call {
    let took = started.elapsed();
    let (failed, timed_out) = match outcome {
        Ok(_) => (None, false),
        Err(error) => (
            Some(format!("{error:?}")),
            error.timed_out() == TimedOut::Yes,
        ),
    };
    Call {
        round,
        what,
        took,
        failed,
        timed_out,
    }
}

fn median(times: &[Duration]) -> Duration {
    let mut sorted = times.to_vec();
    sorted.sort();
    sorted.get(sorted.len() / 2).copied().unwrap_or_default()
}
