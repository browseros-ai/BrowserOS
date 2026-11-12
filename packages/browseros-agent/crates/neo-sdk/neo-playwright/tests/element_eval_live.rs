//! The element scripts on a real browser, through `Playwright::invoke`: `locator.evaluate`,
//! `locator.evaluateAll`, `elementHandle.evaluate`, `$eval` and `$$eval` in the main world of
//! the element's frame, answering in Playwright's format. A typed password comes back as the page
//! holds it from every form, from the element's own frame, from a same-origin parent and from
//! inside nested out-of-process frames; a handle out of its document still runs; handle
//! arguments are adopted only from their own document; a strict violation and a `$eval` with no
//! match fail as upstream does; and the host's recheck runs after every preparation, right
//! before the script.
//! The last test runs the same methods as a script, through the real prelude in QuickJS
//! (Runner N). Ported from r10c (`68440df5d`, `b45f541b0`) onto the main-world gateway.
//!
//! Needs a throwaway BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005), for example
//! `PORT_BASE=9601 spikes/eval/browser.sh <profile> &`, then
//! `BROWSEROS_CDP_PORT=9601 cargo test -p neo-playwright --test element_eval_live -- --ignored`.
//! Every test opens its own tab and closes it.

#[path = "support/fixture_server.rs"]
mod fixture_server;
#[path = "support/js_runner.rs"]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "the capture refusal and the click serve pages_live, which includes this too"
)]
mod live_run;
#[path = "support/locator.rs"]
#[expect(
    dead_code,
    reason = "element scripts need no role, text or ref engine parts"
)]
mod locator;

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fixture_server::{FixtureServer, Page, nest_pages};
use js_runner::{OpHost, run_script};
use live_run::{EnvCall, Intercept, LiveRun, TestPage, TestResult, brand, plain};
use locator::{attr, css, enter_frame, locator};
use neo_playwright::{Answer, ErrorCode, PlaywrightError, Refusal};
use neo_trace::ErrorOwner;
use serde_json::{Value, json};
use tokio::time;

/// A password field, three list items, two buttons that make one locator ambiguous, a
/// paragraph a test removes, and a same-origin frame whose script can reach the main frame.
const EVAL: &str = r#"<!doctype html><html><head><title>Eval</title></head><body>
<label>Password <input id="pw" type="password"></label>
<ul id="list"><li>alpha</li><li>beta</li><li>gamma</li></ul>
<button class="twin">One</button><button class="twin">Two</button>
<p id="gone">to be removed</p>
<iframe id="inner" srcdoc="<button id=reach>Reach</button>"></iframe>
</body></html>"#;

/// Opens a tab on `path` of the fixture server (the nested-frame pages and `/eval.html`), runs
/// `body` on it, and closes the tab whatever happened.
async fn on_page<F, Fut>(path: &str, body: F) -> TestResult
where
    F: FnOnce(LiveRun, TestPage) -> Fut,
    Fut: Future<Output = (LiveRun, TestResult)>,
{
    let run = LiveRun::connect().await?;
    let mut pages = nest_pages();
    pages.insert("/eval.html".to_owned(), Page::html(EVAL));
    let server = FixtureServer::start(pages).await?;
    let page = run.new_page().await?;
    let url = server.url("127.0.0.1", path);
    let opened = run
        .invoke(
            "page.goto",
            json!({ "call": { "timeoutMs": 30_000 }, "page": brand(&page), "url": url }),
        )
        .await;
    if let Err(error) = opened {
        run.close_page(&page).await;
        return Err(error.into());
    }
    let (run, result) = body(run, page.clone()).await;
    run.close_page(&page).await;
    result
}

/// A locator for one CSS selector in the main frame, displayed as `locator('<selector>')`.
fn at(page: &TestPage, selector: &str) -> Value {
    locator(page, &[css(selector)], &format!("locator('{selector}')"))
}

/// `selector` in the frames down `titles` from the main frame (the nested fixture's iframes
/// carry their page's name as their title).
fn down(page: &TestPage, titles: &[&str], selector: &str) -> Value {
    let mut parts = Vec::new();
    for title in titles {
        parts.push(attr(&format!("[title=\"{title}\"s]")));
        parts.push(enter_frame());
    }
    parts.push(css(selector));
    locator(
        page,
        &parts,
        &format!("{titles:?} >> locator('{selector}')"),
    )
}

/// `selector` queried from `handle`'s node, as `elementHandle.$eval` and `$$eval` send it.
fn rooted(page: &TestPage, handle: &Value, selector: &str) -> Value {
    let mut target = at(page, selector);
    if let Some(fields) = target.as_object_mut() {
        fields.insert("root".to_owned(), handle.clone());
    }
    target
}

/// An argument in Playwright's in-page format, as the prelude writes its JSON text: a string
/// leaf is itself, and no argument is `undefined`.
fn arg_text(arg: Option<&str>) -> String {
    arg.map_or_else(
        || r#"{"v":"undefined"}"#.to_owned(),
        |arg| json!(arg).to_string(),
    )
}

