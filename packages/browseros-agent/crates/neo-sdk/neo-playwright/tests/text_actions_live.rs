//! Text and form actions, the keyboard and the mouse, and actions on element handles, through
//! `Playwright::invoke` against a real browser.
//!
//! Each test asserts what the page itself saw: the fixtures log the events they received, and
//! the test reads that log, or a field's raw value, from the page's own main world (an
//! out-of-process frame's through a CDP session of the test's own), never through the crate.
//! Edits through a `<label>` run as scripts, through the real prelude in QuickJS, because the
//! label locators are the prelude's own (`getByLabel`).
//!
//! Needs BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p neo-playwright --test text_actions_live -- --ignored`. Every test opens
//! and closes its own tabs.

#[path = "support/fixture_server.rs"]
mod fixture_server;
#[path = "support/js_runner.rs"]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "the capture refusal, the trusted click and the hang serve the other live suites"
)]
mod live_run;
#[path = "support/locator.rs"]
#[expect(
    dead_code,
    reason = "the role, text and ref parts serve frames_live and snapshot_live"
)]
mod locator;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs, process};

use fixture_server::{FixtureServer, Page, nest_pages};
use js_runner::{OpHost, run_script};
use live_run::{EnvCall, LiveRun, TestPage, TestResult, brand};
use locator::{attr, css, enter_frame, locator};
use neo_playwright::{ErrorCode, PlaywrightError, Refusal};
use neo_trace::ErrorOwner;
use serde_json::{Value, json};
use tokio::time;

/// Recorded from playwright-core 1.64.1 by scripts/capture-goldens.mjs: each case's calls and
/// the CDP input messages Playwright sent for them.
const KEY_GOLDENS: &str = include_str!("data/key-events.json");

/// The fixtures under `/actions/`, beside the nested one under `/nest/`.
fn pages() -> HashMap<String, Page> {
    let mut pages = nest_pages();
    for (name, body) in [
        ("page", include_str!("data/actions/page.html")),
        ("form", include_str!("data/actions/form.html")),
        ("next", include_str!("data/actions/next.html")),
        ("labels", include_str!("data/actions/labels.html")),
        // The authored file-input fixture retained for the live action tests.
        ("files", include_str!("data/actions/files.html")),
    ] {
        pages.insert(format!("/actions/{name}.html"), Page::html(body));
    }
    pages
}

/// `locator(css)` on `page`.
fn at(page: &TestPage, selector: &str) -> Value {
    locator(page, &[css(selector)], &format!("locator('{selector}')"))
}

/// `locator(css)` rooted at an element handle, as `elementHandle.$(css)` sends it.
fn under(handle: &Value, page: &TestPage, selector: &str) -> TestResult<Value> {
    let mut rooted = at(page, selector);
    rooted
        .as_object_mut()
        .ok_or("a locator brand is an object")?
        .insert("root".to_owned(), handle.clone());
    Ok(rooted)
}

/// One op on `target`, strict, with Playwright's `timeoutMs` and the op's own `fields`.
async fn act(
    run: &LiveRun,
    op: &str,
    target: &Value,
    fields: Value,
    timeout_ms: u64,
) -> Result<Value, PlaywrightError> {
    let mut input = json!({ "call": { "api": op, "timeoutMs": timeout_ms }, "target": target,
                            "strict": "strict" });
    if let (Some(input), Value::Object(fields)) = (input.as_object_mut(), fields) {
        input.extend(fields);
    }
    run.invoke(op, input).await
}

/// A background tab core opened for the agent, on `url`.
async fn agent_tab(run: &LiveRun, url: &str) -> TestResult<TestPage> {
    let page = run.new_page().await?;
    run.invoke(
        "page.goto",
        json!({ "call": { "timeoutMs": 30_000 }, "page": brand(&page), "url": url,
                "waitUntil": "load" }),
    )
    .await?;
    Ok(page)
}

/// What the page logged, in order.
async fn page_log(run: &LiveRun, page: &TestPage) -> TestResult<Vec<String>> {
    Ok(serde_json::from_value(
        run.eval(page, "window.__log").await?,
    )?)
}

/// `locator.elementHandle` on `target`, which must hand back a handle.
async fn handle_of(run: &LiveRun, target: &Value) -> TestResult<Value> {
    let handle = act(run, "locator.elementHandle", target, json!({}), 5000).await?;
    assert_eq!(
        handle.get("$neo").and_then(Value::as_str),
        Some("element"),
        "a handle brand: {handle}"
    );
    Ok(handle)
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_actions_on_an_element_handle_reach_its_node_and_fail_once_it_is_gone()
-> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/page.html")).await?;
    let go = handle_of(&run, &at(&page, "#go")).await?;
    let twice = handle_of(&run, &at(&page, "#twice")).await?;
    let field = handle_of(&run, &at(&page, "#field")).await?;
    let label = handle_of(&run, &at(&page, "#agree-label")).await?;
    act(&run, "locator.click", &go, json!({}), 0).await?;
    act(&run, "locator.dblclick", &twice, json!({}), 0).await?;
    act(&run, "locator.focus", &field, json!({}), 0).await?;
    // `label.$('input')` then `check()`: a locator rooted at the label's node.
    let agree = under(&label, &page, "input")?;
    act(&run, "locator.check", &agree, json!({}), 5000).await?;
    let log = page_log(&run, &page).await?;
    for expected in [
        "click go visible",
        "dblclick twice visible",
        "focus field",
        "click agree visible",
    ] {
        assert!(
            log.iter().any(|entry| entry == expected),
            "{expected}: {log:?}"
        );
    }
    assert_eq!(
        run.eval(&page, "document.getElementById('agree').checked")
            .await?,
        json!(true),
        "the rooted locator checked the box inside the label"
    );

    run.eval(&page, "document.getElementById('go').remove()")
        .await?;
    let started = Instant::now();
    let gone = act(&run, "locator.click", &go, json!({}), 5000)
        .await
        .err()
        .ok_or("a handle whose node left the page cannot be clicked")?;
    assert_eq!(
        (gone.error_code(), gone.message().lines().next()),
        (
            Some(ErrorCode::ElementDetached),
            Some("locator.click: Element is not attached to the DOM")
        ),
        "{gone}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a detached handle fails at once, where a locator would wait"
    );
    run.close_page(&page).await;
    Ok(())
}

