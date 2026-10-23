//! `CaptureHub` through its interface: a scripted browser stands in for Chromium and a static
//! table for the session's tabs.

use super::{
    CaptureError, CaptureHub, CaptureTab, Quality, SessionView,
    tab_capture::STOP_GRACE,
    testing::{ScriptedCdp, ScriptedTransport, StaticSessionTabs},
};
use futures_util::StreamExt;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

struct Harness {
    hub: Arc<CaptureHub>,
    browser: Arc<ScriptedCdp>,
    tabs: Arc<StaticSessionTabs>,
    shutdown: CancellationToken,
}

fn harness() -> Harness {
    let browser = ScriptedCdp::new();
    let tabs = Arc::new(StaticSessionTabs::default());
    let shutdown = CancellationToken::new();
    let hub = CaptureHub::new(
        tabs.clone(),
        Arc::new(ScriptedTransport(browser.clone())),
        shutdown.clone(),
    );
    Harness {
        hub,
        browser,
        tabs,
        shutdown,
    }
}

fn tab(tab_id: i64) -> CaptureTab {
    CaptureTab {
        tab_id,
        target_id: format!("target-{tab_id}"),
        url: format!("https://example.com/{tab_id}"),
    }
}

fn live(tabs: &[i64]) -> SessionView {
    SessionView {
        live: true,
        tabs: tabs.iter().copied().map(tab).collect(),
    }
}

/// The stream's next item, or `None` once it has ended. A stream that neither yields nor ends
/// within 5 s fails the test, so `None` always means "ended", never "stalled".
async fn next<S: futures_util::Stream + Unpin>(stream: &mut S) -> Option<S::Item> {
    match tokio::time::timeout(Duration::from_secs(5), stream.next()).await {
        Ok(item) => item,
        Err(_) => panic!("stream neither yielded nor ended within 5 s"),
    }
}

#[tokio::test]
async fn rejects_sessions_with_nothing_to_watch() {
    let h = harness();
    h.tabs.set(
        "ended",
        SessionView {
            live: false,
            tabs: Vec::new(),
        },
    );
    h.tabs.set("empty", live(&[]));
    h.tabs.set("open", live(&[1]));

    let cases = [
        ("missing", None, "session not found"),
        ("ended", None, "session has ended"),
        ("empty", None, "session has no open tab"),
        ("open", Some(9), "tab is not open in this session"),
    ];
    for (session, pinned, message) in cases {
        let screencast = h.hub.screencast(session, Quality::High, pinned).await.err();
        let video = h.hub.video(session, Quality::High, pinned).await.err();
        assert_eq!(
            screencast.map(|error| error.to_string()).as_deref(),
            Some(message),
            "{session}"
        );
        assert_eq!(
            video.map(|error| error.to_string()).as_deref(),
            Some(message),
            "{session}"
        );
    }
    assert!(h.browser.sent("Target.attachToTarget").is_empty());
}

#[tokio::test]
async fn screencast_streams_the_current_tab_with_the_presets_settings() {
    let h = harness();
    h.tabs.set("s", live(&[2, 1]));

    let mut frames = h
        .hub
        .screencast("s", Quality::Low, None)
        .await
        .map_err(|e| e.to_string())
        .unwrap_or_else(|e| panic!("{e}"));

    let attach = h.browser.wait_for("Target.attachToTarget", 1).await;
    assert_eq!(
        attach[0].params,
        json!({ "targetId": "target-2", "flatten": true })
    );
    let start = h.browser.wait_for("Page.startScreencast", 1).await;
    assert_eq!(start[0].session.as_deref(), Some("capture-1"));
    assert_eq!(
        start[0].params,
        json!({ "format": "jpeg", "quality": 50, "maxWidth": 480, "maxHeight": 300, "maxFramesInFlight": 1 })
    );

    h.browser
        .emit_frame("capture-1", b"jpeg-1", 1_800_000_000.5);
    let frame = next(&mut frames).await;
    assert_eq!(
        frame.as_ref().map(|f| f.jpeg.as_ref()),
        Some(&b"jpeg-1"[..])
    );
    assert_eq!(
        frame
            .as_ref()
            .map(|f| (f.tab_id, f.url.as_str(), f.captured_at)),
        Some((2, "https://example.com/2", 1_800_000_000_500))
    );
    let ack = h.browser.wait_for("Page.screencastFrameAck", 1).await;
    assert_eq!(ack[0].params, json!({ "sessionId": 7 }));
}