/// The prelude's script for `function`, with a string argument or none.
fn script(function: &str, arg: Option<&str>) -> Value {
    json!({ "run": { "expression": function, "isFunction": true, "arg": arg_text(arg) } })
}

/// `locator.evaluate(function, arg)` as the prelude sends it for `api`, with a timeout, its
/// answer as plain JSON.
async fn evaluate(
    run: &LiveRun,
    api: &str,
    target: &Value,
    function: &str,
    arg: Option<&str>,
) -> Result<Value, PlaywrightError> {
    let input = json!({ "call": { "api": api, "timeoutMs": 10_000 }, "target": target,
                        "strict": "strict", "script": script(function, arg) });
    run.invoke("locator.evaluate", input).await.map(plain)
}

/// `page.$eval(selector, function)` or `elementHandle.$eval`: the first match now.
async fn eval_first(
    run: &LiveRun,
    api: &str,
    target: &Value,
    selector: &str,
    function: &str,
) -> Result<Value, PlaywrightError> {
    run.invoke(
        "locator.evaluate",
        json!({ "call": { "api": api, "timeoutMs": 0 }, "target": target, "strict": "firstMatch",
                "noWait": true, "selector": selector, "script": script(function, None) }),
    )
    .await
    .map(plain)
}

/// `locator.evaluateAll(function, arg)`, or a `$$eval` as `api`.
async fn evaluate_all(
    run: &LiveRun,
    api: &str,
    target: &Value,
    function: &str,
    arg: Option<&str>,
) -> Result<Value, PlaywrightError> {
    let input = json!({ "call": { "api": api, "timeoutMs": 0 }, "target": target,
                        "script": script(function, arg) });
    run.invoke("locator.evaluateAll", input).await.map(plain)
}

/// `page.evaluate(function, handles)`: the handles as the argument's `{h: i}`, in order.
async fn evaluate_with(
    run: &LiveRun,
    page: &TestPage,
    function: &str,
    handles: &[&Value],
) -> Result<Value, PlaywrightError> {
    let items: Vec<Value> = (0..handles.len())
        .map(|index| json!({ "h": index }))
        .collect();
    let input = json!({ "call": { "api": "page.evaluate", "timeoutMs": 0 }, "page": brand(page),
        "script": { "run": { "expression": function, "isFunction": true,
            "arg": json!({ "a": items, "id": 1 }).to_string(), "handles": handles } } });
    run.invoke("page.evaluate", input).await.map(plain)
}

/// `locator.elementHandle()` on `target`, which must hand back a handle.
async fn handle_of(run: &LiveRun, target: &Value) -> TestResult<Value> {
    let handle = run
        .invoke(
            "locator.waitFor",
            json!({ "call": { "api": "locator.elementHandle", "timeoutMs": 10_000 },
                "target": target, "strict": "strict", "state": "attached",
                "omitReturnValue": false }),
        )
        .await?;
    if handle.get("$neo").and_then(Value::as_str) != Some("element") {
        return Err(format!("not an element handle: {handle}").into());
    }
    Ok(handle)
}

/// The op was marked sent once, right after the recheck that allowed it.
fn sent_once(run: &LiveRun, what: &str) {
    let calls = run.calls();
    let sent: Vec<usize> = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| **call == EnvCall::MarkSent)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(sent.len(), 1, "{what}: marked sent once: {calls:?}");
    let recheck = sent.first().and_then(|index| index.checked_sub(1));
    assert_eq!(
        recheck.and_then(|index| calls.get(index)),
        Some(&EnvCall::Recheck),
        "{what}: the recheck runs right before the script: {calls:?}"
    );
}