/// One page-level input op, as `page.keyboard.*` and `page.mouse.*` send it (no timeout).
async fn device(
    run: &LiveRun,
    op: &str,
    page: &TestPage,
    fields: Value,
) -> Result<Value, PlaywrightError> {
    let mut input = json!({ "call": { "api": op, "timeoutMs": 0 }, "page": brand(page) });
    if let (Some(input), Value::Object(fields)) = (input.as_object_mut(), fields) {
        input.extend(fields);
    }
    run.invoke(op, input).await
}

/// Every number as an f64: Playwright writes `0` where Rust writes `0.0`, which CDP reads alike.
fn by_value(value: Value) -> Value {
    match value {
        Value::Number(number) => number.as_f64().map_or(Value::Number(number), Value::from),
        Value::Array(items) => items.into_iter().map(by_value).collect(),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, by_value(value)))
                .collect(),
        ),
        other => other,
    }
}

/// One call of a recorded case: `page.keyboard.<method>('<argument>')`, or a locator method on
/// the selector the case's `page.locator('<selector>')` named.
struct CaseCall {
    method: String,
    argument: String,
    locator: Option<String>,
}

/// The calls of a recorded case, in order, read from its source text.
fn case_calls(case: &str) -> Vec<CaseCall> {
    let mut calls = Vec::new();
    let mut selector = None;
    let mut rest = case;
    while let Some((head, tail)) = rest.split_once("('") {
        let Some((argument, after)) = tail.split_once("')") else {
            break;
        };
        let method = head.rsplit_once('.').map_or(head, |(_, method)| method);
        if method == "locator" {
            selector = Some(argument.to_owned());
        } else {
            calls.push(CaseCall {
                method: method.to_owned(),
                argument: argument.to_owned(),
                locator: if head.contains("keyboard") {
                    None
                } else {
                    selector.clone()
                },
            });
        }
        rest = after;
    }
    calls
}

/// The recorded profile for the browser's platform, as the crate picks `ControlOrMeta` by it.
async fn profile(run: &LiveRun, page: &TestPage) -> TestResult<&'static str> {
    let agent = run.eval(page, "navigator.userAgent").await?;
    Ok(
        if agent
            .as_str()
            .is_some_and(|agent| agent.contains("Macintosh"))
        {
            "mac"
        } else {
            "linux"
        },
    )
}

/// Runs one recorded call through the crate.
async fn replay(run: &LiveRun, page: &TestPage, call: &CaseCall) -> Result<Value, PlaywrightError> {
    let argument = json!(call.argument);
    match (&call.locator, call.method.as_str()) {
        (None, "press") => {
            let fields = json!({ "key": argument });
            device(run, "keyboard.press", page, fields).await
        }
        (None, "down") => device(run, "keyboard.down", page, json!({ "key": argument })).await,
        (None, "up") => device(run, "keyboard.up", page, json!({ "key": argument })).await,
        (None, "type") => {
            let fields = json!({ "text": argument });
            device(run, "keyboard.type", page, fields).await
        }
        (None, "insertText") => {
            device(
                run,
                "keyboard.insertText",
                page,
                json!({ "text": argument }),
            )
            .await
        }
        (Some(selector), "press") => {
            let fields = json!({ "key": argument, "options": {} });
            act(run, "locator.press", &at(page, selector), fields, 5000).await
        }
        (Some(selector), "pressSequentially") => {
            let fields = json!({ "text": argument, "options": {} });
            act(
                run,
                "locator.pressSequentially",
                &at(page, selector),
                fields,
                5000,
            )
            .await
        }
        (Some(selector), "fill") => {
            let fields = json!({ "value": argument, "options": {} });
            act(run, "locator.fill", &at(page, selector), fields, 5000).await
        }
        (_, other) => Err(PlaywrightError::new(
            ErrorCode::Internal,
            format!("the goldens call {other}, which this replay does not know"),
        )),
    }
}

/// Empties #field and #area, focuses #field and clears the page's log, as the capture
/// started each case.
const FRESH_FIELD: &str = "(() => { for (const id of ['field', 'area']) \
    document.getElementById(id).value = ''; document.getElementById('field').focus(); \
    window.__log = []; })()";

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_every_recorded_key_call_sends_what_playwright_sent_and_the_page_gets_keys()
-> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/form.html")).await?;
    let goldens: Value = serde_json::from_str(KEY_GOLDENS)?;
    let profile = profile(&run, &page).await?;
    let cases = goldens["profiles"][profile]
        .as_array()
        .ok_or("the goldens have the profile")?;
    let mut mismatches = Vec::new();
    for case in cases {
        let source = case["call"].as_str().unwrap_or_default();
        run.eval(&page, FRESH_FIELD).await?;
        run.record_input();
        for call in case_calls(source) {
            replay(&run, &page, &call)
                .await
                .map_err(|error| format!("{source}: {error}"))?;
        }
        let sent = by_value(Value::Array(run.recorded_input()));
        if sent != by_value(case["messages"].clone()) {
            mismatches.push(format!("{source}: sent {sent}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{profile}: sent other input than Playwright: {mismatches:#?}"
    );
    run.eval(&page, FRESH_FIELD).await?;
    let typed = json!({ "text": "Hi é" });
    device(&run, "keyboard.type", &page, typed).await?;
    assert_eq!(
        run.eval(&page, "document.getElementById('field').value")
            .await?,
        json!("Hi é"),
        "the focused field took the typed text"
    );
    let log = page_log(&run, &page).await?;
    assert!(
        log.iter().any(|entry| entry == "keydown field H KeyH"),
        "the page saw trusted key events: {log:?}"
    );
    run.close_page(&page).await;
    Ok(())
}

/// The center of the element `selector` names, in the main viewport.
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

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_mouse_moves_presses_clicks_and_scrolls_where_it_was_told() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/page.html")).await?;
    let (x, y) = center(&run, &page, "#go").await?;
    let to = json!({ "x": x, "y": y, "steps": 3 });
    device(&run, "mouse.move", &page, to).await?;
    device(&run, "mouse.down", &page, json!({})).await?;
    device(&run, "mouse.up", &page, json!({})).await?;
    let (x, y) = center(&run, &page, "#twice").await?;
    let twice = json!({ "x": x, "y": y, "clickCount": 2 });
    device(&run, "mouse.click", &page, twice).await?;
    let wheel = json!({ "deltaX": 0, "deltaY": 600 });
    device(&run, "mouse.wheel", &page, wheel).await?;
    let log = page_log(&run, &page).await?;
    for expected in [
        "mousedown go visible",
        "mouseup go visible",
        "click go visible",
        "dblclick twice visible",
    ] {
        assert!(
            log.iter().any(|entry| entry == expected),
            "{expected}: {log:?}"
        );
    }
    let mut scrolled = json!(0);
    for _ in 0..50 {
        scrolled = run.eval(&page, "window.scrollY").await?;
        if scrolled.as_f64().is_some_and(|y| y > 0.0) {
            break;
        }
        time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        scrolled.as_f64().is_some_and(|y| y > 0.0),
        "the wheel scrolled the page: {scrolled}"
    );
    run.close_page(&page).await;
    Ok(())
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

/// The nested fixture's frames: the titles down to each, its field's label, and how its own
/// main world is reached (`None`: the page's; `Some(host, file)`: that out-of-process frame's),
/// with the same-origin path below it.
struct NestFrame {
    titles: &'static [&'static str],
    name: &'static str,
    process: Option<(&'static str, &'static str)>,
    below: &'static [&'static str],
}