#[tokio::test]
async fn viewers_of_one_tab_share_one_capture() {
    let h = harness();
    h.tabs.set("s", live(&[1]));

    let mut first = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut second = h
        .hub
        .screencast("s", Quality::High, Some(1))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    h.browser.emit_frame("capture-1", b"shared", 1.0);

    assert_eq!(
        next(&mut first).await.map(|f| f.jpeg),
        Some(b"shared"[..].into())
    );
    assert_eq!(
        next(&mut second).await.map(|f| f.jpeg),
        Some(b"shared"[..].into())
    );
    assert_eq!(h.browser.sent("Page.startScreencast").len(), 1);

    // A different quality is a different capture.
    let _low = h
        .hub
        .screencast("s", Quality::Low, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(h.browser.wait_for("Page.startScreencast", 2).await.len(), 2);
}

#[tokio::test]
async fn a_late_viewer_first_gets_the_newest_frame() {
    let h = harness();
    h.tabs.set("s", live(&[1]));
    let mut first = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    h.browser.emit_frame("capture-1", b"static page", 1.0);
    assert!(next(&mut first).await.is_some());

    // A static page sends no new frames; the joiner still sees it.
    let mut late = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        next(&mut late).await.map(|f| f.jpeg),
        Some(b"static page"[..].into())
    );
}

#[tokio::test(start_paused = true)]
async fn acks_are_paced_to_the_quality_frame_rate() {
    let h = harness();
    h.tabs.set("s", live(&[1]));
    let mut frames = h
        .hub
        .screencast("s", Quality::Low, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));

    h.browser.emit_frame("capture-1", b"one", 1.0);
    assert!(next(&mut frames).await.is_some());
    h.browser.wait_for("Page.screencastFrameAck", 1).await;
    let first_ack = tokio::time::Instant::now();

    h.browser.emit_frame("capture-1", b"two", 2.0);
    assert!(next(&mut frames).await.is_some());
    h.browser.wait_for("Page.screencastFrameAck", 2).await;

    // `low` is 4 fps: the second ack waits a quarter second after the first.
    assert!(tokio::time::Instant::now() - first_ack >= Duration::from_millis(250));
}

