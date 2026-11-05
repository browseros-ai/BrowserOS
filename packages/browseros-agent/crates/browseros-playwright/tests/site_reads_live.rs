//! Sealed-world reads masked with the read frame's whole site, on a real browser, through
//! `Playwright::invoke`: the frames a main-world script reaches (decisions.md, "Main-world sweep
//! scope") are the frames the page's own script copies from. The page's script copies a
//! password typed in a frame of its site into the top document: its heading, an attribute, a
//! plain input and its title. Every read of the top document must then mask it, though the top
//! frame's own sweep never sees that field.
//!
//! The frame is embedded three ways (`tests/data/site_copy/top.html`):
//! - in process: a same-origin child on the top page's own process and CDP session, on a page
//!   with no out-of-process frame at all;
//! - out of process: under a cross-site bridge, so on a CDP session of its own;
//! - behind a shadow root: the same, with the bridge's iframe in a closed shadow root, which
//!   the top page's `window.length` does not count.
//!
//! A fourth page copies into a child frame instead (`tests/data/site_copy/pair.html`): a frame
//! of another site copies its hidden same-site sibling's password into its own heading, which
//! an ai-mode snapshot stitches in.
//!
//! The last test prints what reads cost; run it with `--nocapture`.
//!
//! Needs a throwaway BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005), for example
//! `PORT_BASE=9671 spikes/eval/browser.sh <profile> &`, then
//! `BROWSEROS_CDP_PORT=9671 cargo test -p browseros-playwright --test site_reads_live -- --ignored`.
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

use browseros_playwright::{ErrorCode, PlaywrightError};
use fixture_server::{FixtureServer, Page};
use live_run::{LiveRun, TestPage, TestResult, brand};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::time;

/// One test at a time: the cost test times its ops, and each test connects its own
/// `Playwright`, whose frame registry attaches to every frame target in the browser.
static ALONE: Semaphore = Semaphore::const_new(1);

