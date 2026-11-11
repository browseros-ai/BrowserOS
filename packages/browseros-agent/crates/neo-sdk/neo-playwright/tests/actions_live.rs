//! Pointer actions through `Playwright::invoke`, against a real browser: a click at every level
//! of the nested 4-level fixture (F3), `frameLocator` chains 3 and 4 deep (F8), a frame its
//! cross-site parent must scroll into view first, a covered nested frame (F9), background agent
//! tabs and adopted tabs under the focus lease, a click dropped mid-burst, a moving element, a
//! refused recheck, `confirm()` opened by a click, and `dblclick`, `hover`, `check`, `dragTo`,
//! `focus`, `blur`, `dispatchEvent` and `scrollIntoViewIfNeeded`.
//!
//! What each test asserts is what the page itself saw: every fixture logs the events it
//! received, read from its own main world (an out-of-process frame through a CDP session of
//! the test's own), never through the crate.
//!
//! Needs BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p neo-playwright --test actions_live -- --ignored`. The fixtures are
//! served on `127.0.0.1`, `localhost` and `[::1]`, three sites; every test opens and closes
//! its own tabs.

#[path = "support/fixture_server.rs"]
mod fixture_server;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "the capture refusal and the plain trusted click serve pages_live and events_live"
)]
mod live_run;
#[path = "support/locator.rs"]
#[expect(
    dead_code,
    reason = "the text and ref parts serve frames_live and snapshot_live"
)]
mod locator;

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fixture_server::{FixtureServer, Page, nest_pages};
use live_run::{EnvCall, LiveRun, TestPage, TestResult, brand};
use locator::{attr, css, enter_frame, locator, role};
use neo_playwright::{Answer, ErrorCode, PlaywrightError, Refusal};
use neo_trace::ErrorOwner;
use serde_json::{Value, json};
use tokio::time;

/// The actions fixture under `/actions/`, beside the nested one under `/nest/`.
fn pages() -> HashMap<String, Page> {
    let mut pages = nest_pages();
    for (name, body) in [
        ("page", include_str!("data/actions/page.html")),
        ("next", include_str!("data/actions/next.html")),
        ("cover-top", include_str!("data/actions/cover-top.html")),
        ("cover-l1", include_str!("data/actions/cover-l1.html")),
        ("cover-l2", include_str!("data/actions/cover-l2.html")),
        ("far-top", include_str!("data/actions/far-top.html")),
        ("far-b", include_str!("data/actions/far-b.html")),
        ("far-c", include_str!("data/actions/far-c.html")),
    ] {
        pages.insert(format!("/actions/{name}.html"), Page::html(body));
    }
    pages
}

/// `locator(css)` on `page`.
fn at(page: &TestPage, selector: &str) -> Value {
    locator(page, &[css(selector)], &format!("locator('{selector}')"))
}

/// One action op's input: `strict` locator semantics, Playwright's `timeoutMs`, `options`.
fn action(api: &str, target: &Value, options: Value, timeout_ms: u64) -> Value {
    let mut input = json!({ "call": { "api": api, "timeoutMs": timeout_ms }, "target": target,
                            "strict": "strict" });
    if let Some(fields) = input.as_object_mut() {
        fields.insert("options".to_owned(), options);
    }
    input
}

async fn click(run: &LiveRun, target: &Value, options: Value) -> Result<Value, PlaywrightError> {
    run.invoke(
        "locator.click",
        action("locator.click", target, options, 5000),
    )
    .await
}

async fn goto(run: &LiveRun, page: &TestPage, url: &str) -> TestResult {
    run.invoke(
        "page.goto",
        json!({ "call": { "timeoutMs": 30_000 }, "page": brand(page), "url": url,
                "waitUntil": "load" }),
    )
    .await?;
    Ok(())
}

/// A background tab core opened for the agent (`context.newPage`), on `url`.
async fn agent_tab(run: &LiveRun, url: &str) -> TestResult<TestPage> {
    let page = run.new_page().await?;
    goto(run, &page, url).await?;
    Ok(page)
}

/// What the page logged, in order.
async fn page_log(run: &LiveRun, page: &TestPage) -> TestResult<Vec<String>> {
    strings(run.eval(page, "window.__log").await?)
}

fn strings(value: Value) -> TestResult<Vec<String>> {
    Ok(serde_json::from_value(value)?)
}

/// The host calls an op made, without its notes.
fn host_calls(run: &LiveRun) -> Vec<EnvCall> {
    run.calls()
        .into_iter()
        .filter(|call| !matches!(call, EnvCall::Note(_)))
        .collect()
}

