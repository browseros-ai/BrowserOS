//! The echo of a password in a closed shadow root, read through `Playwright::invoke` on a real
//! browser: sdk-privacy's leak test, scenario `echoes` (its findings, item 1). The page's own
//! script copies the field's value into the top document (`tests/data/closed_roots/echo.html`),
//! and every read of the top document must return that copy masked.
//!
//! The seal finds fields from the document down through open shadow roots, and its journal
//! listens on the document, so a closed root hides its field from both until Neo hands the
//! root to the seal's `watch`. The field gets its value five ways, one test each:
//! - typed over CDP, as a user types, before Neo's first read of the page;
//! - set by the page with its own `input` events after Neo touched the page, which the journal
//!   must hear as well;
//! - in a closed root the page attaches, after Neo touched the page, to a div that was there
//!   from the start;
//! - in a closed root under 70 levels of elements;
//! - typed over CDP into a closed root the page attached after Neo touched the page, and whose
//!   host it removed right after, before any sweep could scan for it.
//!
//! The last test prints what reads cost; run it with `--nocapture`.
//!
//! Needs a throwaway BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005), for example
//! `PORT_BASE=9821 spikes/eval/browser.sh <profile> &`, then
//! `BROWSEROS_CDP_PORT=9821 cargo test -p browseros-playwright --test closed_roots_live -- --ignored`.
//! Every test opens its own tab and closes it, one test at a time.

#[path = "support/fixture_server.rs"]
#[expect(dead_code, reason = "no nested fixture or scripts here")]
mod fixture_server;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "the capture refusal and the click serve pages_live, which includes this too"
)]
mod live_run;

use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant};

use browseros_playwright::PlaywrightError;
use fixture_server::{FixtureServer, Page};
use live_run::{LiveRun, TestPage, TestResult, brand};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::time;

/// One test at a time: the cost test times its ops, and each test connects its own
/// `Playwright`, whose frame registry attaches to every frame target in the browser.
static ALONE: Semaphore = Semaphore::const_new(1);

/// The fixture's path on the server.
const ECHO: &str = "/closed_roots/echo.html";

/// The page's copy of the field's value, as every read must return it.
const COPY: &str = "Typed [private]";

fn pages() -> HashMap<String, Page> {
    let page = Page::html(include_str!("data/closed_roots/echo.html"))
        .with_header("cache-control", "no-store");
    HashMap::from([(ECHO.to_owned(), page)])
}

