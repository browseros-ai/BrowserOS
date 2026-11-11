//! Aria snapshots through `Playwright::invoke` against a real browser: the required
//! nested-frame tests F1, F2, F4, F5, F6 and F7 of design D22 on the 4-level fixture, a
//! locator-level snapshot inside a frame, typed secrets echoed in page text coming back
//! masked, and a child frame that answers too late: partial when it is of another site,
//! withholding the snapshot when it is of the page's own.
//!
//! The live tests need BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p neo-playwright --test snapshot_live -- --ignored`. They serve the
//! fixtures on `127.0.0.1`, `localhost` and `[::1]`; F6 serves its unresponsive frame on
//! `hang.localtest.me`, a public name for 127.0.0.1, so that name must resolve. The cross-site
//! late child is on `late.localhost`, which Chromium resolves to loopback itself.

#[path = "support/fixture_server.rs"]
mod fixture_server;
#[path = "support/live_browser.rs"]
#[expect(
    dead_code,
    reason = "shared with frames_live.rs, which reads the journal"
)]
mod live_browser;
#[path = "support/locator.rs"]
#[expect(
    dead_code,
    reason = "shared with frames_live.rs, which also selects by text"
)]
mod locator;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use fixture_server::{FixtureServer, Page, nest_pages};
use live_browser::{LiveBrowser, TestPage, TestResult};
use locator::{aria_ref, attr, css, enter_frame, locator, role};
use neo_playwright::ErrorCode;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::time;

/// One test at a time. Each test connects its own `Playwright`, whose frame registry attaches
/// to every frame target in the browser, so eight nested pages loading at once can miss
/// `open`'s 5 s load wait (measured: 2 runs in 15), and F6 measures its own snapshot's time.
static ALONE: Semaphore = Semaphore::const_new(1);

/// Each frame of the nested fixture: the iframe titles down to it, and the name its button and
/// field are labelled with (`B2 button`, `B2 field`).
const FRAMES: [(&[&str], &str); 8] = [
    (&[], "Top"),
    (&["a1"], "A1"),
    (&["b1"], "B1"),
    (&["a1", "b2"], "B2"),
    (&["b1", "c2"], "C2"),
    (&["a1", "b2", "b3"], "B3"),
    (&["b1", "c2", "a3"], "A3"),
    (&["b1", "c2", "a3", "c4"], "C4"),
];

/// One node a snapshot printed with a ref, and the refs of the iframe nodes it sits under,
/// outermost first.
#[derive(Clone, Debug)]
struct Printed {
    role: String,
    name: Option<String>,
    r#ref: String,
    iframes: Vec<String>,
}

/// One frame of the fixture as a snapshot printed it.
#[derive(Clone, Debug)]
struct Frame {
    /// The prefix of every ref its document printed: empty (seq 0) or `f<seq>`.
    prefix: String,
    /// The iframe nodes its content sits under, outermost first.
    iframes: Vec<String>,
    button: String,
    field: String,
}

