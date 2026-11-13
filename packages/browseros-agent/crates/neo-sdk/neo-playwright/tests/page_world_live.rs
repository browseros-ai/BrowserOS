//! The page ops that need an isolated world or the main world, on a real browser, through
//! `Playwright::invoke`: `evaluate` and `waitForFunction` in the main world, `title` and
//! `content` read in the isolated world, `setContent` with its load states, and `waitForURL`. That `setContent`
//! retires the document's seq is checked in `src/world/live_tests.rs`, where seqs are visible.
//!
//! Needs a throwaway BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005), for example
//! `bun run test:browser -- cargo test -p neo-playwright --test page_world_live -- --ignored`.
//! Every test opens its own tab and closes it.

#[path = "support/fixture_server.rs"]
#[expect(
    dead_code,
    reason = "scripts and extra headers serve the world live tests"
)]
mod fixture_server;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "the capture refusal and the click serve pages_live, which includes this too"
)]
mod live_run;
#[path = "support/page_server.rs"]
mod page_server;

use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant};

use fixture_server::{FixtureServer, Page};
use live_run::{EnvCall, Intercept, LiveRun, TestPage, TestResult, brand};
use neo_playwright::{Answer, ErrorCode, PlaywrightError, Refusal, TimedOut};
use neo_trace::ErrorOwner;
use page_server::{PageServer, SLOW};
use serde_json::{Value, json};
use tokio::time;

/// Two password fields, one of them in a same-origin frame, and text that holds no secret.
const LOGIN: &str = r#"<!doctype html><html><head><title>Login</title></head><body>
<label>Password <input id="pw" type="password"></label>
<label>Later <input id="late" type="password"></label>
<p id="note">nothing secret here</p>
<iframe id="inner" srcdoc="<label>Code <input id=code type=password></label>"></iframe>
</body></html>"#;

const OTHER: &str = "<!doctype html><title>Other</title><p>another document</p>";

/// A password field whose value the page reflects into its attribute and echoes in its text,
/// and one that keeps the value its markup gave it.
const REFLECT: &str = r#"<!doctype html><html><head><title>Account</title></head><body>
<label>Password <input id="pw" type="password"></label>
<label>Old <input id="old" type="password" value="attr-secret-1"></label>
<p id="echo"></p>
</body></html>"#;

/// A step a navigation passes through on its way to `/done.html#end`.
const STEP: &str =
    "<!doctype html><title>Step</title><script>location.replace('/done.html#end')</script>";

const DONE: &str = "<!doctype html><title>Done</title><p>done</p>";

/// A doctype with a public and a system identifier.
const XHTML: &str = r#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd">
<html><head><title>Strict</title></head><body><p>strict</p></body></html>"#;

const FIELD: &str =
    r#"<!doctype html><title>Field</title><label>Password <input type="password"></label>"#;

/// A settings page whose server put a secret in its password field, as an API key page does.
const FILLED: &str = r#"<!doctype html><title>Filled</title>
<label>API key <input type="password" value="filled-secret-93"></label>"#;

/// Where a framed test's child frame navigates to: a field of its own, filled by its server.
const MOVED: &str = r#"<!doctype html><title>Moved</title>
<label>Token <input type="password" value="moved-secret-57"></label>"#;

/// A page on 127.0.0.1 holding one frame of its own site and one of another (`localhost`), an
/// out-of-process frame. `{other}` is the fixture server's `localhost` origin.
const FRAMED: &str = r#"<!doctype html><html><head><title>Framed</title></head><body>
<label>Password <input id="pw" type="password"></label>
<iframe name="same" src="/field.html"></iframe>
<iframe name="cross" src="{other}/filled.html"></iframe>
</body></html>"#;

async fn fixtures() -> TestResult<FixtureServer> {
    let pages = HashMap::from([
        ("/field.html".to_owned(), Page::html(FIELD)),
        ("/filled.html".to_owned(), Page::html(FILLED)),
        ("/login.html".to_owned(), Page::html(LOGIN)),
        ("/other.html".to_owned(), Page::html(OTHER)),
        ("/reflect.html".to_owned(), Page::html(REFLECT)),
        ("/xhtml.html".to_owned(), Page::html(XHTML)),
        ("/step.html".to_owned(), Page::html(STEP)),
        ("/done.html".to_owned(), Page::html(DONE)),
    ]);
    Ok(FixtureServer::start(pages).await?)
}

/// Opens a tab on `path` of the fixture server, runs `body` on it, and closes the tab whatever
/// happened.
async fn on_page<F, Fut>(path: &str, body: F) -> TestResult
where
    F: FnOnce(LiveRun, FixtureServer, TestPage) -> Fut,
    Fut: Future<Output = (LiveRun, TestResult)>,
{
    let run = LiveRun::connect().await?;
    let server = fixtures().await?;
    let page = run.new_page().await?;
    if let Err(error) = goto(&run, &page, &server.url("127.0.0.1", path)).await {
        run.close_page(&page).await;
        return Err(error.into());
    }
    let (run, result) = body(run, server, page.clone()).await;
    run.close_page(&page).await;
    result
}

async fn goto(run: &LiveRun, page: &TestPage, url: &str) -> Result<Value, PlaywrightError> {
    run.invoke(
        "page.goto",
        json!({ "call": { "timeoutMs": 30_000 }, "page": brand(page), "url": url }),
    )
    .await
}

/// A script's fields as the prelude sends them beside an op's addressing, `arg` already in
/// Playwright's in-page format.
fn script(expression: &str, is_function: bool, arg: &Value) -> Value {
    json!({ "expression": expression, "isFunction": is_function,
            "arg": { "value": arg.to_string(), "handles": [] } })
}

/// `input` with `fields` added, as the prelude spreads a script beside an op's addressing.
fn with_fields(mut input: Value, fields: Value) -> Value {
    if let (Some(input), Value::Object(fields)) = (input.as_object_mut(), fields) {
        input.extend(fields);
    }
    input
}

