//! The public host API on a real BrowserOS neo: session admission and artifacts, background
//! follow/painting, focus while capture stops, bounded stills, idle completion and privacy.
//! Uses throwaway browser profiles and skips unless `NEO_E2E_BROWSER` names the binary.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "host tests use only part of the shared browser driver"
)]
mod browser;

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use neo_runtime::{
    Frame, Frames, Host, HostOptions, HostStore, LiveError, MemoryStore, PageId, Quality,
    RunOutcome, RunRequest, RunStatus, Runtime, RuntimeOptions, SessionKey, VideoChunks,
};
use serde_json::json;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

use browser::{Fixture, Tab};

/// Neo: bound each real-browser host scenario, including fixture startup, stream creation and task joins, so regressions fail with the scenario name.
const SCENARIO_TIMEOUT: Duration = Duration::from_mins(2);

const ANIMATED: &str = r"<title>host fixture</title><style>@keyframes slide {from{transform:translateX(0)}to{transform:translateX(300px)}} #box{width:100px;height:100px;background:#c33;animation:slide .6s linear infinite alternate} p{font:12px/14px monospace;margin:0}</style><div id=box></div><button id=go>Go</button><script>window.clicks=[];document.querySelector('button').onclick=()=>window.clicks.push(document.hasFocus())</script>";

async fn setup(idle: Duration) -> Option<(Tab, Runtime, Arc<MemoryStore>)> {
    let binary = browser::e2e_browser()?;
    let mut text = String::new();
    for line in 0..500 {
        write!(text, "<p>{line}: a text heavy local fixture with enough detail to exercise JPEG encoding and the audit image size ladder.</p>").unwrap_or_else(|error| panic!("fixture HTML: {error}"));
    }
    let fixture = Fixture {
        pages: vec![
            ("/one".to_string(), format!("{ANIMATED}{text}")),
            ("/two".to_string(), format!("{ANIMATED}{text}")),
        ],
        ..Fixture::default()
    };
    let tab = Tab::open(binary, fixture, "/one").await;
    let store = Arc::new(MemoryStore::default());
    let mut options = HostOptions::new(tab.cdp_port(), 4, Arc::clone(&store) as Arc<dyn HostStore>);
    options.idle = idle;
    let runtime = Runtime::connect_with(
        tab.cdp.clone(),
        RuntimeOptions {
            host: Some(options),
        },
    )
    .await
    .unwrap_or_else(|error| panic!("host fixture: {error}"));
    Some((tab, runtime, store))
}

async fn run(runtime: &Runtime, tab: &Tab, key: &SessionKey, code: String) -> RunOutcome {
    runtime
        .run(RunRequest {
            code,
            page: PageId::new(tab.target_id.clone()),
            timeout: Duration::from_secs(30),
            cancel: CancellationToken::new(),
            session: Some(key.clone()),
            artifacts: None,
        })
        .await
}

async fn remember(store: &MemoryStore, key: &SessionKey, outcome: &RunOutcome) {
    let pages: Vec<_> = outcome
        .pages
        .iter()
        .map(|page| PageId::new(page.id.clone()))
        .collect();
    store.record_run(key, &outcome.run_id, &pages).await;
}

