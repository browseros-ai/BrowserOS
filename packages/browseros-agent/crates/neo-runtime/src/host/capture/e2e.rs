//! Live capture on a real BrowserOS neo. A background tab, behind another, streams screencast
//! frames once its presence turns focus emulation on (the neo-host-events design's probes 1
//! and 2, kept as a regression test), and `Page.startScreenRecording` produces an AV1
//! fragmented MP4 with a fragment within 5 s (probe 5). Runs only when `NEO_E2E_BROWSER` names
//! the browser binary (`tests/support/browser.rs`); otherwise it passes as a skip.

#[path = "../../../tests/support/browser.rs"]
#[expect(dead_code, reason = "these tests drive a tab, not a run")]
mod browser;

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::json;
use tokio::sync::watch;
use tokio::time;
use tokio_util::sync::CancellationToken;

use super::{CaptureHub, Quality};
use crate::host::ids::TabId;
use crate::host::link::HostLink;
use crate::host::presence::Presences;
use crate::host::view::{FollowView, ShownTab};
use crate::run::PageId;
use browser::{Fixture, Tab};

/// A page that never stops changing, so a tab that paints keeps sending frames: a compositor
/// animation runs at the display rate while the tab paints, and not at all while it does not.
const ANIMATED: &str = "<title>animated</title><style>@keyframes slide { from { transform: \
    translateX(0) } to { transform: translateX(300px) } } #box { width: 120px; height: 120px; \
    background: #c33; animation: slide 0.6s linear infinite alternate }</style><div id=box></div>";

#[tokio::test(flavor = "multi_thread")]
async fn a_background_tab_streams_through_its_presence_and_records_av1() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let fixture = Fixture {
        pages: vec![("/animated".to_string(), ANIMATED.to_string())],
        ..Fixture::default()
    };
    let tab = Tab::open(binary, fixture, "/animated").await;
    // Another tab in front: the fixture's tab now paints only while a session keeps it painting.
    tab.cdp
        .send(
            "Target.createTarget",
            json!({ "url": format!("{}/front", tab.origin) }),
            None,
        )
        .await
        .expect("open a tab in front");
    wait_for_visibility(&tab, "hidden").await;

    let shutdown = CancellationToken::new();
    // The test's own client stands in for the host side's socket: the fixture keeps the
    // browser's port to itself, and nothing else on this client takes screencast frames.
    let link = HostLink::over(tab.cdp.clone(), shutdown.clone());
    let presences = Arc::new(Presences::new(Arc::clone(&link)));
    let hub = CaptureHub::new(link, presences, shutdown.clone());
    let shown = ShownTab {
        target: PageId::new(tab.target_id.clone()),
        tab: tab_id(&tab).await,
        url: format!("{}/animated", tab.origin),
    };
    let tab_id = shown.tab;
    let (view, _) = watch::channel(FollowView {
        live: true,
        withheld: false,
        tabs: vec![shown],
    });

    let mut frames = hub
        .screencast(view.subscribe(), Quality::Low, None)
        .await
        .expect("a screencast of the background tab");
    let mut seen = Vec::new();
    while seen.len() < 3 {
        let frame = time::timeout(Duration::from_secs(10), frames.next())
            .await
            .expect("frames keep coming from the background tab")
            .expect("the screencast stays open");
        seen.push(frame);
    }
    assert!(
        seen.iter()
            .all(|frame| frame.tab == tab_id && frame.jpeg.starts_with(&[0xff, 0xd8])),
        "JPEG frames of the background tab"
    );
    assert_eq!(
        tab.evaluate("document.visibilityState").await,
        json!("visible"),
        "the presence keeps the background tab painting"
    );

    let mut video = hub
        .video(view.subscribe(), Quality::High, None)
        .await
        .expect("a recording of the background tab");
    let started = Instant::now();
    let init = time::timeout(Duration::from_secs(5), video.next())
        .await
        .expect("the init segment within 5 s")
        .expect("the video stays open");
    assert!(
        [b"ftyp", b"moov", b"av01"]
            .iter()
            .all(|kind| contains(&init, *kind)),
        "an AV1 fragmented MP4's init segment"
    );
    let left = Duration::from_secs(5).saturating_sub(started.elapsed());
    let fragment = time::timeout(left, video.next())
        .await
        .expect("a fragment within 5 s of the recording's start")
        .expect("the video stays open");
    assert!(
        [b"moof", b"mdat"]
            .iter()
            .all(|kind| contains(&fragment, *kind)),
        "a keyframe-led fragment"
    );

    drop((frames, video));
    shutdown.cancel();
    tab.close().await;
}

/// The fixture tab's BrowserOS tab id, as the tab directory would have it.
async fn tab_id(tab: &Tab) -> TabId {
    let info = tab
        .cdp
        .send(
            "Browser.getTabInfo",
            json!({ "targetId": tab.target_id }),
            None,
        )
        .await
        .expect("Browser.getTabInfo");
    info["tab"]["tabId"]
        .as_i64()
        .and_then(TabId::parse)
        .unwrap_or_else(|| panic!("Browser.getTabInfo answered {info}"))
}

/// Waits until the fixture tab reports `visibility`; a tab sent behind another reports `hidden`
/// a moment later.
async fn wait_for_visibility(tab: &Tab, visibility: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let now = tab.evaluate("document.visibilityState").await;
        if now == json!(visibility) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the tab stayed {now} instead of {visibility}"
        );
        time::sleep(Duration::from_millis(50)).await;
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
