//! Retrying `expect` matchers on a real browser, through `Playwright::invoke`: every v1 locator
//! and page matcher passing and failing as scripts write them (the real prelude in QuickJS),
//! `.not`, a matcher that holds only after the page changes, the timeout message and call log,
//! `toHaveValue` on a typed password through the seal, a password's `value` attribute compared
//! as markup after an edit, and a matcher four frames deep.
//!
//! Needs a throwaway BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005), for example
//! `PORT_BASE=9511 spikes/eval/browser.sh <profile> &`, then
//! `BROWSEROS_CDP_PORT=9511 cargo test -p neo-playwright --test expect_live -- --ignored`.
//! Every test opens its own tab and closes it.

#[path = "support/fixture_server.rs"]
mod fixture_server;
#[path = "support/js_runner.rs"]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "the expect tests set pages up with scripts, not trusted clicks or captures"
)]
mod live_run;
#[path = "support/locator.rs"]
#[expect(
    dead_code,
    reason = "these locators are CSS, frame titles and roles, never text or refs"
)]
mod locator;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fixture_server::{FixtureServer, Page, nest_pages};
use js_runner::{OpHost, run_script};
use live_run::{EnvCall, LiveRun, TestPage, TestResult, brand};
use locator::{attr, css, enter_frame, locator, role};
use neo_playwright::{ErrorCode, PlaywrightError, TimedOut};
use neo_trace::ErrorOwner;
use serde_json::{Map, Value, json};

/// The nested fixture, and this suite's page under `/expect/page.html`.
fn pages() -> HashMap<String, Page> {
    let mut pages = nest_pages();
    pages.insert(
        "/expect/page.html".to_owned(),
        Page::html(include_str!("data/expect/page.html")).with_header("cache-control", "no-store"),
    );
    pages
}

/// Opens a tab on `path`, runs `body` on it, and closes the tab whatever happened.
async fn on_page<F, Fut>(path: &str, body: F) -> TestResult
where
    F: FnOnce(LiveRun, TestPage) -> Fut,
    Fut: Future<Output = (LiveRun, TestResult)>,
{
    let server = FixtureServer::start(pages()).await?;
    let run = LiveRun::connect().await?;
    let page = run.new_page().await?;
    let opened = run
        .invoke(
            "page.goto",
            json!({ "call": { "timeoutMs": 30_000 }, "page": brand(&page),
                    "url": server.url("127.0.0.1", path), "waitUntil": "load" }),
        )
        .await;
    let (run, result) = match opened {
        Ok(_) => body(run, page.clone()).await,
        Err(error) => (run, Err(error.into())),
    };
    run.close_page(&page).await;
    result
}

/// One `expect.locator` input, shaped as the prelude's `retry` sends it: the matcher, the
/// header's argument and Playwright's Expected line, then `FrameExpectParams`.
fn on_locator(
    target: &Value,
    matcher: &str,
    (expectation, expected): (&str, &str),
    params: &Value,
    timeout_ms: u64,
) -> Value {
    json!({
        "call": { "api": format!("expect.{matcher}"), "timeoutMs": timeout_ms },
        "target": target,
        "matcher": matcher,
        "expectation": expectation,
        "expected": expected,
        "params": params,
    })
}

/// `page.locator(selector)` on `page`.
fn css_locator(page: &TestPage, selector: &str) -> Value {
    locator(page, &[css(selector)], &format!("locator('{selector}')"))
}

/// `toHaveValue` params for one expected value, a string or a `RegExp`'s source and flags.
fn value_params(expected: &Value, is_not: bool) -> Value {
    json!({ "expression": "to.have.value", "expectedText": [expected], "isNot": is_not })
}

fn failure(result: Result<Value, PlaywrightError>) -> TestResult<PlaywrightError> {
    match result {
        Ok(value) => Err(format!("the matcher passed with {value}").into()),
        Err(error) => Ok(error),
    }
}

/// What the host saw besides notes: an expect op must neither recheck nor mark itself sent.
fn effects(run: &LiveRun) -> Vec<EnvCall> {
    run.calls()
        .into_iter()
        .filter(|call| !matches!(call, EnvCall::Note(_)))
        .collect()
}

