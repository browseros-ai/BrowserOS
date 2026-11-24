//! Frames through `Playwright::invoke`, against a real browser: frame locator chains into each
//! of the 8 frames of the nested 4-level fixture (same-process children, OOPIFs, an OOPIF
//! inside an OOPIF), `count` at each level of a chain 4 deep, strict mode inside a frame, a
//! frame that navigated and refs the page never printed. Also checks the hand-built locator
//! input against the vendored parser's output.
//!
//! The live tests need BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `bun run test:browser -- cargo test -p neo-playwright --test frames_live -- --ignored`. They serve the
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
    reason = "most scripts reach pages through the runner; only the comparison row uses a few \
              of live_run's direct helpers"
)]
mod live_run;
#[path = "support/locator.rs"]
mod locator;

use std::collections::BTreeSet;
use std::error::Error;
use std::mem;
use std::sync::Arc;
use std::time::{Duration, Instant};

use browseros_cdp::{CdpClient, ConnectOptions};
use browseros_core::{CdpConnection, PageId, ProtocolSession, SessionId, TabId, TargetId};
use fixture_server::{FixtureServer, Page, nest_pages};
use js_runner::OpHost;
use live_browser::{LiveBrowser, TestPage, TestResult};
use live_run::LiveRun;
use locator::{aria_ref, attr, css, enter_frame, locator, role, text};
use neo_playwright::ErrorCode;
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
             JSON.parse(await __neo_test_eval(page._tab, expression)).ok\n\
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

/// How long the target watcher's frame list may take to agree with the browser's own tree after
/// a change: the change's frame events, or a new frame session's attach and its seed read, each
/// behind a tick and a tree read in the tab's line at most. Read from code, not measured; the
/// comparison row prints what each step took.
const LIST_AGREES_BOUND: Duration = Duration::from_secs(5);

/// One frame as the comparison row compares it: id, parent, name, and URL with its fragment.
type ComparedFrame = (String, Option<String>, String, String);

/// `TabFrames`' list, built from a tab's CDP events and seed reads only, equals the browser's own
/// frame tree (`Page.getFrameTree`, out-of-process frames read from their own targets) on the
/// nested four-level fixture: after its load; after a same-site and a cross-site iframe are
/// added; after one is removed; after a child navigates (dropping its old subtree); after that
/// child moves to another site (a cross-process swap); after the main frame navigates; and on a
/// tab another client opened, which Neo never acted on. The list holds the main frame first and
/// is whole (its seed landed). The browser's tree is read on a connection of the test's own, so no
/// session of the run's is touched.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_event_built_frame_list_equals_the_browsers_tree() -> TestResult {
    let mut pages = nest_pages();
    pages.insert(
        "/leaf.html".to_owned(),
        Page::html("<!doctype html><title>leaf</title>"),
    );
    let server = FixtureServer::start(pages).await?;
    let run = LiveRun::connect().await?;
    let client = Arc::new(CdpClient::connect(ConnectOptions::new(live_run::cdp_port())).await?);
    let transport: Arc<dyn CdpConnection> = Arc::<CdpClient>::clone(&client);
    let page = run.new_page().await?;
    let mut outside = None;
    let result = async {
        let target = run
            .browser()
            .pages
            .list()
            .await?
            .into_iter()
            .find(|info| info.tab_id == page.tab)
            .ok_or("the run's tab is listed")?
            .target_id;
        let top = server.url("127.0.0.1", "/nest/top.html");
        goto(&run, &page, &top).await?;
        agree(&run, &transport, &target, "load", |frames| {
            frames.len() == 8
        })
        .await?;

        add_frames(&run, &page, &server).await?;
        agree(&run, &transport, &target, "add", |frames| {
            frames.len() == 10
        })
        .await?;
        run.eval(
            &page,
            "document.querySelector('iframe[name=\"same-added\"]').remove(); true",
        )
        .await?;
        agree(&run, &transport, &target, "remove", |frames| {
            frames.len() == 9
        })
        .await?;

        for (step, url) in [
            ("child", server.url("127.0.0.1", "/leaf.html?child")),
            ("swap", server.url("localhost", "/leaf.html?swap")),
        ] {
            run.eval(
                &page,
                &format!("document.querySelector('iframe[title=\"a1\"]').src = {url:?}; true"),
            )
            .await?;
            // a1's own subtree (b2, b3) went with its old document.
            agree(&run, &transport, &target, step, |frames| {
                frames.len() == 7 && frames.iter().any(|(_, _, _, at)| *at == url)
            })
            .await?;
        }
        goto(&run, &page, &format!("{top}?again")).await?;
        agree(&run, &transport, &target, "main", |frames| {
            frames.len() == 8 && frames.iter().all(|(_, _, _, at)| at.ends_with("?again"))
        })
        .await?;

        untouched_agrees(&run, &transport, &top, &mut outside).await?;
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    if let Some(untouched) = outside {
        let _closed = transport
            .send(
                "Target.closeTarget",
                json!({ "targetId": untouched.as_str() }),
                None,
            )
            .await;
    }
    run.close_page(&page).await;
    client.disconnect().await;
    result
}