/// Nothing was marked sent since the calls were last cleared.
fn not_sent(run: &LiveRun, what: &str) {
    let calls = run.calls();
    assert!(
        !calls.contains(&EnvCall::MarkSent),
        "{what}: nothing was sent: {calls:?}"
    );
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_an_element_script_reads_a_typed_password_as_the_page_holds_it() -> TestResult {
    on_page("/eval.html", |run, page| async move {
        let result = async {
            run.eval(
                &page,
                "document.getElementById('pw').value = 'hunter2-typed'",
            )
            .await?;
            run.clear_calls();
            let value = evaluate(
                &run,
                "locator.evaluate",
                &at(&page, "#pw"),
                "(el) => el.value",
                None,
            )
            .await?;
            assert_eq!(
                value,
                json!("hunter2-typed"),
                "el.value of a typed password"
            );
            sent_once(&run, "locator.evaluate");

            let echoed = evaluate(
                &run,
                "locator.evaluate",
                &at(&page, "#pw"),
                "(el, suffix) => 'you typed ' + el.value + suffix",
                Some("!"),
            )
            .await?;
            assert_eq!(
                echoed,
                json!("you typed hunter2-typed!"),
                "fn(element, arg)"
            );

            let handle = handle_of(&run, &at(&page, "#pw")).await?;
            let by_handle = evaluate(
                &run,
                "elementHandle.evaluate",
                &handle,
                "(el) => [el.id, el.value, el.isConnected]",
                None,
            )
            .await?;
            assert_eq!(by_handle, json!(["pw", "hunter2-typed", true]));
            let first = eval_first(
                &run,
                "page.$eval",
                &at(&page, "#pw"),
                "#pw",
                "(el) => el.value",
            )
            .await?;
            assert_eq!(first, json!("hunter2-typed"), "page.$eval");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// Each of the other element forms reads the typed password too.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_every_element_form_reads_a_typed_password() -> TestResult {
    on_page("/eval.html", |run, page| async move {
        let result = async {
            run.eval(
                &page,
                "document.getElementById('pw').value = 'hunter2-typed'",
            )
            .await?;
            let all = evaluate_all(
                &run,
                "locator.evaluateAll",
                &at(&page, "input"),
                "(els) => els.map((el) => el.value)",
                None,
            )
            .await?;
            assert_eq!(all, json!(["hunter2-typed"]), "locator.evaluateAll");
            let form = handle_of(&run, &at(&page, "label")).await?;
            let under = eval_first(
                &run,
                "elementHandle.$eval",
                &rooted(&page, &form, "input"),
                "input",
                "(el) => el.value",
            )
            .await?;
            assert_eq!(under, json!("hunter2-typed"), "elementHandle.$eval");
            let every = evaluate_all(
                &run,
                "elementHandle.$$eval",
                &rooted(&page, &form, "input"),
                "(els) => els.map((el) => el.value)",
                None,
            )
            .await?;
            assert_eq!(every, json!(["hunter2-typed"]), "elementHandle.$$eval");

            let inner = locator(
                &page,
                &[css("#inner"), enter_frame(), css("#reach")],
                "locator('#inner').contentFrame().locator('#reach')",
            );
            let reached = evaluate(
                &run,
                "locator.evaluate",
                &inner,
                "(el) => [el.textContent, el.ownerDocument === document, \
                 parent.document.getElementById('pw').value]",
                None,
            )
            .await?;
            assert_eq!(
                reached,
                json!(["Reach", true, "hunter2-typed"]),
                "the script runs in the element's own frame, and reaches its same-origin parent"
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
async fn tests_that_evaluate_all_runs_on_every_match_as_one_array() -> TestResult {
    on_page("/eval.html", |run, page| async move {
        let result = async {
            run.eval(&page, "document.getElementById('pw').value = 'hunter2-typed'")
                .await?;
            run.clear_calls();
            let joined = evaluate_all(
                &run,
                "locator.evaluateAll",
                &at(&page, "#list li"),
                "(els, sep) => els.map((el) => el.textContent).join(sep)",
                Some("|"),
            )
            .await?;
            assert_eq!(joined, json!("alpha|beta|gamma"), "fn(elements, arg), in document order");
            sent_once(&run, "locator.evaluateAll");
            let shape = evaluate_all(
                &run,
                "locator.evaluateAll",
                &at(&page, "#list li"),
                "(els) => [els.length, Array.isArray(els), els.every((el) => el.ownerDocument === document)]",
                None,
            )
            .await?;
            assert_eq!(shape, json!([3, true, true]), "one array of the page's own elements");

            let none = evaluate_all(&run, "locator.evaluateAll", &at(&page, ".nothing"), "(els) => [els.length, document.title]", None)
                .await?;
            assert_eq!(none, json!([0, "Eval"]), "no match runs the function on []");
            let reached = locator(&page, &[css("#inner"), enter_frame(), css(".nothing")],
                "locator('#inner').contentFrame().locator('.nothing')");
            let in_start = evaluate_all(&run, "locator.evaluateAll", &reached,
                "(els) => [els.length, document.title]", None).await?;
            assert_eq!(in_start, json!([0, "Eval"]),
                "a frame reached with no match runs fn([]) in the starting frame, as upstream");
            let hop = locator(&page, &[css("#missing"), enter_frame(), css("li")],
                "locator('#missing').contentFrame().locator('li')");
            let missing = evaluate_all(&run, "locator.evaluateAll", &hop,
                "(els) => [els.length, document.title]", None).await?;
            assert_eq!(missing, json!([0, "Eval"]), "a frame on the way missing: the same");

            let values = evaluate_all(&run, "page.$$eval", &at(&page, "input"), "(els) => els.map((el) => el.value)", None)
                .await?;
            assert_eq!(values, json!(["hunter2-typed"]), "a $$eval over fields");

            let list = handle_of(&run, &at(&page, "#list")).await?;
            let under = evaluate_all(
                &run,
                "elementHandle.$$eval",
                &rooted(&page, &list, "li"),
                "(els) => els.map((el) => el.textContent)",
                None,
            )
            .await?;
            assert_eq!(under, json!(["alpha", "beta", "gamma"]), "elementHandle.$$eval");
            let one = eval_first(&run, "elementHandle.$eval", &rooted(&page, &list, "li"), "li",
                "(el) => el.textContent").await?;
            assert_eq!(one, json!("alpha"), "elementHandle.$eval takes the first match");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_an_element_inside_nested_oopifs_runs_in_its_own_frame() -> TestResult {
    on_page("/nest/top.html", |run, page| async move {
        let result = async {
            let c4 = &["b1", "c2", "a3", "c4"];
            let read = evaluate(
                &run,
                "locator.evaluate",
                &down(&page, c4, "button"),
                "(el) => [el.textContent, document.title, location.hostname]",
                None,
            )
            .await?;
            assert_eq!(
                read,
                json!(["C4 button", "C4", "[::1]"]),
                "c4's own main world"
            );

            let typed = evaluate(
                &run,
                "locator.evaluate",
                &down(&page, c4, "input"),
                "(el) => { el.type = 'password'; el.value = 'c4-secret-99'; return el.value; }",
                None,
            )
            .await?;
            assert_eq!(typed, json!("c4-secret-99"), "what the script typed");
            let again = evaluate(
                &run,
                "locator.evaluate",
                &down(&page, c4, "input"),
                "(el) => el.value",
                None,
            )
            .await?;
            assert_eq!(again, json!("c4-secret-99"), "the field keeps it");

            let handle = handle_of(&run, &down(&page, c4, "button")).await?;
            let by_handle = evaluate(
                &run,
                "elementHandle.evaluate",
                &handle,
                "(el) => el.textContent",
                None,
            )
            .await?;
            assert_eq!(by_handle, json!("C4 button"), "a handle inside four frames");

            // b3 is in b2's process, under b2's session: a script in b3 reaches b2's field.
            evaluate(
                &run,
                "locator.evaluate",
                &down(&page, &["a1", "b2"], "input"),
                "(el) => { el.type = 'password'; el.value = 'b2-secret-11'; return 1; }",
                None,
            )
            .await?;
            let parent = evaluate(
                &run,
                "locator.evaluate",
                &down(&page, &["a1", "b2", "b3"], "button"),
                "(el) => [el.textContent, parent.document.querySelector('input').value]",
                None,
            )
            .await?;
            assert_eq!(
                parent,
                json!(["B3 button", "b2-secret-11"]),
                "b3's script reaches its parent b2, on the same session"
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
async fn tests_that_a_strict_violation_fails_before_any_script() -> TestResult {
    on_page("/eval.html", |run, page| async move {
        let result = async {
            run.clear_calls();
            let error = evaluate(&run, "locator.evaluate", &at(&page, ".twin"), "(el) => el.textContent", None)
                .await
                .err()
                .ok_or("two matches of a strict locator fail")?;
            assert_eq!(error.error_code(), Some(ErrorCode::StrictModeViolation));
            assert!(
                error
                    .message()
                    .starts_with("locator.evaluate: Error: strict mode violation: locator('.twin') resolved to 2 elements"),
                "{error}"
            );
            not_sent(&run, "a strict violation");
            let first = eval_first(&run, "page.$eval", &at(&page, ".twin"), ".twin",
                "(el) => el.textContent").await?;
            assert_eq!(first, json!("One"), "page.$eval is not strict");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// A handle out of its document still runs, as upstream's does (spike S-detached): a removed
/// password input, and one inside a container removed after a handle to the container was
/// taken.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_detached_handle_runs() -> TestResult {
    on_page("/eval.html", |run, page| async move {
        let result = async {
            run.eval(
                &page,
                "document.getElementById('pw').value = 'detached-secret-1'",
            )
            .await?;
            let field = handle_of(&run, &at(&page, "#pw")).await?;
            let label = handle_of(&run, &at(&page, "label")).await?;
            run.eval(
                &page,
                "window.label = document.querySelector('label'); \
                             document.getElementById('pw').remove()",
            )
            .await?;
            run.clear_calls();
            let removed = evaluate(
                &run,
                "elementHandle.evaluate",
                &field,
                "(el) => [el.isConnected, el.value, 'typed ' + el.value]",
                None,
            )
            .await?;
            assert_eq!(
                removed,
                json!([false, "detached-secret-1", "typed detached-secret-1"]),
                "a removed password input runs"
            );
            sent_once(&run, "a detached handle's script");
            run.eval(
                &page,
                "label.append(Object.assign(document.createElement('input'), \
                             { type: 'password', value: 'detached-secret-2' })); label.remove()",
            )
            .await?;
            let inside = evaluate(
                &run,
                "elementHandle.evaluate",
                &label,
                "(el) => [el.isConnected, el.querySelector('input').value]",
                None,
            )
            .await?;
            assert_eq!(
                inside,
                json!([false, "detached-secret-2"]),
                "a container removed after its handle was taken runs"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// A handle into a replaced document fails as upstream's evaluate there does (spike
/// S-detached), with nothing sent.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_handle_of_a_replaced_document_cannot_run() -> TestResult {
    on_page("/eval.html", |run, page| async move {
        let result = async {
            let list = handle_of(&run, &at(&page, "#list")).await?;
            run.eval(&page, "location.reload()").await?;
            time::sleep(Duration::from_millis(500)).await;
            run.clear_calls();
            let reloaded = evaluate(&run, "elementHandle.evaluate", &list, "(el) => el.id", None)
                .await
                .err()
                .ok_or("a handle into a replaced document cannot run")?;
            assert_eq!(
                reloaded.message(),
                "elementHandle.evaluate: Execution context was destroyed, most likely because of \
                 a navigation"
            );
            not_sent(&run, "a handle of a replaced document");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// `$eval` with no match fails at once, quoting the selector as the script wrote it, and
/// `locator.evaluate` waits for its element until the timeout; neither sends a script.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_missing_match_fails_with_nothing_sent() -> TestResult {
    on_page("/eval.html", |run, page| async move {
        let result = async {
            let list = handle_of(&run, &at(&page, "#list")).await?;
            run.clear_calls();
            let started = Instant::now();
            let missing = eval_first(&run, "page.$eval", &at(&page, "#nowhere"), "#nowhere",
                "(el) => el.id")
                .await
                .err()
                .ok_or("$eval with no match fails")?;
            assert!(started.elapsed() < Duration::from_secs(1), "it does not wait");
            assert_eq!(missing.error_code(), Some(ErrorCode::InvalidInput));
            assert_eq!(
                missing.message(),
                "page.$eval: Failed to find element matching selector \"#nowhere\""
            );
            let under = eval_first(&run, "elementHandle.$eval", &rooted(&page, &list, "b"), "b",
                "(el) => el.id")
                .await
                .err()
                .ok_or("elementHandle.$eval with no match fails")?;
            assert_eq!(
                under.message(),
                "elementHandle.$eval: Failed to find element matching selector \"b\""
            );
            let waited = run
                .invoke(
                    "locator.evaluate",
                    json!({ "call": { "api": "locator.evaluate", "timeoutMs": 300 },
                            "target": at(&page, "#nowhere"), "strict": "strict",
                            "script": script("(el) => el.id", None) }),
                )
                .await
                .err()
                .ok_or("locator.evaluate waits for its element until the timeout")?;
            assert_eq!(waited.error_code(), Some(ErrorCode::NoMatch));
            assert_eq!(
                waited.message(),
                "locator.evaluate: Timeout 300ms exceeded.\nCall log:\n  - waiting for locator('#nowhere')\n"
            );
            not_sent(&run, "no element, no script");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// A handle argument is adopted into its own document only: one of the script frame's
/// document runs; one of a same-origin child frame is refused with upstream's adoption error,
/// where upstream adopts it through a context id (spike S-adopt, a decided deviation); one of
/// another page and one of a replaced document fail with upstream's own messages; and none of
/// those refusals sends anything.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_handle_argument_is_adopted_from_its_own_document_only() -> TestResult {
    on_page("/eval.html", |run, page| async move {
        let result = async {
            run.eval(&page, "document.getElementById('pw').value = 'handle-secret-3'").await?;
            let field = handle_of(&run, &at(&page, "#pw")).await?;
            let list = handle_of(&run, &at(&page, "#list")).await?;
            run.clear_calls();
            let same = evaluate_with(&run, &page, "([a, b]) => [a.id, b.id, a.value]",
                &[&field, &list]).await?;
            assert_eq!(same, json!(["pw", "list", "handle-secret-3"]), "handles of the page's document");
            sent_once(&run, "a script with handle arguments");
            let one = run
                .invoke("locator.evaluate", json!({
                    "call": { "api": "locator.evaluate", "timeoutMs": 10_000 },
                    "target": at(&page, "#list"), "strict": "strict",
                    "script": { "run": { "expression": "(el, f) => [el.id, f.id]", "isFunction": true,
                        "arg": r#"{"h":0}"#, "handles": [&field] } } }))
                .await
                .map(plain)?;
            assert_eq!(one, json!(["list", "pw"]), "an element script takes a handle too");
            let inner = locator(&page, &[css("#inner"), enter_frame(), css("#reach")],
                "locator('#inner').contentFrame().locator('#reach')");
            let child = handle_of(&run, &inner).await?;
            let other = run.new_page().await?;
            run.clear_calls();
            let adopted = evaluate_with(&run, &page, "([a]) => a.id", &[&child])
                .await
                .err()
                .ok_or("a child frame's handle is not adopted into the main frame")?;
            assert_eq!(
                adopted.message(),
                "page.evaluate: Unable to adopt element handle from a different document"
            );
            let mut foreign = field.clone();
            foreign["page"] = brand(&other);
            let elsewhere = evaluate_with(&run, &page, "([a]) => a.id", &[&foreign])
                .await
                .err()
                .ok_or("another page's handle is refused")?;
            assert_eq!(elsewhere.message(), "page.evaluate: Frame has been detached.");
            not_sent(&run, "a handle that cannot be adopted");
            run.close_page(&other).await;
            run.eval(&page, "location.reload()").await?;
            time::sleep(Duration::from_millis(500)).await;
            run.clear_calls();
            let retired = evaluate_with(&run, &page, "([a]) => a.id", &[&list])
                .await
                .err()
                .ok_or("a handle of a replaced document is refused")?;
            assert_eq!(
                retired.message(),
                "page.evaluate: Protocol error (DOM.describeNode): Cannot find context with \
                 specified id"
            );
            not_sent(&run, "a handle of a replaced document");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// Every element form is one send: the host's recheck, then the mark, nothing between, as for
/// `page.evaluate`. A locator's wait also logs its call-log notes before it, which are no sends.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_each_element_send_is_rechecked_then_marked() -> TestResult {
    on_page("/eval.html", |run, page| async move {
        let result = async {
            let list = handle_of(&run, &at(&page, "#list")).await?;
            let forms: [(&str, Value, bool); 6] = [
                ("locator.evaluate", at(&page, "#pw"), false),
                ("elementHandle.evaluate", list.clone(), false),
                ("page.$eval", at(&page, "li"), true),
                ("elementHandle.$eval", rooted(&page, &list, "li"), true),
                ("locator.evaluateAll", at(&page, "li"), false),
                ("elementHandle.$$eval", rooted(&page, &list, "li"), false),
            ];
            for (api, target, first) in forms {
                run.clear_calls();
                if api.ends_with("evaluateAll") || api.ends_with("$$eval") {
                    evaluate_all(&run, api, &target, "(els) => els.length", None).await?;
                } else if first {
                    eval_first(&run, api, &target, "li", "(el) => el.id").await?;
                } else {
                    evaluate(&run, api, &target, "(el) => el.id", None).await?;
                }
                let sends: Vec<EnvCall> = run
                    .calls()
                    .into_iter()
                    .filter(|call| matches!(call, EnvCall::Recheck | EnvCall::MarkSent))
                    .collect();
                assert_eq!(sends, [EnvCall::Recheck, EnvCall::MarkSent], "{api}");
            }
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// The owner taking over while the op prepares its script.
fn takeover() -> Intercept {
    Intercept::Refuse(Refusal {
        owner: ErrorOwner::Host,
        code: "owner_has_control".to_owned(),
        message: "the owner has control of this tab".to_owned(),
        host_code: None,
    })
}

/// Each element script names its main-world objects (`DOM.resolveNode`) before the host's
/// recheck: a takeover while they resolve refuses the op with nothing sent and no script run,
/// and a cancel there leaves it not sent.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_takeover_while_the_script_is_prepared_sends_nothing() -> TestResult {
    on_page("/eval.html", |run, page| async move {
        let result = async {
            let script = "(subject) => { window.ran = true; return 1; }";
            let handle = handle_of(&run, &at(&page, "#list")).await?;
            let cases = [
                (
                    "locator.evaluate",
                    at(&page, "#pw"),
                    "the handle's node, then the element",
                ),
                (
                    "elementHandle.evaluate",
                    handle.clone(),
                    "the handle's node, then the element",
                ),
                (
                    "locator.evaluateAll",
                    at(&page, "#list li"),
                    "the three elements",
                ),
                (
                    "locator.evaluateAll",
                    at(&page, ".nothing"),
                    "the frame's document",
                ),
            ];
            for (api, target, resolving) in cases {
                run.clear_calls();
                run.intercept("DOM.resolveNode", takeover());
                let refused = if api == "locator.evaluateAll" {
                    evaluate_all(&run, api, &target, script, None).await
                } else {
                    evaluate(&run, api, &target, script, None).await
                };
                let error = refused.err().ok_or(format!(
                    "{api}: a takeover while {resolving} resolve refuses"
                ))?;
                assert_eq!(
                    (error.owner(), error.code(), error.answer()),
                    (ErrorOwner::Host, "owner_has_control", Answer::Definite),
                    "{api}: {error}"
                );
                assert!(
                    run.calls().contains(&EnvCall::Recheck),
                    "{api}: {:?}",
                    run.calls()
                );
                not_sent(&run, api);
            }
            assert_eq!(
                run.eval(&page, "window.ran === true").await?,
                json!(false),
                "no script ran"
            );

            run.clear_calls();
            run.intercept("DOM.resolveNode", Intercept::Hang);
            let cancelled = time::timeout(
                Duration::from_secs(2),
                evaluate_all(
                    &run,
                    "locator.evaluateAll",
                    &at(&page, "#list li"),
                    script,
                    None,
                ),
            )
            .await;
            assert!(
                cancelled.is_err(),
                "the op waits on the elements until the host drops it"
            );
            not_sent(&run, "a cancel before the script");

            run.clear_calls();
            assert_eq!(
                evaluate_all(
                    &run,
                    "locator.evaluateAll",
                    &at(&page, "#list li"),
                    script,
                    None
                )
                .await?,
                json!(1)
            );
            sent_once(&run, "an evaluateAll that runs");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_element_scripts_run_through_the_prelude() -> TestResult {
    let server = FixtureServer::start([("/eval.html".to_owned(), Page::html(EVAL))].into()).await?;
    let host: Arc<dyn OpHost> = Arc::new(LiveRun::connect().await?);
    let seed = json!({ "page": null, "pages": [], "browserVersion": "" });
    let script = format!(
        "try {{\n\
           await page.goto({url})\n\
           const field = page.locator('#pw')\n\
           await field.evaluate((el, typed) => {{ el.value = typed }}, 'runner-secret-42')\n\
           const value = await field.evaluate((el) => el.value)\n\
           const all = await page.locator('#list li').evaluateAll((els, sep) => els.map((el) => el.textContent).join(sep), '|')\n\
           const list = await page.$('#list')\n\
           const first = await list.$eval('li', (el, suffix) => el.textContent + suffix, '!')\n\
           const every = await list.$$eval('li', (els) => els.length)\n\
           const own = await list.evaluate((el) => el.id)\n\
           const byPage = await page.$eval('#pw', (el) => el.value)\n\
           const pageAll = await page.$$eval('li', (els) => els.map((el) => el.textContent))\n\
           let missing = null\n\
           try {{ await list.$eval('b', (el) => el) }} catch (error) {{ missing = error.message }}\n\
           return {{ value, all, first, every, own, byPage, pageAll, missing }}\n\
         }} finally {{\n\
           await page.close()\n\
         }}",
        url = json!(server.url("127.0.0.1", "/eval.html")),
    );
    let result = run_script(&host, &seed, &script).await?;
    assert_eq!(
        result,
        json!({
            "value": "runner-secret-42",
            "all": "alpha|beta|gamma",
            "first": "alpha!",
            "every": 3,
            "own": "list",
            "byPage": "runner-secret-42",
            "pageAll": ["alpha", "beta", "gamma"],
            "missing": "elementHandle.$eval: Failed to find element matching selector \"b\"",
        }),
        "each answer is what Playwright gives"
    );
    Ok(())
}

/// The opening of a pin script: `record(key, call)` keeps what one call threw as the script sees
/// it, or `'passed'`: the error's name and its message's first line, and `stack` when a page error
/// carries the main world's frames, whose line numbers belong to the injected main-world script.
const RECORD: &str = "const seen = {}\n\
const record = async (key, call) => {\n\
  try { await call(); seen[key] = 'passed' }\n\
  catch (error) {\n\
    const [first, ...frames] = error.message.split('\\nCall log:')[0].split('\\n    at ')\n\
    seen[key] = { name: error.name, message: first, ...(frames.length ? { stack: true } : {}) }\n\
  }\n\
}\n";

/// What a script sees when an evaluation fails, through the real prelude: upstream's messages and
/// error names, `$eval`'s no-match error quoting the selector as the script wrote it, and the
/// refusals of a script that cannot run, raised with nothing sent. The evaluating ops' wire may
/// change shape (task 006); none of these may.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_evaluation_failures_read_as_playwright_and_refusals_send_nothing() -> TestResult
{
    let server = FixtureServer::start([("/eval.html".to_owned(), Page::html(EVAL))].into()).await?;
    let run = Arc::new(LiveRun::connect().await?);
    let page = run.new_page().await?;
    let opened = run
        .invoke(
            "page.goto",
            json!({ "call": { "timeoutMs": 30_000 }, "page": brand(&page),
                    "url": server.url("127.0.0.1", "/eval.html") }),
        )
        .await;
    let result = match opened {
        Ok(_) => evaluation_failures(&run, &page).await,
        Err(error) => Err(error.into()),
    };
    run.close_page(&page).await;
    result
}

async fn evaluation_failures(run: &Arc<LiveRun>, page: &TestPage) -> TestResult {
    refusals(run, page).await?;
    failures(run, page).await
}

/// Scripts that cannot run: refused with upstream's message, and nothing sent.
async fn refusals(run: &Arc<LiveRun>, page: &TestPage) -> TestResult {
    let host: Arc<dyn OpHost> = Arc::<LiveRun>::clone(run);
    let init = json!({ "page": brand(page) });
    // `Math.max` prints as native code, which upstream's server cannot normalize; an argument
    // holding a function fails upstream's client after a locator's wait.
    run.clear_calls();
    let refused = run_script(
        &host,
        &init,
        &format!(
            "{RECORD}\
             const pw = await page.$('#pw')\n\
             const sent = () => {{}}\n\
             await record('page.evaluate', () => page.evaluate(Math.max))\n\
             await record('frame.evaluate', () => page.mainFrame().evaluate(Math.max))\n\
             await record('page.waitForFunction', () => page.waitForFunction(Math.max))\n\
             await record('page.$eval', () => page.$eval('#pw', Math.max))\n\
             await record('page.$$eval', () => page.$$eval('li', Math.max))\n\
             await record('locator.evaluate', () => page.locator('#pw').evaluate(Math.max))\n\
             await record('locator.evaluateAll', () => page.locator('li').evaluateAll(Math.max))\n\
             await record('locator.waitForFunction', () => page.locator('#pw').waitForFunction(Math.max))\n\
             await record('elementHandle.evaluate', () => pw.evaluate(Math.max))\n\
             const list = await page.$('#list')\n\
             await record('elementHandle.$eval', () => list.$eval('li', Math.max))\n\
             await record('elementHandle.$eval no match first', () => pw.$eval('*', Math.max))\n\
             await record('locator.evaluate arg', () => page.locator('#pw').evaluate((el) => el, sent))\n\
             return seen"
        ),
    )
    .await?;
    let calls = run.calls();
    assert!(
        !calls.iter().any(|call| matches!(call, EnvCall::MarkSent)),
        "a refused script sends nothing: {calls:?}"
    );
    let refusal = |api: &str| json!({ "name": "Error", "message": format!("{api}: Passed function is not well-serializable!") });
    assert_eq!(
        refused,
        json!({
            "page.evaluate": refusal("page.evaluate"),
            "frame.evaluate": refusal("frame.evaluate"),
            "page.waitForFunction": refusal("page.waitForFunction"),
            "page.$eval": refusal("page.$eval"),
            "page.$$eval": refusal("page.$$eval"),
            "locator.evaluate": refusal("locator.evaluate"),
            "locator.evaluateAll": refusal("locator.evaluateAll"),
            "locator.waitForFunction": refusal("locator.waitForFunction"),
            "elementHandle.evaluate": refusal("elementHandle.evaluate"),
            "elementHandle.$eval": refusal("elementHandle.$eval"),
            // Upstream's server finds the element before it normalizes the expression.
            "elementHandle.$eval no match first": {
                "name": "Error",
                "message": "elementHandle.$eval: Failed to find element matching selector \"*\"",
            },
            // Upstream's client serializes the argument after the locator's wait.
            "locator.evaluate arg": {
                "name": "Error",
                "message": "locator.evaluate: Attempting to serialize unexpected value: () => {}",
            },
        }),
        "refusals"
    );
    Ok(())
}

/// Scripts that run, or wait, and fail as upstream's do.
async fn failures(run: &Arc<LiveRun>, page: &TestPage) -> TestResult {
    let host: Arc<dyn OpHost> = Arc::<LiveRun>::clone(run);
    let init = json!({ "page": brand(page) });
    let failed = run_script(
        &host,
        &init,
        &format!(
            "{RECORD}\
             const list = await page.$('#list')\n\
             await record('page.$eval', () => page.$eval('#nowhere', (el) => el))\n\
             await record('frame.$eval', () => page.mainFrame().$eval('#nowhere', (el) => el))\n\
             await record('elementHandle.$eval', () => list.$eval('b', (el) => el))\n\
             await record('locator.evaluate missing', () => page.locator('#nowhere').evaluate((el) => el, () => {{}}, {{ timeout: 300 }}))\n\
             await record('page.evaluate arg', () => page.evaluate((x) => x, () => {{}}))\n\
             await record('page.evaluate throws', () => page.evaluate(() => {{ throw new Error('boom') }}))\n\
             await record('locator.evaluate throws', () => page.locator('#pw').evaluate(() => {{ throw new Error('boom') }}))\n\
             await record('locator.evaluateAll throws', () => page.locator('li').evaluateAll(() => {{ throw new Error('boom') }}))\n\
             await record('page.waitForFunction throws', () => page.waitForFunction(() => {{ throw new Error('boom') }}))\n\
             await record('page.waitForFunction timeout', () => page.waitForFunction(() => false, null, {{ timeout: 300 }}))\n\
             await record('locator.waitForFunction timeout', () => page.locator('#pw').waitForFunction(() => false, null, {{ timeout: 300 }}))\n\
             await record('page.waitForFunction polling', () => page.waitForFunction(() => false, null, {{ polling: 'often' }}))\n\
             return seen"
        ),
    )
    .await?;
    let error = |message: &str| json!({ "name": "Error", "message": message });
    let timeout = |api: &str| json!({ "name": "TimeoutError", "message": format!("{api}: Timeout 300ms exceeded.") });
    let thrown = |api: &str| json!({ "name": "Error", "message": format!("{api}: Error: boom"), "stack": true });
    assert_eq!(
        failed,
        json!({
            "page.$eval": error("page.$eval: Failed to find element matching selector \"#nowhere\""),
            "frame.$eval": error("frame.$eval: Failed to find element matching selector \"#nowhere\""),
            "elementHandle.$eval": error("elementHandle.$eval: Failed to find element matching selector \"b\""),
            // A locator waits before upstream's client serializes the argument, so the wait's
            // timeout comes first.
            "locator.evaluate missing": timeout("locator.evaluate"),
            // A frame's evaluate has no wait: upstream's client throws before anything is sent.
            "page.evaluate arg": error("Attempting to serialize unexpected value: () => {}"),
            "page.evaluate throws": thrown("page.evaluate"),
            "locator.evaluate throws": thrown("locator.evaluate"),
            "locator.evaluateAll throws": thrown("locator.evaluateAll"),
            "page.waitForFunction throws": thrown("page.waitForFunction"),
            "page.waitForFunction timeout": timeout("page.waitForFunction"),
            "locator.waitForFunction timeout": timeout("locator.waitForFunction"),
            "page.waitForFunction polling": error("Unknown polling option: often"),
        }),
        "failures"
    );
    Ok(())
}
