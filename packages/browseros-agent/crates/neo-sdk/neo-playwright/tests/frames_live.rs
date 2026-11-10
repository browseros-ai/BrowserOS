//! Frames through `Playwright::invoke`, against a real browser: frame locator chains into each
//! of the 8 frames of the nested 4-level fixture (same-process children, OOPIFs, an OOPIF
//! inside an OOPIF), `count` at each level of a chain 4 deep, strict mode inside a frame, a
//! frame that navigated, refs the page never printed, and the journal from a password field
//! inside an out-of-process iframe. Also checks the hand-built locator input against the
//! vendored parser's output.
//!
//! The live tests need BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p neo-playwright --test frames_live -- --ignored`. They serve the
//! fixtures on `127.0.0.1`, `localhost` and `[::1]`, and open and close their own tabs.

#[path = "support/fixture_server.rs"]
mod fixture_server;
#[path = "support/js_runner.rs"]
mod js_runner;
#[path = "support/live_browser.rs"]
#[expect(
    dead_code,
    reason = "shared with snapshot_live.rs, which reads the swept values"
)]
mod live_browser;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "these scripts reach pages through the runner, not live_run's direct helpers"
)]
mod live_run;
#[path = "support/locator.rs"]
mod locator;

use std::sync::Arc;
use std::time::{Duration, Instant};

use browseros_core::sensitive_fields::SensitiveKind;
use browseros_core::{PageId, TabId};
use fixture_server::{FixtureServer, Page, nest_pages};
use js_runner::OpHost;
use live_browser::{Journaled, LiveBrowser, TestPage, TestResult};
use live_run::LiveRun;
use locator::{aria_ref, attr, css, enter_frame, locator, role, text};
use neo_playwright::{ErrorCode, TypedBy};
use serde_json::{Value, json};
use tokio::time;

/// Each frame of the nested fixture: the iframe titles down to it, and its button's text.
const FRAMES: [(&[&str], &str); 8] = [
    (&[], "Top button"),
    (&["a1"], "A1 button"),
    (&["b1"], "B1 button"),
    (&["a1", "b2"], "B2 button"),
    (&["b1", "c2"], "C2 button"),
    (&["a1", "b2", "b3"], "B3 button"),
    (&["b1", "c2", "a3"], "A3 button"),
    (&["b1", "c2", "a3", "c4"], "C4 button"),
];

/// `frameLocator('[title=…]')` down `titles`, then `last`.
fn down(page: &TestPage, titles: &[&str], last: Value) -> Value {
    use std::fmt::Write as _;
    let mut parts = Vec::new();
    let mut display = "page".to_owned();
    for title in titles {
        parts.push(attr(&format!("[title=\"{title}\"s]")));
        parts.push(enter_frame());
        let _infallible = write!(display, ".frameLocator('[title=\"{title}\"]')");
    }
    parts.push(last);
    locator(page, &parts, &display)
}

fn text_content(target: &Value) -> Value {
    json!({ "call": { "api": "locator.textContent", "timeoutMs": 10_000 }, "target": target, "strict": "strict" })
}

fn count(target: &Value) -> Value {
    json!({ "call": { "api": "locator.count" }, "target": target })
}

/// Opens the nested fixture once every frame, down to c4, has its button.
async fn nested(live: &LiveBrowser, server: &FixtureServer) -> TestResult<TestPage> {
    let page = live
        .open(&server.url("127.0.0.1", "/nest/top.html"))
        .await?;
    let c4 = down(&page, &["b1", "c2", "a3", "c4"], role("button"));
    let read = live
        .invoke("locator.textContent", text_content(&c4))
        .await?;
    assert_eq!(read?, json!("C4 button"), "the deepest frame loaded");
    Ok(page)
}