async fn shown(host: &Host, key: &SessionKey) {
    time::timeout(Duration::from_secs(10), async {
        while host.view(key).is_none_or(|view| view.current_tab.is_none()) {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|error| panic!("current tab was never published: {error}"));
}

async fn next(frames: &mut Frames) -> Frame {
    time::timeout(Duration::from_secs(10), frames.next())
        .await
        .unwrap_or_else(|error| panic!("frame deadline: {error}"))
        .unwrap_or_else(|| panic!("stream ended before a frame"))
}

async fn video_part(video: &mut VideoChunks, deadline: Instant, purpose: &str) -> Bytes {
    time::timeout_at(deadline, video.next())
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {purpose}"))
        .unwrap_or_else(|| panic!("video ended before {purpose}"))
}

async fn ended(frames: &mut Frames) {
    time::timeout(Duration::from_secs(10), async {
        while frames.next().await.is_some() {}
    })
    .await
    .unwrap_or_else(|error| panic!("stream did not end: {error}"));
}

async fn images(store: &MemoryStore, outcome: &RunOutcome) {
    time::timeout(Duration::from_secs(10), async {
        while !store
            .artifacts(&outcome.run_id)
            .await
            .values()
            .any(|row| row.screenshot.is_some())
        {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|error| panic!("screenshot was never stored: {error}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn follows_a_created_background_tab_paints_after_the_run_and_records_av1() {
    time::timeout(SCENARIO_TIMEOUT, async {
        let Some((tab, runtime, store)) = setup(Duration::from_mins(1)).await else {
            return;
        };
        let host = runtime.host().unwrap();
        let first = host.admit_run(None).await;
        let key = first.session().clone();
        let initial = run(
            &runtime,
            &tab,
            &key,
            "return await page.title()".to_string(),
        )
        .await;
        assert_eq!(initial.status, RunStatus::Ok, "{initial:?}");
        remember(&store, &key, &initial).await;
        drop(first);
        shown(host, &key).await;
        let mut frames = host.screencast(&key, Quality::Low, None).await.unwrap();
        let original = next(&mut frames).await.tab;
        let pass = host.admit_run(Some(&key)).await;
        let outcome = run(&runtime, &tab, &key, format!(
            "await page.goto({}); const other = await context.newPage(); await other.goto({}); for(let i=0;i<200;i++) await other.title(); return other.url()",
            json!(format!("{}/one", tab.origin)), json!(format!("{}/two", tab.origin)))).await;
        assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
        remember(&store, &key, &outcome).await;
        drop(pass);
        let created = outcome.pages.iter().find(|page| page.created).unwrap();
        let info = tab
            .cdp
            .send("Browser.getTabInfo", json!({"targetId":created.id}), None)
            .await
            .unwrap();
        let real_id = info["tab"]["tabId"].as_i64().unwrap();
        let view = host.view(&key).unwrap();
        let current = view.current_tab.unwrap();
        assert_eq!(current.page.as_str(), created.id);
        assert_eq!(current.tab.get(), real_id);
        assert_ne!(current.tab, original);
        time::timeout(Duration::from_secs(10), async {
            while next(&mut frames).await.tab.get() != real_id {}
        })
        .await
        .expect("follower never switched to the created tab");
        for _ in 0..3 {
            assert_eq!(
                next(&mut frames).await.tab.get(),
                real_id,
                "background frames continue after RunEnded"
            );
        }

        let mut video = host.video(&key, Quality::Low, None).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let init = video_part(&mut video, deadline, "video initialization").await;
        assert!(
            [b"ftyp", b"moov", b"av01"]
                .iter()
                .all(|kind| init.windows(4).any(|part| part == *kind))
        );
        let fragment = video_part(&mut video, deadline, "first video fragment").await;
        assert!(
            [b"moof", b"mdat"]
                .iter()
                .all(|kind| fragment.windows(4).any(|part| part == *kind))
        );
        drop(video);
        host.end_session(&key);
        ended(&mut frames).await;
        assert!(
            host.wait_complete(&key, Instant::now() + Duration::from_secs(10))
                .await
                .unwrap()
                .complete
        );
        let rows = store.artifacts(&outcome.run_id).await;
        assert_eq!(
            rows.len(),
            outcome.calls.len() + 1,
            "every call and root has its after-call facts"
        );
        assert!(rows.values().any(|row| row.tab.get() == real_id));
        let mut screenshots = 0;
        for row in rows.values() {
            if let Some(jpeg) = &row.screenshot {
                screenshots += 1;
                assert!(jpeg.len() <= 64 * 1024);
                assert!(jpeg.starts_with(&[0xff, 0xd8]));
            }
        }
        assert!(
            screenshots > 0,
            "the text-heavy page produced a bounded JPEG"
        );
        runtime.shutdown().await;
        tab.close().await;
    })
    .await
    .expect("follows_a_created_background_tab_paints_after_the_run_and_records_av1 did not finish within two minutes");
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_start_stop_keeps_a_background_clicking_run_focused() {
    time::timeout(SCENARIO_TIMEOUT, async {
        let Some((tab, runtime, store)) = setup(Duration::from_mins(1)).await else {
            return;
        };
        tab.cdp
            .send(
                "Target.createTarget",
                json!({"url":format!("{}/front",tab.origin)}),
                None,
            )
            .await
            .unwrap();
        let host = runtime.host().unwrap();
        let pass = host.admit_run(None).await;
        let key = pass.session().clone();
        let running = tokio::spawn({
            let runtime = runtime.clone();
            let key = key.clone();
            let target = tab.target_id.clone();
            async move {
                runtime.run(RunRequest { code:"for(let i=0;i<80;i++){await page.click('#go');await new Promise(r=>setTimeout(r,100))}return await page.title()".to_string(),
                page:PageId::new(target),timeout:Duration::from_secs(30),cancel:CancellationToken::new(),session:Some(key),artifacts:None }).await
            }
        });
        shown(host, &key).await;
        let mut low = host.screencast(&key, Quality::Low, None).await.unwrap();
        next(&mut low).await;
        drop(low);
        assert_eq!(tab.evaluate("document.hasFocus()").await, true);
        // The low capture stops after its five-second departure grace, while this run keeps clicking.
        time::sleep(Duration::from_millis(5500)).await;
        assert!(
            !running.is_finished(),
            "capture stop occurs under the active run"
        );
        assert_eq!(tab.evaluate("document.hasFocus()").await, true);
        let mut high = host.screencast(&key, Quality::High, None).await.unwrap();
        next(&mut high).await;
        host.end_session(&key);
        ended(&mut high).await;
        assert_eq!(
            tab.evaluate("document.hasFocus()").await,
            true,
            "presence remains while an ended session still has an in-flight run"
        );
        let outcome = running.await.unwrap();
        assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
        let clicks = tab.evaluate("window.clicks").await;
        assert_eq!(clicks.as_array().unwrap().len(), 80);
        assert!(
            clicks
                .as_array()
                .unwrap()
                .iter()
                .all(|focused| focused == true)
        );
        remember(&store, &key, &outcome).await;
        drop(pass);
        assert!(
            host.wait_complete(&key, Instant::now() + Duration::from_secs(10))
                .await
                .unwrap()
                .complete
        );
        runtime.shutdown().await;
        tab.close().await;
    })
    .await
    .expect("capture_start_stop_keeps_a_background_clicking_run_focused did not finish within two minutes");
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_end_closes_the_stream_and_waits_for_artifacts() {
    time::timeout(SCENARIO_TIMEOUT, async {
        let Some((tab, runtime, store)) = setup(Duration::from_millis(250)).await else {
            return;
        };
        let host = runtime.host().unwrap();
        let pass = host.admit_run(None).await;
        let key = pass.session().clone();
        let outcome = run(
            &runtime,
            &tab,
            &key,
            "return await page.title()".to_string(),
        )
        .await;
        assert_eq!(outcome.status, RunStatus::Ok);
        remember(&store, &key, &outcome).await;
        shown(host, &key).await;
        let mut frames = host.screencast(&key, Quality::Low, None).await.unwrap();
        next(&mut frames).await;
        drop(pass);
        ended(&mut frames).await;
        let view = host
            .wait_complete(&key, Instant::now() + Duration::from_secs(10))
            .await
            .unwrap();
        assert!(!view.live);
        assert!(view.complete);
        assert!(view.ended_at.is_some());
        assert_eq!(
            store.artifacts(&outcome.run_id).await.len(),
            outcome.calls.len() + 1
        );
        runtime.shutdown().await;
        tab.close().await;
    })
    .await
    .expect("idle_end_closes_the_stream_and_waits_for_artifacts did not finish within two minutes");
}

#[tokio::test(flavor = "multi_thread")]
async fn withhold_ends_streams_and_deletes_stored_screenshots() {
    time::timeout(SCENARIO_TIMEOUT, async {
        let Some((tab, runtime, store)) = setup(Duration::from_mins(1)).await else {
            return;
        };
        let host = runtime.host().unwrap();
        let pass = host.admit_run(None).await;
        let key = pass.session().clone();
        let outcome = run(
            &runtime,
            &tab,
            &key,
            "await page.click('#go'); return await page.title()".to_string(),
        )
        .await;
        assert_eq!(outcome.status, RunStatus::Ok);
        remember(&store, &key, &outcome).await;
        images(&store, &outcome).await;
        shown(host, &key).await;
        let mut frames = host.screencast(&key, Quality::Low, None).await.unwrap();
        next(&mut frames).await;
        let mut video = host.video(&key, Quality::Low, None).await.unwrap();
        host.withhold(&key).await.unwrap();
        assert!(host.view(&key).unwrap().withheld);
        ended(&mut frames).await;
        assert!(
            time::timeout(Duration::from_secs(5), video.next())
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            host.screencast(&key, Quality::Low, None).await.err(),
            Some(LiveError::Withheld)
        );
        assert!(
            store
                .artifacts(&outcome.run_id)
                .await
                .values()
                .all(|row| row.screenshot.is_none())
        );
        host.withhold(&key).await.unwrap();
        drop(pass);
        host.end_session(&key);
        assert!(
            host.wait_complete(&key, Instant::now() + Duration::from_secs(10))
                .await
                .unwrap()
                .complete
        );
        assert!(
            store
                .artifacts(&outcome.run_id)
                .await
                .values()
                .all(|row| row.screenshot.is_none())
        );
        runtime.shutdown().await;
        tab.close().await;
    })
    .await
    .expect(
        "withhold_ends_streams_and_deletes_stored_screenshots did not finish within two minutes",
    );
}