#[tokio::test(start_paused = true)]
async fn the_capture_stops_a_grace_period_after_its_last_viewer_leaves() {
    let h = harness();
    h.tabs.set("s", live(&[1]));
    let frames = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    h.browser.wait_for("Page.startScreencast", 1).await;

    drop(frames);
    tokio::time::sleep(STOP_GRACE / 2).await;
    assert!(h.browser.sent("Page.stopScreencast").is_empty());
    // Coming back within the grace period reuses the running capture.
    let again = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(h.browser.sent("Page.startScreencast").len(), 1);

    drop(again);
    tokio::time::sleep(STOP_GRACE + Duration::from_secs(2)).await;
    assert_eq!(h.browser.wait_for("Page.stopScreencast", 1).await.len(), 1);
    assert_eq!(
        h.browser.wait_for("Target.detachFromTarget", 1).await.len(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn screencast_follows_the_session_to_its_new_current_tab() {
    let h = harness();
    h.tabs.set("s", live(&[1, 2]));
    let mut frames = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    h.browser.wait_for("Page.startScreencast", 1).await;

    // The agent's latest tool call is now on tab 2.
    h.tabs.set("s", live(&[2, 1]));
    let attach = h.browser.wait_for("Target.attachToTarget", 2).await;
    assert_eq!(attach[1].params["targetId"], "target-2");
    h.browser.wait_for("Page.startScreencast", 2).await;

    h.browser.emit_frame("capture-2", b"tab two", 1.0);
    let frame = loop {
        let frame = next(&mut frames).await;
        if frame.as_ref().is_none_or(|f| f.tab_id == 2) {
            break frame;
        }
    };
    assert_eq!(
        frame.map(|f| (f.tab_id, f.jpeg)),
        Some((2, b"tab two"[..].into()))
    );

    // Nobody watches tab 1 any more, so its capture stops after the grace period.
    tokio::time::sleep(STOP_GRACE + Duration::from_secs(2)).await;
    let stops = h.browser.wait_for("Page.stopScreencast", 1).await;
    assert_eq!(stops[0].session.as_deref(), Some("capture-1"));
}

#[tokio::test(start_paused = true)]
async fn streams_end_with_the_session() {
    let h = harness();
    h.tabs.set("s", live(&[1]));
    let mut frames = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut video = h
        .hub
        .video("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));

    h.tabs.set(
        "s",
        SessionView {
            live: false,
            tabs: Vec::new(),
        },
    );

    assert!(next(&mut frames).await.is_none());
    assert!(next(&mut video).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_pinned_stream_ends_when_its_tab_closes() {
    let h = harness();
    h.tabs.set("s", live(&[1, 2]));
    let mut frames = h
        .hub
        .screencast("s", Quality::High, Some(1))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    h.browser.wait_for("Page.startScreencast", 1).await;

    // Closing a tab detaches its sessions and releases the session's claim on it.
    h.tabs.set("s", live(&[2]));
    h.browser.emit_detached("capture-1");

    assert!(next(&mut frames).await.is_none());
    // The stream ended with its tab; it did not jump to the session's other tab.
    assert_eq!(h.browser.sent("Target.attachToTarget").len(), 1);
}

#[tokio::test]
async fn shutdown_ends_open_streams() {
    let h = harness();
    h.tabs.set("s", live(&[1]));
    let mut frames = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));

    h.shutdown.cancel();

    assert!(next(&mut frames).await.is_none());
}

fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = u32::try_from(8 + body.len())
        .unwrap_or(u32::MAX)
        .to_be_bytes()
        .to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

#[tokio::test(start_paused = true)]
async fn video_sends_the_init_segment_then_fragments_and_late_viewers_join_at_a_fragment() {
    let h = harness();
    h.tabs.set("s", live(&[1]));
    let init = [mp4_box(b"ftyp", b"av01"), mp4_box(b"moov", b"tracks")].concat();
    let first = [mp4_box(b"moof", b"1"), mp4_box(b"mdat", b"keyframe-1")].concat();
    let second = [mp4_box(b"moof", b"2"), mp4_box(b"mdat", b"keyframe-2")].concat();

    let mut early = h
        .hub
        .video("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let start = h.browser.wait_for("Page.startScreenRecording", 1).await;
    assert_eq!(
        start[0].params,
        json!({ "maxWidth": 1280, "maxHeight": 800, "frameRate": 15 })
    );
    h.browser
        .push_recording("capture-1", &[init.clone(), first.clone()].concat());

    assert_eq!(next(&mut early).await.as_deref(), Some(&init[..]));
    assert_eq!(next(&mut early).await.as_deref(), Some(&first[..]));

    let mut late = h
        .hub
        .video("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(h.browser.sent("Page.startScreenRecording").len(), 1);
    assert_eq!(next(&mut late).await.as_deref(), Some(&init[..]));
    h.browser.push_recording("capture-1", &second);
    assert_eq!(next(&mut late).await.as_deref(), Some(&second[..]));
    assert_eq!(next(&mut early).await.as_deref(), Some(&second[..]));
}

#[tokio::test(start_paused = true)]
async fn video_waits_out_reads_before_the_recording_exists() {
    let h = harness();
    h.tabs.set("s", live(&[1]));
    // Chromium answers "Read failed" until the recording's first write.
    h.browser.fail_reads("capture-1", 3);
    let init = [mp4_box(b"ftyp", b"av01"), mp4_box(b"moov", b"tracks")].concat();

    let mut video = h
        .hub
        .video("s", Quality::Low, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    h.browser.wait_for("IO.read", 3).await;
    h.browser.push_recording("capture-1", &init);

    assert_eq!(next(&mut video).await.as_deref(), Some(&init[..]));
}

#[tokio::test]
async fn screencast_and_video_of_one_tab_use_separate_cdp_sessions() {
    let h = harness();
    h.tabs.set("s", live(&[1]));
    let _frames = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let _video = h
        .hub
        .video("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));

    // Chromium refuses a screencast and a screen recording in one session.
    let screencast = h.browser.wait_for("Page.startScreencast", 1).await;
    let recording = h.browser.wait_for("Page.startScreenRecording", 1).await;
    assert_ne!(screencast[0].session, recording[0].session);
}

#[test]
fn quality_parses_only_the_published_presets() {
    assert_eq!(Quality::parse("low"), Some(Quality::Low));
    assert_eq!(Quality::parse("high"), Some(Quality::High));
    assert_eq!(Quality::parse("medium"), None);
    let _ = CaptureError::NoOpenTab;
}

/// Whether `stream` is still open with nothing to deliver for `wait`.
async fn still_open<S: futures_util::Stream + Unpin>(stream: &mut S, wait: Duration) -> bool {
    tokio::time::timeout(wait, stream.next()).await.is_err()
}

#[tokio::test(start_paused = true)]
async fn a_viewer_that_stops_reading_does_not_keep_an_ended_session_capturing() {
    let h = harness();
    h.tabs.set("s", live(&[1]));
    let screencast = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let video = h
        .hub
        .video("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let init = [mp4_box(b"ftyp", b"av01"), mp4_box(b"moov", b"tracks")].concat();
    let fragments = (0..6)
        .map(|n| [mp4_box(b"moof", &[n]), mp4_box(b"mdat", b"frame")].concat())
        .collect::<Vec<_>>();
    h.browser.wait_for("Page.startScreenRecording", 1).await;

    // Neither viewer reads anything while the browser keeps producing.
    for n in 0..6 {
        h.browser.emit_frame("capture-1", &[n], f64::from(n));
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    h.browser
        .push_recording("capture-2", &[init, fragments.concat()].concat());
    tokio::time::sleep(Duration::from_secs(1)).await;

    h.tabs.set(
        "s",
        SessionView {
            live: false,
            tabs: Vec::new(),
        },
    );
    tokio::time::sleep(Duration::from_secs(3)).await;

    assert_eq!(h.browser.sent("Page.stopScreencast").len(), 1);
    assert_eq!(h.browser.sent("Page.stopScreenRecording").len(), 1);
    drop((screencast, video));
}

#[tokio::test(start_paused = true)]
async fn ending_the_session_stops_its_capture_without_the_grace_period() {
    let h = harness();
    h.tabs.set("s", live(&[1]));
    let mut frames = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    h.browser.wait_for("Page.startScreencast", 1).await;

    h.tabs.set(
        "s",
        SessionView {
            live: false,
            tabs: Vec::new(),
        },
    );
    assert!(next(&mut frames).await.is_none());
    tokio::time::sleep(STOP_GRACE / 4).await;

    assert_eq!(h.browser.sent("Page.stopScreencast").len(), 1);
}

#[tokio::test(start_paused = true)]
async fn an_unpinned_screencast_waits_for_the_sessions_next_tab() {
    let h = harness();
    h.tabs.set("s", live(&[1]));
    let mut frames = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    h.browser.wait_for("Page.startScreencast", 1).await;

    // The agent closes its only tab before opening the next one.
    h.tabs.set("s", live(&[]));
    h.browser.emit_detached("capture-1");
    assert!(still_open(&mut frames, Duration::from_secs(3)).await);

    h.tabs.set("s", live(&[2]));
    h.browser.wait_for("Page.startScreencast", 2).await;
    h.browser.emit_frame("capture-2", b"next tab", 1.0);
    assert_eq!(
        next(&mut frames).await.map(|f| (f.tab_id, f.jpeg)),
        Some((2, b"next tab"[..].into()))
    );
}

#[tokio::test(start_paused = true)]
async fn an_earlier_grace_timer_does_not_cut_a_later_grace_period_short() {
    let h = harness();
    h.tabs.set("s", live(&[1]));
    let first = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    h.browser.wait_for("Page.startScreencast", 1).await;

    drop(first); // t=0: the first grace period starts
    tokio::time::sleep(Duration::from_secs(2)).await;
    let second = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    tokio::time::sleep(Duration::from_secs(2)).await;
    drop(second); // t=4: a new grace period, ending at t=9

    tokio::time::sleep(Duration::from_millis(1500)).await; // t=5.5
    assert!(h.browser.sent("Page.stopScreencast").is_empty());
    tokio::time::sleep(STOP_GRACE).await;
    assert_eq!(h.browser.sent("Page.stopScreencast").len(), 1);
    assert_eq!(h.browser.sent("Page.startScreencast").len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_slow_tab_switch_does_not_keep_a_departed_viewer_capturing() {
    let h = harness();
    h.tabs.set("s", live(&[1, 2]));
    let frames = h
        .hub
        .screencast("s", Quality::High, None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    h.browser.wait_for("Page.startScreencast", 1).await;

    // The agent moves to tab 2 while the browser is slow to attach.
    h.browser.delay_attaches(Duration::from_secs(60));
    h.tabs.set("s", live(&[2, 1]));
    h.browser.wait_for("Target.attachToTarget", 2).await;
    drop(frames);

    // The viewer left mid-switch, so tab 1's capture stops after its grace period, not after
    // the browser finally answers.
    tokio::time::sleep(STOP_GRACE + Duration::from_secs(2)).await;
    let stops = h.browser.sent("Page.stopScreencast");
    assert_eq!(
        stops.first().and_then(|stop| stop.session.as_deref()),
        Some("capture-1")
    );
}
