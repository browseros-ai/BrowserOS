//! Frame getters in QuickJS against same-origin, OOPIF and nested documents on a fresh browser.
#![expect(
    clippy::disallowed_methods,
    reason = "the test drives an independent CDP session as the browser oracle"
)]

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "shared fixture also supports navigation and evaluation"
)]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::RunStatus;
use serde_json::{Value, json};
use tokio::time;

const ROOT: &str = r#"<!doctype html><title>Frame getters</title>
<iframe name="same" id="same" src="/same"></iframe><iframe name="remote" id="remote"></iframe>
<button id="remove" onclick="document.querySelector('#remote').remove()">Remove</button>
<button id="navigate-same" onclick="document.querySelector('#same').src='/same-next'">Navigate same</button><p id="nav-status">waiting</p>
<script>
addEventListener('message', event => { if (event.data === 'same-ready') document.querySelector('#nav-status').textContent = 'same-ready'; });
document.querySelector('#remote').src = location.origin.replace('127.0.0.1', 'localhost') + '/remote';
</script>"#;

const SCRIPT: &str = r"
const deadline = Date.now() + 10000;
let frames;
do {
  frames = page.frames();
  if (frames.length === 5 && frames.every(frame => frame.url() !== 'about:blank')) break;
  await new Promise(resolve => setTimeout(resolve, 10));
} while (Date.now() < deadline);
if (frames.length !== 5) throw new Error('Expected all five frames');
const main = page.mainFrame();
const same = page.frame('same');
const nested = page.frame('nested');
const remote = page.frame('remote');
const remoteNested = page.frame('remote-nested');
const rows = frames.map(frame => ({
  name: frame.name(),
  url: frame.url(),
  parent: frame.parentFrame()?.name() ?? null,
})).sort((a, b) => a.name.localeCompare(b.name));
const identity = frames[0] === main && page.frames().every((frame, index) => frame === frames[index]) &&
  same.parentFrame() === main && nested.parentFrame() === same &&
  remote.parentFrame() === main && remoteNested.parentFrame() === remote &&
  main.childFrames().includes(same) && main.childFrames().includes(remote) &&
  same.childFrames()[0] === nested && remote.childFrames()[0] === remoteNested &&
  frames.every(frame => frame.page() === page && !frame.isDetached());
const matching = page.frame({url: '**/remote'}) === remote &&
  page.frame({url: /\/remote$/g}) === remote &&
  page.frame({url: /\/remote$/g}) === remote &&
  page.frame({url: page.url().replace('http:', 'HTTP:')}) === main &&
  page.frame({name: 'same', url: '**/absent'}) === same && page.frame('sam') === null;
await page.click('#remove');
while (!remote.isDetached() && Date.now() < deadline)
  await new Promise(resolve => setTimeout(resolve, 10));
return {rows, identity, matching,
  detachedGetters: remote.url() === rows.find(frame => frame.name === 'remote').url && remote.name() === 'remote' && remote.parentFrame() === main && remote.childFrames().length === 0,
  detached: remote.isDetached() && remoteNested.isDetached(),
  survivors: page.frames().map(frame => frame.name()).sort(),
  sameStillAttached: !same.isDetached() && same.childFrames()[0] === nested,
};
";

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_frame_getters_keep_identity_and_track_oopif_removal() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, fixture(), "/frame-getters").await;
    let origin = tab.origin.clone();
    let remote_origin = origin.replace("127.0.0.1", "localhost");
    let remote_url = format!("{remote_origin}/remote");
    let mut oopif = false;
    for _ in 0..100 {
        let targets = tab
            .cdp
            .send("Target.getTargets", json!({}), None)
            .await
            .unwrap();
        oopif = targets["targetInfos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["type"] == "iframe" && target["url"] == remote_url);
        if oopif {
            break;
        }
        time::sleep(Duration::from_millis(20)).await;
    }
    assert!(oopif, "the remote frame must be a real iframe target");

    let outcome = tab.start_run(SCRIPT.to_string()).await.await.unwrap();
    let second = tab
        .start_run("return page.frames().map(frame => frame.name()).sort()".to_string())
        .await
        .await
        .unwrap();
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value,
        Some(json!({
            "rows": [
                {"name": "", "url": format!("{origin}/frame-getters"), "parent": null},
                {"name": "nested", "url": format!("{origin}/nested"), "parent": "same"},
                {"name": "remote", "url": remote_url, "parent": ""},
                {"name": "remote-nested", "url": format!("{remote_origin}/remote-nested"), "parent": "remote"},
                {"name": "same", "url": format!("{origin}/same"), "parent": ""},
            ],
            "identity": true,
            "matching": true,
            "detachedGetters": true,
            "detached": true,
            "survivors": ["", "nested", "same"],
            "sameStillAttached": true,
        })),
        "{outcome:?}"
    );
    assert_eq!(second.status, RunStatus::Ok, "{second:?}");
    assert_eq!(second.value, Some(json!(["", "nested", "same"])));
}