const NEST: [NestFrame; 8] = [
    NestFrame {
        titles: &[],
        name: "Top",
        process: None,
        below: &[],
    },
    NestFrame {
        titles: &["a1"],
        name: "A1",
        process: None,
        below: &["a1"],
    },
    NestFrame {
        titles: &["b1"],
        name: "B1",
        process: Some(("localhost", "/nest/b1.html")),
        below: &[],
    },
    NestFrame {
        titles: &["a1", "b2"],
        name: "B2",
        process: Some(("localhost", "/nest/b2.html")),
        below: &[],
    },
    NestFrame {
        titles: &["b1", "c2"],
        name: "C2",
        process: Some(("[::1]", "/nest/c2.html")),
        below: &[],
    },
    NestFrame {
        titles: &["a1", "b2", "b3"],
        name: "B3",
        process: Some(("localhost", "/nest/b2.html")),
        below: &["b3"],
    },
    NestFrame {
        titles: &["b1", "c2", "a3"],
        name: "A3",
        process: Some(("127.0.0.1", "/nest/a3.html")),
        below: &[],
    },
    NestFrame {
        titles: &["b1", "c2", "a3", "c4"],
        name: "C4",
        process: Some(("[::1]", "/nest/c4.html")),
        below: &[],
    },
];

/// The typed values a nested frame's own page logged (`frame.js`), by field label.
async fn nest_values(
    run: &LiveRun,
    server: &FixtureServer,
    page: &TestPage,
    frame: &NestFrame,
) -> TestResult<Value> {
    let mut path = "window".to_owned();
    for title in frame.below {
        path = format!("{path}.document.querySelector('iframe[title={title}]').contentWindow");
    }
    let expression = format!("{path}.__state.values");
    Ok(match frame.process {
        None => run.eval(page, &expression).await?,
        Some((host, file)) => {
            run.frame_eval(page, &server.url(host, file), &expression)
                .await?
        }
    })
}

/// `fill(value)` on `target`.
async fn fill(run: &LiveRun, target: &Value, value: &str) -> Result<Value, PlaywrightError> {
    let fields = json!({ "value": value, "options": {} });
    act(run, "locator.fill", target, fields, 5000).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_each_nested_frame_holds_exactly_its_own_filled_value() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/nest/top.html")).await?;
    let deepest = down(&page, &["b1", "c2", "a3", "c4"], css("input"));
    act(&run, "locator.textContent", &deepest, json!({}), 10_000).await?;
    for frame in &NEST {
        let field = down(&page, frame.titles, css("input"));
        fill(&run, &field, &format!("{} typed", frame.name))
            .await
            .map_err(|error| format!("{}: {error}", frame.name))?;
    }
    let mut logged = Vec::new();
    let mut expected = Vec::new();
    for frame in &NEST {
        logged.push((frame.name, nest_values(&run, &server, &page, frame).await?));
        let label = format!("{} field", frame.name);
        expected.push((
            frame.name,
            json!({ label: format!("{} typed", frame.name) }),
        ));
    }
    assert_eq!(
        logged, expected,
        "each frame's own field holds its value, and nothing landed in another frame"
    );
    run.close_page(&page).await;
    Ok(())
}

