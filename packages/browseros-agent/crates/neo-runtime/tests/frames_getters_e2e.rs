//! Frame getters in QuickJS against same-origin, OOPIF and nested documents on a fresh browser.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "shared fixture also supports navigation and evaluation"
)]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::RunStatus;
use serde_json::json;
use tokio::time;

const ROOT: &str = r#"<!doctype html><title>Frame getters</title>
<iframe name="same" src="/same"></iframe><iframe name="remote" id="remote"></iframe>
<button id="remove" onclick="document.querySelector('#remote').remove()">Remove</button>
<script>document.querySelector('#remote').src = location.origin.replace('127.0.0.1', 'localhost') + '/remote'</script>"#;

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
let detachedError;
try { remote.url(); } catch (error) { detachedError = error.name; }
return {rows, identity, matching, detachedError,
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
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![
                ("/frame-getters".to_string(), ROOT.to_string()),
                (
                    "/same".to_string(),
                    "<iframe name='nested' src='/nested'></iframe>".to_string(),
                ),
                (
                    "/remote".to_string(),
                    "<iframe name='remote-nested' src='/remote-nested'></iframe>".to_string(),
                ),
            ],
            ..Fixture::default()
        },
        "/frame-getters",
    )
    .await;
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
            "detachedError": "Error",
            "detached": true,
            "survivors": ["", "nested", "same"],
            "sameStillAttached": true,
        })),
        "{outcome:?}"
    );
    assert_eq!(second.status, RunStatus::Ok, "{second:?}");
    assert_eq!(second.value, Some(json!(["", "nested", "same"])));
}
