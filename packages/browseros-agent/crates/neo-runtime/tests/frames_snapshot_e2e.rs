//! Stitched `page.ariaSnapshot({ mode: 'ai' })` through iframes on a real BrowserOS neo: a
//! same-origin frame, a cross-site out-of-process frame (OOPIF: `localhost` under a `127.0.0.1`
//! page), an OOPIF inside that OOPIF, and an OOPIF inside the same-origin frame. Each frame's
//! content appears under its iframe node, carrying path refs (`f4e2`); those refs fill, click
//! and read through `page.locator('aria-ref=…')` in the run that took the snapshot and in a later
//! run with fresh JS state, and a ref minted before its iframe reloaded fails without touching
//! the new document. Every frame reports its input and clicks to the top page with
//! `postMessage`, so effects are read from the page, never through Neo.
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary (`support/browser.rs`); otherwise
//! each test passes as a skip.

#[expect(
    dead_code,
    reason = "shared browser fixture includes helpers used by other suites"
)]
#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::time;

/// The top page, on `127.0.0.1`: a heading, a same-origin frame with fallback text, and a
/// cross-site frame on `localhost`, whose port only the page knows. It keeps what every frame
/// reports in `window.__log`.
const TOP: &str = r"<!doctype html><title>Top</title>
<h1>Top</h1>
<iframe id=same name=same src=/same.html width=520 height=220>Your browser does not support frames</iframe>
<iframe id=cross name=cross width=520 height=260></iframe>
<script>
window.__log = [];
addEventListener('message', (event) => window.__log.push(event.data));
document.getElementById('cross').src = 'http://localhost:' + location.port + '/cross.html';
</script>";

/// What every frame reports with: its frame's name, then what happened.
const REPORT: &str =
    "<script>const report = (what) => top.postMessage(window.name + ':' + what, '*')</script>";

/// The same-origin frame, holding a cross-site frame (`mixed`).
fn same() -> String {
    format!(
        r#"<!doctype html><title>Same</title>{REPORT}
<button onclick="report('click')">Same</button>
<iframe name=mixed width=300 height=120></iframe>
<script>document.querySelector('iframe').src = 'http://localhost:' + location.port + '/leaf.html'</script>"#
    )
}

/// The cross-site frame, holding a frame back on `127.0.0.1` (`nested`), another process again.
fn cross() -> String {
    format!(
        r#"<!doctype html><title>Cross</title>{REPORT}
<input aria-label="Email" oninput="report('input:' + this.value)">
<button onclick="report('click')">Sign in</button>
<iframe name=nested width=300 height=120></iframe>
<script>
document.querySelector('iframe').src = 'http://127.0.0.1:' + location.port + '/leaf.html';
addEventListener('load', () => report('loaded'));
</script>"#
    )
}

/// A page whose frames all share its process: `outer`, holding `inner`.
const LOCAL: &str = r"<!doctype html><title>Local</title>
<h1>Local</h1>
<iframe name=outer src=/outer.html width=520 height=260></iframe>
<script>
window.__log = [];
addEventListener('message', (event) => window.__log.push(event.data));
</script>";

/// `outer`, holding the leaf page on its own origin.
fn outer() -> String {
    format!(
        r#"<!doctype html><title>Outer</title>{REPORT}
<button onclick="report('click')">Outer</button>
<iframe name=inner src=/leaf.html width=300 height=120></iframe>"#
    )
}

/// The innermost frames' page, served to `mixed`, `nested` and `inner`.
fn leaf() -> String {
    format!(
        r#"<!doctype html><title>Leaf</title>{REPORT}
<button onclick="report('click'); document.getElementById('status').textContent = 'Leaf clicked'">Leaf</button>
<p id=status>Leaf ready</p>"#
    )
}

/// The fixture at `path`, once the page and every frame in it have loaded.
async fn open(path: &str) -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![
                ("/top.html".into(), TOP.into()),
                ("/same.html".into(), same()),
                ("/cross.html".into(), cross()),
                ("/leaf.html".into(), leaf()),
                ("/local.html".into(), LOCAL.into()),
                ("/outer.html".into(), outer()),
            ],
            ..Fixture::default()
        },
        path,
    )
    .await;
    // A page's `load` waits for its frames', so the top's means every document is in.
    wait_for(&tab, "document.readyState === 'complete'").await;
    Some(tab)
}

