//! Reads through `Playwright::invoke`, against a real browser: each read on a fixture page,
//! upstream's messages and call logs, a read that waits for a late element, a strict violation,
//! a password read as the page holds it, and a box inside a nested
//! out-of-process iframe placed in the main viewport, where real input lands on it.
//!
//! The live tests need BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p neo-playwright --test reads_live -- --ignored`. They serve their pages
//! on `127.0.0.1`, `localhost` and `[::1]`, and open and close their own tabs.

#[path = "support/fixture_server.rs"]
mod fixture_server;
#[path = "support/live_browser.rs"]
#[expect(
    dead_code,
    reason = "the journal's recording hooks serve frames_live.rs; reads report no journal"
)]
mod live_browser;
#[path = "support/locator.rs"]
#[expect(dead_code, reason = "reads need no text or ref engine parts")]
mod locator;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use browseros_core::SessionId;
use fixture_server::{FixtureServer, Page, nest_pages};
use live_browser::{LiveBrowser, TestPage, TestResult};
use locator::{attr, css, enter_frame, locator, role};
use neo_playwright::ErrorCode;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::time;

const READS: &str = include_str!("data/reads.html");

/// The busy-renderer test takes every permit while it pins one renderer's thread; the tests
/// that time an op which must not wait take one each, so that test's busy core is never what
/// makes them slow.
static BUSY_RENDERER: Semaphore = Semaphore::const_new(QUIET_TESTS as usize);

/// More permits than tests that take one.
const QUIET_TESTS: u32 = 16;

/// A locator for one CSS selector in the main frame, displayed as `locator('<selector>')`.
fn at(page: &TestPage, selector: &str) -> Value {
    locator(page, &[css(selector)], &format!("locator('{selector}')"))
}

/// A strict locator call with a timeout, as `locator.<api>(..)` sends it.
fn call(api: &str, target: &Value, timeout_ms: u64) -> Value {
    json!({ "call": { "api": api, "timeoutMs": timeout_ms }, "target": target, "strict": "strict" })
}

/// Opens the reads fixture in a new tab.
async fn reads_page(live: &LiveBrowser) -> TestResult<(FixtureServer, TestPage)> {
    let mut pages = nest_pages();
    pages.insert("/reads.html".to_owned(), Page::html(READS));
    let server = FixtureServer::start(pages).await?;
    let page = live.open(&server.url("127.0.0.1", "/reads.html")).await?;
    Ok((server, page))
}

