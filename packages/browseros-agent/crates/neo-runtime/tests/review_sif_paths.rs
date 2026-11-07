//! Review check for the run artifact boundary during a directory upload.
#[path = "support/browser.rs"]
#[expect(dead_code, reason = "shared real-browser fixture")]
mod browser;

use std::fs;
use std::os::unix::fs as unix_fs;
use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{PageId, RunRequest, Runtime};
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "multi_thread")]
async fn directory_upload_never_reads_through_an_outside_symlink() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let root = tempfile::tempdir().expect("test directory");
    let artifacts = root.path().join("artifacts");
    let tree = artifacts.join("tree");
    fs::create_dir_all(&tree).expect("tree");
    fs::write(tree.join("inside.txt"), "INSIDE").expect("inside");
    let outside = root.path().join("outside.txt");
    fs::write(&outside, "OUTSIDE").expect("outside fixture");
    unix_fs::symlink(&outside, tree.join("escape.txt")).expect("link");
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![(
                "/upload".into(),
                "<input id=folder type=file webkitdirectory>".into(),
            )],
            ..Fixture::default()
        },
        "/upload",
    )
    .await;
    let runtime = Runtime::connect(tab.cdp.clone()).await.expect("runtime");
    let outcome = runtime.run(RunRequest {
        code: "const upload=await page.locator('#folder').setInputFiles('tree').then(()=> 'resolved',e=>e.name); return {upload,files:await page.evaluate(async()=>Promise.all([...document.querySelector('input').files].map(async f=>({name:f.name,text:await f.text()}))))};".into(),
        page: PageId::new(tab.target_id.clone()),
        timeout: Duration::from_secs(15),
        cancel: CancellationToken::new(),
        session: None,
        artifacts: Some(artifacts),
    }).await;
    runtime.drain().await;
    tab.close().await;
    let value = outcome
        .value
        .clone()
        .unwrap_or_else(|| panic!("the script returned no upload observation: {outcome:?}"));
    assert!(
        value["files"]
            .as_array()
            .expect("files")
            .iter()
            .all(|file| file["text"] != json!("OUTSIDE")),
        "outside file reached the page: {value}"
    );
}