/// Opens a tab on `url`, runs `body` on it, and closes the tab whatever happened.
async fn on_page<F, Fut>(run: LiveRun, url: &str, body: F) -> TestResult
where
    F: FnOnce(LiveRun, TestPage) -> Fut,
    Fut: Future<Output = (LiveRun, TestResult)>,
{
    let page = run.new_page().await?;
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

/// A locator for one CSS selector in the main frame, as the prelude sends `page.locator(..)`.
fn at(page: &TestPage, selector: &str) -> Value {
    json!({
        "$neo": "locator",
        "page": brand(page),
        "chunks": [{ "parts": [{
            "name": "css",
            "body": [{ "simples": [{ "selector": { "css": selector, "functions": [] }, "combinator": "" }] }],
            "source": selector,
        }] }],
        "display": format!("locator('{selector}')"),
    })
}

/// What one read must return.
#[derive(Debug)]
enum Expect {
    /// Exactly this.
    Is(Value),
    /// A result with a string, at any depth, that holds this: a snapshot is JSON nodes.
    Shows(&'static str),
}

/// Every string in `value`, at any depth: the page text a result hands the agent.
fn strings(value: &Value) -> Vec<&str> {
    match value {
        Value::String(text) => vec![text.as_str()],
        Value::Array(items) => items.iter().flat_map(strings).collect(),
        Value::Object(fields) => fields.values().flat_map(strings).collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) => Vec::new(),
    }
}

/// The start of a long result, enough to show what went wrong.
fn shortened(shown: &str) -> &str {
    let end = shown
        .char_indices()
        .nth(400)
        .map_or(shown.len(), |(end, _)| end);
    shown.get(..end).unwrap_or(shown)
}

/// One read of the top document: the API the script called, the op it became, and its input.
struct Read {
    api: &'static str,
    op: &'static str,
    input: Value,
    expect: Expect,
}

impl Read {
    /// Why `got` is not what the read must return, or holds `secret` raw; `None` when it is
    /// right. An error is wrong too: a read refused for a frame Neo could not sweep would hide
    /// the leak this test is for.
    fn mismatch(&self, got: &Result<Value, PlaywrightError>, secret: &str) -> Option<String> {
        let shown = match got {
            Ok(value) => value.to_string(),
            Err(error) => format!("error {:?}: {error}", error.error_code()),
        };
        if shown.contains(secret) {
            return Some(format!("holds {secret} raw: {}", shortened(&shown)));
        }
        let met = match (&self.expect, got) {
            (Expect::Is(want), Ok(value)) => value == want,
            (Expect::Shows(part), Ok(value)) => {
                strings(value).iter().any(|text| text.contains(part))
            }
            _ => false,
        };
        (!met).then(|| format!("expected {:?}, got {}", self.expect, shortened(&shown)))
    }
}

/// The 10 read channels the leak test found raw, and `page.evaluate` returning the copy.
fn reads(page: &TestPage) -> Vec<Read> {
    let element = |api: &'static str, expect| Read {
        api,
        op: api,
        input: json!({ "target": at(page, "#echo"), "strict": "strict" }),
        expect,
    };
    let every = |api: &'static str| Read {
        api,
        op: api,
        input: json!({ "target": at(page, ".echo") }),
        expect: Expect::Is(json!([COPY])),
    };
    let snapshot = |mode: &str| Read {
        api: "page.ariaSnapshot",
        op: "page.ariaSnapshot",
        input: json!({ "page": brand(page), "mode": mode }),
        expect: Expect::Shows(COPY),
    };
    vec![
        element("locator.textContent", Expect::Is(json!(COPY))),
        element("locator.innerText", Expect::Is(json!(COPY))),
        element("locator.innerHTML", Expect::Is(json!(COPY))),
        Read {
            api: "locator.getAttribute",
            op: "locator.getAttribute",
            input: json!({ "target": at(page, "#echo"), "strict": "strict", "name": "data-echo" }),
            expect: Expect::Is(json!("[private]")),
        },
        every("locator.allTextContents"),
        every("locator.allInnerTexts"),
        Read {
            api: "locator.ariaSnapshot",
            op: "locator.ariaSnapshot",
            input: json!({ "target": at(page, "body"), "strict": "strict", "mode": "default" }),
            expect: Expect::Shows(COPY),
        },
        Read {
            api: "page.content",
            op: "page.content",
            input: json!({ "page": brand(page) }),
            expect: Expect::Shows(r#"data-echo="[private]">Typed [private]</p>"#),
        },
        snapshot("default"),
        snapshot("ai"),
        Read {
            api: "page.evaluate",
            op: "page.evaluate",
            input: json!({
                "page": brand(page),
                "script": { "run": {
                    "expression": "() => document.getElementById('echo').textContent",
                    "isFunction": true,
                    "arg": r#"{"v":"undefined"}"#,
                } },
            }),
            expect: Expect::Is(json!(COPY)),
        },
    ]
}

/// Runs `read` as the prelude sends it, with a short timeout.
async fn invoke(run: &LiveRun, read: &Read) -> Result<Value, PlaywrightError> {
    let mut input = read.input.clone();
    if let Some(fields) = input.as_object_mut() {
        fields.insert(
            "call".to_owned(),
            json!({ "api": read.api, "timeoutMs": 5_000 }),
        );
    }
    run.invoke(read.op, input).await
}