/// The call log lines an op mirrored to the host.
fn notes(run: &LiveRun) -> Vec<String> {
    run.calls()
        .into_iter()
        .filter_map(|call| match call {
            EnvCall::Note(line) => Some(line),
            _ => None,
        })
        .collect()
}

/// `Date.now()`, as the prelude stamps a listener.
fn now_ms() -> TestResult<u64> {
    Ok(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?)
}

/// `frameLocator('[title=…]')` down `titles`, then `last`.
fn down(page: &TestPage, titles: &[&str], last: Value) -> Value {
    let mut parts = Vec::new();
    for title in titles {
        parts.push(attr(&format!("[title=\"{title}\"s]")));
        parts.push(enter_frame());
    }
    parts.push(last);
    locator(page, &parts, &format!("frameLocator chain {titles:?}"))
}

/// The nested fixture's frames: the titles down to each, its button, and how its own main
/// world is reached (`None`: the page's; `Some(url part)`: that out-of-process frame's), with
/// the same-origin path below it.
struct NestFrame {
    titles: &'static [&'static str],
    button: &'static str,
    /// The out-of-process frame that hosts it (`b2`), or the page.
    process: Option<(&'static str, &'static str)>,
    /// From the hosting document to this one, through same-origin iframes.
    below: &'static [&'static str],
}

const NEST: [NestFrame; 8] = [
    NestFrame {
        titles: &[],
        button: "Top button",
        process: None,
        below: &[],
    },
    NestFrame {
        titles: &["a1"],
        button: "A1 button",
        process: None,
        below: &["a1"],
    },
    NestFrame {
        titles: &["b1"],
        button: "B1 button",
        process: Some(("localhost", "/nest/b1.html")),
        below: &[],
    },
    NestFrame {
        titles: &["a1", "b2"],
        button: "B2 button",
        process: Some(("localhost", "/nest/b2.html")),
        below: &[],
    },
    NestFrame {
        titles: &["b1", "c2"],
        button: "C2 button",
        process: Some(("[::1]", "/nest/c2.html")),
        below: &[],
    },
    NestFrame {
        titles: &["a1", "b2", "b3"],
        button: "B3 button",
        process: Some(("localhost", "/nest/b2.html")),
        below: &["b3"],
    },
    NestFrame {
        titles: &["b1", "c2", "a3"],
        button: "A3 button",
        process: Some(("127.0.0.1", "/nest/a3.html")),
        below: &[],
    },
    NestFrame {
        titles: &["b1", "c2", "a3", "c4"],
        button: "C4 button",
        process: Some(("[::1]", "/nest/c4.html")),
        below: &[],
    },
];

/// The presses and the clicks a nested frame's own page logged (`frame.js`), in one read.
async fn nest_log(
    run: &LiveRun,
    server: &FixtureServer,
    page: &TestPage,
    frame: &NestFrame,
) -> TestResult<(Vec<String>, Vec<String>)> {
    let mut path = "window".to_owned();
    for title in frame.below {
        path = format!("{path}.document.querySelector('iframe[title={title}]').contentWindow");
    }
    let expression = format!("[{path}.__state.presses, {path}.__state.clicks]");
    let logged = match frame.process {
        None => run.eval(page, &expression).await?,
        Some((host, file)) => {
            run.frame_eval(page, &server.url(host, file), &expression)
                .await?
        }
    };
    Ok(serde_json::from_value(logged)?)
}

/// Opens the nested fixture once its deepest frame has its button.
async fn nested(run: &LiveRun, server: &FixtureServer) -> TestResult<TestPage> {
    let page = agent_tab(run, &server.url("127.0.0.1", "/nest/top.html")).await?;
    let c4 = down(&page, &["b1", "c2", "a3", "c4"], role("button"));
    let read = json!({ "call": { "timeoutMs": 10_000 }, "target": c4, "strict": "strict" });
    assert_eq!(
        run.invoke("locator.textContent", read).await?,
        json!("C4 button"),
        "the deepest frame loaded"
    );
    Ok(page)
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_each_nested_frame_logs_exactly_one_click_on_its_own_button() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = nested(&run, &server).await?;
    for frame in &NEST {
        let button = down(&page, frame.titles, role("button"));
        click(&run, &button, json!({}))
            .await
            .map_err(|error| format!("{}: {error}", frame.button))?;
    }
    let logged = nest_logs(&run, &server, &page).await?;
    let expected: Vec<_> = NEST
        .iter()
        .map(|frame| (frame.button, only(frame.button), only(frame.button)))
        .collect();
    assert_eq!(
        logged, expected,
        "each frame logged exactly its own press and click, and nothing from another frame"
    );
    run.close_page(&page).await;
    Ok(())
}