fn notes(run: &LiveRun) -> Vec<String> {
    run.calls()
        .into_iter()
        .filter_map(|call| match call {
            EnvCall::Note(line) => Some(line),
            EnvCall::Recheck | EnvCall::RecheckReach(_) | EnvCall::MarkSent => None,
        })
        .collect()
}

/// One script line and what it does: pass, or throw `code` with the message's first line.
struct Case {
    label: &'static str,
    script: &'static str,
    fails: Option<(&'static str, &'static str)>,
}

const fn passes(label: &'static str, script: &'static str) -> Case {
    Case {
        label,
        script,
        fails: None,
    }
}

const fn fails(
    label: &'static str,
    script: &'static str,
    code: &'static str,
    first: &'static str,
) -> Case {
    Case {
        label,
        script,
        fails: Some((code, first)),
    }
}

/// Every v1 matcher passing and failing (failures wait 300 ms), `.not`, and the refusals.
const MATCHERS: &[Case] = &[
    passes(
        "toBeAttached",
        "expect(page.locator('#greeting')).toBeAttached()",
    ),
    passes(
        "toBeAttached detached",
        "expect(page.locator('#nope')).toBeAttached({ attached: false })",
    ),
    fails(
        "toBeAttached fails",
        "expect(page.locator('#nope')).toBeAttached({ timeout: 300 })",
        "no_match",
        "expect(locator).toBeAttached() failed",
    ),
    passes(
        "toBeChecked",
        "expect(page.locator('#agree')).toBeChecked()",
    ),
    fails(
        "toBeChecked fails",
        "expect(page.locator('#agree')).toBeChecked({ checked: false, timeout: 300 })",
        "assertion_failed",
        "expect(locator).toBeChecked({ checked: false }) failed",
    ),
    fails(
        "toBeChecked not a checkbox",
        "expect(page.locator('#name')).toBeChecked({ timeout: 300 })",
        "invalid_input",
        "expect(locator).toBeChecked() failed",
    ),
    passes(
        "toBeDisabled",
        "expect(page.locator('#off')).toBeDisabled()",
    ),
    fails(
        "toBeDisabled fails",
        "expect(page.locator('#name')).toBeDisabled({ timeout: 300 })",
        "assertion_failed",
        "expect(locator).toBeDisabled() failed",
    ),
    passes(
        "toBeEditable",
        "expect(page.locator('#name')).toBeEditable()",
    ),
    passes(
        "toBeEditable readonly",
        "expect(page.locator('#fixed')).toBeEditable({ editable: false })",
    ),
    fails(
        "toBeEditable fails",
        "expect(page.locator('#fixed')).toBeEditable({ timeout: 300 })",
        "assertion_failed",
        "expect(locator).toBeEditable() failed",
    ),
    passes("toBeEmpty", "expect(page.locator('#empty')).toBeEmpty()"),
    fails(
        "toBeEmpty fails",
        "expect(page.locator('#name')).toBeEmpty({ timeout: 300 })",
        "assertion_failed",
        "expect(locator).toBeEmpty() failed",
    ),
    passes("toBeEnabled", "expect(page.locator('#name')).toBeEnabled()"),
    fails(
        "toBeEnabled fails",
        "expect(page.locator('#off')).toBeEnabled({ timeout: 300 })",
        "assertion_failed",
        "expect(locator).toBeEnabled() failed",
    ),
    passes("toBeFocused", "expect(page.locator('#name')).toBeFocused()"),
    fails(
        "toBeFocused fails",
        "expect(page.locator('#off')).toBeFocused({ timeout: 300 })",
        "assertion_failed",
        "expect(locator).toBeFocused() failed",
    ),
    passes("toBeHidden", "expect(page.locator('#hidden')).toBeHidden()"),
    passes(
        "toBeHidden missing",
        "expect(page.locator('#nope')).toBeHidden()",
    ),
    fails(
        "toBeHidden fails",
        "expect(page.locator('#greeting')).toBeHidden({ timeout: 300 })",
        "assertion_failed",
        "expect(locator).toBeHidden() failed",
    ),
    passes(
        "toBeInViewport",
        "expect(page.locator('#greeting')).toBeInViewport()",
    ),
    fails(
        "toBeInViewport fails",
        "expect(page.locator('#far')).toBeInViewport({ timeout: 300 })",
        "assertion_failed",
        "expect(locator).toBeInViewport() failed",
    ),
    passes(
        "toBeVisible",
        "expect(page.locator('#greeting')).toBeVisible()",
    ),
    passes(
        "toBeVisible in 1 ms",
        "expect(page.locator('#greeting')).toBeVisible({ timeout: 1 })",
    ),
    fails(
        "toBeVisible fails",
        "expect(page.locator('#hidden')).toBeVisible({ timeout: 300 })",
        "assertion_failed",
        "expect(locator).toBeVisible() failed",
    ),
    fails(
        "toBeVisible missing",
        "expect(page.locator('#nope')).toBeVisible({ timeout: 300 })",
        "no_match",
        "expect(locator).toBeVisible() failed",
    ),
    passes(
        "toContainText",
        "expect(page.locator('#greeting')).toContainText('ull')",
    ),
    passes(
        "toContainText list",
        "expect(page.locator('li')).toContainText(['one', 'three'])",
    ),
    fails(
        "toContainText fails",
        "expect(page.locator('#greeting')).toContainText('xyz', { timeout: 300 })",
        "assertion_failed",
        "expect(locator).toContainText(expected) failed",
    ),
    passes(
        "toHaveAccessibleName",
        "expect(page.locator('#name')).toHaveAccessibleName('Name')",
    ),
    fails(
        "toHaveAccessibleName fails",
        "expect(page.locator('#name')).toHaveAccessibleName('Email', { timeout: 300 })",
        "assertion_failed",
        "expect(locator).toHaveAccessibleName(expected) failed",
    ),
    passes(
        "toHaveAttribute",
        "expect(page.locator('#name')).toHaveAttribute('aria-label')",
    ),
    passes(
        "toHaveAttribute value",
        "expect(page.locator('#name')).toHaveAttribute('value', 'Ada')",
    ),
    fails(
        "toHaveAttribute fails",
        "expect(page.locator('#name')).toHaveAttribute('value', /^B/, { timeout: 300 })",
        "assertion_failed",
        "expect(locator).toHaveAttribute(expected) failed",
    ),
    passes(
        "toHaveClass",
        "expect(page.locator('li').first()).toHaveClass('a b')",
    ),
    passes(
        "toHaveClass pattern",
        "expect(page.locator('li').first()).toHaveClass(/\\bb\\b/)",
    ),
    fails(
        "toHaveClass fails",
        "expect(page.locator('li').first()).toHaveClass('a', { timeout: 300 })",
        "assertion_failed",
        "expect(locator).toHaveClass(expected) failed",
    ),
    passes(
        "toContainClass",
        "expect(page.locator('li').first()).toContainClass('b')",
    ),
    fails(
        "toContainClass fails",
        "expect(page.locator('li').first()).toContainClass('c', { timeout: 300 })",
        "assertion_failed",
        "expect(locator).toContainClass(expected) failed",
    ),
    passes("toHaveCount", "expect(page.locator('li')).toHaveCount(3)"),
    passes(
        "toHaveCount none",
        "expect(page.locator('#nope')).toHaveCount(0)",
    ),
    fails(
        "toHaveCount fails",
        "expect(page.locator('li')).toHaveCount(2, { timeout: 300 })",
        "assertion_failed",
        "expect(locator).toHaveCount(expected) failed",
    ),
    passes(
        "toHaveCSS",
        "expect(page.locator('#greeting')).toHaveCSS('display', 'block')",
    ),
    fails(
        "toHaveCSS fails",
        "expect(page.locator('#greeting')).toHaveCSS('display', 'flex', { timeout: 300 })",
        "assertion_failed",
        "expect(locator).toHaveCSS(expected) failed",
    ),
    passes(
        "toHaveId",
        "expect(page.locator('h1')).toHaveId('greeting')",
    ),
    fails(
        "toHaveId fails",
        "expect(page.locator('h1')).toHaveId('x', { timeout: 300 })",
        "assertion_failed",
        "expect(locator).toHaveId(expected) failed",
    ),
    passes(
        "toHaveRole",
        "expect(page.locator('h1')).toHaveRole('heading')",
    ),
    fails(
        "toHaveRole fails",
        "expect(page.locator('h1')).toHaveRole('button', { timeout: 300 })",
        "assertion_failed",
        "expect(locator).toHaveRole(expected) failed",
    ),
    passes(
        "toHaveText",
        "expect(page.locator('#greeting')).toHaveText('Hullo')",
    ),
    passes(
        "toHaveText pattern",
        "expect(page.locator('#greeting')).toHaveText(/^H.llo$/)",
    ),
    passes(
        "toHaveText list",
        "expect(page.locator('li')).toHaveText(['one', 'two', 'three'])",
    ),
    fails(
        "toHaveText fails",
        "expect(page.locator('#greeting')).toHaveText('Hello', { timeout: 300 })",
        "assertion_failed",
        "expect(locator).toHaveText(expected) failed",
    ),
    fails(
        "toHaveText strict",
        "expect(page.locator('li')).toHaveText('one', { timeout: 300 })",
        "strict_mode_violation",
        "expect(locator).toHaveText(expected) failed",
    ),
    passes(
        "toHaveValue",
        "expect(page.locator('#name')).toHaveValue('Ada')",
    ),
    fails(
        "toHaveValue fails",
        "expect(page.locator('#name')).toHaveValue('Bob', { timeout: 300 })",
        "assertion_failed",
        "expect(locator).toHaveValue(expected) failed",
    ),
    fails(
        "toHaveValue not an input",
        "expect(page.locator('#greeting')).toHaveValue('Bob', { timeout: 300 })",
        "invalid_input",
        "expect(locator).toHaveValue(expected) failed",
    ),
    passes(
        "toHaveValues",
        "expect(page.locator('#multi')).toHaveValues(['x'])",
    ),
    fails(
        "toHaveValues fails",
        "expect(page.locator('#multi')).toHaveValues(['y'], { timeout: 300 })",
        "assertion_failed",
        "expect(locator).toHaveValues(expected) failed",
    ),
    fails(
        "toHaveJSProperty",
        "expect(page.locator('#name')).toHaveJSProperty('value', 'Ada')",
        "unsupported",
        "expect.toHaveJSProperty: toHaveJSProperty is not supported: Neo matches in a sealed \
         world that does not see the page's own JavaScript objects; use expect.poll(() => \
         locator.evaluate((element) => element.name)).toEqual(value) instead",
    ),
    passes("toHaveTitle", "expect(page).toHaveTitle('Expect page')"),
    passes("toHaveTitle pattern", "expect(page).toHaveTitle(/^Expect/)"),
    fails(
        "toHaveTitle fails",
        "expect(page).toHaveTitle('Nope', { timeout: 300 })",
        "assertion_failed",
        "expect(page).toHaveTitle(expected) failed",
    ),
    passes("toHaveURL", "expect(page).toHaveURL(URL)"),
    passes(
        "toHaveURL pattern",
        "expect(page).toHaveURL(/\\/expect\\/page\\.html$/)",
    ),
    fails(
        "toHaveURL fails",
        "expect(page).toHaveURL('https://example.com/', { timeout: 300 })",
        "assertion_failed",
        "expect(page).toHaveURL(expected) failed",
    ),
    passes(
        "not.toBeVisible",
        "expect(page.locator('#hidden')).not.toBeVisible()",
    ),
    passes(
        "not.toBeVisible missing",
        "expect(page.locator('#nope')).not.toBeVisible()",
    ),
    fails(
        "not.toBeVisible fails",
        "expect(page.locator('#greeting')).not.toBeVisible({ timeout: 300 })",
        "assertion_failed",
        "expect(locator).not.toBeVisible() failed",
    ),
    passes(
        "not.toHaveText",
        "expect(page.locator('#greeting')).not.toHaveText('Hello')",
    ),
    fails(
        "not.toHaveText missing",
        "expect(page.locator('#nope')).not.toHaveText('Hello', { timeout: 300 })",
        "no_match",
        "expect(locator).not.toHaveText(expected) failed",
    ),
    passes(
        "not.toHaveCount",
        "expect(page.locator('li')).not.toHaveCount(2)",
    ),
    fails(
        "not.toHaveCount fails",
        "expect(page.locator('li')).not.toHaveCount(3, { timeout: 300 })",
        "assertion_failed",
        "expect(locator).not.toHaveCount(expected) failed",
    ),
    passes("not.toHaveTitle", "expect(page).not.toHaveTitle('Nope')"),
];

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_every_matcher_passes_and_fails_as_scripts_write_it() -> TestResult {
    let server = FixtureServer::start(pages()).await?;
    let url = server.url("127.0.0.1", "/expect/page.html");
    let run = Arc::new(LiveRun::connect().await?);
    let host: Arc<dyn OpHost> = run;
    let seed = json!({ "page": null, "pages": [], "browserVersion": "" });
    let mut checks = String::new();
    for case in MATCHERS {
        let _infallible = writeln!(
            checks,
            "await check({}, async () => {{ await {} }})",
            json!(case.label),
            case.script
        );
    }
    let script = format!(
        "const URL = {url}\n\
         const results = {{}}\n\
         async function check(label, run) {{\n\
           try {{ await run(); results[label] = null }}\n\
           catch (e) {{ results[label] = {{ code: e.code ?? null, first: e.message.split('\\n')[0] }} }}\n\
         }}\n\
         try {{\n\
           await page.goto(URL)\n\
           const failed = await __neo_test_gesture(page._tab, \"document.querySelector('#name').focus()\")\n\
           if (failed) throw new Error(failed)\n\
           {checks}\
           return results\n\
         }} finally {{\n\
           await page.close()\n\
         }}",
        url = json!(url),
    );
    let results = run_script(&host, &seed, &script).await?;
    let results = results
        .as_object()
        .ok_or("the script returns its results")?;
    let expected: Map<String, Value> = MATCHERS
        .iter()
        .map(|case| {
            let outcome = case.fails.map_or(
                Value::Null,
                |(code, first)| json!({ "code": code, "first": first }),
            );
            (case.label.to_owned(), outcome)
        })
        .collect();
    let wrong: Vec<String> = expected
        .iter()
        .filter(|(label, outcome)| results.get(label.as_str()) != Some(outcome))
        .map(|(label, outcome)| {
            let got = results.get(label.as_str()).unwrap_or(&Value::Null);
            format!("{label}: expected {outcome}, got {got}")
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_matcher_holds_once_the_page_changes() -> TestResult {
    on_page("/expect/page.html", |run, page| async move {
        let result = async {
            run.eval(
                &page,
                "setTimeout(() => { document.querySelector('#later').style.display = 'block'; \
                 document.querySelector('#status').textContent = 'done' }, 400)",
            )
            .await?;
            run.clear_calls();
            let started = Instant::now();
            let visible = on_locator(
                &css_locator(&page, "#later"),
                "toBeVisible",
                ("", "Expected: visible"),
                &json!({ "expression": "to.be.visible", "isNot": false }),
                5000,
            );
            assert_eq!(run.invoke("expect.locator", visible).await?, Value::Null);
            let waited = started.elapsed();
            let status = on_locator(
                &css_locator(&page, "#status"),
                "toHaveText",
                ("expected", "Expected: \"done\""),
                &json!({ "expression": "to.have.text", "isNot": false,
                         "expectedText": [{ "string": "done", "normalizeWhiteSpace": true }] }),
                5000,
            );
            assert_eq!(run.invoke("expect.locator", status).await?, Value::Null);
            assert!(
                waited >= Duration::from_millis(400) && waited < Duration::from_secs(3),
                "passed once the button showed, after {waited:?}"
            );
            let lines = notes(&run);
            assert_eq!(
                lines.first().map(String::as_str),
                Some("Expect \"toBeVisible\" locator('#later') with timeout 5000ms")
            );
            assert!(
                lines
                    .iter()
                    .any(|line| line == "  unexpected value \"hidden\""),
                "it polled while the button was hidden: {lines:#?}"
            );
            assert_eq!(
                effects(&run),
                [],
                "expect is a read: no recheck, never marked sent"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_timeout_reads_as_playwrights_with_its_call_log() -> TestResult {
    on_page("/expect/page.html", |run, page| async move {
        let result = async {
            let started = Instant::now();
            let hidden = on_locator(
                &css_locator(&page, "#hidden"),
                "toBeVisible",
                ("", "Expected: visible"),
                &json!({ "expression": "to.be.visible", "isNot": false }),
                300,
            );
            let error = failure(run.invoke("expect.locator", hidden).await)?;
            let took = started.elapsed();
            assert!(
                took >= Duration::from_millis(300) && took < Duration::from_millis(1500),
                "{took:?}"
            );
            assert_eq!(
                (error.error_code(), error.owner(), error.timed_out()),
                (
                    Some(ErrorCode::AssertionFailed),
                    ErrorOwner::Website,
                    TimedOut::Yes
                )
            );
            let message = error.message();
            let head = "expect(locator).toBeVisible() failed\n\nLocator:  locator('#hidden')\n\
                        Expected: visible\nReceived: hidden\nTimeout:  300ms\n\nCall log:\n  \
                        - Expect \"toBeVisible\" locator('#hidden') with timeout 300ms\n  \
                        - waiting for locator('#hidden')\n";
            assert!(message.starts_with(head), "{message}");
            let polls = message.get(head.len()..).unwrap_or_default();
            let (count, rest) = polls
                .trim_start()
                .split_once(" × ")
                .ok_or_else(|| format!("the polls compress: {message}"))?;
            assert!(
                count.parse::<u32>().is_ok_and(|count| count >= 3),
                "{message}"
            );
            assert_eq!(
                rest,
                "locator resolved to <div id=\"hidden\">secret</div>\n      \
                 - unexpected value \"hidden\"\n"
            );

            let missing = on_locator(
                &css_locator(&page, "#nope"),
                "toBeVisible",
                ("", "Expected: visible"),
                &json!({ "expression": "to.be.visible", "isNot": false }),
                300,
            );
            let error = failure(run.invoke("expect.locator", missing).await)?;
            assert_eq!(
                (error.error_code(), error.owner(), error.timed_out()),
                (Some(ErrorCode::NoMatch), ErrorOwner::Script, TimedOut::Yes)
            );
            assert_eq!(
                error.message(),
                "expect(locator).toBeVisible() failed\n\nLocator: locator('#nope')\n\
                 Expected: visible\nTimeout: 300ms\nError: element(s) not found\n\nCall log:\n  \
                 - Expect \"toBeVisible\" locator('#nope') with timeout 300ms\n  \
                 - waiting for locator('#nope')\n"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_typed_password_is_compared_through_the_seal() -> TestResult {
    on_page("/expect/page.html", |run, page| async move {
        let result = async {
            // The page's own world, where the seal is not: what the field holds once typed.
            run.eval(&page, "document.querySelector('#pw').value = 'hunter2'")
                .await?;
            let password = css_locator(&page, "#pw");
            let hunter2 = value_params(&json!({ "string": "hunter2" }), false);
            run.clear_calls();
            let right = on_locator(
                &password,
                "toHaveValue",
                ("expected", "Expected: \"hunter2\""),
                &hunter2,
                5000,
            );
            assert_eq!(run.invoke("expect.locator", right).await?, Value::Null);
            assert!(effects(&run).is_empty(), "expectations have no effects");
            let label = locator(&page, &[css("label[for=pw]")], "locator('label')");
            let through_label = on_locator(
                &label,
                "toHaveValue",
                ("expected", "Expected: \"hunter2\""),
                &hunter2,
                5000,
            );
            assert_eq!(
                run.invoke("expect.locator", through_label).await?,
                Value::Null,
                "a label leads to its field, as upstream's retarget does"
            );

            let wrong = on_locator(
                &password,
                "toHaveValue",
                ("expected", "Expected: \"hunter3\""),
                &value_params(&json!({ "string": "hunter3" }), false),
                300,
            );
            let error = failure(run.invoke("expect.locator", wrong).await)?;
            assert_eq!(error.error_code(), Some(ErrorCode::AssertionFailed));
            let message = error.message();
            assert!(
                message.contains("Expected: \"••••\"\nReceived: \"••••\"\n")
                    && message.contains("      - unexpected value \"••••\"\n"),
                "the masked value is what was received: {message}"
            );
            let not = on_locator(
                &password,
                "toHaveValue",
                ("expected", "Expected: not \"hunter2\""),
                &value_params(&json!({ "string": "hunter2" }), true),
                300,
            );
            let error = failure(run.invoke("expect.locator", not).await)?;
            assert!(
                error
                    .message()
                    .contains("Expected: not \"••••\"\nReceived: \"••••\"\n"),
                "{error}"
            );
            let seen = format!("{:?}", run.calls());
            assert!(
                !seen.contains("hunter"),
                "no secret reached the host's notes: {seen}"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_regexp_against_a_password_is_refused_and_attributes_stay_markup() -> TestResult
{
    on_page("/expect/page.html", |run, page| async move {
        let result = async {
            run.eval(&page, "document.querySelector('#pw').value = 'hunter2'")
                .await?;
            let password = css_locator(&page, "#pw");
            let started = Instant::now();
            let pattern = on_locator(
                &password,
                "toHaveValue",
                ("expected", "Expected pattern: /^hun/"),
                &value_params(&json!({ "regexSource": "^hun", "regexFlags": "" }), false),
                5000,
            );
            let error = failure(run.invoke("expect.locator", pattern).await)?;
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "refused at the first poll, not after the timeout"
            );
            assert_eq!(
                (error.error_code(), error.owner(), error.timed_out()),
                (Some(ErrorCode::Unsupported), ErrorOwner::Neo, TimedOut::No)
            );
            assert!(
                error.message().starts_with(
                    "expect(locator).toHaveValue(expected) failed\n\nLocator: locator('#pw')\n\
                     Expected pattern: \"••••\"\nError: a RegExp is not matched against a \
                     password, card or one-time-code field"
                ),
                "{error}"
            );

            let prefilled = on_locator(
                &css_locator(&page, "#prefilled"),
                "toHaveAttribute",
                ("expected", "Expected: \"opensesame\""),
                &json!({ "expression": "to.have.attribute.value", "expressionArg": "value",
                         "expectedText": [{ "string": "opensesame" }], "isNot": false }),
                5000,
            );
            assert_eq!(run.invoke("expect.locator", prefilled).await?, Value::Null);
            let no_attribute = on_locator(
                &password,
                "toHaveAttribute",
                ("expected", "Expected: \"hunter2\""),
                &json!({ "expression": "to.have.attribute.value", "expressionArg": "value",
                         "expectedText": [{ "string": "hunter2" }], "isNot": false }),
                300,
            );
            let error = failure(run.invoke("expect.locator", no_attribute).await)?;
            assert!(
                error.message().contains("Received: \"\"\n")
                    && error.message().contains("unexpected value \"null\""),
                "the typed value is not the value attribute, as upstream: {error}"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// `toHaveAttribute('value', ...)` params for one expected attribute, a string or a `RegExp`.
fn value_attribute_params(expected: &Value) -> Value {
    json!({ "expression": "to.have.attribute.value", "expressionArg": "value",
            "expectedText": [expected], "isNot": false })
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_password_value_attribute_is_compared_as_markup_once_the_field_is_edited()
-> TestResult {
    on_page("/expect/page.html", |run, page| async move {
        let result = async {
            // The page's own world: the markup keeps the first value, the field holds the
            // second, as once a user typed over a prefilled password.
            run.eval(
                &page,
                "(() => { const field = document.querySelector('#prefilled'); \
                 field.setAttribute('value', 'initial-secret'); field.value = 'edited-secret'; \
                 return true })()",
            )
            .await?;
            let prefilled = css_locator(&page, "#prefilled");
            run.clear_calls();
            let markup = on_locator(
                &prefilled,
                "toHaveAttribute",
                ("expected", "Expected: \"initial-secret\""),
                &value_attribute_params(&json!({ "string": "initial-secret" })),
                2000,
            );
            assert_eq!(
                run.invoke("expect.locator", markup).await?,
                Value::Null,
                "the attribute still holds the markup's value"
            );
            let typed = on_locator(
                &prefilled,
                "toHaveAttribute",
                ("expected", "Expected: \"edited-secret\""),
                &value_attribute_params(&json!({ "string": "edited-secret" })),
                300,
            );
            let error = failure(run.invoke("expect.locator", typed).await)?;
            assert_eq!(error.error_code(), Some(ErrorCode::AssertionFailed));
            let message = error.message();
            assert!(
                message.contains("Expected: \"••••\"\nReceived: \"••••\"\n"),
                "the typed value is not the attribute, and the attribute is received masked: \
                 {message}"
            );
            let pattern = on_locator(
                &prefilled,
                "toHaveAttribute",
                ("expected", "Expected pattern: /^initial/"),
                &value_attribute_params(&json!({ "regexSource": "^initial", "regexFlags": "" })),
                2000,
            );
            let error = failure(run.invoke("expect.locator", pattern).await)?;
            assert_eq!(
                (error.error_code(), error.owner(), error.timed_out()),
                (Some(ErrorCode::Unsupported), ErrorOwner::Neo, TimedOut::No),
                "a RegExp is still refused: {error}"
            );
            let seen = format!("{:?}{message}{error}", run.calls());
            assert!(
                !seen.contains("-secret"),
                "no secret reached a message or the host's notes: {seen}"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_matcher_runs_four_frames_deep() -> TestResult {
    on_page("/nest/top.html", |run, page| async move {
        let result = async {
            let mut parts = Vec::new();
            let mut display = "page".to_owned();
            for title in ["b1", "c2", "a3", "c4"] {
                parts.push(attr(&format!("[title=\"{title}\"s]")));
                parts.push(enter_frame());
                let _infallible = write!(display, ".frameLocator('[title=\"{title}\"]')");
            }
            parts.push(role("button"));
            display.push_str(".getByRole('button')");
            let button = locator(&page, &parts, &display);
            let text = |expected: &str, timeout_ms| {
                on_locator(
                    &button,
                    "toHaveText",
                    ("expected", &format!("Expected: \"{expected}\"")),
                    &json!({ "expression": "to.have.text", "isNot": false,
                             "expectedText": [{ "string": expected, "normalizeWhiteSpace": true }] }),
                    timeout_ms,
                )
            };
            // The frames load while it polls: until then a frame on the way reads as missing.
            assert_eq!(
                run.invoke("expect.locator", text("C4 button", 10_000))
                    .await?,
                Value::Null
            );
            let visible = on_locator(
                &button,
                "toBeVisible",
                ("", "Expected: visible"),
                &json!({ "expression": "to.be.visible", "isNot": false }),
                5000,
            );
            assert_eq!(run.invoke("expect.locator", visible).await?, Value::Null);
            let count = on_locator(
                &button,
                "toHaveCount",
                ("expected", "Expected: 1"),
                &json!({ "expression": "to.have.count", "expectedNumber": 1, "isNot": false }),
                5000,
            );
            assert_eq!(run.invoke("expect.locator", count).await?, Value::Null);
            let error = failure(run.invoke("expect.locator", text("A1 button", 300)).await)?;
            assert_eq!(error.error_code(), Some(ErrorCode::AssertionFailed));
            assert!(
                error.message().contains(&format!(
                    "Locator:  {display}\nExpected: \"A1 button\"\nReceived: \"C4 button\"\n"
                )),
                "{error}"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}