/// A field's value, read raw from the page's own world.
async fn value_of(run: &LiveRun, page: &TestPage, id: &str) -> TestResult<Value> {
    run.eval(page, &format!("document.getElementById('{id}').value"))
        .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_fill_types_sets_or_clears_as_upstream() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/form.html")).await?;
    run.clear_calls();
    fill(&run, &at(&page, "#field"), "plain words").await?;
    assert!(
        run.calls()
            .contains(&EnvCall::Note("  fill(\"plain words\")".to_owned())),
        "a plain field's value shows in the call log: {:?}",
        run.calls()
    );
    run.clear_calls();
    fill(&run, &at(&page, "#password"), "hunter2-new").await?;
    assert!(
        run.calls()
            .contains(&EnvCall::Note("  fill(\"hunter2-new\")".to_owned())),
        "a password field's value shows as upstream's call log shows it: {:?}",
        run.calls()
    );
    fill(&run, &at(&page, "#secret"), "fresh").await?;
    fill(&run, &at(&page, "#number"), " 42 ").await?;
    fill(&run, &at(&page, "#date"), "2026-11-05").await?;
    fill(&run, &at(&page, "#editable"), "edited").await?;
    fill(&run, &at(&page, "#area"), "first").await?;
    fill(&run, &at(&page, "#area"), "").await?;
    let values = [
        value_of(&run, &page, "field").await?,
        value_of(&run, &page, "password").await?,
        value_of(&run, &page, "secret").await?,
        value_of(&run, &page, "number").await?,
        value_of(&run, &page, "date").await?,
        run.eval(&page, "document.getElementById('editable').textContent")
            .await?,
        value_of(&run, &page, "area").await?,
    ];
    assert_eq!(
        values,
        [
            json!("plain words"),
            json!("hunter2-new"),
            json!("fresh"),
            json!("42"),
            json!("2026-11-05"),
            json!("edited"),
            json!(""),
        ],
        "each field holds exactly the filled value"
    );

    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_fill_refuses_what_upstream_refuses_and_waits_where_upstream_waits() -> TestResult
{
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/form.html")).await?;
    for (selector, timeout, code, message) in [
        (
            "#button",
            5000,
            ErrorCode::InvalidInput,
            "locator.fill: Error: Element is not an <input>, <textarea>, <select> or \
             [contenteditable] and does not have a role allowing [aria-readonly]",
        ),
        (
            "#upload",
            5000,
            ErrorCode::InvalidInput,
            "locator.fill: Error: Input of type \"file\" cannot be filled",
        ),
        (
            "#readonly",
            600,
            ErrorCode::NotActionable,
            "locator.fill: Timeout 600ms exceeded.",
        ),
        (
            "#hidden-field",
            600,
            ErrorCode::NotActionable,
            "locator.fill: Timeout 600ms exceeded.",
        ),
    ] {
        let fields = json!({ "value": "x", "options": {} });
        let error = act(&run, "locator.fill", &at(&page, selector), fields, timeout)
            .await
            .err()
            .ok_or(format!("{selector} cannot be filled"))?;
        assert_eq!(
            (error.error_code(), error.message().lines().next()),
            (Some(code), Some(message)),
            "{selector}: {error}"
        );
    }
    let number = act(
        &run,
        "locator.fill",
        &at(&page, "#number"),
        json!({ "value": "abc", "options": {} }),
        5000,
    )
    .await
    .err()
    .ok_or("a number input refuses text")?;
    assert_eq!(
        number.message().lines().next(),
        Some("locator.fill: Error: Cannot type text into input[type=number]")
    );
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_refused_recheck_fills_and_types_nothing() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/form.html")).await?;
    run.refuse_rechecks(Some(Refusal {
        owner: ErrorOwner::Host,
        code: "owner_has_control".to_owned(),
        message: "The owner is using this tab.".to_owned(),
        host_code: None,
    }));
    run.eval(&page, "document.getElementById('field').focus()")
        .await?;
    run.clear_calls();
    run.record_input();
    let filled = fill(&run, &at(&page, "#field"), "never").await.err();
    // A date input takes its value in the probe's edit itself, so the edit must follow Sent.
    let dated = fill(&run, &at(&page, "#date"), "2026-11-05").await.err();
    let selected = select(&run, &page, "#color", json!([{ "value": "g" }]))
        .await
        .err();
    let typed = device(&run, "keyboard.type", &page, json!({ "text": "never" }))
        .await
        .err();
    run.refuse_rechecks(None);
    for refused in [filled, dated, selected, typed] {
        let refused = refused.ok_or("a refused recheck fails the op")?;
        assert_eq!(refused.code(), "owner_has_control", "{refused}");
    }
    assert!(
        !run.calls().contains(&EnvCall::MarkSent),
        "nothing marked sent"
    );
    assert_eq!(
        (
            value_of(&run, &page, "field").await?,
            value_of(&run, &page, "date").await?,
            value_of(&run, &page, "color").await?,
            run.recorded_input()
        ),
        (json!(""), json!(""), json!("r"), Vec::new()),
        "and nothing reached the page"
    );
    run.close_page(&page).await;
    Ok(())
}

/// `selectOption(options)` on `selector`, with a 600 ms timeout.
async fn select(
    run: &LiveRun,
    page: &TestPage,
    selector: &str,
    options: Value,
) -> Result<Value, PlaywrightError> {
    let fields = json!({ "options": options, "elements": [] });
    act(
        run,
        "locator.selectOption",
        &at(page, selector),
        fields,
        600,
    )
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_select_option_and_select_text_do_what_upstreams_do() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/form.html")).await?;
    assert_eq!(
        select(&run, &page, "#color", json!([{ "valueOrLabel": "Green" }])).await?,
        json!(["g"])
    );
    assert_eq!(
        select(
            &run,
            &page,
            "#many",
            json!([{ "value": "1" }, { "index": 2 }])
        )
        .await?,
        json!(["1", "3"])
    );
    assert_eq!(
        (
            value_of(&run, &page, "color").await?,
            run.eval(
                &page,
                "[...document.getElementById('many').selectedOptions].map((o) => o.value)"
            )
            .await?,
        ),
        (json!("g"), json!(["1", "3"]))
    );
    let log = page_log(&run, &page).await?;
    assert!(log.iter().any(|entry| entry == "change color g"), "{log:?}");
    for (options, line) in [
        (json!([{ "value": "nope" }]), "  did not find some options"),
        (
            json!([{ "value": "b" }]),
            "  option being selected is not enabled",
        ),
    ] {
        run.clear_calls();
        let error = select(&run, &page, "#color", options)
            .await
            .err()
            .ok_or("a missing or disabled option is never selected")?;
        assert_eq!(
            error.error_code(),
            Some(ErrorCode::NotActionable),
            "{error}"
        );
        assert!(
            run.calls().contains(&EnvCall::Note(line.to_owned())),
            "{line}: {error}"
        );
    }
    let not_select = select(&run, &page, "#field", json!([{ "value": "a" }]))
        .await
        .err()
        .ok_or("an input is no select")?;
    assert_eq!(
        not_select.message().lines().next(),
        Some("locator.selectOption: Error: Element is not a <select> element")
    );

    // selectText on a textarea selects its whole text.
    act(
        &run,
        "locator.selectText",
        &at(&page, "#secret"),
        json!({}),
        5000,
    )
    .await?;
    let selection = run
        .eval(
            &page,
            "(() => { const t = document.getElementById('secret'); \
             return [t.selectionStart, t.selectionEnd, t.value.length, \
             document.activeElement === t]; })()",
        )
        .await?;
    assert_eq!(
        selection,
        json!([0, 22, 22, true]),
        "the whole secret is selected and focused"
    );
    act(
        &run,
        "locator.selectText",
        &at(&page, "#field"),
        json!({}),
        5000,
    )
    .await?;
    assert_eq!(
        run.eval(&page, "document.activeElement.id").await?,
        json!("field")
    );
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_an_element_handle_fills_its_own_field() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/form.html")).await?;
    let field = handle_of(&run, &at(&page, "#field")).await?;
    let password = handle_of(&run, &at(&page, "#password")).await?;
    fill(&run, &field, "by handle").await?;
    run.clear_calls();
    fill(&run, &password, "secret by handle").await?;
    assert_eq!(
        (
            value_of(&run, &page, "field").await?,
            value_of(&run, &page, "password").await?
        ),
        (json!("by handle"), json!("secret by handle"))
    );
    run.eval(&page, "document.getElementById('field').remove()")
        .await?;
    let gone = fill(&run, &field, "again")
        .await
        .err()
        .ok_or("a detached handle cannot be filled")?;
    assert_eq!(
        gone.error_code(),
        Some(ErrorCode::ElementDetached),
        "{gone}"
    );
    run.close_page(&page).await;
    Ok(())
}

/// `press(key)` on `target`.
async fn press(run: &LiveRun, target: &Value, key: &str) -> Result<Value, PlaywrightError> {
    let fields = json!({ "key": key, "options": {} });
    act(run, "locator.press", target, fields, 5000).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_press_and_press_sequentially_focus_type_and_wait_for_the_navigation()
-> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/form.html")).await?;
    run.clear_calls();
    let typed = json!({ "text": "pa55", "options": { "delay": 5 } });
    act(
        &run,
        "locator.pressSequentially",
        &at(&page, "#password"),
        typed,
        5000,
    )
    .await?;
    let calls = run.calls();
    assert!(
        calls.contains(&EnvCall::Note("elementHandle.type(\"pa55\")".to_owned())),
        "the call log names what was typed, as upstream's does: {calls:?}"
    );
    let handle = handle_of(&run, &at(&page, "#field")).await?;
    press(&run, &handle, "x").await?;
    press(&run, &at(&page, "#field"), "Shift+ArrowLeft").await?;
    press(&run, &at(&page, "#field"), "y").await?;
    assert_eq!(
        (
            value_of(&run, &page, "password").await?,
            value_of(&run, &page, "field").await?,
        ),
        (json!("pa55"), json!("y")),
        "the keys reached the focused fields: x selected by Shift+ArrowLeft, then typed over"
    );

    press(&run, &at(&page, "#search"), "Enter").await?;
    assert_eq!(
        run.eval(&page, "location.pathname").await?,
        json!("/actions/next.html"),
        "press waited for the navigation Enter requested"
    );

    let unknown = press(&run, &at(&page, "h1"), "Shift+Foo")
        .await
        .err()
        .ok_or("an unknown key fails")?;
    assert_eq!(
        (unknown.error_code(), unknown.message().lines().next()),
        (
            Some(ErrorCode::InvalidInput),
            Some("locator.press: Unknown key: \"Foo\"")
        )
    );
    run.close_page(&page).await;
    Ok(())
}

/// Temporary files for one test, removed when it drops.
struct TempFiles {
    dir: PathBuf,
}

impl TempFiles {
    fn new() -> TestResult<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let dir = env::temp_dir().join(format!("r9-files-{}-{stamp}", process::id()));
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    /// A file named `name` holding `body`, by its absolute path.
    fn file(&self, name: &str, body: &str) -> TestResult<String> {
        let path = self.dir.join(name);
        fs::write(&path, body)?;
        Ok(path.to_string_lossy().into_owned())
    }
}

impl Drop for TempFiles {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.dir);
    }
}