/// Polls `condition` in the top page's main world for up to 10 s.
async fn wait_for(tab: &Tab, condition: &str) {
    time::timeout(Duration::from_secs(10), async {
        while tab.evaluate(condition).await != json!(true) {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|error| panic!("{condition} never held: {error}"));
}

async fn run(tab: &Tab, code: &str) -> RunOutcome {
    tab.start_run(code.to_string())
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"))
}

/// The script's return value, after checking the run itself went well.
fn value(outcome: &RunOutcome) -> Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    outcome
        .value
        .clone()
        .unwrap_or_else(|| panic!("the script returns a value: {outcome:#?}"))
}

/// Waits until the top page has received `clicks` click reports, plus `input` when given: a
/// frame's `postMessage` reaches the top page after the action that sent it has returned.
async fn wait_for_reports(tab: &Tab, clicks: usize, input: Option<&str>) {
    let input = serde_json::to_string(&input).unwrap_or_else(|error| panic!("{error}"));
    wait_for(
        tab,
        &format!(
            "window.__log.filter((report) => report.endsWith(':click')).length >= {clicks}
             && ({input} === null || window.__log.includes({input}))"
        ),
    )
    .await;
}

/// What the frames reported, in the order the top page received it, without load notices.
async fn reports(tab: &Tab) -> Vec<String> {
    let log = tab.evaluate("window.__log").await;
    log.as_array()
        .unwrap_or_else(|| panic!("window.__log is {log}"))
        .iter()
        .filter_map(Value::as_str)
        .filter(|report| !report.ends_with(":loaded"))
        .map(str::to_string)
        .collect()
}

/// Finds the refs of the fixture's controls in `yaml` (an `ai` snapshot) by their text and the
/// outermost iframe they sit in, so the two `Leaf` frames are told apart. Defines `refs`.
const FIND_REFS: &str = r#"
const lines = yaml.split('\n');
const indent = (i) => lines[i].length - lines[i].trimStart().length;
const refAt = (i) => /\[ref=([a-z0-9]+)\]/.exec(lines[i] ?? '')?.[1] ?? null;
const parentOf = (i) => {
  for (let j = i - 1; j >= 0; j--) if (indent(j) < indent(i)) return j;
  return -1;
};
const outerFrame = (i) => {
  let outer = null;
  for (let j = parentOf(i); j >= 0; j = parentOf(j))
    if (/^\s*- iframe\b/.test(lines[j])) outer = refAt(j);
  return outer;
};
const at = (text, outer) =>
  lines.findIndex((line, i) => line.includes(text) && (outer === undefined || outerFrame(i) === outer));
const sameFrame = outerFrame(at('button "Same"'));
const crossFrame = outerFrame(at('textbox "Email"'));
const refs = {
  same: refAt(at('button "Same"')),
  mixedLeaf: refAt(at('button "Leaf"', sameFrame)),
  email: refAt(at('textbox "Email"')),
  signIn: refAt(at('button "Sign in"')),
  nestedLeaf: refAt(at('button "Leaf"', crossFrame)),
  nestedStatus: refAt(at('Leaf ready', crossFrame)),
};
"#;

/// The script that acts on every ref in `refs`, in one frame each, then reads back through
/// them: what [`assert_acted`] checks.
fn act(email: &str) -> String {
    format!(
        r"
const ref = (name) => page.locator('aria-ref=' + refs[name]);
await ref('email').fill('{email}');
await ref('signIn').click();
await ref('nestedLeaf').click();
await ref('same').click();
await ref('mixedLeaf').click();
const acted = {{
  email: await ref('email').inputValue(),
  status: await ref('nestedStatus').textContent(),
  signIn: await ref('signIn').textContent(),
}};
"
    )
}

/// What [`act`] returned, and the clicks and input the frames reported: each click in its own
/// frame once, and the input only in the cross-site frame.
async fn assert_acted(tab: &Tab, acted: &Value, email: &str) {
    assert_eq!(
        acted,
        &json!({ "email": email, "status": "Leaf clicked", "signIn": "Sign in" }),
        "reads through refs in an OOPIF and an OOPIF inside it"
    );
    wait_for_reports(tab, 4, Some(&format!("cross:input:{email}"))).await;
    let reports = reports(tab).await;
    // Frames in different processes report through the browser, in no fixed order.
    let mut clicks: Vec<&String> = reports
        .iter()
        .filter(|report| report.ends_with(":click"))
        .collect();
    clicks.sort();
    assert_eq!(
        clicks,
        ["cross:click", "mixed:click", "nested:click", "same:click"],
        "every ref clicked its own frame's button, once: {reports:?}"
    );
    let inputs: Vec<&String> = reports
        .iter()
        .filter(|report| report.contains(":input:"))
        .collect();
    assert!(
        inputs.iter().all(|input| input.starts_with("cross:input:")),
        "only the cross-site frame got input: {reports:?}"
    );
    assert_eq!(
        inputs.last().map(|input| input.as_str()),
        Some(format!("cross:input:{email}").as_str()),
        "{reports:?}"
    );
}

/// The `fN` prefix a node's ref takes inside the document of the iframe whose ref is
/// `iframe`: `e4` → `f4`, `f4e4` → `f4f4`.
fn path_prefix(iframe: &str) -> String {
    let (outer, number) = iframe
        .rsplit_once('e')
        .unwrap_or_else(|| panic!("{iframe} is not a ref"));
    format!("{outer}f{number}")
}

/// Checks every ref of an `ai` snapshot against the iframes it sits in: a node outside every
/// iframe has a document-local ref (`eN`), and a node inside one carries that iframe's path.
/// Returns how many nodes sat inside an iframe.
fn assert_path_refs(yaml: &str) -> usize {
    let mut open: Vec<(usize, String)> = Vec::new();
    let mut inside = 0;
    for line in yaml.lines() {
        let indent = line.len() - line.trim_start().len();
        while open.last().is_some_and(|(depth, _)| *depth >= indent) {
            open.pop();
        }
        let Some(aria_ref) = line
            .split("[ref=")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
        else {
            continue;
        };
        let (prefix, local) = match open.last() {
            Some((_, iframe)) => {
                inside += 1;
                let prefix = path_prefix(iframe);
                let local = aria_ref.strip_prefix(prefix.as_str()).unwrap_or_else(|| {
                    panic!("{line:?} sits in iframe {iframe} but its ref lacks {prefix}:\n{yaml}")
                });
                (prefix, local)
            }
            None => (String::new(), aria_ref),
        };
        assert!(
            local.strip_prefix('e').is_some_and(|number| {
                !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
            }),
            "{line:?}: {aria_ref} is not {prefix}eN:\n{yaml}"
        );
        if line.trim_start().starts_with("- iframe ") {
            open.push((indent, aria_ref.to_string()));
        }
    }
    inside
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_an_ai_snapshot_shows_every_frames_content_under_its_iframe() {
    let Some(tab) = open("/top.html").await else {
        return;
    };
    let outcome = run(
        &tab,
        &format!(
            "const yaml = await page.ariaSnapshot({{ mode: 'ai' }});
             {FIND_REFS}
             return {{ yaml, refs, plain: await page.ariaSnapshot() }};"
        ),
    )
    .await;
    let result = value(&outcome);
    tab.close().await;
    let yaml = result["yaml"].as_str().unwrap_or_default();
    let refs = &result["refs"];
    for (name, aria_ref) in refs.as_object().into_iter().flatten() {
        assert!(aria_ref.is_string(), "no ref for {name} in:\n{yaml}");
    }
    assert!(
        assert_path_refs(yaml) >= 12,
        "every frame's nodes are in the snapshot:\n{yaml}"
    );
    let reference = |name: &str| refs[name].as_str().unwrap_or_default().to_string();
    let depth = |name: &str| reference(name).matches('f').count();
    assert_eq!(
        (depth("same"), depth("email"), depth("signIn")),
        (1, 1, 1),
        "the same-origin and the cross-site frame's controls sit one frame down:\n{yaml}"
    );
    assert_eq!(
        (
            depth("mixedLeaf"),
            depth("nestedLeaf"),
            depth("nestedStatus")
        ),
        (2, 2, 2),
        "and their frames' controls two:\n{yaml}"
    );
    assert_ne!(
        reference("mixedLeaf").split('f').nth(1),
        reference("nestedLeaf").split('f').nth(1),
        "the two Leaf documents sit under different top-level iframes:\n{yaml}"
    );
    assert!(
        !yaml.contains("Your browser does not support frames"),
        "a stitched iframe drops its fallback text, as Playwright's does:\n{yaml}"
    );

    let plain = result["plain"].as_str().unwrap_or_default();
    assert!(
        plain.contains("- iframe: Your browser does not support frames")
            && !plain.contains("Same")
            && !plain.contains("Email"),
        "the default mode has no refs to enter frames by and prints them as the page gave them, \
         as Playwright's does:\n{plain}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_refs_inside_frames_act_in_the_run_that_took_the_snapshot() {
    let Some(tab) = open("/top.html").await else {
        return;
    };
    let outcome = run(
        &tab,
        &format!(
            "const yaml = await page.ariaSnapshot({{ mode: 'ai' }});
             {FIND_REFS}
             {act}
             return acted;",
            act = act("alice@example.com")
        ),
    )
    .await;
    let acted = value(&outcome);
    assert_acted(&tab, &acted, "alice@example.com").await;
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_refs_inside_frames_act_in_a_later_run_with_fresh_js_state() {
    let Some(tab) = open("/top.html").await else {
        return;
    };
    let first = run(
        &tab,
        &format!(
            "const yaml = await page.ariaSnapshot({{ mode: 'ai' }});
             {FIND_REFS}
             return refs;"
        ),
    )
    .await;
    let refs = value(&first);
    assert!(reports(&tab).await.is_empty(), "a snapshot touches nothing");

    // A new run: a new realm, new sessions and new OOPIF attachments. Only the refs carry over,
    // as text, the way an agent hands them from one run to the next.
    let second = run(
        &tab,
        &format!(
            "const refs = {refs};
             {act}
             return acted;",
            act = act("bob@example.com")
        ),
    )
    .await;
    let acted = value(&second);
    assert_acted(&tab, &acted, "bob@example.com").await;
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_ref_from_before_its_iframe_reloaded_fails_and_touches_nothing() {
    let Some(tab) = open("/top.html").await else {
        return;
    };
    let first = run(
        &tab,
        &format!(
            "const yaml = await page.ariaSnapshot({{ mode: 'ai' }});
             {FIND_REFS}
             return refs;"
        ),
    )
    .await;
    let refs = value(&first);

    tab.evaluate("const cross = document.getElementById('cross'); cross.src = cross.src")
        .await;
    wait_for(
        &tab,
        "window.__log.filter((report) => report === 'cross:loaded').length === 2",
    )
    .await;

    let stale = run(
        &tab,
        &format!(
            "const refs = {refs};
             const ref = (name) => page.locator('aria-ref=' + refs[name]);
             const failed = (error) => ({{ failed: error.name }});
             return {{
               fill: await ref('email').fill('mallory@example.com', {{ timeout: 1000 }})
                 .then(() => 'filled', failed),
               click: await ref('signIn').click({{ timeout: 1000 }}).then(() => 'clicked', failed),
               read: await ref('nestedStatus').textContent({{ timeout: 1000 }}).catch(failed),
               email: await page
                 .locator('#cross >> internal:control=enter-frame >> internal:label=\"Email\"i')
                 .inputValue(),
             }};"
        ),
    )
    .await;
    let result = value(&stale);
    for action in ["fill", "click", "read"] {
        assert!(
            result[action]["failed"]
                .as_str()
                .is_some_and(|name| name == "TimeoutError" || name == "Error"),
            "{action} through a ref of the old document must fail: {result}"
        );
    }
    assert_eq!(
        result["email"], "",
        "the new document's field is untouched: {result}"
    );
    assert!(
        reports(&tab).await.is_empty(),
        "no frame saw input or a click"
    );

    // A fresh snapshot mints refs for the new document, and they act.
    let fresh = run(
        &tab,
        &format!(
            "const yaml = await page.ariaSnapshot({{ mode: 'ai' }});
             {FIND_REFS}
             await page.locator('aria-ref=' + refs.email).fill('carol@example.com');
             return await page.locator('aria-ref=' + refs.email).inputValue();"
        ),
    )
    .await;
    assert_eq!(value(&fresh), "carol@example.com");
    wait_for_reports(&tab, 0, Some("cross:input:carol@example.com")).await;
    assert_eq!(
        reports(&tab).await,
        ["cross:input:carol@example.com"],
        "only the new document's field took the input"
    );
    tab.close().await;
}

/// Same-process frames alone: no frame here is a target of its own, so this case separates
/// stitching and path refs from the OOPIF hosts the other cases also need.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_same_process_frame_refs_act_in_a_later_run() {
    let Some(tab) = open("/local.html").await else {
        return;
    };
    let first = run(
        &tab,
        r#"const yaml = await page.ariaSnapshot({ mode: 'ai' });
           const ref = (text) =>
             /\[ref=([a-z0-9]+)\]/.exec(yaml.split('\n').find((line) => line.includes(text)) ?? '')?.[1] ?? null;
           return { yaml, outer: ref('button "Outer"'), leaf: ref('button "Leaf"'), status: ref('Leaf ready') };"#,
    )
    .await;
    let refs = value(&first);
    let yaml = refs["yaml"].as_str().unwrap_or_default();
    assert!(
        assert_path_refs(yaml) >= 4,
        "both frames are stitched:\n{yaml}"
    );
    let depth = |name: &str| {
        refs[name]
            .as_str()
            .map(|aria_ref| aria_ref.matches('f').count())
    };
    assert_eq!(
        (depth("outer"), depth("leaf"), depth("status")),
        (Some(1), Some(2), Some(2)),
        "{yaml}"
    );

    let second = run(
        &tab,
        &format!(
            "const refs = {refs};
             await page.locator('aria-ref=' + refs.outer).click();
             await page.locator('aria-ref=' + refs.leaf).click();
             return await page.locator('aria-ref=' + refs.status).textContent();"
        ),
    )
    .await;
    assert_eq!(value(&second), "Leaf clicked");
    wait_for_reports(&tab, 2, None).await;
    let mut reports = reports(&tab).await;
    reports.sort();
    assert_eq!(
        reports,
        ["inner:click", "outer:click"],
        "each ref clicked its own frame"
    );
    tab.close().await;
}

/// Neo: A scoped main-frame section contains a remote form, with decoys outside the section.
const SCOPED_TOP: &str = r#"<!doctype html><title>Scoped frames</title>
<h1>Parent</h1><label>Email<input id=parent-email value=parent></label>
<button onclick="document.getElementById('parent-status').textContent='changed'">Outside action</button>
<output id=parent-status>idle</output>
<main id=section><h2>Account</h2><iframe id=embedded width=520 height=260></iframe></main>
<script>
window.__log=[];
addEventListener('message',event=>window.__log.push(event.data));
document.getElementById('embedded').src='http://localhost:'+location.port+'/snapshot-form.html';
</script>"#;

/// Neo: Snapshotting the form excludes its sibling text; only a ref click changes its output.
const SCOPED_CHILD: &str = r#"<!doctype html><title>Scoped child</title>
<form aria-label="Remote account"><label>Email<input value=seed></label>
<button type=button onclick="document.getElementById('result').textContent='Remote clicked';parent.postMessage('scoped:click','*')">Remote action</button></form>
<p id=result>idle</p><aside>Excluded child text</aside>"#;

async fn open_scoped() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![
                ("/scoped.html".into(), SCOPED_TOP.into()),
                ("/snapshot-form.html".into(), SCOPED_CHILD.into()),
            ],
            ..Fixture::default()
        },
        "/scoped.html",
    )
    .await;
    wait_for(&tab, "document.readyState === 'complete'").await;
    Some(tab)
}

async fn assert_scoped_parent_untouched(tab: &Tab) {
    assert_eq!(
        tab.evaluate("[document.getElementById('parent-email').value,document.getElementById('parent-status').textContent]").await,
        json!(["parent", "idle"]),
        "the parent decoy controls keep their values",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_scoped_default_snapshots_enter_an_oopif_without_touching_the_parent() {
    let Some(tab) = open_scoped().await else {
        return;
    };
    let outcome = run(
        &tab,
        r"
        return await page.frameLocator('#embedded').locator('form').ariaSnapshot();
    ",
    )
    .await;
    let snapshot = value(&outcome);
    let yaml = snapshot.as_str().unwrap();
    assert!(
        yaml.contains("Remote account")
            && yaml.contains("textbox \"Email\"")
            && yaml.contains("button \"Remote action\""),
        "{yaml}"
    );
    assert!(
        !yaml.contains("Parent")
            && !yaml.contains("Outside")
            && !yaml.contains("Excluded")
            && !yaml.contains("[ref="),
        "{yaml}"
    );
    assert_scoped_parent_untouched(&tab).await;
    assert!(reports(&tab).await.is_empty());
    let targets = tab
        .cdp
        .send(
            "Target.getTargets",
            json!({"filter":[{"type":"iframe"}]}),
            None,
        )
        .await
        .unwrap();
    assert!(
        targets["targetInfos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["type"] == "iframe"
                && target["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with("/snapshot-form.html"))),
        "{targets}"
    );
    tab.close().await;
}

/// A scoped AI snapshot inside a child frame answers refs behind the iframe's path from the main
/// frame, as Playwright's answers page-wide refs: its button's ref clicks that button, and the
/// parent's refs from an earlier page snapshot keep resolving.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_scoped_ai_snapshots_inside_a_child_frame_answer_page_wide_refs() {
    let Some(tab) = open_scoped().await else {
        return;
    };
    let outcome = run(&tab, r#"
        const refOf = (yaml, text) => /\[ref=([a-z0-9]+)\]/.exec(yaml.split('\n').find(line=>line.includes(text)) ?? '')?.[1];
        const full = await page.ariaSnapshot({mode:'ai'});
        const outside = refOf(full, 'button "Outside action"');
        const iframe = refOf(full, '- iframe ');
        const yaml = await page.frameLocator('#embedded').locator('form').ariaSnapshot({mode:'ai'});
        const remote = refOf(yaml, 'button "Remote action"');
        if (!remote) throw new Error('The scoped snapshot omitted the remote control');
        await page.locator('aria-ref='+remote).click();
        return {
          yaml, iframe,
          outside: await page.locator('aria-ref='+outside).count(),
          result: await page.frameLocator('#embedded').locator('#result').textContent(),
        };
    "#).await;
    let result = value(&outcome);
    let yaml = result["yaml"].as_str().unwrap();
    assert!(
        yaml.contains("Remote account") && !yaml.contains("Excluded child text"),
        "{yaml}"
    );
    let prefix = path_prefix(result["iframe"].as_str().unwrap());
    for line in yaml.lines().filter(|line| line.contains("[ref=")) {
        assert!(
            line.contains(&format!("[ref={prefix}e")),
            "{line:?} lacks the frame's path {prefix}:\n{yaml}"
        );
    }
    assert_eq!(
        result["outside"], 1,
        "the parent's refs keep their lifetime"
    );
    assert_eq!(result["result"], "Remote clicked");
    assert_scoped_parent_untouched(&tab).await;
    wait_for(&tab, "window.__log.includes('scoped:click')").await;
    assert_eq!(reports(&tab).await, ["scoped:click"]);
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_scoped_main_ai_snapshots_stitch_frames_and_their_refs_act() {
    let Some(tab) = open_scoped().await else {
        return;
    };
    let outcome = run(&tab, r#"
        const yaml = await page.locator('#section').ariaSnapshot({mode:'ai'});
        const line = yaml.split('\n').find(line=>line.includes('button "Remote action"'));
        const ref = /\[ref=([a-z0-9]+)\]/.exec(line ?? '')?.[1];
        if (!ref) throw new Error('The scoped snapshot omitted the remote control');
        await page.locator('aria-ref='+ref).click();
        return {yaml, ref, result:await page.frameLocator('#embedded').locator('#result').textContent()};
    "#).await;
    let result = value(&outcome);
    let yaml = result["yaml"].as_str().unwrap();
    assert!(
        yaml.contains("Account") && !yaml.contains("Outside action"),
        "{yaml}"
    );
    assert!(assert_path_refs(yaml) > 0, "{yaml}");
    assert_eq!(result["result"], "Remote clicked");
    assert_scoped_parent_untouched(&tab).await;
    wait_for(&tab, "window.__log.includes('scoped:click')").await;
    assert_eq!(reports(&tab).await, ["scoped:click"]);
    tab.close().await;
}

/// Neo: a `visibility: hidden` same-origin iframe and a hidden cross-site (out-of-process) one,
/// each holding a button that reports its clicks to the top page. Neither iframe gets a ref.
const HIDDEN_TOP: &str = r#"<!doctype html><title>Hidden frames</title>
<h1>Hidden frames</h1>
<iframe name=local style="visibility:hidden" srcdoc="<button onclick=&quot;top.postMessage('local:click','*')&quot;>Local</button>"></iframe>
<iframe name=remote style="visibility:hidden"></iframe>
<script>
window.__log=[];
addEventListener('message',event=>window.__log.push(event.data));
document.querySelector('[name=remote]').src='http://localhost:'+location.port+'/hidden-remote.html';
</script>"#;

/// Neo: the hidden cross-site frame's page.
const HIDDEN_REMOTE: &str = r#"<!doctype html><title>Hidden remote</title>
<button onclick="top.postMessage('remote:click','*')">Remote</button>"#;

async fn open_hidden() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![
                ("/hidden.html".into(), HIDDEN_TOP.into()),
                ("/hidden-remote.html".into(), HIDDEN_REMOTE.into()),
            ],
            ..Fixture::default()
        },
        "/hidden.html",
    )
    .await;
    wait_for(&tab, "document.readyState === 'complete'").await;
    Some(tab)
}

/// The frame id an identity ref names: `fi`, 32 lowercase hex digits, then a local `eN`.
fn identity_frame(aria_ref: &str) -> &str {
    let rest = aria_ref
        .strip_prefix("fi")
        .filter(|rest| rest.len() > 32)
        .unwrap_or_else(|| panic!("{aria_ref} does not name a frame by identity"));
    let (id, element) = rest.split_at(32);
    assert!(
        id.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{aria_ref}: {id} is not a lowercase frame id"
    );
    assert!(
        element
            .strip_prefix('e')
            .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())),
        "{aria_ref} does not end in eN"
    );
    id
}

/// An `ai` snapshot inside a frame behind an iframe that gets no ref names that frame by its
/// identity, and the refs act in a later run with fresh JS state, into a same-process frame and an
/// out-of-process one alike, each in its own frame.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_refs_inside_hidden_frames_name_their_frame_and_act_in_a_later_run() {
    let Some(tab) = open_hidden().await else {
        return;
    };
    let first = run(
        &tab,
        r"
        const refOf = (yaml) => /\[ref=([a-z0-9]+)\]/.exec(yaml)?.[1] ?? null;
        await page.frameLocator('[name=remote]').locator('button').waitFor({ state: 'attached' });
        return {
          local: refOf(await page.frame('local').locator('button').ariaSnapshot({ mode: 'ai' })),
          remote: refOf(await page.frame('remote').locator('button').ariaSnapshot({ mode: 'ai' })),
        };
    ",
    )
    .await;
    let refs = value(&first);
    let frame_of = |name: &str| {
        identity_frame(
            refs[name]
                .as_str()
                .unwrap_or_else(|| panic!("no {name} ref: {refs}")),
        )
        .to_string()
    };
    assert_ne!(frame_of("local"), frame_of("remote"), "{refs}");
    assert!(reports(&tab).await.is_empty(), "a snapshot touches nothing");

    // A new run: only the refs carry over, as text, the way an agent hands them on.
    let second = run(
        &tab,
        &format!(
            "const refs = {refs};
             const ref = (name) => page.locator('aria-ref=' + refs[name]);
             await ref('local').dispatchEvent('click');
             await ref('remote').dispatchEvent('click');
             return {{ local: await ref('local').textContent(), remote: await ref('remote').textContent() }};"
        ),
    )
    .await;
    assert_eq!(
        value(&second),
        json!({ "local": "Local", "remote": "Remote" })
    );
    wait_for_reports(&tab, 2, None).await;
    let mut clicks = reports(&tab).await;
    clicks.sort();
    assert_eq!(
        clicks,
        ["local:click", "remote:click"],
        "each ref reached its own frame's button, once"
    );
    tab.close().await;
}