/// Every nested frame's `(button, presses, clicks)`, top first.
async fn nest_logs(
    run: &LiveRun,
    server: &FixtureServer,
    page: &TestPage,
) -> TestResult<Vec<(&'static str, Vec<String>, Vec<String>)>> {
    let mut logs = Vec::new();
    for frame in &NEST {
        let (presses, clicks) = nest_log(run, server, page, frame).await?;
        logs.push((frame.button, presses, clicks));
    }
    Ok(logs)
}

/// A log holding `button` once.
fn only(button: &str) -> Vec<String> {
    vec![button.to_owned()]
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_frame_locator_chains_three_and_four_deep_resolve_and_click() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = nested(&run, &server).await?;
    // page.frameLocator('[title="b1"]').frameLocator('iframe').frameLocator('iframe')
    //     .getByRole('button'), and one level more.
    let iframe = || css("iframe");
    let three = locator(
        &page,
        &[
            attr("[title=\"b1\"s]"),
            enter_frame(),
            iframe(),
            enter_frame(),
            iframe(),
            enter_frame(),
            role("button"),
        ],
        "three deep",
    );
    let four = locator(
        &page,
        &[
            attr("[title=\"b1\"s]"),
            enter_frame(),
            iframe(),
            enter_frame(),
            iframe(),
            enter_frame(),
            iframe(),
            enter_frame(),
            role("button"),
        ],
        "four deep",
    );
    click(&run, &three, json!({})).await?;
    click(&run, &four, json!({})).await?;
    let logged = nest_logs(&run, &server, &page).await?;
    let expected: Vec<_> = NEST
        .iter()
        .map(|frame| {
            let target = ["A3 button", "C4 button"].contains(&frame.button);
            let log = if target {
                only(frame.button)
            } else {
                Vec::new()
            };
            (frame.button, log.clone(), log)
        })
        .collect();
    // The first click scrolls B1 and the second C2, each in another process than the button's
    // frame: a press the browser routed by its old layout shows up in an ancestor's presses.
    assert_eq!(
        logged, expected,
        "A3 and C4 each logged exactly one press and one click, and no other frame saw a press"
    );
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_click_its_parent_frame_scrolls_for_reaches_only_the_inner_frame() -> TestResult
{
    click_after_parent_scrolls(json!({}), 4).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_forced_click_its_parent_frame_scrolls_for_reaches_only_the_inner_frame()
-> TestResult {
    // `force` arms no interceptor and waits for no stable rect, so the press would follow the
    // parent's scroll at once, by the browser's old layout, if the route check needed either.
    click_after_parent_scrolls(json!({ "force": true }), 8).await
}

/// `rounds` clicks with `options` on C's button in far-top, each in a fresh tab that B, C's
/// cross-site parent, must scroll first: agent tabs, then tabs core did not open, which click
/// under the focus lease.
async fn click_after_parent_scrolls(options: Value, rounds: usize) -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let url = server.url("127.0.0.1", "/actions/far-top.html");
    for round in 0..rounds {
        // Agent tabs, then tabs core did not open, which click under the focus lease.
        let page = if round % 2 == 0 {
            agent_tab(&run, &url).await?
        } else {
            run.open_adopted(&url).await?
        };
        let button = down(&page, &["b", "c"], role("button"));
        let read = json!({ "call": { "timeoutMs": 10_000 }, "target": button, "strict": "strict" });
        assert_eq!(
            run.invoke("locator.textContent", read).await?,
            json!("C button")
        );
        click(&run, &button, options.clone()).await?;
        let b_url = server.url("localhost", "/actions/far-b.html");
        let b = run.frame_eval(&page, &b_url, "window.__log").await?;
        let c_url = server.url("[::1]", "/actions/far-c.html");
        let c = run.frame_eval(&page, &c_url, "window.__log").await?;
        assert_eq!(
            (b, c),
            (json!([]), json!(["mousedown c-button", "click c-button"])),
            "round {round}: C's button took the press, and B, which scrolled C into view, saw none"
        );
        run.close_page(&page).await;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_covered_nested_frame_fails_not_actionable_naming_the_overlay_unsent()
-> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/cover-top.html")).await?;
    let inner = down(&page, &["l1", "l2"], css("#inner"));
    let read = json!({ "call": { "timeoutMs": 10_000 }, "target": inner, "strict": "strict" });
    assert_eq!(
        run.invoke("locator.textContent", read).await?,
        json!("L2 button")
    );
    run.clear_calls();

    let started = Instant::now();
    let error = run
        .invoke(
            "locator.click",
            action("locator.click", &inner, json!({}), 1500),
        )
        .await
        .err()
        .ok_or("a covered frame must not be clicked")?;
    assert!(
        started.elapsed() >= Duration::from_millis(1400),
        "it waited"
    );
    assert_eq!(
        error.error_code(),
        Some(ErrorCode::NotActionable),
        "{error}"
    );
    assert_eq!(error.owner(), ErrorOwner::Website);
    assert!(
        error
            .message()
            .starts_with("locator.click: Timeout 1500ms exceeded."),
        "{error}"
    );
    assert!(
        error.message().contains("<div id=\"overlay\"")
            && error.message().contains("intercepts pointer events"),
        "the call log names the overlay: {error}"
    );
    assert!(
        !host_calls(&run).contains(&EnvCall::MarkSent),
        "no input was sent: {:?}",
        host_calls(&run)
    );
    let l2 = run
        .frame_eval(
            &page,
            &server.url("[::1]", "/actions/cover-l2.html"),
            "window.__log",
        )
        .await?;
    let l1 = run
        .frame_eval(
            &page,
            &server.url("localhost", "/actions/cover-l1.html"),
            "window.__log",
        )
        .await?;
    assert_eq!(
        (l1, l2),
        (json!([]), json!([])),
        "neither frame saw a press"
    );
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_clicks_land_in_an_agent_tab_and_in_an_adopted_tab_under_a_restored_lease()
-> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let url = server.url("127.0.0.1", "/actions/page.html");

    let agent = agent_tab(&run, &url).await?;
    click(&run, &at(&agent, "#go"), json!({})).await?;
    assert_eq!(
        page_log(&run, &agent).await?,
        [
            "mousedown go visible",
            "mouseup go visible",
            "click go visible"
        ],
        "core keeps focus emulation on in its own tabs"
    );

    let adopted = run.open_adopted(&url).await?;
    assert_eq!(
        run.eval(&adopted, "document.visibilityState").await?,
        json!("hidden"),
        "a background tab the agent did not open is hidden"
    );
    let started = Instant::now();
    click(&run, &at(&adopted, "#go"), json!({})).await?;
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the lease keeps a background click fast: {:?}",
        started.elapsed()
    );
    assert_eq!(
        page_log(&run, &adopted).await?,
        [
            "mousedown go visible",
            "mouseup go visible",
            "click go visible"
        ],
        "the lease emulated focus for the click"
    );
    // Visibility, not hasFocus(): the trusted click itself focused the page (measured).
    time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        run.eval(&adopted, "document.visibilityState").await?,
        json!("hidden"),
        "and gave it back afterwards"
    );
    run.close_page(&agent).await;
    run.close_page(&adopted).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_click_dropped_mid_burst_disarms_releases_and_restores_the_lease() -> TestResult
{
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = run
        .open_adopted(&server.url("127.0.0.1", "/actions/page.html"))
        .await?;
    // A trial click holds its press for 3 s, with its interceptor blocking every event; the
    // host drops it in the middle, as a cancelled run does.
    let twice = at(&page, "#twice");
    let trial = click(&run, &twice, json!({ "trial": true, "delay": 3000 }));
    assert!(
        time::timeout(Duration::from_millis(600), trial)
            .await
            .is_err(),
        "the click was still in its delay"
    );
    assert!(
        host_calls(&run).contains(&EnvCall::MarkSent),
        "it had sent its press"
    );
    time::sleep(Duration::from_millis(200)).await;

    assert_eq!(
        run.eval(&page, "document.visibilityState").await?,
        json!("hidden"),
        "the lease was restored on drop"
    );
    let log = page_log(&run, &page).await?;
    assert!(
        !log.iter().any(|entry| entry.contains("twice")),
        "the owed release went out under the trial's interceptor, so nothing reached the \
         button: {log:?}"
    );
    // Well inside the probe's own expiry, 3 s past the op's 5 s timeout: only the guard's
    // disarm can explain this.
    run.click(&page, "#go").await?;
    let log = page_log(&run, &page).await?;
    assert!(
        log.iter().any(|entry| entry.starts_with("click go")),
        "the interceptor was disarmed, so the page's own clicks reach it: {log:?}"
    );
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_trial_click_whose_delay_outlasts_three_seconds_reaches_nothing() -> TestResult
{
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/page.html")).await?;
    click(
        &run,
        &at(&page, "#go"),
        json!({ "trial": true, "delay": 3500 }),
    )
    .await?;
    assert_eq!(
        page_log(&run, &page).await?,
        Vec::<String>::new(),
        "the interceptor still blocked the release 3.5 s after the press"
    );
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_moving_element_is_clicked_once_stable() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/page.html")).await?;
    run.eval(&page, "startMoving()").await?;
    run.clear_calls();
    click(&run, &at(&page, "#mover"), json!({})).await?;
    let log = page_log(&run, &page).await?;
    assert!(
        log.contains(&"mover stopped=true".to_owned()),
        "clicked only after it stopped: {log:?}"
    );
    assert!(
        notes(&run)
            .iter()
            .any(|line| line == "  element is not stable"),
        "and the call log says it waited: {:?}",
        notes(&run)
    );
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_refused_recheck_sends_nothing() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/page.html")).await?;
    run.clear_calls();
    run.refuse_rechecks(Some(Refusal {
        owner: ErrorOwner::Host,
        code: "owner_has_control".to_owned(),
        message: "The owner is using this tab.".to_owned(),
        host_code: None,
    }));
    let error = click(&run, &at(&page, "#go"), json!({}))
        .await
        .err()
        .ok_or("a refused click must fail")?;
    run.refuse_rechecks(None);
    assert_eq!(
        (error.owner(), error.code(), error.message()),
        (
            ErrorOwner::Host,
            "owner_has_control",
            "The owner is using this tab."
        )
    );
    assert_eq!(host_calls(&run), [EnvCall::Recheck], "nothing marked sent");
    assert!(
        page_log(&run, &page).await?.is_empty(),
        "and nothing reached the page"
    );
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_refused_click_leaves_the_mouse_where_it_was() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/page.html")).await?;
    let (x, y) = center(&run, &page, "#dst").await?;
    device(
        &run,
        "mouse.move",
        &page,
        json!({ "x": x, "y": y, "options": {} }),
    )
    .await?;
    run.refuse_rechecks(Some(Refusal {
        owner: ErrorOwner::Host,
        code: "owner_has_control".to_owned(),
        message: "The owner is using this tab.".to_owned(),
        host_code: None,
    }));
    // Forced, so no interceptor is armed: the refusal leaves nothing behind but the plan.
    let refused = click(&run, &at(&page, "#go"), json!({ "force": true })).await;
    run.refuse_rechecks(None);
    assert!(refused.is_err(), "the recheck refused the click");
    device(&run, "mouse.down", &page, json!({ "options": {} })).await?;
    device(&run, "mouse.up", &page, json!({ "options": {} })).await?;
    assert_eq!(
        page_log(&run, &page).await?,
        [
            "mousedown dst visible",
            "mouseup dst visible",
            "click dst visible"
        ],
        "the press went where the mouse was before the refused click planned its move"
    );
    run.close_page(&page).await;
    Ok(())
}

/// The center of the first element matching `selector`, in the page's CSS pixels.
async fn center(run: &LiveRun, page: &TestPage, selector: &str) -> TestResult<(f64, f64)> {
    let center = run
        .eval(
            page,
            &format!(
                "(() => {{ const r = document.querySelector({selector:?}).getBoundingClientRect(); \
                 return [r.x + r.width / 2, r.y + r.height / 2]; }})()"
            ),
        )
        .await?;
    let coordinate = |index: usize| center.get(index).and_then(Value::as_f64);
    match (coordinate(0), coordinate(1)) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Err(format!("{selector} has no box: {center}").into()),
    }
}

/// A `mouse.*` op on `page`, with `fields` beside the page.
async fn device(
    run: &LiveRun,
    op: &str,
    page: &TestPage,
    fields: Value,
) -> Result<Value, PlaywrightError> {
    let mut input = json!({ "call": { "api": op, "timeoutMs": 5000 }, "page": brand(page) });
    if let (Some(input), Value::Object(fields)) = (input.as_object_mut(), fields) {
        input.extend(fields);
    }
    run.invoke(op, input).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_confirm_opened_by_a_click_is_answered_and_the_click_finishes() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/page.html")).await?;
    let ask = at(&page, "#ask");

    // Nobody listens: Neo dismisses it after its grace, and the click finishes.
    let started = Instant::now();
    click(&run, &ask, json!({})).await?;
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "the dialog held the input until Neo dismissed it"
    );
    assert_eq!(
        page_log(&run, &page).await?.last().map(String::as_str),
        Some("confirm false")
    );

    // A listener accepts it while the click waits on it.
    run.invoke(
        "events.listen",
        json!({ "page": brand(&page), "event": "dialog", "on": true, "since": now_ms()? }),
    )
    .await?;
    let answered = async {
        let pumped = run.invoke("events.next", json!({})).await?;
        run.invoke(
            "dialog.accept",
            json!({ "page": brand(&page), "dialog": pumped["events"][0]["dialog"]["id"] }),
        )
        .await
    };
    let (clicked, answered) = tokio::join!(click(&run, &ask, json!({})), answered);
    answered?;
    clicked?;
    assert_eq!(
        page_log(&run, &page).await?.last().map(String::as_str),
        Some("confirm true")
    );

    // A listener that never answers: the click ends at its timeout, ambiguous, naming why.
    let unanswered = run
        .invoke(
            "locator.click",
            action("locator.click", &ask, json!({}), 1500),
        )
        .await
        .err()
        .ok_or("a dialog nobody answers must not let the click finish")?;
    assert_eq!(unanswered.answer(), Answer::Ambiguous, "{unanswered}");
    assert!(
        unanswered.message().contains("a confirm dialog is open"),
        "{unanswered}"
    );
    let pumped = run.invoke("events.next", json!({})).await?;
    run.invoke(
        "dialog.dismiss",
        json!({ "page": brand(&page), "dialog": pumped["events"][0]["dialog"]["id"] }),
    )
    .await?;
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_drag_to_drops_one_element_on_another() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/page.html")).await?;
    let input = json!({ "call": { "api": "locator.dragTo", "timeoutMs": 5000 },
                        "target": at(&page, "#src"), "strict": "strict",
                        "to": at(&page, "#dst"), "options": { "steps": 5 } });
    run.invoke("locator.dragTo", input).await?;
    let log = page_log(&run, &page).await?;
    assert!(
        log.contains(&"dragstart src".to_owned()) && log.contains(&"drop dst payload".to_owned()),
        "Chromium carried an HTML5 drag from plain mouse moves: {log:?}"
    );
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_drag_to_a_missing_destination_lets_the_button_go() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = drag_page(&run, &server).await?;
    let missing = run
        .invoke("locator.dragTo", drag_to_missing(&page, &json!({}), 1000))
        .await
        .err()
        .ok_or("a drop target that never appears fails the drag")?;
    assert_eq!(missing.error_code(), Some(ErrorCode::NoMatch), "{missing}");
    move_over_dst(&run, &page).await?;
    click(&run, &at(&page, "#go"), json!({})).await?;
    let log = page_log(&run, &page).await?;
    let index = |wanted: &str| log.iter().position(|entry| entry == wanted);
    assert!(
        log.iter()
            .filter(|entry| entry.starts_with("move "))
            .all(|entry| entry == "move buttons=0"),
        "no move after the failed drag found the left button down: {log:?}"
    );
    assert!(
        matches!(
            (index("mouseup src visible"), index("mousedown go visible")),
            (Some(up), Some(down)) if up < down
        ),
        "the drag let go where it pressed, before the next input: {log:?}"
    );
    for expected in [
        "mousedown go visible",
        "mouseup go visible",
        "click go visible",
    ] {
        assert!(
            index(expected).is_some(),
            "the next click works: {expected} in {log:?}"
        );
    }
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_drag_the_host_drops_lets_the_button_go() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = drag_page(&run, &server).await?;
    // Dropped while it waits for its drop target, as a cancelled run drops it.
    let waiting = run.invoke("locator.dragTo", drag_to_missing(&page, &json!({}), 5000));
    assert!(
        time::timeout(Duration::from_millis(600), waiting)
            .await
            .is_err(),
        "the drag was still waiting for #missing"
    );
    move_over_dst(&run, &page).await?;
    let log = page_log(&run, &page).await?;
    assert!(
        log.iter().any(|entry| entry == "mouseup src visible"),
        "the dropped drag let go where it pressed: {log:?}"
    );
    assert_eq!(
        log.last().map(String::as_str),
        Some("move buttons=0"),
        "before the next move: {log:?}"
    );
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_trial_drag_to_a_missing_destination_clicks_nothing() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = drag_page(&run, &server).await?;
    let trial = drag_to_missing(&page, &json!({ "trial": true }), 1000);
    let missing = run
        .invoke("locator.dragTo", trial)
        .await
        .err()
        .ok_or("a trial drag to a missing target fails too")?;
    assert_eq!(missing.error_code(), Some(ErrorCode::NoMatch), "{missing}");
    move_over_dst(&run, &page).await?;
    assert_eq!(
        page_log(&run, &page).await?,
        ["move buttons=0", "move buttons=0"],
        "the page saw the trial's approach and the next move, with the button up, and no \
         press, release, click or drag: they went out under the press's block-all interceptor"
    );
    run.close_page(&page).await;
    Ok(())
}