/// `setInputFiles(paths)` on `target`, as the prelude sends it, with a 5 s timeout.
async fn set_files(
    run: &LiveRun,
    target: &Value,
    files: &[&str],
) -> Result<Value, PlaywrightError> {
    set_files_within(run, target, files, 5000).await
}

async fn set_files_within(
    run: &LiveRun,
    target: &Value,
    files: &[&str],
    timeout_ms: u64,
) -> Result<Value, PlaywrightError> {
    act(
        run,
        "locator.setInputFiles",
        target,
        json!({ "files": { "paths": files } }),
        timeout_ms,
    )
    .await
}

/// The names of the files `#id` holds now, as the page sees them.
async fn file_names(run: &LiveRun, page: &TestPage, id: &str) -> TestResult<Value> {
    let script = format!("[...document.getElementById('{id}').files].map((f) => f.name)");
    run.eval(page, &script).await
}

/// files.html's record of the files `#id` holds: name, type, size, text and relative path.
async fn files_of(run: &LiveRun, page: &TestPage, id: &str) -> TestResult<Value> {
    let mut files = run.eval(page, &format!("window.__files('{id}')")).await?;
    for file in files.as_array_mut().into_iter().flatten() {
        file.as_object_mut().map(|file| file.remove("lastModified"));
    }
    Ok(files)
}

/// The input, change and cancel events files.html saw since the last call, in order.
async fn events_of(run: &LiveRun, page: &TestPage) -> TestResult<Value> {
    run.eval(page, "window.__takeEvents()").await
}

/// One event as files.html records it.
fn event(kind: &str, target: &str, trusted: bool) -> Value {
    json!({ "type": kind, "target": target, "trusted": trusted, "composed": kind == "input",
            "bubbles": true })
}

/// A file as files.html records it, for a file with no relative path.
fn file(name: &str, kind: &str, text: &str) -> Value {
    json!({ "name": name, "type": kind, "size": text.len(), "text": text, "relativePath": "" })
}

fn sent_anything(run: &LiveRun) -> bool {
    run.calls()
        .iter()
        .any(|call| matches!(call, EnvCall::Recheck | EnvCall::MarkSent))
}

/// A relative path is the client's: it resolves against Neo's working directory, the test's
/// (the crate's) here, and only the absolute path reaches Chromium. On a fresh tab, because
/// `DOM.setFileInputFiles` with a relative path crashes the target (spike S3).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_relative_path_resolves_against_the_working_directory_and_never_reaches_cdp()
-> TestResult {
    let run = Arc::new(LiveRun::connect().await?);
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/files.html")).await?;
    let relative = "tests/data/files/one.txt";
    script_on(
        &run,
        &page,
        &format!("await page.setInputFiles('#single', '{relative}'); return null"),
    )
    .await?;
    assert_eq!(
        files_of(&run, &page, "single").await?,
        json!([file("one.txt", "text/plain", "one\n")]),
        "the page reads the file the relative path names"
    );
    let cwd = env::current_dir()?;
    let absolute = cwd.join(relative).to_string_lossy().into_owned();
    let sent: Vec<Value> = run
        .file_inputs_sent()
        .iter()
        .map(|params| params["files"].clone())
        .collect();
    assert_eq!(
        sent,
        [json!([absolute])],
        "Chromium got the absolute path only"
    );
    assert_eq!(
        events_of(&run, &page).await?,
        json!([
            event("input", "single", true),
            event("change", "single", true)
        ]),
        "Chromium set the files natively: trusted events"
    );
    run.close_page(&page).await;
    Ok(())
}