/// `op` on `selector` with a 5 s timeout, which must succeed.
async fn read(live: &LiveBrowser, page: &TestPage, op: &str, selector: &str) -> TestResult<Value> {
    Ok(live
        .invoke(op, call(op, &at(page, selector), 5_000))
        .await??)
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_each_element_read_returns_what_the_page_holds() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    assert_eq!(
        read(&live, &page, "locator.textContent", "#title").await?,
        json!("Hello there")
    );
    assert_eq!(
        read(&live, &page, "locator.innerText", "#title").await?,
        json!("Hello there")
    );
    assert_eq!(
        read(&live, &page, "locator.innerHTML", "#title").await?,
        json!("Hello <b>there</b>")
    );
    assert_eq!(
        read(&live, &page, "locator.innerText", "#hidden").await?,
        json!("Hidden text"),
        "a display: none element's innerText is its text"
    );
    let attribute = |name: &str| {
        json!({ "call": { "api": "locator.getAttribute", "timeoutMs": 5_000 },
                "target": at(&page, "#title"), "strict": "strict", "name": name })
    };
    assert_eq!(
        live.invoke("locator.getAttribute", attribute("title"))
            .await??,
        json!("Greeting")
    );
    assert_eq!(
        live.invoke("locator.getAttribute", attribute("lang"))
            .await??,
        Value::Null,
        "a missing attribute is null"
    );
    assert_eq!(
        read(&live, &page, "locator.inputValue", "#name").await?,
        json!("Ada")
    );
    assert_eq!(
        read(&live, &page, "locator.inputValue", "#name-label").await?,
        json!("Ada"),
        "inputValue follows a label to its control, as upstream's retarget does"
    );
    assert_eq!(
        read(&live, &page, "locator.boundingBox", "#box").await?,
        json!({ "x": 40.0, "y": 300.0, "width": 120.0, "height": 30.0 })
    );
    assert_eq!(
        read(&live, &page, "locator.boundingBox", "#hidden").await?,
        Value::Null,
        "no layout box, no bounding box"
    );
    let invisible = read(&live, &page, "locator.boundingBox", "#invisible").await?;
    assert!(
        invisible["width"].as_f64().is_some_and(|width| width > 0.0),
        "visibility: hidden keeps its box, as DOM.getBoxModel does: {invisible}"
    );

    let not_html = live
        .invoke(
            "locator.innerText",
            call("locator.innerText", &at(&page, "#logo"), 5_000),
        )
        .await?
        .err()
        .ok_or("an svg has no innerText")?;
    assert_eq!(not_html.error_code(), Some(ErrorCode::InvalidInput));
    assert_eq!(
        not_html.message(),
        "locator.innerText: Error: Node is not an HTMLElement\nCall log:\n  - waiting for locator('#logo')\n"
    );
    let not_a_field = live
        .invoke(
            "locator.inputValue",
            call("locator.inputValue", &at(&page, "#title"), 5_000),
        )
        .await?
        .err()
        .ok_or("a heading has no value")?;
    assert_eq!(
        not_a_field.message(),
        "locator.inputValue: Error: Node is not an <input>, <textarea> or <select> element\nCall log:\n  - waiting for locator('#title')\n"
    );
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_read_waits_for_a_late_element_and_times_out_without_one() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    live.main_world(
        &page,
        "setTimeout(() => { const p = document.createElement('p'); p.id = 'late'; \
         p.textContent = 'Late text'; document.body.append(p) }, 600)",
    )
    .await?;
    let started = Instant::now();
    assert_eq!(
        read(&live, &page, "locator.textContent", "#late").await?,
        json!("Late text")
    );
    assert!(
        started.elapsed() >= Duration::from_millis(500),
        "the read waited for the element: {:?}",
        started.elapsed()
    );

    let started = Instant::now();
    let missing = live
        .invoke(
            "locator.innerHTML",
            call("locator.innerHTML", &at(&page, "#never"), 300),
        )
        .await?
        .err()
        .ok_or("an element that never comes times out")?;
    assert_eq!(missing.error_code(), Some(ErrorCode::NoMatch));
    assert_eq!(
        missing.message(),
        "locator.innerHTML: Timeout 300ms exceeded.\nCall log:\n  - waiting for locator('#never')\n"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "it ended at its own timeout"
    );
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_strict_read_of_two_elements_fails_at_once() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    let buttons = at(&page, "nav button");
    let started = Instant::now();
    let failed = live
        .invoke(
            "locator.innerText",
            call("locator.innerText", &buttons, 5_000),
        )
        .await?
        .err()
        .ok_or("two buttons under strict mode must fail")?;
    assert_eq!(failed.error_code(), Some(ErrorCode::StrictModeViolation));
    assert_eq!(
        failed.message(),
        "locator.innerText: Error: strict mode violation: locator('nav button') resolved to 2 elements:\n    \
         1) <button>One</button> aka getByRole('button', { name: 'One' })\n    \
         2) <button>Two</button> aka getByRole('button', { name: 'Two' })\n\
         \nCall log:\n  - waiting for locator('nav button')\n"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "strict mode fails at once"
    );
    let first = json!({ "call": { "api": "page.innerText", "timeoutMs": 5_000 },
                        "target": buttons, "strict": "firstMatch" });
    assert_eq!(
        live.invoke("locator.innerText", first).await??,
        json!("One"),
        "the page shortcut takes the first match"
    );
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_password_reads_as_the_page_holds_it() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    live.main_world(
        &page,
        "document.getElementById('pw').value = 'typed-secret'",
    )
    .await?;
    assert_eq!(
        read(&live, &page, "locator.inputValue", "#pw").await?,
        json!("typed-secret")
    );
    assert_eq!(
        read(&live, &page, "locator.inputValue", "#pw-label").await?,
        json!("typed-secret"),
        "through its label too"
    );
    let value_attribute = json!({ "call": { "api": "locator.getAttribute", "timeoutMs": 5_000 },
                                  "target": at(&page, "#pw"), "strict": "strict", "name": "value" });
    assert_eq!(
        live.invoke("locator.getAttribute", value_attribute)
            .await??,
        json!("hunter2"),
        "the value attribute is the page's markup"
    );
    assert_eq!(
        read(&live, &page, "locator.inputValue", "#name").await?,
        json!("Ada")
    );
    assert_eq!(
        live.main_world(&page, "document.getElementById('pw').value")
            .await?,
        json!("typed-secret"),
        "the page's own world reads the same value"
    );
    assert_eq!(
        live.main_world(&page, "document.getElementById('pw').getAttribute('value')")
            .await?,
        json!("hunter2")
    );
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_state_reads_answer_as_upstreams_element_state() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    for (op, selector, expected) in [
        ("locator.isEnabled", "#name", true),
        ("locator.isEnabled", "#off", false),
        ("locator.isDisabled", "#off", true),
        ("locator.isDisabled", "#name", false),
        ("locator.isEditable", "#name", true),
        ("locator.isEditable", "#readonly", false),
        ("locator.isChecked", "#agree", true),
        ("locator.isChecked", "#later", false),
        ("locator.isVisible", "#title", true),
        ("locator.isVisible", "#hidden", false),
        ("locator.isVisible", "#invisible", false),
        ("locator.isVisible", "#missing", false),
        ("locator.isHidden", "#hidden", true),
        ("locator.isHidden", "#title", false),
        ("locator.isHidden", "#missing", true),
    ] {
        assert_eq!(
            read(&live, &page, op, selector).await?,
            json!(expected),
            "{op} on {selector}"
        );
    }
    for (op, selector, message) in [
        (
            "locator.isChecked",
            "#title",
            "locator.isChecked: Error: Not a checkbox or radio button\nCall log:\n  - waiting for locator('#title')\n",
        ),
        (
            "locator.isEditable",
            "#title",
            "locator.isEditable: Error: Element is not an <input>, <textarea>, <select> or [contenteditable] and does not have a role allowing [aria-readonly]\nCall log:\n  - waiting for locator('#title')\n",
        ),
    ] {
        let failed = live
            .invoke(op, call(op, &at(&page, selector), 5_000))
            .await?
            .err()
            .ok_or(format!("{op} on a heading must fail"))?;
        assert_eq!(failed.error_code(), Some(ErrorCode::InvalidInput));
        assert_eq!(failed.message(), message);
    }
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_is_visible_answers_at_once_while_is_disabled_waits() -> TestResult {
    let _quiet = BUSY_RENDERER.acquire().await?;
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    live.main_world(
        &page,
        "setTimeout(() => { const b = document.createElement('button'); b.id = 'soon'; \
         b.disabled = true; b.textContent = 'Soon'; document.body.append(b) }, 600)",
    )
    .await?;
    let started = Instant::now();
    assert_eq!(
        read(&live, &page, "locator.isVisible", "#soon").await?,
        json!(false),
        "not there yet is not visible"
    );
    assert_eq!(
        read(&live, &page, "locator.isHidden", "#soon").await?,
        json!(true)
    );
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "neither waited for the element, whatever the call's timeout: {:?}",
        started.elapsed()
    );
    assert_eq!(
        read(&live, &page, "locator.isDisabled", "#soon").await?,
        json!(true)
    );
    assert!(
        started.elapsed() >= Duration::from_millis(500),
        "isDisabled waited for the element: {:?}",
        started.elapsed()
    );
    assert_eq!(
        read(&live, &page, "locator.isVisible", "#soon").await?,
        json!(true)
    );

    let both = live
        .invoke(
            "locator.isVisible",
            call("locator.isVisible", &at(&page, "nav button"), 0),
        )
        .await?
        .err()
        .ok_or("isVisible is strict too")?;
    assert_eq!(both.error_code(), Some(ErrorCode::StrictModeViolation));
    assert!(
        both.message().starts_with(
            "locator.isVisible: Error: strict mode violation: locator('nav button') resolved to 2 elements:"
        ) && both
            .message()
            .ends_with("\nCall log:\n    - checking visibility of locator('nav button')\n"),
        "upstream's message and call-log line: {}",
        both.message()
    );
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_an_element_in_a_hidden_iframe_is_not_visible() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = live
        .open(&server.url("127.0.0.1", "/nest/top.html"))
        .await?;
    let a1 = button_down(&page, &["a1"]);
    assert_eq!(
        live.invoke(
            "locator.textContent",
            call("locator.textContent", &a1, 10_000)
        )
        .await??,
        json!("A1 button")
    );
    let visible = || live.invoke("locator.isVisible", call("locator.isVisible", &a1, 0));
    assert_eq!(visible().await??, json!(true));
    live.main_world(
        &page,
        "document.querySelector('iframe[title=a1]').style.visibility = 'hidden'",
    )
    .await?;
    assert_eq!(
        visible().await??,
        json!(false),
        "the button is visible in its own document, but its iframe is not"
    );
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_every_match_reads_never_wait_and_read_each_match() -> TestResult {
    let _quiet = BUSY_RENDERER.acquire().await?;
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    let every = |op: &str, target: &Value| json!({ "call": { "api": op, "timeoutMs": 0 }, "target": target });
    assert_eq!(
        live.invoke(
            "locator.allTextContents",
            every("locator.allTextContents", &at(&page, "li"))
        )
        .await??,
        json!(["First", "  Second  tail", ""]),
        "textContent keeps whitespace and hidden text"
    );
    assert_eq!(
        live.invoke(
            "locator.allInnerTexts",
            every("locator.allInnerTexts", &at(&page, "li"))
        )
        .await??,
        json!(["First", "Second", ""]),
        "innerText is the rendered text"
    );
    live.main_world(
        &page,
        "setTimeout(() => { const p = document.createElement('p'); p.className = 'later'; \
         document.body.append(p) }, 300)",
    )
    .await?;
    let started = Instant::now();
    assert_eq!(
        live.invoke(
            "locator.allTextContents",
            every("locator.allTextContents", &at(&page, ".later"))
        )
        .await??,
        json!([]),
        "nothing matches yet, and it does not wait"
    );
    assert!(started.elapsed() < Duration::from_millis(250));
    live.close(&page).await?;

    let server = FixtureServer::start(nest_pages()).await?;
    let nest = live
        .open(&server.url("127.0.0.1", "/nest/top.html"))
        .await?;
    let b1 = button_down(&nest, &["b1"]);
    let loaded = live
        .invoke(
            "locator.textContent",
            call("locator.textContent", &b1, 10_000),
        )
        .await??;
    assert_eq!(loaded, json!("B1 button"));
    assert_eq!(
        live.invoke("locator.allInnerTexts", every("locator.allInnerTexts", &b1))
            .await??,
        json!(["B1 button"]),
        "every match inside an out-of-process frame"
    );
    assert_eq!(
        live.invoke(
            "locator.allTextContents",
            every("locator.allTextContents", &button_down(&nest, &["nowhere"]))
        )
        .await??,
        json!([]),
        "a frame that is not there matches nothing"
    );
    live.close(&nest).await
}

/// `locator.elementHandle` on `target`, which must hand back a handle.
async fn handle_of(live: &LiveBrowser, target: &Value) -> TestResult<Value> {
    let handle = live
        .invoke(
            "locator.elementHandle",
            call("locator.elementHandle", target, 5_000),
        )
        .await??;
    assert_eq!(
        handle.get("$neo").and_then(Value::as_str),
        Some("element"),
        "a handle brand: {handle}"
    );
    Ok(handle)
}

/// `op` on an element handle, as `elementHandle.<method>()` sends it (no timeout).
async fn on_handle(
    live: &LiveBrowser,
    op: &str,
    handle: &Value,
) -> TestResult<Result<Value, neo_playwright::PlaywrightError>> {
    let api = op.replace("locator.", "elementHandle.");
    live.invoke(op, call(&api, handle, 0)).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_an_element_handle_reads_its_node_until_the_node_goes() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    let title = handle_of(&live, &at(&page, "#title")).await?;
    assert_eq!(
        (title["page"].clone(), title["display"].clone()),
        (
            page.brand(),
            json!("<h1 id=\"title\" title=\"Greeting\">…</h1>")
        ),
        "the handle names its page and prints upstream's preview"
    );
    assert_eq!(
        on_handle(&live, "locator.textContent", &title).await??,
        json!("Hello there")
    );
    assert_eq!(
        on_handle(&live, "locator.isVisible", &title).await??,
        json!(true)
    );
    assert_eq!(
        on_handle(&live, "locator.boundingBox", &title).await??,
        read(&live, &page, "locator.boundingBox", "#title").await?,
        "a handle's box is its locator's"
    );
    let password = handle_of(&live, &at(&page, "#pw")).await?;
    assert_eq!(
        on_handle(&live, "locator.inputValue", &password).await??,
        json!("hunter2"),
        "a handle reads the field as its locator does"
    );

    live.main_world(&page, "document.getElementById('title').remove()")
        .await?;
    let gone = on_handle(&live, "locator.textContent", &title)
        .await?
        .err()
        .ok_or("a handle whose node left the document is detached")?;
    assert_eq!(gone.error_code(), Some(ErrorCode::ElementDetached));
    assert_eq!(
        gone.message(),
        "elementHandle.textContent: Element is not attached to the DOM"
    );
    assert_eq!(
        on_handle(&live, "locator.isHidden", &title).await??,
        json!(true),
        "a detached node is hidden, as upstream answers"
    );

    live.main_world(&page, "location.reload()").await?;
    time::sleep(Duration::from_millis(500)).await;
    let reloaded = on_handle(&live, "locator.inputValue", &password)
        .await?
        .err()
        .ok_or("a handle into a replaced document is detached for good")?;
    assert_eq!(reloaded.error_code(), Some(ErrorCode::ElementDetached));
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_handle_inside_an_oopif_reads_and_boxes_like_its_locator() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = live
        .open(&server.url("127.0.0.1", "/nest/top.html"))
        .await?;
    let c2 = button_down(&page, &["b1", "c2"]);
    let handle = handle_of(&live, &c2).await?;
    assert_eq!(
        on_handle(&live, "locator.innerText", &handle).await??,
        json!("C2 button")
    );
    assert_eq!(
        on_handle(&live, "locator.boundingBox", &handle).await??,
        live.invoke(
            "locator.boundingBox",
            call("locator.boundingBox", &c2, 5_000)
        )
        .await??,
        "the handle finds its frame's parents from the links its hops recorded"
    );
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_dollar_dollar_hands_out_all_600_matches_in_order() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    live.main_world(
        &page,
        "const list = document.createElement('ol'); list.id = 'many'; \
         for (let i = 0; i < 600; i++) { const li = document.createElement('li'); \
         li.textContent = `item ${i}`; list.append(li) } document.body.append(list)",
    )
    .await?;
    let every =
        json!({ "call": { "api": "page.$$", "timeoutMs": 0 }, "target": at(&page, "#many li") });
    let handles = live.invoke("locator.elementHandles", every).await??;
    let handles = handles.as_array().ok_or("a list of handles")?;
    assert_eq!(
        handles.len(),
        600,
        "one handle per match, past the probe's 512-pin limit"
    );
    let nodes: HashSet<u64> = handles
        .iter()
        .filter_map(|handle| handle.get("node").and_then(Value::as_u64))
        .collect();
    assert_eq!(nodes.len(), 600, "600 distinct nodes");
    for index in [0, 1, 511, 512, 599] {
        let handle = handles.get(index).ok_or("a handle per index")?;
        assert_eq!(
            on_handle(&live, "locator.textContent", handle).await??,
            json!(format!("item {index}")),
            "in document order"
        );
    }
    live.close(&page).await
}

/// `selector` queried from `handle`'s node, as `elementHandle.$` and `$$` send it.
fn rooted(page: &TestPage, handle: &Value, selector: &str) -> Value {
    let mut target = locator(page, &[css(selector)], &format!("locator('{selector}')"));
    if let Some(fields) = target.as_object_mut() {
        fields.insert("frame".to_owned(), Value::Null);
        fields.insert("root".to_owned(), handle.clone());
    }
    target
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_handles_dollar_and_dollar_dollar_query_under_its_node() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    let first = |target: Value| {
        json!({ "call": { "api": "elementHandle.$", "timeoutMs": 0 }, "target": target,
                "strict": "firstMatch", "noWait": true })
    };
    let every = |target: Value| json!({ "call": { "api": "elementHandle.$$", "timeoutMs": 0 }, "target": target });
    let nav = handle_of(&live, &at(&page, "nav")).await?;
    let one = live
        .invoke(
            "locator.elementHandle",
            first(rooted(&page, &nav, "button")),
        )
        .await??;
    assert_eq!(
        one["display"], "<button>One</button>",
        "the first match under nav"
    );
    let all = live
        .invoke(
            "locator.elementHandles",
            every(rooted(&page, &nav, "button")),
        )
        .await??;
    let mut texts = Vec::new();
    for handle in all.as_array().ok_or("a list of handles")? {
        texts.push(on_handle(&live, "locator.textContent", handle).await??);
    }
    assert_eq!(texts, [json!("One"), json!("Two")], "both of nav's buttons");

    let title = handle_of(&live, &at(&page, "#title")).await?;
    assert_eq!(
        live.invoke(
            "locator.elementHandle",
            first(rooted(&page, &title, "button"))
        )
        .await??,
        Value::Null,
        "the page's buttons are not under the heading"
    );
    let bold = live
        .invoke("locator.elementHandles", every(rooted(&page, &title, "b")))
        .await??;
    assert_eq!(bold.as_array().map(Vec::len), Some(1), "{bold}");

    live.main_world(&page, "document.querySelector('nav').remove()")
        .await?;
    let gone = live
        .invoke(
            "locator.elementHandle",
            first(rooted(&page, &nav, "button")),
        )
        .await?
        .err()
        .ok_or("a handle whose node is gone roots nothing")?;
    assert_eq!(gone.error_code(), Some(ErrorCode::ElementDetached));
    live.close(&page).await?;

    // Under a handle inside an OOPIF two levels down, in that frame's own document.
    let server = FixtureServer::start(nest_pages()).await?;
    let nest = live
        .open(&server.url("127.0.0.1", "/nest/top.html"))
        .await?;
    let c2_body = {
        let mut parts = Vec::new();
        for title in ["b1", "c2"] {
            parts.push(attr(&format!("[title=\"{title}\"s]")));
            parts.push(enter_frame());
        }
        parts.push(css("body"));
        locator(&nest, &parts, "c2 body")
    };
    let body = handle_of(&live, &c2_body).await?;
    let button = live
        .invoke(
            "locator.elementHandle",
            first(rooted(&nest, &body, "button")),
        )
        .await??;
    assert_eq!(
        on_handle(&live, "locator.innerText", &button).await??,
        json!("C2 button")
    );
    live.close(&nest).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_dollar_answers_now_and_dollar_dollar_hands_every_match() -> TestResult {
    let _quiet = BUSY_RENDERER.acquire().await?;
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    let first_now = |selector: &str| {
        json!({ "call": { "api": "page.$", "timeoutMs": 0 }, "target": at(&page, selector),
                "strict": "firstMatch", "noWait": true })
    };
    let started = Instant::now();
    assert_eq!(
        live.invoke("locator.elementHandle", first_now("#nowhere"))
            .await??,
        Value::Null,
        "page.$ is null when nothing matches"
    );
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "and it does not wait"
    );
    let first = live
        .invoke("locator.elementHandle", first_now("nav button"))
        .await??;
    assert_eq!(first["display"], "<button>One</button>", "the first match");

    let every =
        json!({ "call": { "api": "page.$$", "timeoutMs": 0 }, "target": at(&page, "nav button") });
    let handles = live.invoke("locator.elementHandles", every).await??;
    let handles = handles.as_array().ok_or("a list of handles")?;
    let mut texts = Vec::new();
    for handle in handles {
        texts.push(on_handle(&live, "locator.textContent", handle).await??);
    }
    assert_eq!(texts, [json!("One"), json!("Two")], "one handle per match");
    let none =
        json!({ "call": { "api": "page.$$", "timeoutMs": 0 }, "target": at(&page, "#nowhere") });
    assert_eq!(
        live.invoke("locator.elementHandles", none).await??,
        json!([])
    );
    live.close(&page).await
}

/// `locator.waitFor({ state, timeout })`.
fn wait_for(target: &Value, state: &str, timeout_ms: u64) -> Value {
    json!({ "call": { "api": "locator.waitFor", "timeoutMs": timeout_ms }, "target": target,
            "strict": "strict", "state": state, "omitReturnValue": true })
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_wait_for_waits_for_each_of_its_four_states() -> TestResult {
    let _quiet = BUSY_RENDERER.acquire().await?;
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    let late = at(&page, "#late");
    // Each change comes from the test 300 ms into its wait, not from the page's own timers,
    // which a background tab throttles.
    for (state, change) in [
        (
            "attached",
            "const p = document.createElement('p'); p.id = 'late'; p.textContent = 'Late'; \
             p.style.display = 'none'; document.body.append(p)",
        ),
        (
            "visible",
            "document.getElementById('late').style.display = 'block'",
        ),
        (
            "hidden",
            "document.getElementById('late').style.display = 'none'",
        ),
        ("detached", "document.getElementById('late').remove()"),
    ] {
        let started = Instant::now();
        let change_later = async {
            time::sleep(Duration::from_millis(300)).await;
            live.main_world(&page, change).await
        };
        let wait = async {
            let waited = live
                .invoke("locator.waitFor", wait_for(&late, state, 5_000))
                .await;
            (waited, started.elapsed())
        };
        let ((waited, took), changed) = tokio::join!(wait, change_later);
        changed?;
        assert_eq!(waited??, Value::Null);
        assert!(
            took >= Duration::from_millis(300),
            "{state} waited for the page: {took:?}"
        );
    }
    let started = Instant::now();
    for state in ["hidden", "detached"] {
        assert_eq!(
            live.invoke("locator.waitFor", wait_for(&late, state, 5_000))
                .await??,
            Value::Null,
            "nothing there is already {state}"
        );
    }
    assert!(started.elapsed() < Duration::from_millis(250));

    let handle = live
        .invoke(
            "locator.waitFor",
            json!({ "call": { "api": "page.waitForSelector", "timeoutMs": 5_000 },
                    "target": at(&page, "#name"), "strict": "firstMatch", "state": "visible",
                    "omitReturnValue": false }),
        )
        .await??;
    assert_eq!(
        handle["display"], "<input id=\"name\" value=\"Ada\"/>",
        "page.waitForSelector hands back the element: {handle}"
    );
    let not_a_locator_state = live
        .invoke("locator.waitFor", wait_for(&late, "enabled", 5_000))
        .await?
        .err()
        .ok_or("a locator waits for four states only")?;
    assert_eq!(
        not_a_locator_state.message(),
        "locator.waitFor: state: expected one of (attached|detached|visible|hidden)"
    );
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_wait_for_times_out_with_upstreams_call_log() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    let hidden = live
        .invoke(
            "locator.waitFor",
            wait_for(&at(&page, "#hidden"), "visible", 600),
        )
        .await?
        .err()
        .ok_or("a display: none element never becomes visible")?;
    assert_eq!(hidden.error_code(), Some(ErrorCode::NotActionable));
    let message = hidden.message();
    assert!(
        message.starts_with(
            "locator.waitFor: Timeout 600ms exceeded.\nCall log:\n  - waiting for locator('#hidden') to be visible\n"
        ) && message.contains(" × locator resolved to hidden <p id=\"hidden\">Hidden text</p>\n"),
        "upstream's lines, the repeated poll compressed: {message}"
    );
    let missing = live
        .invoke(
            "locator.waitFor",
            wait_for(&at(&page, "#nowhere"), "attached", 300),
        )
        .await?
        .err()
        .ok_or("nothing ever matches")?;
    assert_eq!(missing.error_code(), Some(ErrorCode::NoMatch));
    assert_eq!(
        missing.message(),
        "locator.waitFor: Timeout 300ms exceeded.\nCall log:\n  - waiting for locator('#nowhere')\n"
    );
    let first_of_two = json!({ "call": { "api": "page.waitForSelector", "timeoutMs": 300 },
                               "target": at(&page, "nav button"), "strict": "firstMatch",
                               "state": "hidden", "omitReturnValue": false });
    let stays = live
        .invoke("locator.waitFor", first_of_two)
        .await?
        .err()
        .ok_or("the first button never hides")?;
    assert_eq!(stays.error_code(), Some(ErrorCode::WaitTimeout));
    assert!(
        stays.message().contains(
            "locator resolved to 2 elements. Proceeding with the first one: <button>One</button>"
        ),
        "{}",
        stays.message()
    );
    let strict = live
        .invoke(
            "locator.waitFor",
            wait_for(&at(&page, "nav button"), "detached", 5_000),
        )
        .await?
        .err()
        .ok_or("a strict wait on two elements fails")?;
    assert_eq!(strict.error_code(), Some(ErrorCode::StrictModeViolation));
    live.close(&page).await
}

/// Upstream's `should survive cross-process navigation` (page-wait-for-selector-2.spec.ts:23):
/// a wait goes on polling while its page navigates within its site and then from site to site,
/// and finds the element in the last process's document. A poll that lands mid-navigation, its
/// frame gone before selector resolution, is retried in the current document. The page
/// changes process four times so that some poll lands mid-way (one change failed 2 of 3 runs
/// before the fix).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_wait_survives_a_cross_process_navigation() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(HashMap::from([
        ("/empty.html".to_owned(), Page::html("<p>empty</p>")),
        (
            "/box.html".to_owned(),
            Page::html("<div class=\"box\">box</div>"),
        ),
    ]))
    .await?;
    let page = live.open(&server.url("127.0.0.1", "/empty.html")).await?;
    let goto = |url: String| live.invoke("page.goto", json!({ "page": page.brand(), "url": url }));
    let wait = live.invoke(
        "locator.waitFor",
        json!({ "call": { "api": "page.waitForSelector", "timeoutMs": 10_000 },
                "target": at(&page, ".box"), "strict": "firstMatch", "state": "visible",
                "omitReturnValue": true }),
    );
    let navigate = async {
        goto(server.url("127.0.0.1", "/empty.html?again")).await??;
        live.invoke("page.reload", json!({ "page": page.brand() }))
            .await??;
        for host in ["localhost", "127.0.0.1", "localhost"] {
            goto(server.url(host, "/empty.html")).await??;
        }
        goto(server.url("127.0.0.1", "/box.html")).await??;
        TestResult::Ok(())
    };
    let (waited, navigated) = tokio::join!(wait, navigate);
    navigated?;
    assert_eq!(
        waited??,
        Value::Null,
        "the wait found the box on the other site"
    );
    live.close(&page).await
}