/// The fixture, under `/site_copy/`.
fn site_copy_pages() -> HashMap<String, Page> {
    [
        ("top", include_str!("data/site_copy/top.html")),
        ("bridge", include_str!("data/site_copy/bridge.html")),
        ("field", include_str!("data/site_copy/field.html")),
        ("pair", include_str!("data/site_copy/pair.html")),
        ("sibling", include_str!("data/site_copy/sibling.html")),
    ]
    .into_iter()
    .map(|(name, body)| {
        let page = Page::html(body).with_header("cache-control", "no-store");
        (format!("/site_copy/{name}.html"), page)
    })
    .collect()
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

/// Types `secret` into the same-site frame's password field, as a user would, and runs the
/// page's own script that copies it into the top document.
async fn type_and_copy(run: &LiveRun, page: &TestPage, secret: &str) -> TestResult {
    let ready = "(() => { try { return !!field(); } catch (_) { return false; } })()";
    let started = Instant::now();
    while run.eval(page, ready).await? != json!(true) {
        if started.elapsed() > Duration::from_secs(10) {
            return Err("the same-site field frame never loaded".into());
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    run.eval(page, &format!("field().value = {secret:?}; copyField()"))
        .await?;
    Ok(())
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

/// One read of the top document and its input; `call.api` names the method the script ran.
struct Read {
    api: &'static str,
    op: &'static str,
    input: Value,
}

/// What one read must hand the agent once the page copied the secret into the top document.
/// Every expected string is the fixture's copy (`copyField`) with the secret masked: `[private]`
/// where Neo's site mask replaced it, `••••` where the seal masks a password field's own value.
#[derive(Debug)]
enum Expect {
    /// The read succeeds with exactly this result.
    Is(Value),
    /// The read succeeds, and each of these is part of one of the result's strings.
    Shows(Vec<&'static str>),
    /// The read fails with `code`, and its message previews each of these.
    Fails {
        code: ErrorCode,
        previews: Vec<&'static str>,
    },
}

impl Expect {
    /// Why `got` is not what this expects, or holds `secret` raw; `None` when it is right.
    fn mismatch(&self, got: &Result<Value, PlaywrightError>, secret: &str) -> Option<String> {
        let shown = match got {
            Ok(value) => value.to_string(),
            Err(error) => format!("error {:?}: {error}", error.error_code()),
        };
        if shown.contains(secret) {
            return Some(format!("holds {secret} raw: {shown}"));
        }
        let met = match (self, got) {
            (Self::Is(want), Ok(value)) => value == want,
            (Self::Shows(parts), Ok(value)) => {
                let texts = strings(value);
                parts
                    .iter()
                    .all(|part| texts.iter().any(|text| text.contains(part)))
            }
            (Self::Fails { code, previews }, Err(error)) => {
                let message = error.to_string();
                error.error_code() == Some(*code)
                    && previews.iter().all(|preview| message.contains(preview))
            }
            _ => false,
        };
        (!met).then(|| format!("expected {self:?}, got {shown}"))
    }
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

/// Whether an ai-mode snapshot of the top page reaches the field's frame and stitches it in. A
/// frame in a closed shadow root prints no iframe node to follow.
#[derive(Clone, Copy)]
enum FieldFrame {
    Stitched,
    Unreached,
}

/// One read of the top document, and what it must return.
struct Case {
    read: Read,
    expect: Expect,
}

/// Every sealed-world read of the top document whose output holds page text: the document
/// reads, each element read string, a handle's preview, a strict violation's previews, and the
/// snapshots, each with what it must return.
fn reads(page: &TestPage, field: FieldFrame) -> Vec<Case> {
    let case = |read, expect| Case { read, expect };
    let document = |api: &'static str| Read {
        api,
        op: api,
        input: json!({ "page": brand(page) }),
    };
    let element = |api: &'static str, selector: &str| Read {
        api,
        op: api,
        input: json!({ "target": at(page, selector), "strict": "strict" }),
    };
    let every = |api: &'static str| Read {
        api,
        op: api,
        input: json!({ "target": at(page, ".who") }),
    };
    let snapshot = |mode: &str| Read {
        api: "page.ariaSnapshot",
        op: "page.ariaSnapshot",
        input: json!({ "page": brand(page), "mode": mode }),
    };
    let heading = || Expect::Is(json!("Signed in with [private]"));
    let both = || Expect::Is(json!(["Signed in with [private]", "[private]"]));
    let ai_shows = match field {
        FieldFrame::Stitched => vec!["Signed in with [private]", "••••"],
        FieldFrame::Unreached => vec!["Signed in with [private]"],
    };
    vec![
        case(document("page.title"), Expect::Is(json!("[private]"))),
        case(
            document("page.content"),
            Expect::Shows(vec![
                "<title>[private]</title>",
                r#"<h1 class="who" data-who="[private]">Signed in with [private]</h1>"#,
                r#"<p class="who">[private]</p>"#,
            ]),
        ),
        case(element("locator.textContent", "h1"), heading()),
        case(element("locator.innerText", "h1"), heading()),
        case(element("locator.innerHTML", "h1"), heading()),
        case(
            Read {
                api: "locator.getAttribute",
                op: "locator.getAttribute",
                input: json!({ "target": at(page, "h1"), "strict": "strict", "name": "data-who" }),
            },
            Expect::Is(json!("[private]")),
        ),
        case(
            element("locator.inputValue", "#echo"),
            Expect::Is(json!("[private]")),
        ),
        case(every("locator.allTextContents"), both()),
        case(every("locator.allInnerTexts"), both()),
        case(
            Read {
                api: "page.$",
                op: "locator.elementHandle",
                input: json!({ "target": at(page, "h1"), "strict": "firstMatch", "noWait": true }),
            },
            Expect::Shows(vec![
                r#"<h1 class="who" data-who="[private]">Signed in with [private]</h1>"#,
            ]),
        ),
        // Two matches, on purpose: the strict violation's message previews both.
        case(
            element("locator.textContent", ".who"),
            Expect::Fails {
                code: ErrorCode::StrictModeViolation,
                previews: vec![
                    r#"1) <h1 class="who" data-who="[private]">Signed in with [private]</h1>"#,
                    r#"2) <p class="who">[private]</p>"#,
                ],
            },
        ),
        case(
            snapshot("default"),
            Expect::Shows(vec!["Signed in with [private]"]),
        ),
        case(
            Read {
                api: "locator.ariaSnapshot",
                op: "locator.ariaSnapshot",
                input: json!({ "target": at(page, "body"), "strict": "strict", "mode": "default" }),
            },
            Expect::Shows(vec!["Signed in with [private]"]),
        ),
        case(snapshot("ai"), Expect::Shows(ai_shows)),
    ]
}

/// The reads the cost test times, each on the one element every timed page has.
fn timed_reads(page: &TestPage) -> Vec<Read> {
    let mut timed: Vec<Read> = reads(page, FieldFrame::Unreached)
        .into_iter()
        .map(|case| case.read)
        .filter(|read| matches!(read.api, "page.title" | "page.content"))
        .collect();
    for api in ["locator.textContent", "locator.innerText"] {
        timed.push(Read {
            api,
            op: api,
            input: json!({ "target": at(page, "h1"), "strict": "strict" }),
        });
    }
    for mode in ["default", "ai"] {
        timed.push(Read {
            api: "page.ariaSnapshot",
            op: "page.ariaSnapshot",
            input: json!({ "page": brand(page), "mode": mode }),
        });
    }
    timed
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

/// Runs every read on `page` once the page copied `secret`, and fails with each read that did
/// not return what it must, the copy masked: an error, a timeout or `neo / unsupported` from an
/// ordinary read fails it as surely as the raw secret does.
async fn reads_mask(run: &LiveRun, page: &TestPage, secret: &str, field: FieldFrame) -> TestResult {
    type_and_copy(run, page, secret).await?;
    let mut wrong = Vec::new();
    for Case { read, expect } in reads(page, field) {
        let got = invoke(run, &read).await;
        if let Some(mismatch) = expect.mismatch(&got, secret) {
            wrong.push(format!("{} ({}): {mismatch}", read.api, read.op));
        }
    }
    assert!(
        wrong.is_empty(),
        "the page copied {secret} from a frame of its site, so every read of the top document \
         returns its copy masked; these did not:\n{}",
        wrong.join("\n")
    );
    let swept = run.swept();
    assert!(
        swept.iter().any(|value| value == secret),
        "the host is told of {secret}, so its own mask knows it in later results: {swept:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_reads_mask_a_value_copied_from_a_same_site_frame_in_process() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(site_copy_pages()).await?;
    let url = server.url("127.0.0.1", "/site_copy/top.html?inproc");
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            let frames = run
                .eval(
                    &page,
                    "[window.length, frames[0].location.host === location.host]",
                )
                .await?;
            assert_eq!(
                frames,
                json!([1, true]),
                "one same-origin child, so the page has no out-of-process frame"
            );
            reads_mask(&run, &page, "inproc-secret-41", FieldFrame::Stitched).await
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_reads_mask_a_value_copied_from_a_same_site_frame_out_of_process() -> TestResult
{
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(site_copy_pages()).await?;
    let url = server.url("127.0.0.1", "/site_copy/top.html?oopif");
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = reads_mask(&run, &page, "oopif-secret-52", FieldFrame::Stitched).await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_reads_mask_a_value_copied_from_a_frame_under_a_shadow_root_oopif() -> TestResult
{
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(site_copy_pages()).await?;
    let url = server.url("127.0.0.1", "/site_copy/top.html?shadow");
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            assert_eq!(
                run.eval(&page, "window.length").await?,
                json!(0),
                "a frame in a shadow root is not one of window's frames"
            );
            reads_mask(&run, &page, "shadow-secret-63", FieldFrame::Unreached).await
        }
        .await;
        (run, result)
    })
    .await
}

/// A page on 127.0.0.1 embeds two frames of localhost (`tests/data/site_copy/pair.html`): a
/// visible sibling, and a hidden password field that no snapshot prints and so no snapshot hop
/// reaches. The sibling's own script copies the field's typed value into its heading. The top
/// frame's site holds no field and the sibling's own sweep finds none, so only the sibling's
/// site sweep, taken with its read, knows the value.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_snapshot_masks_a_child_frame_s_copy_from_a_hidden_frame_of_its_site()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(site_copy_pages()).await?;
    let url = server.url("127.0.0.1", "/site_copy/pair.html");
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            let secret = "pair-secret-74";
            let started = Instant::now();
            let copy = format!("copyField({secret:?}), copied");
            while run.eval(&page, &copy).await? != json!(true) {
                if started.elapsed() > Duration::from_secs(10) {
                    return Err("the sibling never copied the hidden field's value".into());
                }
                time::sleep(Duration::from_millis(50)).await;
            }
            // Default mode reads the top frame alone; ai mode stitches the sibling in.
            for (mode, shows) in [
                ("default", "\"name\":\"Pair\""),
                ("ai", "Signed in with [private]"),
            ] {
                let read = Read {
                    api: "page.ariaSnapshot",
                    op: "page.ariaSnapshot",
                    input: json!({ "page": brand(&page), "mode": mode }),
                };
                let snapshot = invoke(&run, &read).await?.to_string();
                assert!(
                    !snapshot.contains(secret) && snapshot.contains(shows),
                    "the {mode} snapshot shows {shows}, the sibling's copy masked: {snapshot}"
                );
            }
            let swept = run.swept();
            assert!(
                swept.iter().any(|value| value == secret),
                "the host is told of {secret}: {swept:?}"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// A page with one frame and a password of its own, the common case.
const ONE: &str = "<!doctype html><title>One</title><h1>One</h1><label>Password \
                   <input type=\"password\"></label>";

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

/// Prints what each read costs on a one-frame page, and on the pages with a same-site frame.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn measures_the_cost_of_a_read() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let mut pages = site_copy_pages();
    pages.insert("/one.html".to_owned(), Page::html(ONE));
    let server = FixtureServer::start(pages).await?;
    let urls = [
        ("one frame", server.url("127.0.0.1", "/one.html")),
        (
            "same-site frame in process",
            server.url("127.0.0.1", "/site_copy/top.html?inproc"),
        ),
        (
            "same-site frame out of process",
            server.url("127.0.0.1", "/site_copy/top.html?oopif"),
        ),
    ];
    for (name, url) in urls {
        on_page(LiveRun::connect().await?, &url, |run, page| async move {
            let result = async {
                // Lets the frames load, so every call reads the same tree.
                time::sleep(Duration::from_millis(1_500)).await;
                for read in timed_reads(&page) {
                    let Cost { median, slowest } = cost(40, || invoke(&run, &read)).await?;
                    let mode = read.input.get("mode").and_then(Value::as_str);
                    report(&format!(
                        "{}{}, {name}: median {median:?}, slowest {slowest:?}",
                        read.api,
                        mode.map(|mode| format!(" {mode}")).unwrap_or_default()
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
