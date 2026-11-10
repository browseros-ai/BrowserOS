//! Download reports on a real BrowserOS neo (`src/browser_representations/downloads.rs`, `src/chromium_bridge/browser_connection_settings.rs`): a run that
//! listens for downloads gets the browser's own file, in the profile's download folder, and once
//! the last listening run of a `Runtime` ends the browser handles downloads as it did before: the
//! user's next download lands in that folder under the browser's name, and the connection hears
//! of it no more. While another run of the same `Runtime` still listens, the reports stay on.
//!
//! The fixture's `Runtime` shares the test's CDP client, so the test sees on it every report the
//! runs turned on.
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the test drives an independent CDP session as the browser oracle"
)]

#[expect(
    dead_code,
    reason = "the shared fixture also provides navigation helpers"
)]
#[path = "support/browser.rs"]
mod browser;

use std::fs;
use std::path::Path;
use std::time::Duration;

use browser::{Fixture, Tab};
use browseros_cdp::CdpEvent;
use neo_runtime::{PageId, RunOutcome, RunRequest, RunStatus, Runtime};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

const PAGE: &str = r#"<!doctype html><html><body><a id="data" href="data:text/plain,hello%20data" download="data.txt">Data</a></body></html>"#;

/// How long the test waits for a download report that must not come.
const QUIET: Duration = Duration::from_millis(700);

/// How long a download of the fixture's few bytes may take to land on disk.
const SAVED_WITHIN: Duration = Duration::from_secs(10);

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: vec![("/download.html".into(), PAGE.into())],
                ..Fixture::default()
            },
            "/download.html",
        )
        .await,
    )
}

fn request(target: &str, code: &str) -> RunRequest {
    RunRequest {
        code: code.to_string(),
        page: PageId::new(target.to_string()),
        timeout: Duration::from_secs(30),
        cancel: CancellationToken::new(),
        artifacts: None,
    }
}

fn value(outcome: &RunOutcome) -> &Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    outcome.value.as_ref().unwrap_or(&Value::Null)
}

/// Clicks the link with a user gesture over the test's own session, as the user would.
async fn click_link(tab: &Tab) {
    tab.cdp
        .send(
            "Runtime.evaluate",
            json!({ "expression": "document.querySelector('#data').click()", "userGesture": true }),
            Some(&tab.driver),
        )
        .await
        .unwrap_or_else(|error| panic!("click the link: {error}"));
}

/// The finished files in `folder`: the browser writes a download as `.crdownload` first.
fn saved_files(folder: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.ends_with(".crdownload"))
        .collect();
    names.sort();
    names
}