/// Neo: The same observed frame graph is retained by each closed-page getter case.
const RETAINED_SETUP: &str = r"
const until = Date.now() + 10000;
let frames;
do {
  frames = page.frames();
  if (frames.length === 5 && frames.every(frame => !frame.url().endsWith('about:blank'))) break;
  await new Promise(resolve => setTimeout(resolve, 10));
} while (Date.now() < until);
if (frames.length !== 5) throw new Error('Expected all five frames');
const main = page.mainFrame();
const same = page.frame('same');
const nested = page.frame('nested');
const remote = page.frame('remote');
const remoteNested = page.frame('remote-nested');
const retained = [main, same, nested, remote, remoteNested];
";

async fn after_close(source: &str) -> Option<(Value, String)> {
    let binary = browser::e2e_browser()?;
    let tab = Tab::open(binary, fixture(), "/frame-getters").await;
    let origin = tab.origin.clone();
    let outcome = tab
        .start_run(format!("{RETAINED_SETUP}\n{source}"))
        .await
        .await
        .unwrap_or_else(|error| panic!("getter run task failed: {error}"));
    tab.close().await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    let value = outcome
        .value
        .unwrap_or_else(|| panic!("getter returned no value"));
    Some((value, origin))
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_retained_frame_names_survive_page_close() {
    let Some((value, _)) =
        after_close("await page.close(); return retained.map(frame => frame.name());").await
    else {
        return;
    };
    assert_eq!(
        value,
        json!(["", "same", "nested", "remote", "remote-nested"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_retained_frame_urls_survive_page_close() {
    let Some((value, origin)) =
        after_close("await page.close(); return retained.map(frame => frame.url());").await
    else {
        return;
    };
    let remote = origin.replace("127.0.0.1", "localhost");
    assert_eq!(
        value,
        json!([
            format!("{origin}/frame-getters"),
            format!("{origin}/same"),
            format!("{origin}/nested"),
            format!("{remote}/remote"),
            format!("{remote}/remote-nested"),
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_the_last_observed_main_frame_url_survives_page_close() {
    let Some((value, _)) = after_close(
        r"
await page.goto(page.url().slice(0, page.url().lastIndexOf('/')) + '/new-root');
const observed = main.url();
await page.close();
return observed.endsWith('/new-root') && main.url() === observed;
",
    )
    .await
    else {
        return;
    };
    assert_eq!(value, true);
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_main_document_commit_detaches_retained_children_before_close() {
    let Some((value, _)) = after_close(
        r"
await page.goto(page.url().slice(0, page.url().lastIndexOf('/')) + '/new-root');
await page.close();
return {
  detached: retained.slice(1).every(frame => frame.isDetached()),
  kept: same.name() === 'same' && same.url().endsWith('/same') &&
    nested.name() === 'nested' && nested.url().endsWith('/nested'),
  parents: same.parentFrame() === main && nested.parentFrame() === same,
  children: retained.every(frame => frame.childFrames().length === 0),
  main: !main.isDetached() && main.url().endsWith('/new-root'),
};
",
    )
    .await
    else {
        return;
    };
    assert_eq!(
        value,
        json!({"detached":true,"kept":true,"parents":true,"children":true,"main":true})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_same_document_navigation_preserves_retained_children_after_close() {
    let Some((value, _)) = after_close(
        r"
await page.goto(page.url() + '#same-doc');
await page.close();
return {
  attached: retained.every(frame => !frame.isDetached()),
  parents: same.parentFrame() === main && nested.parentFrame() === same,
  children: main.childFrames().length === 2 && same.childFrames()[0] === nested,
  main: main.url().endsWith('#same-doc'),
};
",
    )
    .await
    else {
        return;
    };
    assert_eq!(
        value,
        json!({"attached":true,"parents":true,"children":true,"main":true})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_child_document_commit_detaches_its_descendants_before_close() {
    let Some((value, _)) = after_close(
        r"
await page.click('#navigate-same');
await page.getByText('same-ready', {exact:true}).waitFor();
await page.close();
return {
  detached: nested.isDetached(),
  kept: nested.name() === 'nested' && nested.url().endsWith('/nested'),
  parent: nested.parentFrame() === same,
  children: same.childFrames().length === 0 && main.childFrames().includes(same),
  other: !remote.isDetached() && remote.childFrames()[0] === remoteNested,
  same: !same.isDetached() && same.name() === 'same' && same.url().endsWith('/same-next'),
};
",
    )
    .await
    else {
        return;
    };
    assert_eq!(
        value,
        json!({"detached":true,"kept":true,"parent":true,"children":true,"other":true,"same":true})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_page_close_does_not_detach_retained_frames() {
    let Some((value, _)) =
        after_close("await page.close(); return retained.map(frame => frame.isDetached());").await
    else {
        return;
    };
    assert_eq!(value, json!([false, false, false, false, false]));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_retained_frame_parents_keep_identity_after_page_close() {
    let Some((value, _)) = after_close(
        r"
await page.close();
return main.parentFrame() === null && same.parentFrame() === main &&
  nested.parentFrame() === same && remote.parentFrame() === main &&
  remoteNested.parentFrame() === remote;
",
    )
    .await
    else {
        return;
    };
    assert_eq!(value, true);
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_retained_frame_children_keep_identity_after_page_close() {
    let Some((value, _)) = after_close(
        r"
await page.close();
return main.childFrames().length === 2 && main.childFrames()[0] === same &&
  main.childFrames()[1] === remote && same.childFrames()[0] === nested &&
  remote.childFrames()[0] === remoteNested && nested.childFrames().length === 0 &&
  remoteNested.childFrames().length === 0;
",
    )
    .await
    else {
        return;
    };
    assert_eq!(value, true);
}

async fn detach_then_close(refresh: bool) {
    let source = format!(
        r"
await page.click('#remove');
{}
await page.close();
return {{
  detached: remote.isDetached() && remoteNested.isDetached(),
  parents: remote.parentFrame() === main && remoteNested.parentFrame() === remote,
  children: remote.childFrames().length === 0 && remoteNested.childFrames().length === 0 &&
    main.childFrames().length === 1 && main.childFrames()[0] === same,
  kept: remote.name() === 'remote' && remote.url().endsWith('/remote') &&
    remoteNested.name() === 'remote-nested' && remoteNested.url().endsWith('/remote-nested'),
  surviving: !same.isDetached() && !nested.isDetached(),
}};
",
        if refresh {
            "if (!remote.isDetached()) throw new Error('Expected real detach before close');"
        } else {
            ""
        }
    );
    let Some((value, _)) = after_close(&source).await else {
        return;
    };
    assert_eq!(
        value,
        json!({
            "detached": true, "parents": true, "children": true,
            "kept": true, "surviving": true,
        }),
        "real detach must preserve metadata and survive page closure"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_observed_detach_survives_page_close() {
    detach_then_close(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_detach_after_the_last_getter_is_not_revived_by_page_close() {
    detach_then_close(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_retained_metadata_does_not_allow_closed_frame_actions() {
    let Some((value, _)) = after_close(
        r"
await page.close();
const name = remote.name();
try { await remote.title(); return false; }
catch (error) { return name === 'remote' && error.name === 'Error'; }
",
    )
    .await
    else {
        return;
    };
    assert_eq!(value, true);
}

fn fixture() -> Fixture {
    Fixture {
        pages: vec![
            ("/frame-getters".to_string(), ROOT.to_string()),
            (
                "/same".to_string(),
                "<iframe name='nested' src='/nested'></iframe>".to_string(),
            ),
            (
                "/same-next".to_string(),
                "<p>New document</p><script>parent.postMessage('same-ready','*')</script>"
                    .to_string(),
            ),
            (
                "/remote".to_string(),
                "<iframe name='remote-nested' src='/remote-nested'></iframe>".to_string(),
            ),
        ],
        ..Fixture::default()
    }
}