/// `elementHandle.waitForElementState(state, { timeout })`.
fn element_state(handle: &Value, state: &str, timeout_ms: u64) -> Value {
    json!({ "call": { "api": "elementHandle.waitForElementState", "timeoutMs": timeout_ms },
            "target": handle, "strict": "strict", "state": state, "omitReturnValue": true })
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_handle_waits_for_its_element_states() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    for (selector, state, change) in [
        (
            "#off",
            "enabled",
            "document.getElementById('off').disabled = false",
        ),
        (
            "#hidden",
            "visible",
            "document.getElementById('hidden').style.display = 'block'",
        ),
        (
            "#name",
            "hidden",
            "document.getElementById('name').style.visibility = 'hidden'",
        ),
    ] {
        let handle = handle_of(&live, &at(&page, selector)).await?;
        let started = Instant::now();
        let wait = async {
            let waited = live
                .invoke("locator.waitFor", element_state(&handle, state, 5_000))
                .await;
            (waited, started.elapsed())
        };
        let change_later = async {
            time::sleep(Duration::from_millis(300)).await;
            live.main_world(&page, change).await
        };
        let ((waited, took), changed) = tokio::join!(wait, change_later);
        changed?;
        assert_eq!(waited??, Value::Null);
        assert!(
            took >= Duration::from_millis(300),
            "{state} waited for the page: {took:?}"
        );
    }
    let title = handle_of(&live, &at(&page, "#title")).await?;
    assert_eq!(
        live.invoke("locator.waitFor", element_state(&title, "stable", 5_000))
            .await??,
        Value::Null,
        "a heading that does not move is stable"
    );

    let fixed = handle_of(&live, &at(&page, "#readonly")).await?;
    let never = live
        .invoke("locator.waitFor", element_state(&fixed, "editable", 400))
        .await?
        .err()
        .ok_or("a readonly field never becomes editable")?;
    assert_eq!(never.error_code(), Some(ErrorCode::NotActionable));
    let message = never.message();
    assert!(
        message.starts_with(
            "elementHandle.waitForElementState: Timeout 400ms exceeded.\nCall log:\n  \
             - attempting wait for editable action\n"
        ) && message.contains("× element is not editable")
            && message.contains("- retrying wait for editable action")
            && message.contains("- waiting 20ms"),
        "upstream's retry lines: {message}"
    );
    let attached = live
        .invoke("locator.waitFor", element_state(&fixed, "attached", 400))
        .await?
        .err()
        .ok_or("a handle has no attached state to wait for")?;
    assert_eq!(
        attached.message(),
        "elementHandle.waitForElementState: state: expected one of (visible|hidden|stable|enabled|disabled|editable)"
    );

    live.main_world(&page, "document.getElementById('readonly').remove()")
        .await?;
    assert_eq!(
        live.invoke("locator.waitFor", element_state(&fixed, "hidden", 5_000))
            .await??,
        Value::Null,
        "a node that is gone is hidden"
    );
    let gone = live
        .invoke("locator.waitFor", element_state(&fixed, "enabled", 5_000))
        .await?
        .err()
        .ok_or("a node that is gone is never enabled")?;
    assert_eq!(gone.error_code(), Some(ErrorCode::ElementDetached));
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_busy_renderer_cannot_hold_a_wait_past_its_timeout() -> TestResult {
    let _busy = BUSY_RENDERER.acquire_many(QUIET_TESTS).await?;
    let live = LiveBrowser::connect().await?;
    // Measured: each tab this suite opens gets a renderer of its own, same site or not, so
    // busying this one holds no other test's CDP calls.
    let (_server, page) = reads_page(&live).await?;
    let invisible = handle_of(&live, &at(&page, "#invisible")).await?;
    let hidden = at(&page, "#hidden");
    // The page's own thread is busy for 1.2 s from now: every CDP call into the renderer,
    // finding a handle's node first among them, waits for it.
    live.main_world(
        &page,
        "setTimeout(() => { const end = Date.now() + 1200; while (Date.now() < end) {} }, 0)",
    )
    .await?;
    time::sleep(Duration::from_millis(50)).await;
    let live = &live;
    let timed = |op: &'static str, input: Value| async move {
        let started = Instant::now();
        let waited = live.invoke(op, input).await;
        (op, waited, started.elapsed())
    };
    let waits = tokio::join!(
        timed("locator.waitFor", element_state(&invisible, "visible", 300)),
        timed("locator.waitFor", wait_for(&hidden, "visible", 300)),
        timed(
            "locator.textContent",
            call("locator.textContent", &at(&page, "#nothing"), 300)
        ),
    );
    for (api, waited, took) in [waits.0, waits.1, waits.2] {
        let error = waited?.err().ok_or("the element never gets there")?;
        assert!(
            error.message().contains("Timeout 300ms exceeded."),
            "{api}: {}",
            error.message()
        );
        assert!(
            took < Duration::from_millis(900),
            "{api} ends at its own deadline, not when the renderer frees up: {took:?}"
        );
    }
    time::sleep(Duration::from_millis(1_000)).await;
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_moving_element_is_never_stable() -> TestResult {
    let _quiet = BUSY_RENDERER.acquire().await?;
    let live = LiveBrowser::connect().await?;
    let (_server, page) = reads_page(&live).await?;
    live.main_world(
        &page,
        "const d = document.createElement('div'); d.id = 'mover'; \
         d.style.cssText = 'position: absolute; top: 400px; width: 20px; height: 20px'; \
         document.body.append(d)",
    )
    .await?;
    let mover = handle_of(&live, &at(&page, "#mover")).await?;
    // The test moves it every few milliseconds while the wait runs, rather than a CSS
    // animation, which stops in a tab a parallel test's new tab has put in the background.
    let started = Instant::now();
    let keep_moving = async {
        let mut left = 0;
        while started.elapsed() < Duration::from_millis(700) {
            left = (left + 7) % 300;
            let step = format!("document.getElementById('mover').style.left = '{left}px'");
            live.main_world(&page, &step).await?;
            time::sleep(Duration::from_millis(4)).await;
        }
        TestResult::Ok(())
    };
    let (waited, moved) = tokio::join!(
        live.invoke("locator.waitFor", element_state(&mover, "stable", 500)),
        keep_moving
    );
    moved?;
    let moving = waited?
        .err()
        .ok_or("an element that keeps moving is never stable")?;
    assert_eq!(moving.error_code(), Some(ErrorCode::NotActionable));
    assert!(
        moving.message().contains("- element is not stable"),
        "{}",
        moving.message()
    );
    live.close(&page).await
}

/// A session of the test's own to the out-of-process frame at `url`. The URL carries the
/// test's own server port, so a tab another test left open is never the one found.
async fn attach_frame(live: &LiveBrowser, url: &str) -> TestResult<SessionId> {
    let targets = live
        .browser
        .cdp("Target.getTargets", json!({}), None)
        .await?;
    let frame = targets
        .get("targetInfos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|info| {
            info.get("type").and_then(Value::as_str) == Some("iframe")
                && info.get("url").and_then(Value::as_str) == Some(url)
        })
        .ok_or(format!("{url} is its own target"))?;
    let attached = live
        .browser
        .cdp(
            "Target.attachToTarget",
            json!({ "targetId": frame.get("targetId"), "flatten": true }),
            None,
        )
        .await?;
    let session = attached
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or("no session id")?;
    Ok(SessionId::from(session))
}