/// The actions fixture, with every mouse move logged with the buttons it saw held.
async fn drag_page(run: &LiveRun, server: &FixtureServer) -> TestResult<TestPage> {
    let page = agent_tab(run, &server.url("127.0.0.1", "/actions/page.html")).await?;
    run.eval(
        &page,
        "document.addEventListener('mousemove', (e) => __log.push(`move buttons=${e.buttons}`), \
         true)",
    )
    .await?;
    Ok(page)
}

/// `dragTo` from `#src` onto `#missing`, which never appears.
fn drag_to_missing(page: &TestPage, options: &Value, timeout_ms: u64) -> Value {
    json!({ "call": { "api": "locator.dragTo", "timeoutMs": timeout_ms },
            "target": at(page, "#src"), "strict": "strict",
            "to": at(page, "#missing"), "options": options })
}

/// `mouse.move` onto `#dst`, which takes nothing when pressed.
async fn move_over_dst(run: &LiveRun, page: &TestPage) -> TestResult {
    let (x, y) = center(run, page, "#dst").await?;
    device(
        run,
        "mouse.move",
        page,
        json!({ "x": x, "y": y, "options": {} }),
    )
    .await?;
    Ok(())
}

/// Runs `op` on `target` with a 5 s timeout. Only the pointer actions carry `options`.
async fn run_op(
    run: &LiveRun,
    op: &str,
    target: &Value,
    options: Value,
) -> Result<Value, PlaywrightError> {
    let mut input = action(op, target, options, 5000);
    let pointer = matches!(
        op,
        "locator.click"
            | "locator.dblclick"
            | "locator.hover"
            | "locator.check"
            | "locator.uncheck"
    );
    if let Some(input) = input.as_object_mut()
        && !pointer
    {
        input.remove("options");
    }
    run.invoke(op, input).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_dblclick_hover_check_and_uncheck_do_what_upstreams_do() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/page.html")).await?;
    run_op(&run, "locator.dblclick", &at(&page, "#twice"), json!({})).await?;
    run_op(&run, "locator.hover", &at(&page, "#hover"), json!({})).await?;
    run_op(&run, "locator.check", &at(&page, "#agree"), json!({})).await?;
    run_op(&run, "locator.check", &at(&page, "#agree"), json!({})).await?;
    run_op(
        &run,
        "locator.uncheck",
        &at(&page, "#agree-label"),
        json!({}),
    )
    .await?;
    let log = page_log(&run, &page).await?;
    let count = |prefix: &str| log.iter().filter(|entry| entry.starts_with(prefix)).count();
    assert_eq!(count("dblclick twice"), 1, "{log:?}");
    assert!(log.contains(&"enter hover".to_owned()), "{log:?}");
    assert_eq!(
        count("click agree visible"),
        2,
        "checked once, then the second check sent nothing, then unchecked through its label, \
         whose click reaches the box: {log:?}"
    );
    assert_eq!(
        run.eval(&page, "document.getElementById('agree').checked")
            .await?,
        json!(false)
    );

    let stuck = run_op(&run, "locator.check", &at(&page, "#stuck"), json!({}))
        .await
        .err()
        .ok_or("a checkbox that refuses the change must fail")?;
    assert_eq!(
        (stuck.error_code(), stuck.message().lines().next()),
        (
            Some(ErrorCode::PageError),
            Some("locator.check: Clicking the checkbox did not change its state")
        ),
        "{stuck}"
    );
    let radio = run_op(&run, "locator.uncheck", &at(&page, "#choice"), json!({}))
        .await
        .err()
        .ok_or("no click unchecks a radio")?;
    assert_eq!(
        (radio.error_code(), radio.message().lines().next()),
        (
            Some(ErrorCode::PageError),
            Some(
                "locator.uncheck: Cannot uncheck radio button. Radio buttons can only be \
                 unchecked by selecting another radio button in the same group."
            )
        ),
        "{radio}"
    );
    let log = page_log(&run, &page).await?;
    assert!(
        !log.iter().any(|entry| entry.contains(" choice ")),
        "uncheck refused the radio before it pressed anything: {log:?}"
    );
    let not_a_box = run_op(&run, "locator.check", &at(&page, "#go"), json!({}))
        .await
        .err()
        .ok_or("a button is no checkbox")?;
    assert_eq!(not_a_box.error_code(), Some(ErrorCode::InvalidInput));
    assert!(
        not_a_box
            .message()
            .contains("Error: Not a checkbox or radio button"),
        "{not_a_box}"
    );
    let disabled = run
        .invoke(
            "locator.click",
            action("locator.click", &at(&page, "#disabled"), json!({}), 600),
        )
        .await
        .err()
        .ok_or("a disabled button never takes a click")?;
    assert_eq!(disabled.error_code(), Some(ErrorCode::NotActionable));
    assert!(
        disabled.message().contains("element is not enabled"),
        "{disabled}"
    );
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_focus_blur_dispatch_event_and_scrolling_reach_the_element() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/page.html")).await?;
    run_op(&run, "locator.focus", &at(&page, "#field"), json!({})).await?;
    run_op(&run, "locator.blur", &at(&page, "#field"), json!({})).await?;
    let ping = json!({ "call": { "api": "locator.dispatchEvent", "timeoutMs": 5000 },
                       "target": at(&page, "#field"), "strict": "strict", "type": "ping",
                       "eventInit": { "detail": 7 } });
    run.invoke("locator.dispatchEvent", ping).await?;
    let log = page_log(&run, &page).await?;
    assert_eq!(
        log,
        [
            "focus field",
            "blur field",
            "ping trusted=false bubbles=true detail=undefined"
        ],
        "a plain Event, as upstream builds one for a type it does not know"
    );

    assert_eq!(run.eval(&page, "scrollY").await?, json!(0));
    let far = at(&page, "#far");
    run_op(&run, "locator.scrollIntoViewIfNeeded", &far, json!({})).await?;
    let in_view = run
        .eval(
            &page,
            "document.getElementById('far').getBoundingClientRect().top < innerHeight",
        )
        .await?;
    assert_eq!(in_view, json!(true), "scrolled into view");
    run_op(&run, "locator.click", &far, json!({ "trial": true })).await?;
    assert!(
        !page_log(&run, &page)
            .await?
            .iter()
            .any(|entry| entry.contains("far")),
        "a trial click sends its events with every one of them blocked"
    );
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_click_waits_for_the_navigation_it_requested() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/page.html")).await?;
    run.clear_calls();
    run_op(&run, "locator.click", &at(&page, "#next"), json!({})).await?;
    assert!(
        notes(&run)
            .iter()
            .any(|line| line == "  navigations have finished"),
        "{:?}",
        notes(&run)
    );
    assert_eq!(
        run.eval(&page, "location.pathname").await?,
        json!("/actions/next.html"),
        "the click returned once the navigation it requested committed"
    );
    run.close_page(&page).await;
    Ok(())
}