/// Every node with a ref in a snapshot's stitched `json`.
fn printed(json: &Value) -> Vec<Printed> {
    fn visit(node: &Value, iframes: &mut Vec<String>, out: &mut Vec<Printed>) {
        let role = field(node, "role").as_str().unwrap_or_default();
        let printed_ref = field(node, "ref").as_str();
        if let Some(r#ref) = printed_ref {
            out.push(Printed {
                role: role.to_owned(),
                name: field(node, "name").as_str().map(str::to_owned),
                r#ref: r#ref.to_owned(),
                iframes: iframes.clone(),
            });
        }
        let frame = printed_ref.filter(|_| role == "iframe");
        if let Some(r#ref) = frame {
            iframes.push(r#ref.to_owned());
        }
        for child in field(node, "children").as_array().into_iter().flatten() {
            visit(child, iframes, out);
        }
        if frame.is_some() {
            iframes.pop();
        }
    }
    let mut out = Vec::new();
    for node in json.as_array().into_iter().flatten() {
        visit(node, &mut Vec::new(), &mut out);
    }
    out
}

/// Every string a snapshot holds, keys aside.
fn strings(json: &Value) -> Vec<String> {
    match json {
        Value::String(text) => vec![text.clone()],
        Value::Array(items) => items.iter().flat_map(strings).collect(),
        Value::Object(fields) => fields.values().flat_map(strings).collect(),
        _ => Vec::new(),
    }
}

/// `value[key]`, which clippy allows only inside the tests themselves.
fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

/// `f3` of `f3e5`; empty for a bare `e5`.
fn prefix(r#ref: &str) -> &str {
    r#ref
        .rfind('e')
        .and_then(|at| r#ref.get(..at))
        .unwrap_or(r#ref)
}

/// The frame named `name` (`B2`) in `printed`: its button and its field, each printed once,
/// in the same frame.
fn frame(printed: &[Printed], name: &str) -> TestResult<Frame> {
    let once = |role: &str, label: &str| -> TestResult<&Printed> {
        let found: Vec<&Printed> = printed
            .iter()
            .filter(|node| node.role == role && node.name.as_deref() == Some(label))
            .collect();
        match found.as_slice() {
            [one] => Ok(*one),
            other => Err(format!("{role} {label:?} is printed {} times", other.len()).into()),
        }
    };
    let button = once("button", &format!("{name} button"))?;
    let field = once("textbox", &format!("{name} field"))?;
    assert_eq!(
        button.iframes, field.iframes,
        "{name}'s button and field sit in one frame"
    );
    assert_eq!(
        prefix(&button.r#ref),
        prefix(&field.r#ref),
        "{name}'s refs share its document's prefix"
    );
    Ok(Frame {
        prefix: prefix(&button.r#ref).to_owned(),
        iframes: button.iframes.clone(),
        button: button.r#ref.clone(),
        field: field.r#ref.clone(),
    })
}

/// Every frame of the fixture, by name.
fn frames(printed: &[Printed]) -> TestResult<HashMap<&'static str, Frame>> {
    FRAMES
        .iter()
        .map(|(_, name)| Ok((*name, frame(printed, name)?)))
        .collect()
}

/// F1: every frame's content sits under the iframe line its parent printed, and every ref in
/// the page carries the prefix of the document it sits in, unique page-wide.
fn assert_nested(printed: &[Printed], frames: &HashMap<&str, Frame>) -> TestResult {
    for (titles, name) in FRAMES {
        let frame = &frames[name];
        assert_eq!(
            frame.iframes.len(),
            titles.len(),
            "{name} sits under one iframe line per level"
        );
        let Some((_, parent_titles)) = titles.split_last() else {
            continue;
        };
        let (_, parent) = FRAMES
            .iter()
            .find(|(titles, _)| *titles == parent_titles)
            .ok_or(format!("{name} has a parent in the fixture"))?;
        let parent = &frames[parent];
        assert!(
            frame.iframes.starts_with(&parent.iframes),
            "{name} sits inside its parent's content: {frame:?} in {parent:?}"
        );
        assert_eq!(
            frame.iframes.last().map(|own| prefix(own)),
            Some(parent.prefix.as_str()),
            "{name}'s own iframe line is printed by its parent"
        );
    }
    let by_place: HashMap<&[String], &Frame> = frames
        .values()
        .map(|frame| (frame.iframes.as_slice(), frame))
        .collect();
    for node in printed {
        let frame = by_place
            .get(node.iframes.as_slice())
            .ok_or(format!("{node:?} sits under no frame of the fixture"))?;
        assert_eq!(
            prefix(&node.r#ref),
            frame.prefix,
            "{} sits in the document that printed it",
            node.r#ref
        );
    }
    let prefixes: HashSet<&str> = frames.values().map(|frame| frame.prefix.as_str()).collect();
    assert_eq!(
        prefixes.len(),
        FRAMES.len(),
        "each frame's document has its own prefix: {prefixes:?}"
    );
    let refs: HashSet<&str> = printed.iter().map(|node| node.r#ref.as_str()).collect();
    assert_eq!(refs.len(), printed.len(), "no ref is printed twice");
    assert_eq!(
        frames["Top"].prefix, "",
        "the main frame's first document prints bare refs"
    );
    Ok(())
}

/// A `page.ariaSnapshot` input for `page`.
fn page_snapshot(page: &TestPage, mode: &str) -> Value {
    json!({
        "call": { "api": "page.ariaSnapshot", "timeoutMs": 10_000 },
        "page": page.brand(),
        "mode": mode,
    })
}

fn locator_snapshot(target: &Value, mode: &str) -> Value {
    json!({
        "call": { "api": "locator.ariaSnapshot", "timeoutMs": 10_000 },
        "target": target,
        "strict": "strict",
        "mode": mode,
    })
}

fn by_ref(page: &TestPage, r#ref: &str) -> Value {
    locator(page, &[aria_ref(r#ref)], &format!("getByRef('{ref}')"))
}

fn text_content(target: &Value) -> Value {
    json!({ "call": { "api": "locator.textContent", "timeoutMs": 10_000 }, "target": target, "strict": "strict" })
}

/// `locator.count` of `getByRef(ref).and(getByLabel(label))`: 1 only when the ref names the
/// very element labelled `label`, in the document the ref was printed in.
fn labelled_count(page: &TestPage, r#ref: &str, label: &str) -> Value {
    let label = attr(&format!("[aria-label=\"{label}\"s]"));
    let and = json!({
        "name": "internal:and",
        "body": { "parsed": { "parts": [label] } },
        "source": "labelled",
    });
    let target = locator(page, &[aria_ref(r#ref), and], "getByRef(..).and(..)");
    json!({ "call": { "api": "locator.count" }, "target": target })
}

/// The nested fixture with the unresponsive frame D (`top.html?hang` adds it).
fn nest_and_hang() -> HashMap<String, Page> {
    let mut pages = nest_pages();
    pages.insert(
        "/nest/hang.html".to_owned(),
        Page::html(include_str!("data/nest/hang.html")).with_header("cache-control", "no-store"),
    );
    pages
}

/// The page's ai-mode snapshot, once every frame of the fixture printed its button. The
/// snapshots before that install each frame's world as they reach it.
async fn loaded(live: &LiveBrowser, page: &TestPage) -> TestResult<Value> {
    for _ in 0..100 {
        let snapshot = live
            .invoke("page.ariaSnapshot", page_snapshot(page, "ai"))
            .await??;
        let printed = printed(field(&snapshot, "json"));
        let all = FRAMES.iter().all(|(_, name)| {
            let button = format!("{name} button");
            printed
                .iter()
                .any(|node| node.name.as_deref() == Some(button.as_str()))
        });
        if all {
            return Ok(snapshot);
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    Err("the fixture's frames did not all load within 5 s".into())
}

/// F2: each named frame's button ref reads its own button, and its field ref names its own
/// field (and the button ref does not).
async fn assert_refs_read(
    live: &LiveBrowser,
    page: &TestPage,
    frames: &HashMap<&str, Frame>,
    names: &[&str],
) -> TestResult {
    for name in names {
        let frame = &frames[name];
        let text = live
            .invoke(
                "locator.textContent",
                text_content(&by_ref(page, &frame.button)),
            )
            .await??;
        assert_eq!(
            text,
            json!(format!("{name} button")),
            "{} reads {name}'s own button",
            frame.button
        );
        let label = format!("{name} field");
        let field = live
            .invoke("locator.count", labelled_count(page, &frame.field, &label))
            .await??;
        assert_eq!(field, json!(1), "{} is {name}'s own field", frame.field);
        let button = live
            .invoke("locator.count", labelled_count(page, &frame.button, &label))
            .await??;
        assert_eq!(button, json!(0), "{} is not {name}'s field", frame.button);
    }
    Ok(())
}

/// `ref`, read now, fails at once with `element_detached` and reads nothing.
async fn assert_detached(live: &LiveBrowser, page: &TestPage, r#ref: &str) -> TestResult {
    let started = Instant::now();
    let failed = live
        .invoke("locator.textContent", text_content(&by_ref(page, r#ref)))
        .await?
        .err()
        .ok_or(format!("{ref} names a document that is gone and must fail"))?;
    assert_eq!(
        failed.error_code(),
        Some(ErrorCode::ElementDetached),
        "{ref}: {}",
        failed.message()
    );
    assert!(
        failed.message().starts_with(&format!(
            "locator.textContent: {ref} is not on the page any more; take a new snapshot"
        )),
        "{}",
        failed.message()
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{ref} fails at once instead of waiting out its 10 s timeout"
    );
    Ok(())
}

/// The fixture's iframe targets: the URLs of frames in a process of their own.
async fn iframe_targets(live: &LiveBrowser) -> TestResult<Vec<String>> {
    let targets = live
        .browser
        .cdp("Target.getTargets", json!({}), None)
        .await?;
    Ok(field(&targets, "targetInfos")
        .as_array()
        .into_iter()
        .flatten()
        .filter(|info| field(info, "type") == "iframe")
        .filter_map(|info| field(info, "url").as_str().map(str::to_owned))
        .collect())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_f1_every_frame_sits_under_its_iframe_line_with_its_own_prefix() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = live
        .open(&server.url("127.0.0.1", "/nest/top.html"))
        .await?;
    let snapshot = loaded(&live, &page).await?;
    assert_eq!(snapshot["partial"], json!([]), "every frame was read");
    let printed = printed(&snapshot["json"]);
    assert_nested(&printed, &frames(&printed)?)?;
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_f2_all_sixteen_refs_read_their_own_element_after_later_snapshots_too()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = live
        .open(&server.url("127.0.0.1", "/nest/top.html"))
        .await?;
    let first = printed(&loaded(&live, &page).await?["json"]);
    let before = frames(&first)?;
    let names: Vec<&str> = FRAMES.iter().map(|(_, name)| *name).collect();
    assert_refs_read(&live, &page, &before, &names).await?;

    // A scoped snapshot inside b1, then the page again. The ref map accumulates, so neither
    // takes a ref from an earlier snapshot away, and an unchanged element keeps its ref.
    let b1_body = locator(
        &page,
        &[attr("[title=\"b1\"s]"), enter_frame(), css("body")],
        "page.frameLocator('[title=\"b1\"]').locator('body')",
    );
    live.invoke("locator.ariaSnapshot", locator_snapshot(&b1_body, "ai"))
        .await??;
    let second = live
        .invoke("page.ariaSnapshot", page_snapshot(&page, "ai"))
        .await??;
    let after = frames(&printed(&second["json"]))?;
    for name in &names {
        assert_eq!(
            (&after[name].button, &after[name].field),
            (&before[name].button, &before[name].field),
            "{name}'s elements keep their refs across snapshots"
        );
    }
    assert_refs_read(&live, &page, &before, &names).await?;
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_f4_a_same_origin_frame_inside_a_cross_site_frame_is_read_and_its_refs_resolve()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = live
        .open(&server.url("127.0.0.1", "/nest/top.html"))
        .await?;
    let printed = printed(&loaded(&live, &page).await?["json"]);
    let frames = frames(&printed)?;

    // The layout under test: b2 is cross-site in its own process, b3 shares b2's process, and
    // a1 shares the main frame's.
    let targets = iframe_targets(&live).await?;
    let own_process = |file: &str| targets.iter().any(|url| url.ends_with(file));
    assert!(own_process("/nest/b2.html"), "b2 is an OOPIF: {targets:?}");
    assert!(!own_process("/nest/b3.html"), "b3 shares b2's process");
    assert!(!own_process("/nest/a1.html"), "a1 shares the main process");

    let chain = ["A1", "B2", "B3"];
    for pair in chain.windows(2) {
        let [outer, inner] = pair else {
            continue;
        };
        let (outer, inner) = (&frames[outer], &frames[inner]);
        assert_eq!(inner.iframes.len(), outer.iframes.len() + 1);
        assert!(
            inner.iframes.starts_with(&outer.iframes),
            "{inner:?} inside {outer:?}"
        );
    }
    assert_nested(&printed, &frames)?;
    assert_refs_read(&live, &page, &frames, &chain).await?;
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_f5_an_oopif_inside_an_oopif_four_levels_deep_is_read_and_its_refs_resolve()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = live
        .open(&server.url("127.0.0.1", "/nest/top.html"))
        .await?;
    let printed = printed(&loaded(&live, &page).await?["json"]);
    let frames = frames(&printed)?;

    // The layout under test: every level of b1 → c2 → a3 → c4 is a process of its own, a3
    // being the main frame's site inside two OOPIFs.
    let targets = iframe_targets(&live).await?;
    for file in [
        "/nest/b1.html",
        "/nest/c2.html",
        "/nest/a3.html",
        "/nest/c4.html",
    ] {
        assert!(
            targets.iter().any(|url| url.ends_with(file)),
            "{file} is an OOPIF: {targets:?}"
        );
    }

    let chain = ["B1", "C2", "A3", "C4"];
    for (level, name) in chain.iter().enumerate() {
        assert_eq!(
            frames[name].iframes.len(),
            level + 1,
            "{name} at level {}",
            level + 1
        );
    }
    for pair in chain.windows(2) {
        let [outer, inner] = pair else {
            continue;
        };
        assert!(
            frames[inner].iframes.starts_with(&frames[outer].iframes),
            "{inner} sits inside {outer}"
        );
    }
    assert_nested(&printed, &frames)?;
    assert_refs_read(&live, &page, &frames, &chain).await?;
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_f6_an_unresponsive_frame_is_partial_and_the_rest_returns_within_its_budget()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_and_hang()).await?;
    let page = live
        .open(&server.url("127.0.0.1", "/nest/top.html?hang"))
        .await?;
    loaded(&live, &page).await?;
    // hang.html spins forever from 1.5 s after its load event.
    time::sleep(Duration::from_secs(2)).await;

    let started = Instant::now();
    let snapshot = live
        .invoke("page.ariaSnapshot", page_snapshot(&page, "ai"))
        .await??;
    let took = started.elapsed();
    assert!(
        took < Duration::from_millis(2_500),
        "the budget (2 s) plus 500 ms bounds the snapshot, took {took:?}"
    );
    let partial = snapshot["partial"]
        .as_array()
        .ok_or("partial is a list")?
        .clone();
    assert_eq!(partial.len(), 1, "only the hung frame: {partial:?}");
    assert_eq!(
        partial[0]["frame"],
        json!(server.url("hang.localtest.me", "/nest/hang.html?hang"))
    );
    assert_eq!(partial[0]["reason"], json!("timeout after 2000 ms"));

    let printed = printed(&snapshot["json"]);
    let hung = partial[0]["ref"]
        .as_str()
        .ok_or("a partial frame names its iframe")?;
    let iframe = printed
        .iter()
        .find(|node| node.r#ref == hung)
        .ok_or("the hung frame's iframe keeps its ref")?;
    assert_eq!(iframe.role, "iframe");
    assert!(iframe.iframes.is_empty(), "printed by the main frame");
    assert!(
        !printed
            .iter()
            .any(|node| node.iframes.iter().any(|above| above == hung)),
        "nothing is stitched under the hung frame"
    );
    let frames = frames(&printed)?;
    assert_nested(&printed, &frames)?;
    let names: Vec<&str> = FRAMES.iter().map(|(_, name)| *name).collect();
    assert_refs_read(&live, &page, &frames, &names).await?;
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_f7_a_ref_into_a_frame_that_navigated_or_detached_fails_at_once() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = live
        .open(&server.url("127.0.0.1", "/nest/top.html"))
        .await?;
    let frames = frames(&printed(&loaded(&live, &page).await?["json"]))?;

    // a1 navigates: its document goes, and with it b2 and b3 inside it. a1 is same-origin, so
    // the main world sees its new document load.
    live.main_world(
        &page,
        "document.querySelector('iframe[title=a1]').src = '/nest/b3.html?navigated'",
    )
    .await?;
    let navigated = "document.querySelector('iframe[title=a1]').contentDocument?.querySelector('button')?.textContent === 'B3 button'";
    let mut done = Value::Null;
    for _ in 0..100 {
        done = live.main_world(&page, navigated).await?;
        if done == json!(true) {
            break;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(done, json!(true), "a1 shows its new document");
    for name in ["A1", "B2", "B3"] {
        assert_detached(&live, &page, &frames[name].button).await?;
        assert_detached(&live, &page, &frames[name].field).await?;
    }

    // b1 is removed: its frame detaches, and c2, a3 and c4 with it.
    live.main_world(&page, "document.querySelector('iframe[title=b1]').remove()")
        .await?;
    for name in ["B1", "C2", "A3", "C4"] {
        assert_detached(&live, &page, &frames[name].button).await?;
    }
    assert_refs_read(&live, &page, &frames, &["Top"]).await?;
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_ref_inside_a_hidden_iframe_is_not_visible() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = live
        .open(&server.url("127.0.0.1", "/nest/top.html"))
        .await?;
    let frames = frames(&printed(&loaded(&live, &page).await?["json"]))?;
    let state = |api: &str, name: &str| {
        let op = api.replace("page.", "locator.");
        let input = json!({ "call": { "api": api, "timeoutMs": 0 },
                            "target": by_ref(&page, &frames[name].button), "strict": "strict" });
        let live = &live;
        async move { live.invoke(&op, input).await }
    };
    for name in ["A1", "C4"] {
        assert_eq!(state("page.isVisible", name).await??, json!(true), "{name}");
    }
    // A ref jumps straight to its document, below parents it never walked: a1 is a
    // same-process child of the top, and c4 sits four levels under b1, an OOPIF.
    for title in ["a1", "b1"] {
        live.main_world(
            &page,
            &format!("document.querySelector('iframe[title={title}]').style.visibility = 'hidden'"),
        )
        .await?;
    }
    for name in ["A1", "C4"] {
        assert_eq!(
            state("page.isVisible", name).await??,
            json!(false),
            "{name}'s button is visible in its own document, but an iframe above it is not"
        );
        assert_eq!(state("page.isHidden", name).await??, json!(true), "{name}");
        let waited = live
            .invoke(
                "locator.waitFor",
                json!({ "call": { "api": "locator.waitFor", "timeoutMs": 2_000 },
                        "target": by_ref(&page, &frames[name].button), "strict": "strict",
                        "state": "hidden", "omitReturnValue": true }),
            )
            .await??;
        assert_eq!(waited, Value::Null, "{name} is hidden already");
    }
    assert_refs_read(&live, &page, &frames, &["A1", "C4"]).await?;
    live.close(&page).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_locator_snapshot_is_its_element_with_the_frames_inside_it() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let live = LiveBrowser::connect().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let page = live
        .open(&server.url("127.0.0.1", "/nest/top.html"))
        .await?;
    // Wait for the deepest frame through frame locators, so no page snapshot visits the
    // frames before the locator's own.
    let c4 = locator(
        &page,
        &[
            attr("[title=\"b1\"s]"),
            enter_frame(),
            attr("[title=\"c2\"s]"),
            enter_frame(),
            attr("[title=\"a3\"s]"),
            enter_frame(),
            attr("[title=\"c4\"s]"),
            enter_frame(),
            role("button"),
        ],
        "c4 button",
    );
    let text = live
        .invoke("locator.textContent", text_content(&c4))
        .await??;
    assert_eq!(text, json!("C4 button"));

    let b1_body = locator(
        &page,
        &[attr("[title=\"b1\"s]"), enter_frame(), css("body")],
        "page.frameLocator('[title=\"b1\"]').locator('body')",
    );
    let snapshot = live
        .invoke("locator.ariaSnapshot", locator_snapshot(&b1_body, "ai"))
        .await??;
    assert_eq!(snapshot["partial"], json!([]));
    let printed = printed(&snapshot["json"]);
    for (name, level) in [("B1", 0), ("C2", 1), ("A3", 2), ("C4", 3)] {
        let frame = frame(&printed, name)?;
        assert_eq!(frame.iframes.len(), level, "{name} below b1's body");
        assert_ne!(frame.prefix, "", "{name} is not the main frame's document");
    }
    for name in ["Top", "A1"] {
        let button = format!("{name} button");
        assert!(
            !printed
                .iter()
                .any(|node| node.name.as_deref() == Some(button.as_str())),
            "{name} is outside the locator's element"
        );
    }
    // Refs it minted in frames no page snapshot visited resolve too.
    for name in ["B1", "C4"] {
        let frame = frame(&printed, name)?;
        let text = live
            .invoke(
                "locator.textContent",
                text_content(&by_ref(&page, &frame.button)),
            )
            .await??;
        assert_eq!(text, json!(format!("{name} button")));
    }

    let top_button = locator(&page, &[role("button[name=\"Top button\"s]")], "top button");
    let plain = live
        .invoke(
            "locator.ariaSnapshot",
            locator_snapshot(&top_button, "default"),
        )
        .await??;
    assert_eq!(
        plain,
        json!({ "json": [{ "role": "button", "name": "Top button" }], "partial": [] }),
        "default mode prints no refs"
    );

    let started = Instant::now();
    let two = locator(&page, &[css("button, input")], "locator('button, input')");
    let strict = live
        .invoke("locator.ariaSnapshot", locator_snapshot(&two, "ai"))
        .await?
        .err()
        .ok_or("two matches under strict mode must fail")?;
    assert_eq!(strict.error_code(), Some(ErrorCode::StrictModeViolation));
    assert!(started.elapsed() < Duration::from_secs(2), "and at once");
    live.close(&page).await
}

/// The child frame of the late-child tests: text of its own, and a main thread that spins once
/// the top asks (`postMessage('spin')`), so a test reads it once first and its world is installed
/// before it stops answering. The spin lasts 10 s, five times the snapshot's budget for the frame,
/// and then ends, so the renderer does not stay pinned for the rest of the suite. It cannot say
/// when it started: a message to a frame in another process leaves only once the current task,
/// the spin, is done.
const LATE_CHILD: &str = r"<!doctype html><title>late</title><h1>Late child text</h1><script>
addEventListener('message', (event) => {
  if (event.data !== 'spin') return;
  const until = performance.now() + 10000;
  while (performance.now() < until) {}
});
</script>";

/// The cross-site late child's host, of a site no other test uses. A renderer left spinning stays
/// alive after its tab closes, and a later frame of its site can land in it: F6, whose stalled
/// frame is on `hang.localtest.me`, never loaded its page in 2 of 2 runs right after this test's
/// child had spun there, and passed alone (measured; the renderer's reuse is inferred). Chromium
/// resolves `*.localhost` to loopback itself, and `late.localhost` is a site of its own, apart
/// from `localhost`.
const LATE_CHILD_HOST: &str = "late.localhost";

/// The top of the late-child tests: a one-time-code field whose value the page echoes, and the
/// child frame at `src`, a JavaScript expression.
fn late_top(src: &str) -> String {
    format!(
        r#"<!doctype html><title>top</title><body>
<input id="code" aria-label="Code" autocomplete="one-time-code">
<p id="echo">Confirm your code</p>
<script>
const code = document.getElementById('code');
code.addEventListener('input', () => {{ document.getElementById('echo').textContent = 'Confirm your code ' + code.value; }});
const frame = document.createElement('iframe');
frame.title = 'late';
frame.src = {src};
document.body.append(frame);
</script></body>"#
    )
}

/// Types the code the top echoes, reads the page until the child printed its text (so its world
/// is installed), then makes the child spin.
async fn type_and_stall_child(live: &LiveBrowser, page: &TestPage) -> TestResult {
    live.main_world(
        page,
        "const c = document.getElementById('code'); c.value = '482913'; c.dispatchEvent(new Event('input'))",
    )
    .await?;
    let mut read = false;
    for _ in 0..100 {
        let snapshot = live
            .invoke("page.ariaSnapshot", page_snapshot(page, "ai"))
            .await??;
        if strings(&snapshot)
            .iter()
            .any(|text| text == "Late child text")
        {
            read = true;
            break;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    if !read {
        return Err("the child frame never printed its text".into());
    }
    live.main_world(page, "frames[0].postMessage('spin', '*')")
        .await?;
    // The message reaches the child's process and starts the spin.
    time::sleep(Duration::from_millis(500)).await;
    Ok(())
}

/// A child of another site that answers past the snapshot's 2 s budget for a frame is reported
/// partial, and none of its text comes back; the rest of the page does, the code the top echoes
/// included.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_late_cross_site_child_is_partial_and_the_rest_comes_back() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let live = LiveBrowser::connect().await?;
    let top = late_top(&format!(
        "`http://{LATE_CHILD_HOST}:${{location.port}}/late/child.html`"
    ));
    let server = FixtureServer::start(HashMap::from([
        ("/late/top.html".to_owned(), Page::html(&top)),
        (
            "/late/child.html".to_owned(),
            Page::html(LATE_CHILD).with_header("cache-control", "no-store"),
        ),
    ]))
    .await?;
    let page = live
        .open(&server.url("127.0.0.1", "/late/top.html"))
        .await?;
    type_and_stall_child(&live, &page).await?;

    let started = Instant::now();
    let snapshot = live
        .invoke("page.ariaSnapshot", page_snapshot(&page, "ai"))
        .await??;
    let took = started.elapsed();
    let partial = snapshot["partial"]
        .as_array()
        .ok_or("partial is a list")?
        .clone();
    assert_eq!(partial.len(), 1, "only the late child: {partial:?}");
    assert_eq!(
        (&partial[0]["frame"], &partial[0]["reason"]),
        (
            &json!(server.url(LATE_CHILD_HOST, "/late/child.html")),
            &json!("timeout after 2000 ms")
        ),
    );
    let text = strings(&snapshot);
    assert!(
        !text.iter().any(|text| text.contains("Late child text")),
        "the late child carries no text: {text:?}"
    );
    assert!(
        text.iter().any(|text| text == "Confirm your code 482913"),
        "the rest of the page comes back as it reads: {text:?}"
    );
    assert!(
        took < Duration::from_millis(3_000),
        "the child's 2 s budget bounds the snapshot, took {took:?}"
    );
    live.close(&page).await
}

/// A child of the page's own site that stops answering is reported partial, as a child of any
/// other site is, and the rest of the page comes back within the frame's budget.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn tests_that_a_late_same_site_child_is_partial_like_any_other() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let live = LiveBrowser::connect().await?;
    // Another origin of the page's site, asking for an origin-keyed agent cluster, gets a process
    // of its own (measured), so its spin stalls it alone.
    let child = FixtureServer::start(HashMap::from([(
        "/late/child.html".to_owned(),
        Page::html(LATE_CHILD)
            .with_header("origin-agent-cluster", "?1")
            .with_header("cache-control", "no-store"),
    )]))
    .await?;
    let top = late_top(&format!("'{}'", child.url("127.0.0.1", "/late/child.html")));
    let server = FixtureServer::start(HashMap::from([(
        "/late/top.html".to_owned(),
        Page::html(&top),
    )]))
    .await?;
    let page = live
        .open(&server.url("127.0.0.1", "/late/top.html"))
        .await?;
    type_and_stall_child(&live, &page).await?;

    let started = Instant::now();
    let snapshot = live
        .invoke(
            "page.ariaSnapshot",
            json!({ "call": { "api": "page.ariaSnapshot", "timeoutMs": 5_000 },
                    "page": page.brand(), "mode": "ai" }),
        )
        .await??;
    let took = started.elapsed();
    let partial = snapshot["partial"]
        .as_array()
        .ok_or("partial is a list")?
        .clone();
    assert_eq!(partial.len(), 1, "only the late child: {partial:?}");
    assert_eq!(partial[0]["reason"], json!("timeout after 2000 ms"));
    let text = strings(&snapshot);
    assert!(
        !text.iter().any(|text| text.contains("Late child text")),
        "the late child carries no text: {text:?}"
    );
    assert!(
        text.iter().any(|text| text == "Confirm your code 482913"),
        "the rest of the page comes back: {text:?}"
    );
    assert!(
        took < Duration::from_millis(3_000),
        "the child's 2 s budget bounds the snapshot, took {took:?}"
    );
    live.close(&page).await
}