/// Adds a same-site iframe (`same-added`) and a cross-site one (`cross-added`) to the run's page,
/// each on the fixture's leaf page.
async fn add_frames(
    run: &LiveRun,
    page: &live_run::TestPage,
    server: &FixtureServer,
) -> TestResult {
    let added = json!([
        ["same-added", server.url("127.0.0.1", "/leaf.html")],
        ["cross-added", server.url("localhost", "/leaf.html")]
    ]);
    run.eval(
        page,
        &format!(
            "(() => {{ for (const [name, url] of {added}) {{ \
               const frame = document.createElement('iframe'); \
               frame.name = name; frame.src = url; document.body.append(frame) }} \
             return true }})()"
        ),
    )
    .await?;
    Ok(())
}

/// Opens `top` in a tab of the test's own connection, which Neo never acts on, and waits for the
/// run's list of it to agree with its tree; the run's core must not have attached to it. The tab
/// goes into `outside` as soon as it exists, so the row closes it whatever happens.
async fn untouched_agrees(
    run: &LiveRun,
    transport: &Arc<dyn CdpConnection>,
    top: &str,
    outside: &mut Option<TargetId>,
) -> TestResult {
    let created = transport
        .send(
            "Target.createTarget",
            json!({ "url": top, "background": true }),
            None,
        )
        .await?;
    let untouched = TargetId(
        created
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or("no target id")?
            .to_owned(),
    );
    *outside = Some(untouched.clone());
    agree(run, transport, &untouched, "untouched", |frames| {
        frames.len() == 8
    })
    .await?;
    let listed = run
        .browser()
        .pages
        .list()
        .await?
        .into_iter()
        .find(|info| info.target_id == untouched)
        .ok_or("the untouched tab is listed")?;
    if run
        .browser()
        .pages
        .get_attached_session(listed.page_id)
        .await
        .is_some()
    {
        return Err("the run's core attached to the tab another client opened".into());
    }
    Ok(())
}

/// `page.goto` on the run's page, to its load. The run's own page type is `live_run`'s: this file
/// also names `live_browser`'s.
async fn goto(run: &LiveRun, page: &live_run::TestPage, url: &str) -> TestResult {
    run.invoke(
        "page.goto",
        json!({ "call": { "timeoutMs": 30_000 }, "page": live_run::brand(page), "url": url,
                "waitUntil": "load" }),
    )
    .await?;
    Ok(())
}