#[test]
fn tests_that_hand_built_chunks_are_the_vendored_parsers() {
    // `splitSelectorByFrame` from crates/neo-sdk/neo-playwright/assets/prelude.js, run on the
    // selectors each helper stands for.
    let page = TestPage {
        id: PageId(7),
        tab: TabId(1007),
    };
    let built = locator(
        &page,
        &[
            attr("[title=\"b1\"s]"),
            enter_frame(),
            role("button[name=\"B1 button\"i]"),
        ],
        "x",
    );
    assert_eq!(
        built["chunks"],
        json!([
            { "parts": [{ "name": "internal:attr", "body": "[title=\"b1\"s]", "source": "[title=\"b1\"s]" }] },
            { "parts": [{ "name": "internal:role", "body": "button[name=\"B1 button\"i]", "source": "button[name=\"B1 button\"i]" }] },
        ]),
        "internal:attr=[title=\"b1\"s] >> internal:control=enter-frame >> internal:role=button[name=\"B1 button\"i]"
    );
    assert_eq!(
        css("iframe[title=\"a1\"]"),
        json!({ "name": "css", "body": [{ "simples": [{ "selector": { "css": "iframe[title=\"a1\"]", "functions": [] }, "combinator": "" }] }], "source": "iframe[title=\"a1\"]" }),
        "iframe[title=\"a1\"]"
    );
    assert_eq!(
        text("\"Go\"i"),
        json!({ "name": "internal:text", "body": "\"Go\"i", "source": "\"Go\"i" }),
        "internal:text=\"Go\"i"
    );
    assert_eq!(
        aria_ref("f7e3"),
        json!({ "name": "aria-ref", "body": "f7e3", "source": "f7e3" }),
        "aria-ref=f7e3"
    );
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_frame_locator_chain_reads_each_frames_own_button() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = nested(&live, &server).await?;
    for (titles, button) in FRAMES {
        let read = live
            .invoke(
                "locator.textContent",
                text_content(&down(&page, titles, role("button"))),
            )
            .await?;
        assert_eq!(
            read?,
            json!(button),
            "down {titles:?}, the frame's own button"
        );
    }
    let heading = "C2 (C in B: OOPIF inside OOPIF, level 2)";
    let read = live
        .invoke(
            "locator.textContent",
            text_content(&down(
                &page,
                &["b1", "c2"],
                text(&format!("\"{heading}\"s")),
            )),
        )
        .await?;
    assert_eq!(
        read?,
        json!(heading),
        "getByText inside an OOPIF inside an OOPIF"
    );

    // count is 1 at each level of the 4-deep chain, and never waits.
    let chain = ["b1", "c2", "a3", "c4"];
    for depth in 0..chain.len() {
        let next = attr(&format!("[title=\"{}\"s]", chain[depth]));
        let counted = live
            .invoke("locator.count", count(&down(&page, &chain[..depth], next)))
            .await?;
        assert_eq!(
            counted?,
            json!(1),
            "one iframe {} at depth {depth}",
            chain[depth]
        );
    }
    let buttons = live
        .invoke("locator.count", count(&down(&page, &chain, role("button"))))
        .await?;
    assert_eq!(buttons?, json!(1), "one button at the bottom");
    let missing = live
        .invoke(
            "locator.count",
            count(&down(&page, &["b1", "nowhere"], role("button"))),
        )
        .await?;
    assert_eq!(
        missing?,
        json!(0),
        "a frame that is not there counts nothing"
    );
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_strict_violation_inside_a_frame_reads_as_upstreams() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = nested(&live, &server).await?;
    let started = Instant::now();
    let failed = live
        .invoke(
            "locator.textContent",
            text_content(&down(&page, &["a1", "b2"], css("button, input"))),
        )
        .await?
        .err()
        .ok_or("two matches under strict mode must fail")?;
    assert_eq!(failed.error_code(), Some(ErrorCode::StrictModeViolation));
    let message = failed.message();
    assert!(
        message.starts_with(
            "locator.textContent: Error: strict mode violation: locator('button, input') resolved to 2 elements:\n    1) <button>B2 button</button> aka getByRole('button', { name: 'B2 button' })\n    2) <input aria-label=\"B2 field\"/> aka getByRole('textbox', { name: 'B2 field' })\n"
        ),
        "upstream's message, from inside the OOPIF: {message}"
    );
    assert!(
        message.contains("\nCall log:\n  - waiting for page.frameLocator"),
        "{message}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "strict mode fails at once, it does not wait"
    );
    live.close(&page).await
}