/// Upstream's client stats each path before any channel call, so a missing one fails with
/// Node's message, unprefixed, and nothing is admitted or sent.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_missing_path_fails_as_nodes_stat_unprefixed_and_sends_nothing() -> TestResult
{
    let run = Arc::new(LiveRun::connect().await?);
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/files.html")).await?;
    run.clear_calls();
    let thrown = script_on(
        &run,
        &page,
        "try { await page.setInputFiles('#multi', ['tests/data/files/one.txt', \
         'tests/data/files/missing.txt']) } catch (e) { return [e.name, e.message] }",
    )
    .await?;
    assert_eq!(
        thrown,
        json!([
            "Error",
            "ENOENT: no such file or directory, stat 'tests/data/files/missing.txt'"
        ]),
        "the path as the script wrote it, with no `page.setInputFiles:` prefix"
    );
    assert!(
        !sent_anything(&run),
        "nothing was rechecked or sent: {:?}",
        run.calls()
    );
    assert!(
        run.file_inputs_sent().is_empty(),
        "no path reached Chromium"
    );
    assert_eq!(files_of(&run, &page, "multi").await?, json!([]));
    run.close_page(&page).await;
    Ok(())
}

/// Payloads are built in the page by upstream's injected script: the File has the payload's
/// name, the type its name gives when it names none, and its bytes, and the page sees an
/// untrusted, composed `input` and then `change`. `[]` takes the same path, so it clears an
/// input and fires both events even when the input was already empty.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_payloads_and_no_files_set_the_input_through_upstreams_script() -> TestResult {
    let run = Arc::new(LiveRun::connect().await?);
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/files.html")).await?;
    script_on(
        &run,
        &page,
        "await page.locator('#multi').setInputFiles([
           { name: 'note.txt', mimeType: 'text/x-note', buffer: Buffer.from('hello') },
           { name: 'pic.png', buffer: Buffer.from([0x50, 0x4e, 0x47]) },
         ]); return null",
    )
    .await?;
    assert_eq!(
        files_of(&run, &page, "multi").await?,
        json!([
            file("note.txt", "text/x-note", "hello"),
            file("pic.png", "image/png", "PNG")
        ]),
        "each File is the payload's, its type from its name when it gave none"
    );
    assert_eq!(
        events_of(&run, &page).await?,
        json!([
            event("input", "multi", false),
            event("change", "multi", false)
        ]),
    );
    let recent = run
        .eval(
            &page,
            "Math.abs(document.getElementById('multi').files[0].lastModified - Date.now()) < 60000",
        )
        .await?;
    assert_eq!(
        recent,
        json!(true),
        "a payload's File is stamped when it is built"
    );

    script_on(&run, &page, "await page.setInputFiles('#multi', []); await page.setInputFiles('#empty', []); return null").await?;
    assert_eq!(
        files_of(&run, &page, "multi").await?,
        json!([]),
        "[] clears"
    );
    assert_eq!(
        events_of(&run, &page).await?,
        json!([
            event("input", "multi", false),
            event("change", "multi", false),
            event("input", "empty", false),
            event("change", "empty", false),
        ]),
        "[] fires both events, on an input that held nothing too"
    );
    assert!(
        run.file_inputs_sent().is_empty(),
        "no payload went through Chromium's paths"
    );
    run.close_page(&page).await;
    Ok(())
}

/// Chromium sets paths only on a `type=file` input. Upstream asks it and gets a protocol
/// error, which its locator loop retries until the timeout and a handle reports at once; Neo
/// checks first and sends nothing. A payload into such an input is left alone by upstream's
/// injected script, without an error.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_non_file_input_refuses_paths_unsent_and_takes_payloads_silently() -> TestResult
{
    let run = Arc::new(LiveRun::connect().await?);
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/files.html")).await?;
    run.clear_calls();
    let refused = script_on(
        &run,
        &page,
        "const handle = await page.$('#text');
         const outcome = async (work) => { try { await work; return null } catch (e) { return [e.name, e.message.split('\\n')[0]] } };
         return [
           await outcome(handle.setInputFiles('tests/data/files/one.txt')),
           await outcome(page.setInputFiles('#text', 'tests/data/files/one.txt', { timeout: 500 })),
         ]",
    )
    .await?;
    assert_eq!(
        refused,
        json!([
            [
                "Error",
                "elementHandle.setInputFiles: Protocol error (DOM.setFileInputFiles): Node is not a \
                 file input element"
            ],
            [
                "TimeoutError",
                "page.setInputFiles: Timeout 500ms exceeded."
            ],
        ]),
        "a handle fails at once, a locator retries as upstream's does"
    );
    assert!(
        !sent_anything(&run),
        "neither sent anything: {:?}",
        run.calls()
    );
    assert!(
        run.file_inputs_sent().is_empty(),
        "no path reached Chromium"
    );

    script_on(
        &run,
        &page,
        "await page.setInputFiles('#text', { name: 'x.txt', mimeType: 'text/plain', buffer: \
         Buffer.from('x') }); await page.setInputFiles('#text', []); return null",
    )
    .await?;
    assert_eq!(
        (
            run.eval(&page, "document.getElementById('text').value")
                .await?,
            events_of(&run, &page).await?
        ),
        (json!(""), json!([])),
        "payloads and [] leave a text input alone, with no error"
    );
    run.close_page(&page).await;
    Ok(())
}