/// Waits until no download is still being written in `folder`.
async fn settle(folder: &Path) {
    let deadline = Instant::now() + SAVED_WITHIN;
    while Instant::now() < deadline {
        let writing = fs::read_dir(folder).is_ok_and(|entries| {
            entries
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().ends_with(".crdownload"))
        });
        if !writing {
            return;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits until `folder` holds `count` finished files, and returns them.
async fn wait_for_files(folder: &Path, count: usize) -> Vec<String> {
    let deadline = Instant::now() + SAVED_WITHIN;
    loop {
        let names = saved_files(folder);
        if names.len() >= count || Instant::now() > deadline {
            return names;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
}

/// Whether `events` carries a download report within `within`.
async fn reported(events: &mut broadcast::Receiver<CdpEvent>, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    loop {
        match time::timeout_at(deadline, events.recv()).await {
            Err(_) => return false,
            Ok(Ok(event)) if event.method.starts_with("Browser.download") => return true,
            Ok(_) => {}
        }
    }
}

/// Clicks the link as the user until a download saves without a report, within 10 s: the run's
/// switch-off may still be on its way when the run returns. Returns whether one did, and the
/// folder's files then.
async fn user_download_unreported(tab: &Tab) -> (bool, Vec<String>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        // A run hears a download as it begins, so one may still be writing.
        settle(&tab.downloads).await;
        let before = saved_files(&tab.downloads).len();
        let mut events = tab.cdp.events();
        click_link(tab).await;
        let heard = reported(&mut events, QUIET).await;
        let files = wait_for_files(&tab.downloads, before + 1).await;
        assert_eq!(
            files.len(),
            before + 1,
            "the user's download saved: {files:?}"
        );
        if !heard || Instant::now() > deadline {
            return (!heard, files);
        }
    }
}

const ONE_DOWNLOAD: &str = r"const [download] = await Promise.all([page.waitForEvent('download'), page.click('#data')])
const file = await download.path()
return [download.suggestedFilename(), await download.failure(), file.split('/').pop()]";

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_the_reports_end_with_the_run_and_the_users_downloads_save_as_before() {
    let Some(tab) = open().await else {
        return;
    };
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    let outcome = runtime.run(request(&tab.target_id, ONE_DOWNLOAD)).await;
    assert_eq!(
        value(&outcome),
        &json!(["data.txt", null, "data.txt"]),
        "the run's download is the browser's own file, under its own name"
    );
    assert_eq!(
        saved_files(&tab.downloads),
        ["data.txt"],
        "in the profile's folder"
    );

    let (unreported, files) = user_download_unreported(&tab).await;
    assert!(unreported, "no report reaches the connection after the run");
    assert!(
        files.iter().any(|name| name == "data (1).txt"),
        "the user's download saves in the same folder under the browser's name: {files:?}"
    );

    let outcome = runtime.run(request(&tab.target_id, ONE_DOWNLOAD)).await;
    assert_eq!(
        value(&outcome).get(0),
        Some(&json!("data.txt")),
        "the next run turns the reports on again"
    );
    let (unreported, _) = user_download_unreported(&tab).await;
    assert!(unreported, "and off at its end");
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_the_reports_stay_on_while_another_run_of_the_runtime_listens() {
    let Some(tab) = open().await else {
        return;
    };
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    // The listening run: its evaluate runs once the reports are on (a call waits for them).
    let listening = {
        let runtime = runtime.clone();
        let request = request(
            &tab.target_id,
            r"const waiting = page.waitForEvent('download', { timeout: 20000 })
await page.evaluate(() => { window.__listening = true })
const download = await waiting
return download.suggestedFilename()",
        );
        tokio::spawn(async move { runtime.run(request).await })
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while tab.evaluate("window.__listening === true").await != json!(true) {
        assert!(Instant::now() < deadline, "the listening run started");
        time::sleep(Duration::from_millis(50)).await;
    }
    // A second run of the same Runtime, on a tab of its own, listens, downloads and ends.
    let created = tab
        .cdp
        .send(
            "Target.createTarget",
            json!({ "url": format!("{}/download.html", tab.origin) }),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("open a second tab: {error}"));
    let second = created["targetId"]
        .as_str()
        .unwrap_or_else(|| panic!("Target.createTarget answered {created}"))
        .to_string();
    let deadline = Instant::now() + Duration::from_secs(10);
    let ended = loop {
        let outcome = runtime.run(request(&second, ONE_DOWNLOAD)).await;
        // The new tab may still be loading its page when the run starts.
        if outcome.status == RunStatus::Ok || Instant::now() > deadline {
            break outcome;
        }
        time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(
        value(&ended).get(0),
        Some(&json!("data.txt")),
        "the second run got its own download"
    );
    // Its end left the reports on for the first, which hears the user's download.
    click_link(&tab).await;
    let outcome = time::timeout(Duration::from_secs(25), listening)
        .await
        .unwrap_or_else(|_| panic!("the listening run ended"))
        .unwrap_or_else(|error| panic!("the listening run: {error}"));
    assert_eq!(
        value(&outcome),
        &json!("data.txt"),
        "the first run still heard the download after the second ended"
    );
    // It heard the download as it began: the second run's file and this one, once written.
    assert_eq!(
        wait_for_files(&tab.downloads, 2).await,
        ["data (1).txt", "data.txt"]
    );
    let (unreported, _) = user_download_unreported(&tab).await;
    assert!(unreported, "off once both ended");
    tab.close().await;
}