/// A locator that never matches logs only `waiting for <locator>`, as upstream's does: an
/// attempt's lines wait until the element is found, and no "detached" retry is logged for an
/// element that was never there (zuzu G10).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_locator_matching_nothing_logs_only_its_wait() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/page.html")).await?;
    let nope = at(&page, "#nope");
    let call = |api: &str| json!({ "api": api, "timeoutMs": 400 });
    let tries = [
        (
            "locator.click",
            action("locator.click", &nope, json!({}), 400),
        ),
        (
            "locator.fill",
            json!({ "call": call("locator.fill"), "target": nope, "strict": "strict",
                    "value": "x", "options": {} }),
        ),
        (
            "locator.focus",
            json!({ "call": call("locator.focus"), "target": nope, "strict": "strict" }),
        ),
        (
            "locator.scrollIntoViewIfNeeded",
            json!({ "call": call("locator.scrollIntoViewIfNeeded"), "target": nope,
                    "strict": "strict" }),
        ),
    ];
    for (op, input) in tries {
        let error = run
            .invoke(op, input)
            .await
            .err()
            .ok_or(format!("{op}: #nope never matches"))?;
        assert_eq!(
            error.message(),
            format!("{op}: Timeout 400ms exceeded.\nCall log:\n  - waiting for locator('#nope')\n"),
            "{op} logs only the wait"
        );
    }
    run.close_page(&page).await;
    Ok(())
}