/// A directory goes only to a `webkitdirectory` input, and nothing else does, with upstream's
/// messages. Into one, upstream sets the directory and waits for the page's `input`. Chromium
/// reads a chosen directory only for the tab in front: for a background tab, which every tab Neo
/// opens for the agent is, it fires `cancel` and sets nothing, so the wait runs out, as real
/// Playwright's does on a background tab. In front the files land with their relative paths
/// (measured on BrowserOS neo on macOS, and in Neo's Linux image, spikes-docker S3).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_directory_goes_only_to_a_directory_input_and_lands_there() -> TestResult {
    let run = Arc::new(LiveRun::connect().await?);
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/files.html")).await?;
    let refused = script_on(
        &run,
        &page,
        "const outcome = async (work) => { try { await work; return null } catch (e) { return e.message.split('\\n')[0] } };
         return [
           await outcome(page.setInputFiles('#single', 'tests/data/files/dir')),
           await outcome(page.setInputFiles('#dir', 'tests/data/files/one.txt')),
           await outcome(page.setInputFiles('#dir', [])),
         ]",
    )
    .await?;
    assert_eq!(
        refused,
        json!([
            "page.setInputFiles: Error: File input does not support directories, pass individual \
             files instead",
            "page.setInputFiles: Error: [webkitdirectory] input requires passing a path to a \
             directory",
            "page.setInputFiles: Error: [webkitdirectory] input requires passing a path to a \
             directory",
        ]),
    );
    let background = script_on(
        &run,
        &page,
        "try { await page.setInputFiles('#dir', 'tests/data/files/dir', { timeout: 1500 }) } \
         catch (e) { return [e.name, e.message.split('\\n')[0]] }",
    )
    .await?;
    assert_eq!(
        background,
        json!([
            "TimeoutError",
            "page.setInputFiles: Timeout 1500ms exceeded."
        ]),
        "a background tab never sees the `input` upstream waits for"
    );
    assert_eq!(
        (
            events_of(&run, &page).await?,
            files_of(&run, &page, "dir").await?
        ),
        (json!([event("cancel", "dir", true)]), json!([])),
        "Chromium cancelled the directory in the background tab"
    );
    run.browser()
        .cdp(
            "Browser.activateTab",
            json!({ "tabId": brand(&page)["tab"] }),
            None,
        )
        .await?;
    script_on(
        &run,
        &page,
        "await page.setInputFiles('#dir', 'tests/data/files/dir'); return null",
    )
    .await?;
    let landed = files_of(&run, &page, "dir").await?;
    let paths: Vec<&str> = landed
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|file| file["relativePath"].as_str())
        .collect();
    assert_eq!(paths, ["dir/a.txt", "dir/sub/b.txt"], "{landed}");
    assert_eq!(
        events_of(&run, &page).await?,
        json!([event("input", "dir", true), event("change", "dir", true)]),
    );
    let directory = json!([env::current_dir()?.join("tests/data/files/dir")]);
    assert_eq!(
        run.file_inputs_sent()
            .iter()
            .map(|params| params["files"].clone())
            .collect::<Vec<_>>(),
        [directory.clone(), directory],
        "each attempt sent the directory, absolute"
    );
    run.close_page(&page).await;
    Ok(())
}

/// Upstream's server `access`es each path again right before it sets the files, and its
/// locator loop retries the error, so a file that went away while the locator waited holds the
/// op back without sending anything, and the op goes on once the file is back.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_file_gone_before_the_set_is_retried_and_never_sent_missing() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/files.html")).await?;
    let temp = TempFiles::new()?;
    let late = temp.file("late.txt", "late")?;
    let target = at(&page, "#late");
    let paths = [late.as_str()];
    let waiting = set_files_within(&run, &target, &paths, 5000);
    let meanwhile = async {
        time::sleep(Duration::from_millis(300)).await;
        fs::remove_file(&late)?;
        run.eval(
            &page,
            "document.body.insertAdjacentHTML('beforeend', '<input type=file id=late>')",
        )
        .await?;
        time::sleep(Duration::from_millis(600)).await;
        let held_back = run.file_inputs_sent().is_empty();
        fs::write(&late, "late")?;
        TestResult::Ok(held_back)
    };
    let (set, held_back) = tokio::join!(waiting, meanwhile);
    set?;
    assert!(
        held_back?,
        "nothing reached Chromium while the file was missing"
    );
    assert_eq!(
        files_of(&run, &page, "late").await?,
        json!([file("late.txt", "text/plain", "late")]),
        "the file was set once it was back"
    );
    assert_eq!(run.file_inputs_sent().len(), 1, "and only then");
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_set_input_files_sets_host_files_and_refuses_as_upstream() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/form.html")).await?;
    let temp = TempFiles::new()?;
    let notes = temp.file("notes.txt", "hello from the host")?;
    let photo = temp.file("photo.png", "not really a png")?;

    set_files(&run, &at(&page, "#upload"), &[&notes]).await?;
    set_files(&run, &at(&page, "#uploads"), &[&notes, &photo]).await?;
    // A label's control, as upstream's retarget follows it.
    set_files(&run, &at(&page, "#upload-label"), &[&photo]).await?;
    let handle = handle_of(&run, &at(&page, "#upload")).await?;
    set_files(&run, &handle, &[&photo]).await?;
    assert_eq!(
        (
            file_names(&run, &page, "upload").await?,
            file_names(&run, &page, "uploads").await?,
            file_names(&run, &page, "labelled").await?,
        ),
        (
            json!(["photo.png"]),
            json!(["notes.txt", "photo.png"]),
            json!(["photo.png"]),
        ),
        "each input holds its files, the handle's last"
    );
    let read = run
        .eval(&page, "document.getElementById('uploads').files[0].text()")
        .await?;
    assert_eq!(
        read,
        json!("hello from the host"),
        "the page reads the host file"
    );
    let log = page_log(&run, &page).await?;
    assert!(
        log.iter()
            .any(|entry| entry == "change uploads notes.txt,photo.png"),
        "{log:?}"
    );
    // `[]` is payloads on the wire, as the prelude sends it.
    let cleared = json!({ "files": { "payloads": [] } });
    act(
        &run,
        "locator.setInputFiles",
        &at(&page, "#uploads"),
        cleared,
        5000,
    )
    .await?;
    assert_eq!(
        file_names(&run, &page, "uploads").await?,
        json!([]),
        "no files clears the selection"
    );

    let missing = temp.dir.join("missing.txt").to_string_lossy().into_owned();
    for (selector, files, message) in [
        (
            "#upload",
            vec![notes.as_str(), photo.as_str()],
            "locator.setInputFiles: Error: Non-multiple file input can only accept single file"
                .to_owned(),
        ),
        (
            "#button",
            vec![notes.as_str()],
            "locator.setInputFiles: Error: Node is not an HTMLInputElement".to_owned(),
        ),
        (
            "#upload",
            vec![missing.as_str()],
            format!("ENOENT: no such file or directory, stat '{missing}'"),
        ),
    ] {
        run.clear_calls();
        let error = set_files(&run, &at(&page, selector), &files)
            .await
            .err()
            .ok_or(format!("{selector} refuses {files:?}"))?;
        assert_eq!(
            (error.error_code(), error.message().lines().next()),
            (Some(ErrorCode::InvalidInput), Some(message.as_str())),
            "{selector}: {error}"
        );
        assert!(
            !sent_anything(&run),
            "{selector}: nothing sent: {:?}",
            run.calls()
        );
    }
    assert_eq!(
        file_names(&run, &page, "upload").await?,
        json!(["photo.png"]),
        "a refused call changed nothing"
    );
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_keys_type_into_focused_password_and_plain_fields() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/form.html")).await?;
    for field in ["password", "field"] {
        run.eval(
            &page,
            &format!("document.getElementById('{field}').focus()"),
        )
        .await?;
        for (op, fields) in [
            ("keyboard.type", json!({ "text": "s3cret" })),
            ("keyboard.insertText", json!({ "text": "!" })),
            ("keyboard.press", json!({ "key": "a" })),
        ] {
            run.clear_calls();
            device(&run, op, &page, fields).await?;
            assert!(sent_anything(&run), "{op} into #{field} sends input");
        }
        assert_eq!(value_of(&run, &page, field).await?, json!("s3cret!a"));
    }
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_focus_started_during_a_delayed_type_waits_for_its_last_key() -> TestResult {
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/form.html")).await?;
    let (field, area) = (at(&page, "#field"), at(&page, "#area"));
    let typed = json!({ "text": "abcdef", "options": { "delay": 60 } });
    let typing = act(&run, "locator.pressSequentially", &field, typed, 10_000);
    // Started once the first keys are out, as `Promise.all` would run the two ops.
    let focusing = async {
        time::sleep(Duration::from_millis(250)).await;
        act(&run, "locator.focus", &area, json!({}), 10_000).await
    };
    let (typed, focused) = tokio::join!(typing, focusing);
    typed?;
    focused?;
    assert_eq!(
        (
            value_of(&run, &page, "field").await?,
            value_of(&run, &page, "area").await?,
        ),
        (json!("abcdef"), json!("")),
        "the focus waited for the tab's input, so every character landed in the field"
    );
    assert_eq!(
        run.eval(&page, "document.activeElement.id").await?,
        json!("area"),
        "and then moved the focus"
    );
    run.close_page(&page).await;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_select_option_and_set_input_files_lease_focus_in_an_adopted_tab() -> TestResult
{
    let run = LiveRun::connect().await?;
    let server = FixtureServer::start(pages()).await?;
    let page = run
        .open_adopted(&server.url("127.0.0.1", "/actions/form.html"))
        .await?;
    let temp = TempFiles::new()?;
    let notes = temp.file("notes.txt", "hello from the host")?;
    // Each change as the page saw it, with its visibility then: a background tab is hidden
    // unless focus emulation is on.
    let state = run
        .eval(
            &page,
            "(() => { window.__seen = []; document.addEventListener('change', (e) => \
             window.__seen.push(`${e.target.id} ${document.visibilityState}`), true); \
             return document.visibilityState; })()",
        )
        .await?;
    assert_eq!(
        state,
        json!("hidden"),
        "an adopted background tab is hidden"
    );

    let options = json!({ "options": [{ "valueOrLabel": "Green" }], "elements": [] });
    act(
        &run,
        "locator.selectOption",
        &at(&page, "#color"),
        options,
        5000,
    )
    .await?;
    set_files(&run, &at(&page, "#upload"), &[&notes]).await?;
    assert_eq!(
        run.eval(&page, "window.__seen").await?,
        json!(["color visible", "upload visible"]),
        "both edits ran under the focus lease"
    );
    time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        run.eval(&page, "document.visibilityState").await?,
        json!("hidden"),
        "and the lease was given back afterwards"
    );
    run.close_page(&page).await;
    Ok(())
}