/// Playwright's in-page form of `undefined`, the argument a script given none receives.
fn undefined() -> Value {
    json!({ "v": "undefined" })
}

/// `page.evaluate(fn)` as the prelude sends it.
async fn evaluate(
    run: &LiveRun,
    page: &TestPage,
    function: &str,
) -> Result<Value, PlaywrightError> {
    run.invoke(
        "page.evaluate",
        with_fields(
            json!({ "page": brand(page) }),
            script(function, true, &undefined()),
        ),
    )
    .await
}

/// `page.waitForFunction(fn)` as the prelude sends it, with `extra` fields (`pollingInterval`).
async fn wait_for_function(
    run: &LiveRun,
    page: &TestPage,
    timeout_ms: u64,
    function: &str,
    extra: Value,
) -> Result<Value, PlaywrightError> {
    let input = with_fields(
        json!({ "call": { "timeoutMs": timeout_ms }, "page": brand(page) }),
        script(function, true, &undefined()),
    );
    run.invoke("page.waitForFunction", with_fields(input, extra))
        .await
}

/// An error message's first line: upstream keeps the page's stack lines after it.
fn first_line(error: &PlaywrightError) -> &str {
    error.message().lines().next().unwrap_or_default()
}

/// Fills the page's fields from the page's own world, as a user or the page would.
async fn type_secrets(run: &LiveRun, page: &TestPage) -> TestResult {
    run.eval(
        page,
        "document.getElementById('pw').value = 'hunter2-typed'; \
         document.getElementById('inner').contentDocument.getElementById('code').value = '482913'",
    )
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_evaluate_reads_a_typed_password_as_the_page_holds_it() -> TestResult {
    on_page("/login.html", |run, _server, page| async move {
        let result = async {
            type_secrets(&run, &page).await?;
            run.clear_calls();
            let echoed = evaluate(
                &run,
                &page,
                "() => { const pw = document.getElementById('pw').value; \
                 const code = document.getElementById('inner').contentDocument.getElementById('code').value; \
                 return [pw, 'you typed ' + pw, code, Number(code)]; }",
            )
            .await?;
            assert_eq!(
                echoed,
                json!({ "a": ["hunter2-typed", "you typed hunter2-typed", "482913", 482_913], "id": 1 }),
                "the main frame's and the same-process frame's values, a code as a number too"
            );
            assert!(
                run.calls().starts_with(&[EnvCall::Recheck, EnvCall::MarkSent]),
                "the script ran only after the host's recheck and mark_sent: {:?}",
                run.calls()
            );

            let late = evaluate(
                &run,
                &page,
                "() => { const late = document.getElementById('late'); \
                 late.value = 'late-secret-77'; return late.value; }",
            )
            .await?;
            assert_eq!(
                late,
                json!("late-secret-77"),
                "a value the script itself put in a field"
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
async fn tests_that_evaluate_returns_page_values_no_field_holds_untouched() -> TestResult {
    on_page("/login.html", |run, _server, page| async move {
        let result = async {
            type_secrets(&run, &page).await?;
            let note = evaluate(
                &run,
                &page,
                "() => document.getElementById('note').textContent",
            )
            .await?;
            assert_eq!(note, json!("nothing secret here"));
            let expression = run
                .invoke(
                    "page.evaluate",
                    with_fields(
                        json!({ "page": brand(&page) }),
                        script("document.title", false, &undefined()),
                    ),
                )
                .await?;
            assert_eq!(expression, json!("Login"), "a string is an expression");
            let x = json!({ "o": [{ "k": "x", "v": 21 }], "id": 1 });
            let with_arg = run
                .invoke(
                    "page.evaluate",
                    with_fields(
                        json!({ "page": brand(&page) }),
                        script("({ x }) => x * 2", true, &x),
                    ),
                )
                .await?;
            assert_eq!(with_arg, json!(42));
            let null = json!({ "v": "null" });
            let null_arg = run
                .invoke(
                    "page.evaluate",
                    with_fields(
                        json!({ "page": brand(&page) }),
                        script("(a) => a === null", true, &null),
                    ),
                )
                .await?;
            assert_eq!(null_arg, json!(true), "a null argument stays null");
            let nothing = evaluate(&run, &page, "() => undefined").await?;
            assert_eq!(nothing, undefined(), "undefined comes back as itself");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_thrown_error_reads_as_playwrights_page_evaluate_message() -> TestResult {
    on_page("/login.html", |run, _server, page| async move {
        let result = async {
            type_secrets(&run, &page).await?;
            let cases = [
                (
                    "() => { throw new TypeError('bad thing'); }",
                    "page.evaluate: TypeError: bad thing",
                ),
                (
                    "async () => { throw new Error('later'); }",
                    "page.evaluate: Error: later",
                ),
                (
                    "() => { throw new Error('saw ' + document.getElementById('pw').value); }",
                    "page.evaluate: Error: saw hunter2-typed",
                ),
            ];
            for (function, message) in cases {
                let error = evaluate(&run, &page, function)
                    .await
                    .err()
                    .ok_or("the script throws")?;
                assert_eq!(first_line(&error), message);
                assert!(
                    error
                        .message()
                        .lines()
                        .skip(1)
                        .all(|line| line.starts_with("    at ")),
                    "the page's stack lines follow, as upstream keeps them: {error}"
                );
                assert_eq!(error.error_code(), Some(ErrorCode::PageError));
                assert_eq!(error.owner(), ErrorOwner::Website);
                assert_eq!(error.answer(), Answer::Definite, "the script ran and threw");
            }
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_wait_for_function_polls_until_truthy_and_times_out() -> TestResult {
    on_page("/login.html", |run, _server, page| async move {
        let result = async {
            type_secrets(&run, &page).await?;
            run.eval(
                &page,
                "setTimeout(() => { window.ready = document.getElementById('pw').value; }, 300)",
            )
            .await?;
            let started = Instant::now();
            let ready =
                wait_for_function(&run, &page, 5000, "() => window.ready", json!({})).await?;
            assert_eq!(
                ready,
                json!({ "value": "hunter2-typed", "preview": "hunter2-typed" }),
                "the truthy result and its preview"
            );
            assert!(
                started.elapsed() >= Duration::from_millis(250),
                "it waited for the page: {:?}",
                started.elapsed()
            );

            let counted = wait_for_function(
                &run,
                &page,
                5000,
                "() => (window.polls = (window.polls || 0) + 1) >= 3 && window.polls",
                json!({ "pollingInterval": 20 }),
            )
            .await?;
            assert_eq!(
                counted,
                json!({ "value": 3, "preview": "3" }),
                "each poll runs the function once"
            );

            let started = Instant::now();
            let error = wait_for_function(&run, &page, 500, "() => window.never", json!({}))
                .await
                .err()
                .ok_or("a function that stays falsy times out")?;
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "{:?}",
                started.elapsed()
            );
            assert_eq!(error.error_code(), Some(ErrorCode::WaitTimeout));
            assert_eq!(error.timed_out(), TimedOut::Yes);
            assert!(
                error
                    .message()
                    .starts_with("page.waitForFunction: Timeout 500ms exceeded."),
                "{error}"
            );

            let thrown = wait_for_function(
                &run,
                &page,
                5000,
                "() => { throw new Error('no'); }",
                json!({}),
            )
            .await
            .err()
            .ok_or("a throwing function fails the wait")?;
            assert_eq!(first_line(&thrown), "page.waitForFunction: Error: no");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// The owner taking over while the op names the main-world document (`DOM.resolveNode`).
fn takeover() -> Intercept {
    Intercept::Refuse(Refusal {
        owner: ErrorOwner::Host,
        code: "owner_has_control".to_owned(),
        message: "the owner has control of this tab".to_owned(),
        host_code: None,
    })
}

/// `evaluate` and each `waitForFunction` poll mark the op sent only once the main-world document
/// is named, right before the script: a takeover while it resolves refuses the op with nothing
/// sent, and so does a cancel there.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_main_world_scripts_are_sent_only_after_their_document_resolves() -> TestResult {
    on_page("/login.html", |run, _server, page| async move {
        let result = async {
            let script = "() => { window.ran = true; return 1; }";
            let not_sent = |what: &str| {
                let calls = run.calls();
                assert!(calls.contains(&EnvCall::Recheck), "{what}: {calls:?}");
                assert!(!calls.contains(&EnvCall::MarkSent), "{what}: {calls:?}");
            };
            for op in ["page.evaluate", "page.waitForFunction"] {
                run.clear_calls();
                run.intercept("DOM.resolveNode", takeover());
                let error = run
                    .invoke(
                        op,
                        with_fields(
                            json!({ "call": { "timeoutMs": 5000 }, "page": brand(&page) }),
                            self::script(script, true, &undefined()),
                        ),
                    )
                    .await
                    .err()
                    .ok_or(format!(
                        "{op}: a takeover while the document resolves refuses"
                    ))?;
                assert_eq!(
                    (error.owner(), error.code(), error.answer()),
                    (ErrorOwner::Host, "owner_has_control", Answer::Definite),
                    "{op}: {error}"
                );
                not_sent(op);
            }
            assert_eq!(
                run.eval(&page, "window.ran === true").await?,
                json!(false),
                "no script ran"
            );

            run.clear_calls();
            run.intercept("DOM.resolveNode", Intercept::Hang);
            let cancelled =
                time::timeout(Duration::from_secs(2), evaluate(&run, &page, script)).await;
            assert!(
                cancelled.is_err(),
                "the op waits on the document until the host drops it"
            );
            let calls = run.calls();
            assert!(
                !calls.contains(&EnvCall::MarkSent),
                "a cancel before the script leaves the op not sent: {calls:?}"
            );

            run.clear_calls();
            assert_eq!(evaluate(&run, &page, script).await?, json!(1));
            let calls = run.calls();
            assert_eq!(
                calls
                    .iter()
                    .filter(|call| **call == EnvCall::MarkSent)
                    .count(),
                1,
                "an evaluate that runs is marked sent once: {calls:?}"
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
async fn tests_that_evaluate_runs_in_the_document_the_tab_shows_after_navigations() -> TestResult {
    on_page("/login.html", |run, server, page| async move {
        let result = async {
            let href = "() => location.href";
            let first = evaluate(&run, &page, href).await?;
            assert_eq!(first, json!(server.url("127.0.0.1", "/login.html")));
            // The same renderer: the main world's context is replaced by a new one.
            goto(&run, &page, &server.url("127.0.0.1", "/other.html")).await?;
            let same_site = evaluate(&run, &page, href).await?;
            assert_eq!(same_site, json!(server.url("127.0.0.1", "/other.html")));
            // Another site: another renderer, where context ids start over.
            goto(&run, &page, &server.url("localhost", "/login.html")).await?;
            let cross_site = evaluate(&run, &page, href).await?;
            assert_eq!(cross_site, json!(server.url("localhost", "/login.html")));
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// A main-world call still pending when its page navigates to another site is cut off, as the
/// page's document leaves for one in another renderer. `waitForFunction`'s predicate is meant
/// to be free of side effects, so the wait goes on in the new document, every poll rechecked
/// and marked, and settles there, as upstream's does; a script `page.evaluate` sent may have
/// run, so it goes out once and fails with upstream's message.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_cross_site_navigation_cuts_off_main_world_calls_as_upstreams() -> TestResult {
    on_page("/login.html", |run, server, page| async move {
        let result = async {
            let other = server.url("localhost", "/other.html");
            run.eval(
                &page,
                &format!("setTimeout(() => {{ location.href = {other:?}; }}, 400), true"),
            )
            .await?;
            run.clear_calls();
            let started = Instant::now();
            let waited = wait_for_function(
                &run,
                &page,
                10_000,
                "() => location.hostname === 'localhost' ? document.title : new Promise(() => {})",
                json!({}),
            )
            .await?;
            assert_eq!(
                waited.get("value"),
                Some(&json!("Other")),
                "settled in the new document: {waited}"
            );
            assert!(
                started.elapsed() >= Duration::from_millis(300),
                "it waited for the navigation, {:?}",
                started.elapsed()
            );
            let effects: Vec<EnvCall> = run
                .calls()
                .into_iter()
                .filter(|call| !matches!(call, EnvCall::Note(_)))
                .collect();
            assert!(
                effects.len() >= 4
                    && effects
                        .chunks(2)
                        .all(|pair| pair == [EnvCall::Recheck, EnvCall::MarkSent]),
                "the cut-off poll and one in the new document, each rechecked, then marked: \
                 {effects:?}"
            );
            goto(&run, &page, &server.url("127.0.0.1", "/login.html")).await?;
            run.clear_calls();
            let script = format!(
                "() => {{ setTimeout(() => {{ location.href = {other:?}; }}, 300); \
                 return new Promise(() => {{}}); }}"
            );
            let error = evaluate(&run, &page, &script)
                .await
                .err()
                .ok_or("a script a navigation cut off does not answer")?;
            assert_eq!(
                (error.error_code(), error.message()),
                (
                    Some(ErrorCode::PageError),
                    "page.evaluate: Execution context was destroyed, most likely because of a \
                     navigation."
                ),
            );
            let sent = run
                .calls()
                .iter()
                .filter(|call| **call == EnvCall::MarkSent)
                .count();
            assert_eq!(sent, 1, "the script went out once");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// `page.title()` or `page.content()`'s text.
async fn document_text(run: &LiveRun, page: &TestPage, op: &str) -> TestResult<String> {
    let read = run.invoke(op, json!({ "page": brand(page) })).await?;
    Ok(read
        .as_str()
        .ok_or(format!("{op} returns its text: {read}"))?
        .to_owned())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_title_and_content_read_the_page_as_it_is() -> TestResult {
    on_page("/reflect.html", |run, server, page| async move {
        let result = async {
            run.eval(
                &page,
                "const pw = document.getElementById('pw'); pw.value = 'typed-secret-2'; \
                 pw.setAttribute('value', pw.value); \
                 document.getElementById('echo').textContent = 'you typed ' + pw.value; \
                 document.title = 'Account of typed-secret-2'",
            )
            .await?;
            let title = document_text(&run, &page, "page.title").await?;
            assert_eq!(
                title, "Account of typed-secret-2",
                "a title reads as the page wrote it"
            );

            let content = document_text(&run, &page, "page.content").await?;
            assert!(
                content.starts_with("<!DOCTYPE html><html><head>"),
                "{content}"
            );
            assert!(
                content.contains(r#"<input id="pw" type="password" value="typed-secret-2">"#)
                    && content
                        .contains(r#"<input id="old" type="password" value="attr-secret-1">"#),
                "every value attribute serializes as the markup holds it: {content}"
            );
            assert!(
                content.contains("you typed typed-secret-2"),
                "the page's own text comes back as the page wrote it: {content}"
            );
            let raw = run
                .eval(&page, "document.documentElement.outerHTML")
                .await?;
            assert!(
                raw.as_str()
                    .is_some_and(|raw| raw.contains("typed-secret-2")),
                "the page's own markup still holds the value"
            );

            goto(&run, &page, &server.url("127.0.0.1", "/xhtml.html")).await?;
            let doctype = run
                .eval(
                    &page,
                    "new XMLSerializer().serializeToString(document.doctype)",
                )
                .await?;
            let doctype = doctype.as_str().ok_or("the page serializes its doctype")?;
            let content = document_text(&run, &page, "page.content").await?;
            assert!(
                doctype.contains("PUBLIC") && content.starts_with(&format!("{doctype}<html")),
                "the probe prints the doctype as XMLSerializer does: {doctype} / {content}"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// The HTML serializer writes a typed `&`, `"`, `<` or `>` as an entity, so an echo of the value
/// leaves `content()`, and a script's `outerHTML`, in that form, not the raw one.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_content_serializes_an_echo_as_the_page_markup_does() -> TestResult {
    on_page("/reflect.html", |run, _server, page| async move {
        let result = async {
            run.eval(
                &page,
                r#"const pw = document.getElementById('pw'); pw.value = 'typed&"secret<3>';
                 const echo = document.getElementById('echo');
                 echo.textContent = 'you typed ' + pw.value; echo.setAttribute('data-echo', pw.value)"#,
            )
            .await?;
            let raw = run
                .eval(&page, "document.getElementById('echo').outerHTML")
                .await?;
            assert_eq!(
                raw,
                json!(
                    r#"<p id="echo" data-echo="typed&amp;&quot;secret&lt;3&gt;">you typed typed&amp;"secret&lt;3&gt;</p>"#
                ),
                "the page's own markup holds the escaped forms"
            );

            let content = document_text(&run, &page, "page.content").await?;
            assert!(
                raw.as_str().is_some_and(|raw| content.contains(raw)),
                "content() serializes the echo as the page's markup holds it: {content}"
            );

            let echoed = evaluate(
                &run,
                &page,
                "() => document.getElementById('echo').outerHTML",
            )
            .await?;
            assert_eq!(
                echoed,
                raw,
                "a script's serialized markup comes back as the page serializes it"
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
async fn tests_that_set_content_writes_the_document_and_waits_for_its_load_state() -> TestResult {
    on_page("/login.html", |run, _server, page| async move {
        let result = async {
            let images = PageServer::start().await?;
            let html = format!(
                "<!doctype html><title>Written</title><p id=\"set\">written</p><img src=\"{}\">",
                images.url("/slow.png")
            );
            let set_content = |wait_until: &str| {
                run.invoke(
                    "page.setContent",
                    json!({ "call": { "timeoutMs": 30_000 }, "page": brand(&page),
                            "html": html, "waitUntil": wait_until }),
                )
            };
            // (waitUntil, at least, under): the image holds `load` for SLOW.
            for (wait_until, at_least, under) in [
                ("load", SLOW, SLOW * 3),
                ("domcontentloaded", Duration::ZERO, SLOW),
                ("commit", Duration::ZERO, SLOW),
            ] {
                run.clear_calls();
                let started = Instant::now();
                assert_eq!(set_content(wait_until).await?, Value::Null);
                let took = started.elapsed();
                assert!(
                    took >= at_least && took < under,
                    "waitUntil {wait_until} returned after {took:?}"
                );
                let effects: Vec<EnvCall> = run
                    .calls()
                    .into_iter()
                    .filter(|call| !matches!(call, EnvCall::Note(_)))
                    .collect();
                assert_eq!(
                    effects,
                    [EnvCall::Recheck, EnvCall::MarkSent],
                    "the write is one effect, rechecked and marked before it is sent"
                );
                let title = run
                    .invoke("page.title", json!({ "page": brand(&page) }))
                    .await?;
                assert_eq!(title, json!("Written"));
                let text = evaluate(
                    &run,
                    &page,
                    "() => document.getElementById('set').textContent",
                )
                .await?;
                assert_eq!(text, json!("written"), "the new document is the page's");
            }

            // The image holds `load` past the timeout: the content is set, and the op fails with
            // upstream's timeout message.
            run.clear_calls();
            let timed_out = run
                .invoke(
                    "page.setContent",
                    json!({ "call": { "timeoutMs": 300 }, "page": brand(&page), "html": html }),
                )
                .await
                .err()
                .ok_or("a load past the timeout times out")?;
            assert_eq!(
                (timed_out.error_code(), first_line(&timed_out)),
                (
                    Some(ErrorCode::WaitTimeout),
                    "page.setContent: Timeout 300ms exceeded."
                ),
                "{timed_out}"
            );
            assert!(
                run.calls().contains(&EnvCall::MarkSent),
                "the content was sent: {:?}",
                run.calls()
            );

            run.clear_calls();
            let error = set_content("networkidle")
                .await
                .err()
                .ok_or("networkidle never fires for a written document")?;
            assert_eq!(error.error_code(), Some(ErrorCode::Unsupported));
            assert!(
                run.calls().is_empty(),
                "nothing was sent: {:?}",
                run.calls()
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// Polls `expression` in the page's own world until it is true.
async fn until_true(run: &LiveRun, page: &TestPage, expression: &str) -> TestResult {
    let started = Instant::now();
    while run.eval(page, expression).await? != json!(true) {
        if started.elapsed() > Duration::from_secs(10) {
            return Err(format!("never true: {expression}").into());
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}

/// How a test's tab came to be: the agent's own, which core sets up for its file chooser
/// interception, or one the agent did not open, which core leaves as it found it.
#[derive(Clone, Copy, Debug)]
enum Opened {
    ByAgent,
    Adopted,
}

/// Opens [`FRAMED`] on the fixture server in a tab opened as `opened` says, and runs `body`
/// with the ids of its `same` and `cross` frames, once both loaded.
async fn on_framed<F, Fut>(opened: Opened, body: F) -> TestResult
where
    F: FnOnce(LiveRun, TestPage, Value, Value) -> Fut,
    Fut: Future<Output = (LiveRun, TestResult)>,
{
    let run = LiveRun::connect().await?;
    let server = fixtures().await?;
    let framed = FRAMED.replace("{other}", &server.url("localhost", ""));
    let pages = HashMap::from([
        ("/framed.html".to_owned(), Page::html(&framed)),
        ("/field.html".to_owned(), Page::html(FIELD)),
        ("/filled.html".to_owned(), Page::html(FILLED)),
        ("/moved.html".to_owned(), Page::html(MOVED)),
    ]);
    let framed_server = FixtureServer::start(pages).await?;
    let url = framed_server.url("127.0.0.1", "/framed.html");
    let page = match opened {
        Opened::ByAgent => run.new_page().await?,
        Opened::Adopted => run.open_adopted(&url).await?,
    };
    let loaded = async {
        if matches!(opened, Opened::ByAgent) {
            goto(&run, &page, &url).await?;
        }
        until_true(
            &run,
            &page,
            "document.querySelector('iframe[name=same]')?.contentDocument?.readyState === 'complete'",
        )
        .await?;
        let same = frame_named(&run, &page, "same").await?;
        let cross = frame_named(&run, &page, "cross").await?;
        run.invoke(
            "page.waitForLoadState",
            json!({ "call": { "timeoutMs": 10_000 }, "page": brand(&page), "frame": cross, "state": "load" }),
        )
        .await?;
        TestResult::<(Value, Value)>::Ok((same, cross))
    }
    .await;
    let (same, cross) = match loaded {
        Ok(ids) => ids,
        Err(error) => {
            run.close_page(&page).await;
            return Err(error);
        }
    };
    let (run, result) = body(run, page.clone(), same, cross).await;
    run.close_page(&page).await;
    drop(framed_server);
    result
}

/// The id `page.frames` gives the frame named `name`, waiting for it to be listed.
async fn frame_named(run: &LiveRun, page: &TestPage, name: &str) -> TestResult<Value> {
    let started = Instant::now();
    loop {
        let frames = run
            .invoke("page.frames", json!({ "page": brand(page) }))
            .await?;
        let found = frames
            .get("frames")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|frame| frame.get("name").and_then(Value::as_str) == Some(name))
            .and_then(|frame| frame.get("id").cloned());
        if let Some(id) = found {
            return Ok(id);
        }
        if started.elapsed() > Duration::from_secs(10) {
            return Err(format!("no frame named {name} in {frames}").into());
        }
        time::sleep(Duration::from_millis(50)).await;
    }
}

/// `frame.<method>` as the prelude sends it: the page op named `op`, with the frame's id and
/// the frame form's own name for upstream's prefix.
async fn in_frame(
    run: &LiveRun,
    page: &TestPage,
    frame: &Value,
    api: &str,
    op: &str,
    fields: Value,
) -> Result<Value, PlaywrightError> {
    let mut input = json!({ "call": { "api": api, "timeoutMs": 10_000 }, "page": brand(page),
                            "frame": frame });
    if let (Some(input), Value::Object(fields)) = (input.as_object_mut(), fields) {
        input.extend(fields);
    }
    run.invoke(op, input).await
}

/// A same-site child frame's script reaches its parent's document as the page's own scripts
/// do, so `frame.evaluate` and `frame.waitForFunction` run in the child and read the parent's
/// field too; `frame.title` and `frame.content` read the child's own document.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_same_site_child_frames_script_reads_its_parents_field() -> TestResult {
    on_framed(Opened::ByAgent, |run, page, same, _cross| async move {
        let result = async {
            run.eval(
                &page,
                "document.getElementById('pw').value = 'parent-secret-41'; \
                 frames[0].document.querySelector('input').value = 'child-secret-42'",
            )
            .await?;
            run.clear_calls();
            let script = "() => [document.title, parent.document.getElementById('pw').value, \
                          document.querySelector('input').value]";
            let read = in_frame(
                &run,
                &page,
                &same,
                "frame.evaluate",
                "page.evaluate",
                script_input(script),
            )
            .await?;
            assert_eq!(
                read,
                json!({ "a": ["Field", "parent-secret-41", "child-secret-42"], "id": 1 }),
                "the script ran in the child, and read both frames' values"
            );
            assert_eq!(
                run.calls(),
                [
                    EnvCall::Recheck,
                    EnvCall::MarkSent,
                    EnvCall::Recheck,
                    EnvCall::MarkSent
                ],
                "the rejected cross-context call and direct retry each recheck and mark their send"
            );
            let waited = in_frame(
                &run,
                &page,
                &same,
                "frame.waitForFunction",
                "page.waitForFunction",
                script_input("() => parent.document.getElementById('pw').value"),
            )
            .await?;
            assert_eq!(
                waited.get("value"),
                Some(&json!("parent-secret-41")),
                "frame.waitForFunction runs in the child too: {waited}"
            );
            let title =
                in_frame(&run, &page, &same, "frame.title", "page.title", json!({})).await?;
            assert_eq!(
                title,
                json!("Field"),
                "frame.title reads the child's document"
            );
            let content = in_frame(
                &run,
                &page,
                &same,
                "frame.content",
                "page.content",
                json!({}),
            )
            .await?;
            let content = content.as_str().unwrap_or_default();
            assert!(
                content.contains("<title>Field</title>") && !content.contains("Framed"),
                "frame.content serializes the child's document: {content}"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// An out-of-process frame's script runs on that frame's own session, and reads its own field's
/// value, filled by its server.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_an_out_of_process_frames_script_reads_its_own_field() -> TestResult {
    on_framed(Opened::ByAgent, |run, page, _same, cross| async move {
        let result = async {
            run.clear_calls();
            let read = in_frame(
                &run,
                &page,
                &cross,
                "frame.evaluate",
                "page.evaluate",
                script_input("() => [location.hostname, document.querySelector('input').value]"),
            )
            .await?;
            assert_eq!(
                read,
                json!({ "a": ["localhost", "filled-secret-93"], "id": 1 }),
                "the script ran in the out-of-process frame"
            );
            let title =
                in_frame(&run, &page, &cross, "frame.title", "page.title", json!({})).await?;
            assert_eq!(title, json!("Filled"));
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// A child frame that navigates to another site mid-wait moves to another renderer, and so to
/// another CDP session, while it stays the same frame: upstream's `frame.waitForFunction` goes
/// on polling in the new document. Neo's does too, in each way a frame can move: out of
/// process to another site, out of process back to its parent's site, and in process out to
/// another site. The predicate is true only in the new document, where it reads the new
/// document's own field; every poll is one send, rechecked and marked.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_frame_wait_for_function_follows_its_frame_into_another_process() -> TestResult {
    for (name, host) in [
        ("cross", "[::1]"),
        ("cross", "127.0.0.1"),
        ("same", "localhost"),
    ] {
        on_framed(Opened::ByAgent, |run, page, same, cross| async move {
            let result = async {
                let frame = if name == "cross" { &cross } else { &same };
                run.eval(
                    &page,
                    &format!(
                        "setTimeout(() => {{ document.querySelector('iframe[name={name}]').src = \
                         `http://{host}:${{location.port}}/moved.html`; }}, 400)"
                    ),
                )
                .await?;
                run.clear_calls();
                let predicate = format!(
                    "() => location.hostname === '{host}' && document.title === 'Moved' && \
                     [location.hostname, document.querySelector('input').value]"
                );
                let started = Instant::now();
                let waited = in_frame(
                    &run,
                    &page,
                    frame,
                    "frame.waitForFunction",
                    "page.waitForFunction",
                    script_input(&predicate),
                )
                .await
                .map_err(|error| format!("{name} to {host}: {error}"))?;
                assert_eq!(
                    waited.get("value"),
                    Some(&json!({ "a": [host, "moved-secret-57"], "id": 1 })),
                    "{name} to {host}: polled on in the new document: {waited}"
                );
                assert!(
                    started.elapsed() >= Duration::from_millis(300),
                    "{name} to {host}: it waited for the navigation, {:?}",
                    started.elapsed()
                );
                let effects: Vec<EnvCall> = run
                    .calls()
                    .into_iter()
                    .filter(|call| !matches!(call, EnvCall::Note(_)))
                    .collect();
                assert!(
                    !effects.is_empty()
                        && effects
                            .chunks(2)
                            .all(|pair| pair == [EnvCall::Recheck, EnvCall::MarkSent]),
                    "{name} to {host}: every poll rechecked, then marked: {effects:?}"
                );
                Ok(())
            }
            .await;
            (run, result)
        })
        .await?;
    }
    Ok(())
}

/// `frame.waitForURL` on a same-site child frame that navigates to another site mid-wait, and so
/// moves out of the page's process to a renderer of its own, on another session: the wait
/// follows the frame there and matches the navigation that committed in its new renderer, as
/// upstream's does (red before the follow: it waited out its 10 s timeout on the old session).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_frame_wait_for_url_follows_its_frame_out_of_process() -> TestResult {
    on_framed(Opened::ByAgent, |run, page, same, _cross| async move {
        let result = async {
            run.eval(
                &page,
                "setTimeout(() => { document.querySelector('iframe[name=same]').src = \
                 `http://localhost:${location.port}/moved.html`; }, 400), true",
            )
            .await?;
            let started = Instant::now();
            let waited = frame_wait_for_url(&run, &page, &same, "**/moved.html").await?;
            let url = waited
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            assert!(
                url.starts_with("http://localhost:") && url.ends_with("/moved.html"),
                "matched in the frame's new renderer: {waited}"
            );
            assert!(
                started.elapsed() >= Duration::from_millis(300),
                "it waited for the navigation, {:?}",
                started.elapsed()
            );
            // Keep the same Frame through both directions of a renderer handoff. A route
            // cached by the previous wait must not keep naming its retired OOPIF session.
            for (host, path) in [("127.0.0.1", "filled.html"), ("[::1]", "moved.html")] {
                run.eval(
                    &page,
                    &format!(
                        "setTimeout(() => {{ document.querySelector('iframe[name=same]').src = \
                         `http://{host}:${{location.port}}/{path}`; }}, 100), true"
                    ),
                )
                .await?;
                let waited = frame_wait_for_url(&run, &page, &same, &format!("**/{path}")).await?;
                assert!(
                    waited["url"]
                        .as_str()
                        .is_some_and(|url| url.starts_with(&format!("http://{host}:"))),
                    "the same frame followed its current renderer: {waited}"
                );
            }
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_url_wait_rejects_when_its_page_closes() -> TestResult {
    on_page("/login.html", |run, _server, page| async move {
        let result = async {
            // Both native operations retain state while pending; box the wait before joining.
            let waiting = Box::pin(run.invoke(
                "page.waitForURL",
                json!({ "call": { "timeoutMs": 2000 }, "page": brand(&page),
                        "url": { "glob": "**/never" }, "waitUntil": "load" }),
            ));
            let (waited, closed) = tokio::join!(waiting, async {
                time::sleep(Duration::from_millis(50)).await;
                run.invoke("page.close", json!({ "page": brand(&page) }))
                    .await
            });
            closed?;
            let error = waited.err().ok_or("a closed page must reject its wait")?;
            assert_eq!(error.error_code(), Some(ErrorCode::TargetClosed));
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// `frame.setContent` replaces a child frame's document, in process and out of it, and leaves
/// the page's own document alone; it waits for the child's load state as the page form does.
/// On a tab the agent did not open, core enables no `Page` domain on the out-of-process frame's
/// session, whose `documentOpened` the wait needs, so the op's own document watch does
/// (red without it: the out-of-process write timed out there).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_frame_set_content_writes_the_child_frames_document() -> TestResult {
    for opened in [Opened::ByAgent, Opened::Adopted] {
        on_framed(opened, |run, page, same, cross| async move {
            let result = async {
                for (frame, name) in [(&cross, "CrossWritten"), (&same, "SameWritten")] {
                    run.clear_calls();
                    let started = Instant::now();
                    let html =
                        format!("<!doctype html><title>{name}</title><p id=\"w\">{name}</p>");
                    let written = in_frame(
                        &run,
                        &page,
                        frame,
                        "frame.setContent",
                        "page.setContent",
                        json!({ "html": html }),
                    )
                    .await?;
                    assert_eq!(written, Value::Null, "{opened:?} {name}");
                    assert!(
                        started.elapsed() < Duration::from_secs(5),
                        "{opened:?} {name}: the write's load state was seen, after {:?}",
                        started.elapsed()
                    );
                    let effects: Vec<EnvCall> = run
                        .calls()
                        .into_iter()
                        .filter(|call| !matches!(call, EnvCall::Note(_)))
                        .collect();
                    assert_eq!(
                        effects,
                        [EnvCall::Recheck, EnvCall::MarkSent],
                        "{opened:?} {name}"
                    );
                    let title =
                        in_frame(&run, &page, frame, "frame.title", "page.title", json!({}))
                            .await?;
                    assert_eq!(title, json!(name), "{opened:?}");
                }
                assert_eq!(
                    run.eval(&page, "document.title").await?,
                    json!("Framed"),
                    "{opened:?}: the page's own document stays"
                );
                Ok(())
            }
            .await;
            (run, result)
        })
        .await?;
    }
    Ok(())
}

/// `frame.waitForURL(glob)` as the prelude sends it.
async fn frame_wait_for_url(
    run: &LiveRun,
    page: &TestPage,
    frame: &Value,
    glob: &str,
) -> Result<Value, PlaywrightError> {
    in_frame(
        run,
        page,
        frame,
        "frame.waitForURL",
        "page.waitForURL",
        json!({ "url": { "glob": glob } }),
    )
    .await
}

/// `frame.waitForURL` watches the child frame's own document: a URL already there resolves at
/// once, in process and out of it, and a later navigation of the frame matches, its URL
/// returned for the prelude's `frame.url()`, on the agent's own tab and on one it did not open.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_frame_wait_for_url_watches_the_child_frames_navigations() -> TestResult {
    for opened in [Opened::ByAgent, Opened::Adopted] {
        on_framed(opened, |run, page, same, cross| async move {
            let result = async {
                for (frame, path) in [(&same, "/field.html"), (&cross, "/filled.html")] {
                    let waited =
                        frame_wait_for_url(&run, &page, frame, &format!("**{path}")).await?;
                    let url = waited
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    assert!(url.ends_with(path), "{path}: {waited}");
                }
                run.eval(
                    &page,
                    "setTimeout(() => { frames[0].location.href = '/filled.html?moved'; }, 300)",
                )
                .await?;
                let started = Instant::now();
                let waited = frame_wait_for_url(&run, &page, &same, "**/filled.html?moved").await?;
                assert!(
                    waited
                        .get("url")
                        .and_then(Value::as_str)
                        .is_some_and(|url| url.ends_with("/filled.html?moved")),
                    "{waited}"
                );
                assert!(
                    started.elapsed() >= Duration::from_millis(200),
                    "it waited for the navigation, {:?}",
                    started.elapsed()
                );
                Ok(())
            }
            .await;
            (run, result)
        })
        .await?;
    }
    Ok(())
}

/// A frame the page removed is detached: its script forms fail as upstream's do (measured on
/// stock Chromium and BrowserOS with Playwright 1.64.1), with nothing sent, and `frame.title`
/// answers `""`, as upstream's swallows the failure.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_detached_frames_forms_fail_as_upstreams_do() -> TestResult {
    on_framed(Opened::ByAgent, |run, page, same, _cross| async move {
        let result = async {
            run.eval(
                &page,
                "document.querySelector('iframe[name=same]').remove()",
            )
            .await?;
            let forms = [
                ("frame.evaluate", "page.evaluate", script_input("() => 1")),
                (
                    "frame.waitForFunction",
                    "page.waitForFunction",
                    script_input("() => true"),
                ),
                (
                    "frame.setContent",
                    "page.setContent",
                    json!({ "html": "<p>x</p>" }),
                ),
                ("frame.content", "page.content", json!({})),
            ];
            for (api, op, fields) in forms {
                run.clear_calls();
                let error = in_frame(&run, &page, &same, api, op, fields)
                    .await
                    .err()
                    .ok_or(format!("{api} on a detached frame fails"))?;
                assert_eq!(
                    first_line(&error),
                    format!("{api}: Frame was detached"),
                    "{error}"
                );
                assert!(
                    !run.calls().contains(&EnvCall::MarkSent),
                    "{api}: nothing was sent: {:?}",
                    run.calls()
                );
            }
            let title =
                in_frame(&run, &page, &same, "frame.title", "page.title", json!({})).await?;
            assert_eq!(title, json!(""), "upstream's title of a detached frame");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// A function script as the prelude sends it, its argument `undefined`.
fn script_input(function: &str) -> Value {
    script(function, true, &undefined())
}

/// `page.waitForURL(url)` as the prelude sends it, with `timeout_ms`.
async fn wait_for_url(
    run: &LiveRun,
    page: &TestPage,
    url: Value,
    timeout_ms: u64,
) -> Result<Value, PlaywrightError> {
    run.invoke(
        "page.waitForURL",
        json!({ "call": { "timeoutMs": timeout_ms }, "page": brand(page), "url": url }),
    )
    .await
}

fn notes(run: &LiveRun) -> Vec<String> {
    run.calls()
        .into_iter()
        .filter_map(|call| match call {
            EnvCall::Note(line) => Some(line),
            EnvCall::Recheck | EnvCall::MarkSent => None,
        })
        .collect()
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_wait_for_url_matches_a_glob_a_regexp_and_a_url_already_there() -> TestResult {
    on_page("/login.html", |run, server, page| async move {
        let result = async {
            run.clear_calls();
            let here = wait_for_url(&run, &page, json!({ "glob": "**/login.html" }), 5000).await?;
            assert_eq!(
                here,
                json!({ "url": server.url("127.0.0.1", "/login.html") })
            );
            assert!(
                notes(&run).is_empty(),
                "a URL already there only waits for its load state: {:?}",
                notes(&run)
            );

            // Through a step that moves on at once, to a URL with a fragment.
            run.eval(
                &page,
                "setTimeout(() => { location.href = '/step.html'; }, 300)",
            )
            .await?;
            let done =
                wait_for_url(&run, &page, json!({ "glob": "**/done.html#end" }), 10_000).await?;
            let done_url = server.url("127.0.0.1", "/done.html#end");
            assert_eq!(done, json!({ "url": done_url }));
            assert_eq!(
                notes(&run),
                [
                    "waiting for navigation to \"**/done.html#end\" until \"load\"".to_owned(),
                    format!(
                        "  navigated to \"{}\"",
                        server.url("127.0.0.1", "/step.html")
                    ),
                    format!("  navigated to \"{done_url}\""),
                ],
                "upstream's log: every navigation until the match"
            );

            run.clear_calls();
            run.eval(
                &page,
                "setTimeout(() => { history.pushState({}, '', '/pushed-STEP-7'); }, 300)",
            )
            .await?;
            let pushed = wait_for_url(
                &run,
                &page,
                json!({ "regexSource": "step-(\\d)$", "regexFlags": "i" }),
                10_000,
            )
            .await?;
            assert_eq!(
                pushed,
                json!({ "url": server.url("127.0.0.1", "/pushed-STEP-7") }),
                "a RegExp with JavaScript's flags matches a same-document navigation"
            );
            assert_eq!(
                notes(&run).first().map(String::as_str),
                Some("waiting for navigation until \"load\""),
                "upstream names only a string pattern in its first line"
            );

            let error = wait_for_url(&run, &page, json!({ "glob": "**/never" }), 500)
                .await
                .err()
                .ok_or("a URL that never comes times out")?;
            assert_eq!(error.error_code(), Some(ErrorCode::WaitTimeout));
            assert!(
                error
                    .message()
                    .starts_with("page.waitForURL: Timeout 500ms exceeded."),
                "{error}"
            );
            assert!(
                error
                    .message()
                    .contains("waiting for navigation to \"**/never\" until \"load\""),
                "{error}"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}