/// Ends a session [`attach_frame`] opened; the frame's target lives on.
async fn detach(live: &LiveBrowser, session: &SessionId) {
    let _detached = live
        .browser
        .cdp(
            "Target.detachFromTarget",
            json!({ "sessionId": session.as_str() }),
            None,
        )
        .await;
}

/// `expression`'s value in the main world of the frame `session` is attached to.
async fn in_frame(live: &LiveBrowser, session: &SessionId, expression: &str) -> TestResult<Value> {
    let result = live
        .browser
        .cdp(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true }),
            Some(session),
        )
        .await?;
    Ok(result
        .pointer("/result/value")
        .cloned()
        .unwrap_or(Value::Null))
}

/// Clicks at `(x, y)` on the page's root session, with the tab in front, until the fixture
/// frame `frame` records a click (`window.__state.clicks`), and returns what it recorded.
async fn click_until_seen(
    live: &LiveBrowser,
    page: &TestPage,
    frame: &SessionId,
    x: f64,
    y: f64,
) -> TestResult<Vec<Value>> {
    let session = live.browser.pages.get_session(page.id.clone()).await?;
    for _ in 0..10 {
        session
            .session
            .send::<_, Value>("Page.bringToFront", json!({}))
            .await?;
        mouse(live, page, "mousePressed", x, y).await?;
        mouse(live, page, "mouseReleased", x, y).await?;
        let clicks = in_frame(live, frame, "window.__state.clicks").await?;
        if let Some(clicks) = clicks.as_array().filter(|clicks| !clicks.is_empty()) {
            return Ok(clicks.clone());
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    Err(format!("no click at ({x}, {y}) reached the frame").into())
}

/// One mouse event on the page's root session, in main-viewport CSS pixels.
async fn mouse(live: &LiveBrowser, page: &TestPage, kind: &str, x: f64, y: f64) -> TestResult {
    let session = live.browser.pages.get_session(page.id.clone()).await?;
    session
        .session
        .send::<_, Value>(
            "Input.dispatchMouseEvent",
            json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": 1 }),
        )
        .await?;
    Ok(())
}

/// A box's field as a number.
fn field(rect: &Value, name: &str) -> TestResult<f64> {
    Ok(rect[name].as_f64().ok_or(format!("{rect} has no {name}"))?)
}

/// `frameLocator('[title=…]')` down `titles`, then the frame's button.
fn button_down(page: &TestPage, titles: &[&str]) -> Value {
    let mut parts = Vec::new();
    for title in titles {
        parts.push(attr(&format!("[title=\"{title}\"s]")));
        parts.push(enter_frame());
    }
    parts.push(role("button"));
    locator(page, &parts, &format!("{titles:?} >> getByRole('button')"))
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_box_in_a_nested_oopif_is_where_input_reaches_it() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = live
        .open(&server.url("127.0.0.1", "/nest/top.html"))
        .await?;
    let c4 = button_down(&page, &["b1", "c2", "a3", "c4"]);
    let loaded = live
        .invoke(
            "locator.textContent",
            call("locator.textContent", &c4, 10_000),
        )
        .await??;
    assert_eq!(loaded, json!("C4 button"), "every level loaded");

    // c4 is 4 iframes down (b1, c2 and c4 out of process, a3 its own target), below the fold
    // of every 260 px frame. Each parent scrolls its iframe to its own top, which puts the
    // button in the main viewport. The test's sessions share core's connection, and core takes
    // an attached frame session as that frame's own, so each stays until the ops are done.
    let mut sessions = Vec::new();
    for (host, parent) in [
        ("localhost", "/nest/b1.html"),
        ("[::1]", "/nest/c2.html"),
        ("127.0.0.1", "/nest/a3.html"),
    ] {
        let session = attach_frame(&live, &server.url(host, parent)).await?;
        in_frame(
            &live,
            &session,
            "scrollTo(0, document.querySelector('iframe').offsetTop)",
        )
        .await?;
        sessions.push(session);
    }
    let frame = attach_frame(&live, &server.url("[::1]", "/nest/c4.html")).await?;
    let local = in_frame(
        &live,
        &frame,
        "window.__moves = []; addEventListener('mousemove', (e) => __moves.push([e.clientX, e.clientY]), true); \
         (() => { const r = document.querySelector('button').getBoundingClientRect(); \
         return { x: r.x, y: r.y, width: r.width, height: r.height } })()",
    )
    .await?;
    let box_ = live
        .invoke(
            "locator.boundingBox",
            call("locator.boundingBox", &c4, 5_000),
        )
        .await??;
    assert_eq!(
        (field(&box_, "width")?, field(&box_, "height")?),
        (field(&local, "width")?, field(&local, "height")?),
        "the same box, moved: {box_} from {local}"
    );
    assert!(
        field(&box_, "x")? > field(&local, "x")? && field(&box_, "y")? > field(&local, "y")?,
        "four frames' origins were added: {box_} from {local}"
    );

    // Near the box's top-left corner, so a frame's border left out would miss the button.
    // The browser routes input into a frame by hit-test data it refreshes after the scroll, and
    // only for the tab in front, which a parallel test's new tab may take: the tab comes to
    // the front and the move is sent again until c4 has seen one.
    let (x, y) = (field(&box_, "x")? + 3.0, field(&box_, "y")? + 3.0);
    let session = live.browser.pages.get_session(page.id.clone()).await?;
    let mut moves = Value::Null;
    for _ in 0..20 {
        session
            .session
            .send::<_, Value>("Page.bringToFront", json!({}))
            .await?;
        mouse(&live, &page, "mouseMoved", x, y).await?;
        moves = in_frame(&live, &frame, "__moves").await?;
        if moves.as_array().is_some_and(|moves| !moves.is_empty()) {
            break;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    let reached = moves[0].clone();
    let (Some(reached_x), Some(reached_y)) = (reached[0].as_f64(), reached[1].as_f64()) else {
        return Err(format!("a move at the box's corner ({x}, {y}) never reached c4").into());
    };
    assert!(
        (reached_x - (field(&local, "x")? + 3.0)).abs() <= 1.0
            && (reached_y - (field(&local, "y")? + 3.0)).abs() <= 1.0,
        "a move to the box's corner reached c4 at its button's corner: {reached} for {local}"
    );
    let (center_x, center_y) = (
        field(&box_, "x")? + field(&box_, "width")? / 2.0,
        field(&box_, "y")? + field(&box_, "height")? / 2.0,
    );
    let clicks = click_until_seen(&live, &page, &frame, center_x, center_y).await?;
    assert!(
        !clicks.is_empty() && clicks.iter().all(|click| click == "C4 button"),
        "a click at the box's center lands on c4's button: {clicks:?}"
    );
    sessions.push(frame);
    for session in &sessions {
        detach(&live, session).await;
    }
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_box_in_a_same_process_frame_is_the_pages_own_measure() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = live
        .open(&server.url("127.0.0.1", "/nest/top.html"))
        .await?;
    // a1 shares the top's process and origin, so the page itself can say where its button is.
    let a1 = live
        .invoke(
            "locator.boundingBox",
            call("locator.boundingBox", &button_down(&page, &["a1"]), 5_000),
        )
        .await??;
    let expected = live
        .main_world(
            &page,
            "(() => { const f = document.querySelector('iframe[title=a1]'); \
             const o = f.getBoundingClientRect(); \
             const b = f.contentDocument.querySelector('button').getBoundingClientRect(); \
             return { x: o.x + f.clientLeft + b.x, y: o.y + f.clientTop + b.y, width: b.width, height: b.height } })()",
        )
        .await?;
    for name in ["x", "y", "width", "height"] {
        assert!(
            (field(&a1, name)? - field(&expected, name)?).abs() < 0.01,
            "a1's box, {name}: {a1} against the page's own {expected}"
        );
    }
    live.close(&page).await
}

/// The scaled-frame fixture (`tests/data/scaled/`) on `127.0.0.1`, its OOPIF on `localhost`.
fn scaled_pages() -> HashMap<String, Page> {
    [
        (
            "/scaled/top.html".to_owned(),
            Page::html(include_str!("data/scaled/top.html")),
        ),
        (
            "/scaled/child.html".to_owned(),
            Page::html(include_str!("data/scaled/child.html")),
        ),
    ]
    .into()
}

/// The border box CDP measures for the button of the same-process iframe `id`, in the main
/// viewport (`DOM.getBoxModel` on the page's session, which composes every transform).
async fn measured_box(live: &LiveBrowser, page: &TestPage, id: &str) -> TestResult<[f64; 4]> {
    let session = live
        .browser
        .pages
        .get_session(page.id.clone())
        .await?
        .session;
    let expression =
        format!("document.getElementById('{id}').contentDocument.querySelector('button')");
    let button: Value = session
        .send("Runtime.evaluate", json!({ "expression": expression }))
        .await?;
    let object = button
        .pointer("/result/objectId")
        .cloned()
        .ok_or("the button has an object id")?;
    let model: Value = session
        .send("DOM.getBoxModel", json!({ "objectId": object }))
        .await?;
    let quad: Vec<f64> = model
        .pointer("/model/border")
        .and_then(Value::as_array)
        .ok_or("a border quad")?
        .iter()
        .filter_map(Value::as_f64)
        .collect();
    let xs = quad.iter().step_by(2).copied();
    let ys = quad.iter().skip(1).step_by(2).copied();
    let (left, right) = xs.fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
    let (top, bottom) = ys.fold((f64::MAX, f64::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
    Ok([left, top, right - left, bottom - top])
}

/// A box as `[x, y, width, height]`.
fn rect_of(rect: &Value) -> TestResult<[f64; 4]> {
    Ok([
        field(rect, "x")?,
        field(rect, "y")?,
        field(rect, "width")?,
        field(rect, "height")?,
    ])
}

/// Two boxes agree to a hundredth of a pixel.
fn close(got: [f64; 4], want: [f64; 4]) -> bool {
    got.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.01)
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_box_in_a_scaled_iframe_is_scaled_and_a_rotated_one_is_refused() -> TestResult
{
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(scaled_pages()).await?;
    let page = live
        .open(&server.url("127.0.0.1", "/scaled/top.html"))
        .await?;
    let box_in = |title: &str| {
        let target = button_down(&page, &[title]);
        let live = &live;
        async move {
            live.invoke(
                "locator.boundingBox",
                call("locator.boundingBox", &target, 5_000),
            )
            .await
        }
    };

    // translate(10px, 5px) scale(2) on a 2px border and 3px padding: the button at (10, 8) in
    // the frame, 40 by 20, is at (60, 61), 80 by 40, as CDP measures it.
    let same = rect_of(&box_in("same").await??)?;
    let measured = measured_box(&live, &page, "same").await?;
    assert!(close(same, measured), "{same:?} against CDP's {measured:?}");
    assert!(close(same, [60.0, 61.0, 80.0, 40.0]), "{same:?}");

    // scale(1.5) on an out-of-process iframe: (20 + 15, 300 + 12), 60 by 30. The page's own
    // measures agree: the iframe's rendered rect in the top, and the button's rect in the
    // frame, scaled by the rendered width over the layout width. (A click at the center is
    // no check here: a parallel test's new tab hides this one, and a hidden tab's OOPIF gets
    // no input.)
    let cross = rect_of(&box_in("cross").await??)?;
    assert!(close(cross, [35.0, 312.0, 60.0, 30.0]), "{cross:?}");
    let iframe = live
        .main_world(
            &page,
            "(() => { const f = document.getElementById('cross'); const r = f.getBoundingClientRect(); \
             return [r.x, r.y, r.width / f.offsetWidth, r.height / f.offsetHeight] })()",
        )
        .await?;
    let frame = attach_frame(&live, &server.url("localhost", "/scaled/child.html")).await?;
    let local = in_frame(
        &live,
        &frame,
        "(() => { const r = document.querySelector('button').getBoundingClientRect(); \
         return { x: r.x, y: r.y, width: r.width, height: r.height } })()",
    )
    .await;
    detach(&live, &frame).await;
    let local = rect_of(&local?)?;
    let measure = |index: usize| {
        iframe
            .get(index)
            .and_then(Value::as_f64)
            .ok_or(format!("{iframe} has no item {index}"))
    };
    let (left, top, scale_x, scale_y) = (measure(0)?, measure(1)?, measure(2)?, measure(3)?);
    let page_measure = [
        left + scale_x * local[0],
        top + scale_y * local[1],
        scale_x * local[2],
        scale_y * local[3],
    ];
    assert!(
        close(cross, page_measure),
        "{cross:?} against the page's own {page_measure:?}"
    );

    for title in ["rotated", "skewed", "inside-rotated"] {
        let refused = box_in(title)
            .await?
            .err()
            .ok_or(format!("no box in a {title} iframe"))?;
        assert_eq!(
            refused.error_code(),
            Some(ErrorCode::Unsupported),
            "{title}"
        );
        assert!(
            refused
                .message()
                .contains("rotated, skewed or flipped iframe"),
            "{title}: {}",
            refused.message()
        );
    }
    live.close(&page).await
}