/// Runs `script` on `page` as the agent's run would: the real prelude in QuickJS, every op
/// through `Playwright::invoke`. Answers what the script returned.
async fn script_on(run: &Arc<LiveRun>, page: &TestPage, script: &str) -> TestResult<Value> {
    let init = json!({ "page": brand(page), "pages": [], "browserVersion": "" });
    let host: Arc<dyn OpHost> = Arc::clone(run) as Arc<dyn OpHost>;
    Ok(run_script(&host, &init, script).await?)
}

/// One edit through a label names `shown` in its call log, as upstream's does.
fn assert_logged(run: &LiveRun, script: &str, shown: &str) {
    let calls = run.calls();
    assert!(
        calls
            .iter()
            .any(|call| matches!(call, EnvCall::Note(line) if line.contains(shown))),
        "`{script}` shows `{shown}` in the call log: {calls:?}"
    );
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_an_edit_through_a_label_reaches_the_field_it_changes() -> TestResult {
    let run = Arc::new(LiveRun::connect().await?);
    let server = FixtureServer::start(pages()).await?;
    let page = agent_tab(&run, &server.url("127.0.0.1", "/actions/labels.html")).await?;
    // Each script and what its call log names. Keys pressed on a label reach its field, because
    // Chromium's label hands its focus to the field; then fills through a label naming its
    // password with `for`, through a span inside the label that wraps one, and forced, which
    // skips the states.
    let keys = [
        (
            "await page.locator('#pin-label').pressSequentially('s3cret-typed')",
            "s3cret-typed",
        ),
        (
            "await page.locator('#pin-label').press('x')",
            "press(\"x\")",
        ),
    ];
    let fills = [
        (
            "await page.getByLabel('Password').fill('s3cret-label')",
            "s3cret-label",
        ),
        (
            "await page.locator('#login-label').fill('s3cret-for')",
            "s3cret-for",
        ),
        (
            "await page.locator('#pin-text').fill('s3cret-span')",
            "s3cret-span",
        ),
        (
            "await page.locator('#login-label').fill('s3cret-force', { force: true })",
            "s3cret-force",
        ),
    ];
    for (index, (script, shown)) in keys.into_iter().chain(fills).enumerate() {
        run.clear_calls();
        script_on(&run, &page, script)
            .await
            .map_err(|error| format!("`{script}`: {error}"))?;
        assert_logged(&run, script, shown);
        if index + 1 == keys.len() {
            assert_eq!(
                value_of(&run, &page, "pin").await?,
                json!("s3cret-typedx"),
                "the keys reached the field the label wraps"
            );
        }
    }
    assert_eq!(
        [
            value_of(&run, &page, "login").await?,
            value_of(&run, &page, "pin").await?
        ],
        [json!("s3cret-force"), json!("s3cret-span")],
        "every fill reached the field its label names"
    );

    // A one-time-code select answers its selected values as upstream's does.
    let select = "return await page.locator('#codes-label')\
                  .selectOption([{ label: 'First' }, { label: 'Second' }])";
    run.clear_calls();
    let selected = script_on(&run, &page, select).await?;
    assert_eq!(
        selected,
        json!(["code-118204", "code-907316"]),
        "a one-time-code select answers every selected value"
    );
    run.close_page(&page).await;
    Ok(())
}