/// Neo's first read of the page, which installs its sealed world and sweeps it.
async fn touch(run: &LiveRun, page: &TestPage) -> TestResult {
    run.invoke("page.title", json!({ "page": brand(page) }))
        .await?;
    Ok(())
}

/// Neo's first read of the page, one that holds no page text: it installs the sealed world and
/// sweeps nothing, so only the install can have handed the seal the closed root.
async fn touch_without_a_sweep(run: &LiveRun, page: &TestPage) -> TestResult {
    run.invoke("locator.count", json!({ "target": at(page, "#echo") }))
        .await?;
    Ok(())
}

/// Whether the seal's journal reports `secret` within 2 s.
async fn journaled(run: &LiveRun, secret: &str) -> bool {
    within_2s(|| run.journaled().iter().any(|value| value == secret)).await
}

/// Whether a sweep hands the host `secret` within 2 s.
async fn told(run: &LiveRun, secret: &str) -> bool {
    within_2s(|| run.swept().iter().any(|value| value == secret)).await
}

async fn within_2s(met: impl Fn() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        if met() {
            return true;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Waits until the page's own copy holds `secret` raw, so a masked read proves Neo masked it.
async fn copied(run: &LiveRun, page: &TestPage, secret: &str) -> TestResult {
    let raw = "document.getElementById('echo').dataset.echo";
    let started = Instant::now();
    while run.eval(page, raw).await? != json!(secret) {
        if started.elapsed() > Duration::from_secs(5) {
            return Err(format!("the page never copied {secret} into the top document").into());
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}

/// Runs every read of the page whose copy holds `secret`, and says how each wrong one was.
async fn wrong_reads(run: &LiveRun, page: &TestPage, secret: &str) -> Vec<String> {
    let mut wrong = Vec::new();
    for read in reads(page) {
        let got = invoke(run, &read).await;
        if let Some(mismatch) = read.mismatch(&got, secret) {
            wrong.push(format!("{} ({}): {mismatch}", read.api, read.op));
        }
    }
    wrong
}

/// Fails with every read that did not return the copy masked.
fn assert_masked(wrong: &[String], secret: &str, how: &str) {
    assert!(
        wrong.is_empty(),
        "{how}, and the page copied {secret} into the top document, so every read returns the \
         copy masked; these did not:\n{}",
        wrong.join("\n")
    );
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_reads_mask_a_password_typed_into_a_closed_root_before_neo_touched_the_page()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(pages()).await?;
    let url = server.url("127.0.0.1", ECHO);
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            let secret = "typed-cl0sed-4127";
            assert_eq!(
                run.eval(&page, "fixture.focus()").await?,
                json!(true),
                "the closed root's field takes the focus, so typed text goes there"
            );
            let session = run.browser().pages.get_session(page.id.clone()).await?;
            session
                .session
                .send_value("Input.insertText", json!({ "text": secret }))
                .await?;
            copied(&run, &page, secret).await?;
            let wrong = wrong_reads(&run, &page, secret).await;
            assert_masked(
                &wrong,
                secret,
                "a user typed into a closed root's password field",
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
async fn tests_that_reads_and_the_journal_catch_a_closed_root_value_the_page_set_after_neo_s_touch()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(pages()).await?;
    let url = server.url("127.0.0.1", ECHO);
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            let secret = "planted-cl0sed-5238";
            touch_without_a_sweep(&run, &page).await?;
            run.eval(&page, &format!("fixture.plant({secret:?})"))
                .await?;
            copied(&run, &page, secret).await?;
            let mut wrong = Vec::new();
            // Before any read sweeps: the journal hears the page's events only if the root was
            // watched when they fired, and a later sweep cannot fire them again.
            if !journaled(&run, secret).await {
                wrong.push(format!(
                    "the journal: never reported {secret}, so the host cannot mask it once the \
                     field is cleared"
                ));
            }
            wrong.extend(wrong_reads(&run, &page, secret).await);
            assert_masked(
                &wrong,
                secret,
                "the page set a closed root's password with its own input events after Neo's \
                 first read",
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
async fn tests_that_reads_mask_a_value_in_a_closed_root_attached_after_neo_s_touch() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(pages()).await?;
    let url = server.url("127.0.0.1", ECHO);
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            let secret = "late-cl0sed-6349";
            touch(&run, &page).await?;
            run.eval(&page, &format!("fixture.late({secret:?})"))
                .await?;
            copied(&run, &page, secret).await?;
            let wrong = wrong_reads(&run, &page, secret).await;
            assert_masked(
                &wrong,
                secret,
                "after Neo's first read, the page attached a closed root with a password field \
                 to a div that was there from the start",
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
async fn tests_that_reads_mask_a_value_in_a_closed_root_70_levels_down() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(pages()).await?;
    let url = server.url("127.0.0.1", ECHO);
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            let secret = "deep-cl0sed-7450";
            touch(&run, &page).await?;
            run.eval(&page, &format!("fixture.deep({secret:?})"))
                .await?;
            copied(&run, &page, secret).await?;
            let wrong = wrong_reads(&run, &page, secret).await;
            assert_masked(
                &wrong,
                secret,
                "the page put a closed root with a password field under 70 levels of elements",
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// The page removes a closed root's host right after a user typed into it, so no scan before
/// a sweep finds it: inside the field's own input listener, and on the next task. The seal hears
/// the typed `input` reach the document at the host, and holds the host for Neo, which watches
/// its root and sweeps it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_reads_mask_a_password_typed_into_a_closed_root_the_page_then_removed()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(pages()).await?;
    let url = server.url("127.0.0.1", ECHO);
    for (delay, secret) in [(-1, "removed-cl0sed-8561"), (0, "removed-cl0sed-9672")] {
        on_page(LiveRun::connect().await?, &url, |run, page| async move {
            let result = async {
                touch(&run, &page).await?;
                assert_eq!(
                    run.eval(&page, &format!("fixture.typedThenRemoved({delay})"))
                        .await?,
                    json!(true),
                    "the new closed root's field takes the focus, so typed text goes there"
                );
                let session = run.browser().pages.get_session(page.id.clone()).await?;
                session
                    .session
                    .send_value("Input.insertText", json!({ "text": secret }))
                    .await?;
                copied(&run, &page, secret).await?;
                let started = Instant::now();
                while run.eval(&page, "fixture.removedAll()").await? != json!(true) {
                    if started.elapsed() > Duration::from_secs(5) {
                        return Err("the page never removed the typed root's host".into());
                    }
                    time::sleep(Duration::from_millis(10)).await;
                }
                let mut wrong = Vec::new();
                // Before any read: only the seal's report can have told the host.
                if !told(&run, secret).await {
                    wrong.push(format!(
                        "the host: never told {secret}, so its own mask misses it at every exit"
                    ));
                }
                wrong.extend(wrong_reads(&run, &page, secret).await);
                assert_masked(
                    &wrong,
                    secret,
                    &format!(
                        "a user typed into a closed root the page attached after Neo's first read \
                         and removed {}",
                        if delay < 0 {
                            "inside the field's input listener".to_owned()
                        } else {
                            format!("{delay} ms after the input event")
                        }
                    ),
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

/// Forces a full garbage collection in the page's renderer, after the crate's spawned releases
/// of its handles had their turn.
async fn collect_garbage(run: &LiveRun, page: &TestPage) -> TestResult {
    time::sleep(Duration::from_millis(100)).await;
    let session = run.browser().pages.get_session(page.id.clone()).await?;
    session
        .session
        .send_value("HeapProfiler.collectGarbage", json!({}))
        .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_watched_closed_root_the_page_removed_and_forgot_is_collected() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(pages()).await?;
    let url = server.url("127.0.0.1", ECHO);
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            let secret = "dropped-cl0sed-1783";
            touch(&run, &page).await?;
            run.eval(&page, &format!("fixture.attachDropped({secret:?})"))
                .await?;
            touch(&run, &page).await?;
            assert!(
                told(&run, secret).await,
                "the sweep found the root, so its seal watches it"
            );
            run.eval(&page, "fixture.forgetDropped()").await?;
            touch(&run, &page).await?;
            let mut alive = true;
            for _ in 0..3 {
                collect_garbage(&run, &page).await?;
                alive = run.eval(&page, "fixture.droppedAlive()").await? == json!(true);
                if !alive {
                    break;
                }
            }
            assert!(
                !alive,
                "a removed component's closed root dies with it: neither the seal nor Neo may \
                 keep it, or every removed component tree stays alive and swept until navigation"
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
async fn tests_that_a_closed_root_the_page_removed_and_attached_again_is_still_watched()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(pages()).await?;
    let url = server.url("127.0.0.1", ECHO);
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            let secret = "kept-cl0sed-2894";
            touch(&run, &page).await?;
            run.eval(&page, "fixture.attachKept()").await?;
            touch(&run, &page).await?;
            run.eval(&page, "fixture.detachKept()").await?;
            // A sweep while the root is out of the document, and a collection.
            touch(&run, &page).await?;
            collect_garbage(&run, &page).await?;
            run.eval(&page, &format!("fixture.reattachKept({secret:?})"))
                .await?;
            copied(&run, &page, secret).await?;
            let mut wrong = Vec::new();
            if !journaled(&run, secret).await {
                wrong.push(format!(
                    "the journal: never reported {secret}, so the seal stopped watching the root \
                     while the page kept it"
                ));
            }
            wrong.extend(wrong_reads(&run, &page, secret).await);
            assert_masked(
                &wrong,
                secret,
                "the page removed a watched closed root, kept it, attached it again and set its \
                 password with its own input events",
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// What `calls` timed runs of one op took.
struct Cost {
    median: Duration,
    slowest: Duration,
}

/// The cost of `calls` runs of `op`, after three to warm the worlds up.
async fn cost<F, Fut>(calls: usize, op: F) -> TestResult<Cost>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Value, PlaywrightError>>,
{
    for _warm in 0..3 {
        op().await?;
    }
    let mut took = Vec::with_capacity(calls);
    for _call in 0..calls {
        let started = Instant::now();
        op().await?;
        took.push(started.elapsed());
    }
    took.sort();
    Ok(Cost {
        median: took.get(took.len() / 2).copied().unwrap_or_default(),
        slowest: took.last().copied().unwrap_or_default(),
    })
}

/// One measured line, shown with `--nocapture`.
fn report(line: &str) {
    #[expect(
        clippy::print_stdout,
        reason = "the cost lines are what a reviewer reruns this test for"
    )]
    {
        println!("{line}");
    }
}

/// Prints what reads cost on the one-frame fixture, which holds a closed root, at three sizes.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn measures_the_cost_of_a_read_on_a_page_with_a_closed_root() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(pages()).await?;
    for rows in [0, 1_000, 5_000] {
        let url = server.url("127.0.0.1", &format!("{ECHO}?rows={rows}"));
        on_page(LiveRun::connect().await?, &url, |run, page| async move {
            let result = async {
                let elements = run
                    .eval(&page, "document.querySelectorAll('*').length")
                    .await?;
                for read in reads(&page) {
                    if !matches!(
                        read.api,
                        "locator.textContent" | "page.content" | "page.evaluate"
                    ) {
                        continue;
                    }
                    let Cost { median, slowest } = cost(40, || invoke(&run, &read)).await?;
                    report(&format!(
                        "{}, one frame, {elements} elements: median {median:?}, slowest \
                         {slowest:?}",
                        read.api
                    ));
                }
                Ok(())
            }
            .await;
            (run, result)
        })
        .await?;
    }
    Ok(())
}