/// Waits, within [`LIST_AGREES_BOUND`], until the browser's tree of `target` is `ready` (the
/// step's change took effect) and the run's event-built list equals it: whole, main frame first,
/// and the same frames. Prints how long it took.
#[expect(
    clippy::print_stderr,
    reason = "each step prints how long the list took to agree"
)]
async fn agree(
    run: &LiveRun,
    transport: &Arc<dyn CdpConnection>,
    target: &TargetId,
    step: &str,
    ready: impl Fn(&[ComparedFrame]) -> bool,
) -> TestResult {
    let started = Instant::now();
    // The bound covers the browser's own reads too: a renderer that stalls a read must not
    // stretch a step past it.
    let mut last = String::new();
    let agreed = time::timeout(LIST_AGREES_BOUND, async {
        loop {
            // An iframe target the browser just listed can go before the read attaches to it (a
            // step that drops a subtree destroys its targets): the tree is still changing, so it
            // is read again, and a read that keeps failing ends the step at the bound.
            let tree = match browser_frames(transport, target.as_str()).await {
                Ok(tree) => tree,
                Err(error) => {
                    last = format!("the browser's tree read failed: {error}");
                    time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let listed = run.tab_frames(target);
            let compared: Option<Vec<ComparedFrame>> = listed.as_ref().map(|list| {
                list.frames
                    .iter()
                    .map(|frame| {
                        (
                            frame.id.0.clone(),
                            frame.parent_id.as_ref().map(|parent| parent.0.clone()),
                            frame.name.clone(),
                            frame.url.clone(),
                        )
                    })
                    .collect()
            });
            let whole = listed.as_ref().is_some_and(|list| list.whole);
            let same = compared.as_ref().is_some_and(|list| {
                list.first().is_some_and(|(id, ..)| id == target.as_str())
                    && list.iter().collect::<BTreeSet<_>>() == tree.iter().collect::<BTreeSet<_>>()
            });
            if ready(&tree) && whole && same {
                return tree.len();
            }
            last = format!("(whole {whole})\nlist: {compared:#?}\ntree: {tree:#?}");
            time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    match agreed {
        Ok(frames) => {
            eprintln!(
                "FRAME_LIST step={step} agreed={:?} frames={}",
                started.elapsed(),
                frames
            );
            Ok(())
        }
        Err(_) => Err(format!(
            "{step}: the list did not equal the browser's tree within {LIST_AGREES_BOUND:?} {last}"
        )
        .into()),
    }
}

/// Every frame of tab `target` as the browser reports it now, out-of-process ones included: the
/// tab's own `Page.getFrameTree`, then each iframe target whose parent frame is already listed,
/// level by level, as `page/frame_tree.rs` reads a tab.
async fn browser_frames(
    transport: &Arc<dyn CdpConnection>,
    target: &str,
) -> TestResult<Vec<ComparedFrame>> {
    let mut frames = Vec::new();
    let mut visited = BTreeSet::new();
    let mut read = vec![(target.to_owned(), None::<String>)];
    while !read.is_empty() {
        for (at, parent) in mem::take(&mut read) {
            let tree = target_tree(transport, &at).await?;
            visited.insert(at);
            flatten(&tree, parent, &mut frames);
        }
        let listed = transport
            .send(
                "Target.getTargets",
                json!({ "filter": [{ "type": "iframe" }] }),
                None,
            )
            .await?;
        for info in listed
            .get("targetInfos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(id), Some(parent)) = (
                info.get("targetId").and_then(Value::as_str),
                info.get("parentFrameId").and_then(Value::as_str),
            ) else {
                continue;
            };
            if !visited.contains(id) && frames.iter().any(|(known, ..)| known == parent) {
                read.push((id.to_owned(), Some(parent.to_owned())));
            }
        }
    }
    Ok(frames)
}

/// `target`'s own `Page.getFrameTree`, read on a session of this connection's own, which it then
/// leaves.
async fn target_tree(transport: &Arc<dyn CdpConnection>, target: &str) -> TestResult<Value> {
    let attached = transport
        .send(
            "Target.attachToTarget",
            json!({ "targetId": target, "flatten": true }),
            None,
        )
        .await?;
    let id = attached
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or("no session id")?
        .to_owned();
    let session = ProtocolSession::for_session(Arc::clone(transport), SessionId::from(id.as_str()));
    let tree = async {
        session.send_value("Page.enable", json!({})).await?;
        session.send_value("Page.getFrameTree", json!({})).await
    }
    .await;
    let _detached = transport
        .send("Target.detachFromTarget", json!({ "sessionId": id }), None)
        .await;
    Ok(tree?.get("frameTree").cloned().ok_or("no frame tree")?)
}

/// Appends `node` (a `Page.getFrameTree` frame tree) and its subtree to `out`; `parent` is the
/// frame that embeds `node`'s root when its own tree names none.
fn flatten(node: &Value, parent: Option<String>, out: &mut Vec<ComparedFrame>) {
    let field = |name: &str| {
        node.get("frame")
            .and_then(|frame| frame.get(name))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let id = field("id");
    let parent = node
        .get("frame")
        .and_then(|frame| frame.get("parentId"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or(parent);
    out.push((
        id.clone(),
        parent,
        field("name"),
        format!("{}{}", field("url"), field("urlFragment")),
    ));
    for child in node
        .get("childFrames")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        flatten(child, Some(id.clone()), out);
    }
}

/// A page whose buttons and keys insert an iframe synchronously, on its own site or another.
const ORDER_PAGE: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>order</title></head><body>
<button id="same" style="width:120px;height:40px">same</button>
<button id="cross" style="width:120px;height:40px">cross</button>
<script>
const add = (host) => {
  const f = document.createElement('iframe');
  f.className = 'added';
  f.src = 'http://' + host + ':' + location.port + '/leaf.html';
  document.body.append(f);
};
document.getElementById('same').addEventListener('click', () => add('127.0.0.1'));
document.getElementById('cross').addEventListener('click', () => add('localhost'));
document.addEventListener('keydown', (e) => {
  if (e.key === 's') add('127.0.0.1');
  if (e.key === 'c') add('localhost');
});
</script></body></html>"#;

/// Repetitions of each case of
/// [`tests_that_frames_list_the_iframe_a_write_ops_handler_inserted_right_after_it`].
const ORDER_REPS: usize = 20;

/// After a write op that ends with no answer from its tab's renderer (a forced click, a raw mouse
/// click, a raw key press), `page.frames` lists the iframe the op's handler inserted, same-site
/// and cross-site, right after the op: the op pings the tab's source sessions before it answers,
/// so every frame event the tab sent before is in the watcher's list, which `page.frames` reads.
/// A regression row with no live negative: on the pinned browser the frame events arrived in
/// order 180 of 180 times with no ping (OPTION-B piece 5); the ping's own rules have unit rows.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_frames_list_the_iframe_a_write_ops_handler_inserted_right_after_it()
-> TestResult {
    let mut pages = nest_pages();
    pages.insert("/order.html".to_owned(), Page::html(ORDER_PAGE));
    pages.insert(
        "/leaf.html".to_owned(),
        Page::html("<!doctype html><title>leaf</title>"),
    );
    let server = FixtureServer::start(pages).await?;
    let run = LiveRun::connect().await?;
    let page = run.new_page().await?;
    let result = async {
        goto(&run, &page, &server.url("127.0.0.1", "/order.html")).await?;
        let brand = live_run::brand(&page);
        let center = |id: &str| {
            format!(
                "(() => {{ const r = document.getElementById('{id}').getBoundingClientRect(); \
                 return [r.x + r.width / 2, r.y + r.height / 2]; }})()"
            )
        };
        let point = run.eval(&page, &center("cross")).await?;
        let (x, y) = (
            point.get(0).and_then(Value::as_f64).ok_or("x")?,
            point.get(1).and_then(Value::as_f64).ok_or("y")?,
        );
        let handle = TestPage {
            id: run.core_page(&page).await?,
            tab: page.tab,
        };
        let target = locator(&handle, &[css("#same")], "locator('#same')");
        let cases: [(&str, &str, Value); 4] = [
            (
                "a forced click",
                "locator.click",
                json!({ "call": { "api": "locator.click", "timeoutMs": 5000 },
                        "target": target, "strict": "strict", "force": true }),
            ),
            (
                "a raw mouse click",
                "mouse.click",
                json!({ "call": { "api": "mouse.click", "timeoutMs": 0 }, "page": brand,
                        "x": x, "y": y }),
            ),
            (
                "a raw key press, same-site",
                "keyboard.press",
                json!({ "call": { "api": "keyboard.press", "timeoutMs": 0 }, "page": brand,
                        "key": "s" }),
            ),
            (
                "a raw key press, cross-site",
                "keyboard.press",
                json!({ "call": { "api": "keyboard.press", "timeoutMs": 0 }, "page": brand,
                        "key": "c" }),
            ),
        ];
        for (case, op, input) in cases {
            for rep in 0..ORDER_REPS {
                let before = listed_frames(&run, &page).await?;
                run.invoke(op, input.clone()).await?;
                let after = listed_frames(&run, &page).await?;
                if !after.iter().any(|id| !before.contains(id)) {
                    return Err(format!(
                        "{case}, try {rep}: page.frames right after the op lacks the iframe \
                         its handler inserted: before {before:?}, after {after:?}"
                    )
                    .into());
                }
                run.eval(
                    &page,
                    "document.querySelectorAll('iframe.added').forEach((f) => f.remove()); true",
                )
                .await?;
                time::sleep(Duration::from_millis(100)).await;
            }
        }
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    run.close_page(&page).await;
    result
}

/// The frame ids `page.frames` answers for `page`.
async fn listed_frames(run: &LiveRun, page: &live_run::TestPage) -> TestResult<Vec<String>> {
    let frames = run
        .invoke(
            "page.frames",
            json!({ "call": {}, "page": live_run::brand(page) }),
        )
        .await?;
    Ok(frames
        .get("frames")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|frame| frame.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect())
}