/// `page.frames()` for `page`, as the prelude asks for it: the frame handle ids it sends back.
async fn frame_ids(live: &LiveBrowser, page: &TestPage) -> TestResult<Vec<(String, String)>> {
    let tree = live
        .invoke("page.frames", json!({ "page": page.brand() }))
        .await??;
    let frames = tree
        .get("frames")
        .and_then(Value::as_array)
        .ok_or("page.frames lists frames")?;
    Ok(frames
        .iter()
        .filter_map(|frame| {
            Some((
                frame["id"].as_str()?.to_owned(),
                frame["url"].as_str()?.to_owned(),
            ))
        })
        .collect())
}

/// `frame.locator(selector)` on the child frame `frame`, as the prelude's `locatorBrand` builds
/// it: the frame's wire id beside the chunks.
fn in_frame(page: &TestPage, frame: &str, last: Value, display: &str) -> Value {
    let mut target = locator(page, &[last], display);
    if let Some(fields) = target.as_object_mut() {
        fields.insert("frame".to_owned(), json!(frame));
    }
    target
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_child_frames_locators_and_shortcuts_start_in_that_frame() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = nested(&live, &server).await?;
    let frames = frame_ids(&live, &page).await?;
    // a1 shares the top's process, b1 is an OOPIF, c4 an OOPIF four levels down.
    for (file, button, titles) in [
        ("a1", "A1 button", &["a1"][..]),
        ("b1", "B1 button", &["b1"][..]),
        ("c4", "C4 button", &["b1", "c2", "a3", "c4"][..]),
    ] {
        let (frame, _) = frames
            .iter()
            .find(|(_, url)| url.ends_with(&format!("/nest/{file}.html")))
            .ok_or(format!("page.frames lists {file}"))?;
        let button_in = in_frame(&page, frame, role("button"), "locator('button')");
        let inner = live
            .invoke(
                "locator.innerText",
                json!({ "call": { "api": "frame.innerText", "timeoutMs": 5_000 },
                        "target": button_in, "strict": "firstMatch" }),
            )
            .await??;
        assert_eq!(inner, json!(button), "frame.innerText('button') in {file}");
        let text = live
            .invoke(
                "locator.textContent",
                json!({ "call": { "api": "locator.textContent", "timeoutMs": 5_000 },
                        "target": button_in, "strict": "strict" }),
            )
            .await??;
        assert_eq!(text, json!(button), "frame.locator('button') in {file}");
        let handle = live
            .invoke(
                "locator.waitFor",
                json!({ "call": { "api": "frame.waitForSelector", "timeoutMs": 5_000 },
                        "target": button_in, "strict": "firstMatch", "state": "visible",
                        "omitReturnValue": false }),
            )
            .await??;
        assert_eq!(
            handle["display"],
            json!(format!("<button>{button}</button>")),
            "frame.waitForSelector('button') in {file} hands back its button: {handle}"
        );
        let box_of = |target: &Value| {
            json!({ "call": { "api": "locator.boundingBox", "timeoutMs": 5_000 },
                    "target": target, "strict": "strict" })
        };
        let from_frame = live
            .invoke("locator.boundingBox", box_of(&button_in))
            .await??;
        let from_chain = live
            .invoke(
                "locator.boundingBox",
                box_of(&down(&page, titles, role("button"))),
            )
            .await??;
        assert_eq!(
            from_frame, from_chain,
            "a frame start records the same parents as the frameLocator chain to {file}"
        );
    }
    let gone = live
        .invoke(
            "locator.textContent",
            text_content(&in_frame(&page, "0123456789ABCDEF", role("button"), "x")),
        )
        .await?
        .err()
        .ok_or("a frame the page does not have was detached")?;
    assert_eq!(gone.error_code(), Some(ErrorCode::TargetClosed));
    assert!(
        gone.message()
            .starts_with("locator.textContent: Frame was detached"),
        "{}",
        gone.message()
    );
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_frame_that_navigated_is_read_in_its_new_document() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = nested(&live, &server).await?;
    let a1 = || down(&page, &["a1"], role("button"));
    let before = live
        .invoke("locator.textContent", text_content(&a1()))
        .await?;
    assert_eq!(before?, json!("A1 button"));
    live.main_world(
        &page,
        "document.querySelector('iframe[title=a1]').src = '/nest/b3.html'",
    )
    .await?;
    let mut read = Value::Null;
    for _ in 0..100 {
        read = live
            .invoke("locator.textContent", text_content(&a1()))
            .await??;
        if read == "B3 button" {
            break;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        read,
        json!("B3 button"),
        "the hop entered a new world in the frame's new document"
    );
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_refs_the_page_never_printed_fail_at_once() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = nested(&live, &server).await?;
    let started = Instant::now();
    let unknown = live
        .invoke(
            "locator.textContent",
            text_content(&down(&page, &[], aria_ref("f99e1"))),
        )
        .await?
        .err()
        .ok_or("a ref of a document this page never had must fail")?;
    assert_eq!(unknown.error_code(), Some(ErrorCode::InvalidInput));
    assert!(
        unknown.message().starts_with(
            "locator.textContent: f99e1 is not a ref on this page; take a new snapshot"
        ),
        "{}",
        unknown.message()
    );
    let stale = live
        .invoke(
            "locator.textContent",
            text_content(&down(&page, &[], aria_ref("e4"))),
        )
        .await?
        .err()
        .ok_or("a ref no snapshot minted must fail")?;
    assert_eq!(stale.error_code(), Some(ErrorCode::ElementDetached));
    assert!(
        stale.message().starts_with(
            "locator.textContent: e4 is not on the page any more; take a new snapshot"
        ),
        "the main document is seq 0, but no snapshot printed e4 in it: {}",
        stale.message()
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "neither waits out its timeout"
    );
    live.close(&page).await
}

const LOGIN_TOP: &str = r"<!doctype html><title>login top</title><body><script>
const frame = document.createElement('iframe');
frame.title = 'login';
frame.src = `http://localhost:${location.port}/pw/login.html`;
document.body.append(frame);
</script></body>";

const LOGIN: &str =
    r#"<!doctype html><title>login</title><input id="pw" type="password" aria-label="Password">"#;

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_the_journal_reports_typing_in_an_oopif_password_field() -> TestResult {
    let live = LiveBrowser::connect().await?;
    let mut pages = nest_pages();
    pages.insert("/pw/top.html".to_owned(), Page::html(LOGIN_TOP));
    pages.insert("/pw/login.html".to_owned(), Page::html(LOGIN));
    let server = FixtureServer::start(pages).await?;
    let page = live.open(&server.url("127.0.0.1", "/pw/top.html")).await?;
    // Counting through the frame locator seals the OOPIF's world and sets its session up.
    let field = locator(
        &page,
        &[attr("[title=\"login\"s]"), enter_frame(), css("#pw")],
        "page.frameLocator('[title=\"login\"]').locator('#pw')",
    );
    let mut counted = json!(0);
    for _ in 0..100 {
        counted = live.invoke("locator.count", count(&field)).await??;
        if counted == 1 {
            break;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(counted, json!(1), "the OOPIF's field is there");

    // Type as the owner would, on a session of the test's own to the OOPIF's target.
    let targets = live
        .browser
        .cdp("Target.getTargets", json!({}), None)
        .await?;
    let login = targets["targetInfos"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|info| {
            info["type"] == "iframe"
                && info["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with("/pw/login.html"))
        })
        .ok_or("the login iframe is its own target")?;
    let attached = live
        .browser
        .cdp(
            "Target.attachToTarget",
            json!({ "targetId": login["targetId"], "flatten": true }),
            None,
        )
        .await?;
    let session =
        browseros_core::SessionId::from(attached["sessionId"].as_str().ok_or("no session id")?);
    live.browser
        .cdp(
            "Runtime.evaluate",
            json!({ "expression": "document.getElementById('pw').focus()" }),
            Some(&session),
        )
        .await?;
    live.browser
        .cdp(
            "Input.insertText",
            json!({ "text": "journal-oopif" }),
            Some(&session),
        )
        .await?;
    let expected = Journaled {
        page: page.id.clone(),
        kind: SensitiveKind::Password,
        value: "journal-oopif".to_owned(),
        by: TypedBy::NotAgent,
    };
    assert!(
        live.hooks.wait_for(&expected).await,
        "the OOPIF's seal journaled the value, attributed to its page: {:?}",
        live.hooks.journaled()
    );
    let _detached = live
        .browser
        .cdp(
            "Target.detachFromTarget",
            json!({ "sessionId": session.as_str() }),
            None,
        )
        .await;
    live.close(&page).await
}

/// FR-1: a run that starts on a tab with frames lists every one of them in its first
/// statement, as upstream's `page.frames()` does on a page it connected to. The prelude reads
/// the tab's tree before the script runs, through the `page` provider S3's bootstrap awaits.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_run_on_a_tab_with_frames_lists_them_in_its_first_statement() -> TestResult {
    let server = FixtureServer::start(nest_pages()).await?;
    let host: Arc<dyn OpHost> = Arc::new(LiveRun::connect().await?);
    // A first run opens the tab and waits for the deepest frame of each branch, as a task's
    // earlier run would have left it.
    let opened = js_runner::run_script(
        &host,
        &json!({ "page": null }),
        &format!(
            "await page.goto({top})\n\
             const button = (titles) => titles\n\
               .reduce((at, title) => at.frameLocator(`[title=\"${{title}}\"]`), page)\n\
               .getByRole('button')\n\
             await button(['a1', 'b2', 'b3']).textContent()\n\
             await button(['b1', 'c2', 'a3', 'c4']).textContent()\n\
             return page.toJSON()",
            top = json!(server.url("127.0.0.1", "/nest/top.html")),
        ),
    )
    .await?;
    let listed = js_runner::run_script(
        &host,
        &json!({ "page": opened }),
        "try {\n\
           return page.frames().map((frame) => frame.url().split('/').pop()).sort()\n\
         } finally {\n\
           await page.close()\n\
         }",
    )
    .await?;
    assert_eq!(
        listed,
        json!([
            "a1.html", "a3.html", "b1.html", "b2.html", "b3.html", "c2.html", "c4.html", "top.html"
        ]),
        "all 8 frames, before any op of the script's own"
    );
    Ok(())
}

/// FR-1: a frame the page attaches after load is listed once `waitForTimeout` ends, as
/// upstream's is after its wait's round trip: the wait reads the run's working pages' trees.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_frames_attached_after_load_are_listed_after_wait_for_timeout() -> TestResult {
    let server = FixtureServer::start(nest_pages()).await?;
    let host: Arc<dyn OpHost> = Arc::new(LiveRun::connect().await?);
    let script = format!(
        "try {{\n\
           await page.goto({top})\n\
           const evaluate = async (expression) =>\n\
             JSON.parse(await __neo_test_eval(page._id, page._tab, expression)).ok\n\
           await evaluate(`(() => {{\n\
             const frame = document.createElement('iframe')\n\
             frame.name = 'late'\n\
             frame.onload = () => {{ window.__late = true }}\n\
             frame.src = '/nest/b3.html'\n\
             document.body.append(frame)\n\
             return true\n\
           }})()`)\n\
           for (let tries = 0; !(await evaluate('window.__late === true')); tries++) {{\n\
             if (tries === 100) throw new Error('the late frame never loaded')\n\
             await new Promise((resolve) => setTimeout(resolve, 50))\n\
           }}\n\
           const before = page.frame('late')\n\
           await page.waitForTimeout(300)\n\
           return {{ before, late: page.frame('late')?.url() ?? null }}\n\
         }} finally {{\n\
           await page.close()\n\
         }}",
        top = json!(server.url("127.0.0.1", "/nest/top.html")),
    );
    assert_eq!(
        js_runner::run_script(&host, &json!({ "page": null }), &script).await?,
        json!({ "before": null, "late": server.url("127.0.0.1", "/nest/b3.html") }),
        "listed after the wait, and not before it"
    );
    Ok(())
}
