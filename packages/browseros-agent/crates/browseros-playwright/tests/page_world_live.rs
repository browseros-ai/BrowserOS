//! The page ops that need a sealed world or the main world, on a real browser, through
//! `Playwright::invoke`: `evaluate` and `waitForFunction` behind the sweep, `title` and
//! `content` read through the seal, `setContent` with its load states, and `waitForURL`. That `setContent`
//! retires the document's seq is checked in `src/world/live_tests.rs`, where seqs are visible.
//!
//! Needs a throwaway BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005), for example
//! `PORT_BASE=9461 spikes/eval/browser.sh <profile> &`, then
//! `BROWSEROS_CDP_PORT=9461 cargo test -p browseros-playwright --test page_world_live -- --ignored`.
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

use browseros_playwright::{Answer, ErrorCode, PlaywrightError, Refusal, TimedOut};
use browseros_trace::ErrorOwner;
use fixture_server::{FixtureServer, Page};
use live_run::{EnvCall, Intercept, LiveRun, TestPage, TestResult, brand};
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

async fn fixtures() -> TestResult<FixtureServer> {
    let pages = HashMap::from([
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

/// A script as the prelude sends it, `arg` already in Playwright's in-page format.
fn script(expression: &str, is_function: bool, arg: &Value) -> Value {
    json!({ "run": { "expression": expression, "isFunction": is_function, "arg": arg.to_string() } })
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
        json!({ "page": brand(page), "script": script(function, true, &undefined()) }),
    )
    .await
}

/// `page.waitForFunction(fn)` as the prelude sends it, with `extra` fields (`polling`).
async fn wait_for_function(
    run: &LiveRun,
    page: &TestPage,
    timeout_ms: u64,
    function: &str,
    extra: Value,
) -> Result<Value, PlaywrightError> {
    let mut input = json!({ "call": { "timeoutMs": timeout_ms }, "page": brand(page),
                            "script": script(function, true, &undefined()) });
    if let (Some(input), Value::Object(extra)) = (input.as_object_mut(), extra) {
        input.extend(extra);
    }
    run.invoke("page.waitForFunction", input).await
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
async fn tests_that_evaluate_masks_a_typed_password_the_page_still_holds_raw() -> TestResult {
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
                json!({ "a": ["[private]", "you typed [private]", "[private]", "[private]"], "id": 1 }),
                "the main frame's and the same-process frame's values are masked, a code as a \
                 number too"
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
                json!("[private]"),
                "a value the script itself put in a field is caught by the second sweep"
            );

            let raw = run
                .eval(&page, "document.getElementById('pw').value")
                .await?;
            assert_eq!(raw, json!("hunter2-typed"), "the page keeps its own value");
            let swept = run.swept();
            for secret in ["hunter2-typed", "482913", "late-secret-77"] {
                assert!(
                    swept.iter().any(|value| value == secret),
                    "the host is told every swept value, to register it: {secret}"
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
async fn tests_that_evaluate_returns_page_values_no_field_holds_untouched() -> TestResult {
    on_page("/login.html", |run, _server, page| async move {
        let result = async {
            type_secrets(&run, &page).await?;
            let note = evaluate(&run, &page, "() => document.getElementById('note').textContent").await?;
            assert_eq!(note, json!("nothing secret here"));
            let expression = run
                .invoke(
                    "page.evaluate",
                    json!({ "page": brand(&page), "script": script("document.title", false, &undefined()) }),
                )
                .await?;
            assert_eq!(expression, json!("Login"), "a string is an expression");
            let x = json!({ "o": [{ "k": "x", "v": 21 }], "id": 1 });
            let with_arg = run
                .invoke(
                    "page.evaluate",
                    json!({ "page": brand(&page), "script": script("({ x }) => x * 2", true, &x) }),
                )
                .await?;
            assert_eq!(with_arg, json!(42));
            let null = json!({ "v": "null" });
            let null_arg = run
                .invoke(
                    "page.evaluate",
                    json!({ "page": brand(&page), "script": script("(a) => a === null", true, &null) }),
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
                    "page.evaluate: Error: saw [private]",
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
                json!({ "value": "[private]", "preview": "[private]" }),
                "the truthy result and its preview leave the page masked"
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
                json!({ "polling": 20 }),
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
                        json!({ "call": { "timeoutMs": 5000 }, "page": brand(&page),
                                "script": self::script(script, true, &undefined()) }),
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
async fn tests_that_title_and_content_read_through_the_seal_and_mask_echoes() -> TestResult {
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
            // How often the host was told `value`, to register it as a secret.
            let reported = |value: &str| run.swept().iter().filter(|swept| *swept == value).count();
            let title = document_text(&run, &page, "page.title").await?;
            assert_eq!(
                title, "Account of [private]",
                "a title can echo a typed value"
            );
            assert_eq!(
                (reported("typed-secret-2"), reported("attr-secret-1")),
                (1, 1),
                "title() reports the values it swept"
            );

            let content = document_text(&run, &page, "page.content").await?;
            assert_eq!(
                (reported("typed-secret-2"), reported("attr-secret-1")),
                (2, 2),
                "content() reports the values it swept"
            );
            assert!(
                content.starts_with("<!DOCTYPE html><html><head>"),
                "{content}"
            );
            assert!(
                content.contains(r#"<input id="pw" type="password" value="••••">"#)
                    && content.contains(r#"<input id="old" type="password" value="••••">"#),
                "the seal masks every value attribute it serializes: {content}"
            );
            assert!(
                content.contains("you typed [private]"),
                "the sweep masks the typed value the page echoed in its text: {content}"
            );
            for secret in ["typed-secret-2", "attr-secret-1"] {
                assert!(!content.contains(secret), "{secret} leaked: {content}");
            }
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
async fn tests_that_content_masks_an_echo_the_serializer_escaped() -> TestResult {
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
                content
                    .contains(r#"<p id="echo" data-echo="[private]">you typed [private]</p>"#),
                "the escaped echo in an attribute and in text is masked: {content}"
            );
            assert!(!content.contains("secret"), "the value leaked: {content}");

            let echoed = evaluate(
                &run,
                &page,
                "() => document.getElementById('echo').outerHTML",
            )
            .await?;
            assert_eq!(
                echoed,
                json!(r#"<p id="echo" data-echo="[private]">you typed [private]</p>"#),
                "a script's serialized markup is masked the same way"
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
                assert_eq!(set_content(wait_until).await?, json!({}));
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
            EnvCall::Recheck | EnvCall::MarkSent | EnvCall::MarkSensitiveTarget => None,
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
