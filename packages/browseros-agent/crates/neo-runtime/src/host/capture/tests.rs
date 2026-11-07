//! `CaptureHub` through its interface: the loopback browser in `tests/support/fake_host_cdp.rs`
//! stands in for Chromium, and the test publishes the session's `FollowView` as the ledger
//! would. claw-server's capture suite (`apps/claw-server/src/services/capture/tests.rs:62-594`,
//! origin/main) is ported as acceptance criteria, followed by the rules Neo adds.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde_json::json;
use tokio::sync::watch;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

use super::{CaptureError, CaptureHub, Frame, Frames, Quality, VideoChunks};
use crate::fake_host_cdp::FakeHostCdp;
use crate::host::cdp::{MAX_FRAME_BYTES, STOP_GRACE, VIEWER_STALL};
use crate::host::ids::{EpochMs, TabId};
use crate::host::link::HostLink;
use crate::host::presence::Presences;
use crate::host::view::{FollowView, ShownTab};
use crate::run::PageId;

struct Harness {
    hub: Arc<CaptureHub>,
    browser: FakeHostCdp,
    /// The session's view, as the ledger publishes it.
    view: watch::Sender<FollowView>,
    shutdown: CancellationToken,
}

async fn harness() -> Harness {
    let browser = FakeHostCdp::start().await;
    let shutdown = CancellationToken::new();
    let link = HostLink::connect(browser.port(), shutdown.clone())
        .await
        .expect("connect the host link");
    let presences = Arc::new(Presences::new(Arc::clone(&link)));
    let hub = CaptureHub::new(link, presences, shutdown.clone());
    Harness {
        hub,
        browser,
        view: watch::Sender::new(live(&[])),
        shutdown,
    }
}

impl Harness {
    fn show(&self, view: FollowView) {
        self.view.send_replace(view);
    }

    async fn screencast(&self, quality: Quality, tab: Option<i64>) -> Frames {
        self.hub
            .screencast(self.view.subscribe(), quality, tab.map(tab_id))
            .await
            .expect("open a screencast")
    }

    async fn video(&self, quality: Quality, tab: Option<i64>) -> VideoChunks {
        self.hub
            .video(self.view.subscribe(), quality, tab.map(tab_id))
            .await
            .expect("open a video")
    }

    /// The target the fake attached `session` to.
    fn target_of(&self, session: &str) -> String {
        self.browser
            .attachments()
            .into_iter()
            .find(|attachment| attachment.session == session)
            .map(|attachment| attachment.target)
            .unwrap_or_default()
    }
}

fn tab_id(tab: i64) -> TabId {
    TabId::parse(tab).expect("a positive tab id")
}

fn tab(tab: i64) -> ShownTab {
    ShownTab {
        target: PageId::new(format!("target-{tab}")),
        tab: tab_id(tab),
        url: format!("https://example.com/{tab}"),
    }
}

fn live(tabs: &[i64]) -> FollowView {
    FollowView {
        live: true,
        withheld: false,
        tabs: tabs.iter().copied().map(tab).collect(),
    }
}

fn ended() -> FollowView {
    FollowView {
        live: false,
        withheld: false,
        tabs: Vec::new(),
    }
}

fn mp4_box(kind: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = u32::try_from(8 + body.len())
        .unwrap_or(u32::MAX)
        .to_be_bytes()
        .to_vec();
    out.extend_from_slice(&kind);
    out.extend_from_slice(body);
    out
}

/// The stream's next item, or `None` once it has ended. A stream that neither yields nor ends
/// within 10 s of tokio time fails the test, so `None` always means "ended", never "stalled".
async fn next<S: Stream + Unpin>(stream: &mut S) -> Option<S::Item> {
    time::timeout(Duration::from_secs(10), stream.next())
        .await
        .expect("the stream neither yielded nor ended within 10 s")
}

/// Whether `stream` is still open with nothing to deliver for `wait`.
async fn still_open<S: Stream + Unpin>(stream: &mut S, wait: Duration) -> bool {
    time::timeout(wait, stream.next()).await.is_err()
}

#[tokio::test(start_paused = true)]
async fn rejects_views_with_nothing_to_watch() {
    let h = harness().await;
    let cases = [
        (ended(), None, CaptureError::SessionNotLive),
        (
            FollowView {
                withheld: true,
                ..live(&[1])
            },
            None,
            CaptureError::Withheld,
        ),
        (live(&[]), None, CaptureError::NoOpenTab),
        (live(&[]), Some(9), CaptureError::NoOpenTab),
        (live(&[1]), Some(9), CaptureError::TabNotFound),
    ];
    for (view, pinned, expected) in cases {
        h.show(view.clone());
        let screencast = h
            .hub
            .screencast(h.view.subscribe(), Quality::High, pinned.map(tab_id))
            .await
            .err();
        let video = h
            .hub
            .video(h.view.subscribe(), Quality::High, pinned.map(tab_id))
            .await
            .err();
        assert_eq!(
            screencast.as_ref(),
            Some(&expected),
            "a screencast of {view:?} pinned to {pinned:?}"
        );
        assert_eq!(
            video.as_ref(),
            Some(&expected),
            "a video of {view:?} pinned to {pinned:?}"
        );
    }
    assert!(
        h.browser.sent("Target.attachToTarget").is_empty(),
        "nothing was attached"
    );
}

#[tokio::test(start_paused = true)]
async fn screencast_streams_the_current_tab_with_the_presets_settings() {
    let h = harness().await;
    h.show(live(&[2, 1]));

    let mut frames = h.screencast(Quality::Low, None).await;

    let capture = h.browser.session_of("Page.startScreencast", 0).await;
    assert_eq!(
        h.target_of(&capture),
        "target-2",
        "the current tab is tabs[0]"
    );
    assert_eq!(
        h.browser.sent("Page.startScreencast")[0].params,
        json!({
            "format": "jpeg",
            "quality": 50,
            "maxWidth": 640,
            "maxHeight": 400,
            "maxFramesInFlight": 1,
        }),
        "the low preset's settings"
    );
    let number = h.browser.frame(&capture, b"jpeg-1", Some(1_800_000_000.5));
    assert_eq!(
        next(&mut frames).await,
        Some(Frame {
            jpeg: Bytes::from_static(b"jpeg-1"),
            tab: tab_id(2),
            url: "https://example.com/2".to_string(),
            captured_at: EpochMs::from_millis(1_800_000_000_500).expect("a positive time"),
        }),
        "the frame, stamped with its tab and the browser's frame time"
    );
    let ack = h.browser.wait_for("Page.screencastFrameAck", 1).await;
    assert_eq!(
        (ack[0].session.as_deref(), &ack[0].params),
        (Some(capture.as_str()), &json!({ "sessionId": number })),
        "the ack echoes the frame number on the capture's session"
    );
}

#[tokio::test(start_paused = true)]
async fn viewers_of_one_tab_share_one_capture() {
    let h = harness().await;
    h.show(live(&[1]));

    let mut first = h.screencast(Quality::High, None).await;
    let mut second = h.screencast(Quality::High, Some(1)).await;
    let capture = h.browser.session_of("Page.startScreencast", 0).await;
    h.browser.frame(&capture, b"shared", Some(1.0));

    assert_eq!(
        next(&mut first).await.map(|frame| frame.jpeg),
        Some(Bytes::from_static(b"shared")),
        "the following viewer"
    );
    assert_eq!(
        next(&mut second).await.map(|frame| frame.jpeg),
        Some(Bytes::from_static(b"shared")),
        "the pinned viewer"
    );
    assert_eq!(
        h.browser.sent("Page.startScreencast").len(),
        1,
        "one capture for both"
    );

    let _low = h.screencast(Quality::Low, None).await;
    assert_eq!(
        h.browser.wait_for("Page.startScreencast", 2).await.len(),
        2,
        "a different quality is a different capture"
    );
    assert_eq!(
        h.browser.sent("Emulation.setFocusEmulationEnabled").len(),
        1,
        "both captures share the tab's one presence"
    );
}

#[tokio::test(start_paused = true)]
async fn a_late_viewer_first_gets_the_newest_frame() {
    let h = harness().await;
    h.show(live(&[1]));
    let mut first = h.screencast(Quality::High, None).await;
    let capture = h.browser.session_of("Page.startScreencast", 0).await;
    h.browser.frame(&capture, b"static page", Some(1.0));
    assert!(next(&mut first).await.is_some(), "the first viewer's frame");

    // A static page sends no new frames; the joiner still sees it.
    let mut late = h.screencast(Quality::High, None).await;

    assert_eq!(
        next(&mut late).await.map(|frame| frame.jpeg),
        Some(Bytes::from_static(b"static page")),
        "the late viewer starts with the newest frame"
    );
}

#[tokio::test(start_paused = true)]
async fn acks_are_paced_to_the_quality_frame_rate() {
    let h = harness().await;
    h.show(live(&[1]));
    let _frames = h.screencast(Quality::Low, None).await;
    let capture = h.browser.session_of("Page.startScreencast", 0).await;

    h.browser.frame(&capture, b"one", Some(1.0));
    h.browser.frame(&capture, b"two", Some(2.0));

    let acks = h.browser.wait_for("Page.screencastFrameAck", 2).await;
    // `low` is 4 fps: the second ack waits a quarter second after the first.
    assert!(
        acks[1].at - acks[0].at >= Duration::from_millis(250),
        "acks {:?} apart",
        acks[1].at - acks[0].at
    );
}

#[tokio::test(start_paused = true)]
async fn the_capture_stops_a_grace_period_after_its_last_viewer_leaves() {
    let h = harness().await;
    h.show(live(&[1]));
    let frames = h.screencast(Quality::High, None).await;
    let capture = h.browser.session_of("Page.startScreencast", 0).await;

    drop(frames);
    time::sleep(STOP_GRACE / 2).await;
    assert!(
        h.browser.sent("Page.stopScreencast").is_empty(),
        "still within the grace period"
    );
    let again = h.screencast(Quality::High, None).await;
    assert_eq!(
        h.browser.sent("Page.startScreencast").len(),
        1,
        "coming back within the grace period reuses the running capture"
    );

    drop(again);
    time::sleep(STOP_GRACE + Duration::from_secs(2)).await;
    let stops = h.browser.wait_for("Page.stopScreencast", 1).await;
    assert_eq!(
        stops[0].session.as_deref(),
        Some(capture.as_str()),
        "the capture stopped"
    );
    assert_eq!(
        h.browser
            .wait_for("Target.detachFromTarget", 1)
            .await
            .iter()
            .map(|command| command.params["sessionId"].clone())
            .collect::<Vec<_>>(),
        [json!(capture)],
        "the capture's session was detached; the tab's presence was not"
    );
}

#[tokio::test(start_paused = true)]
async fn screencast_follows_a_view_change_at_once() {
    let h = harness().await;
    h.show(live(&[1, 2]));
    let mut frames = h.screencast(Quality::High, None).await;
    let first = h.browser.session_of("Page.startScreencast", 0).await;

    let changed = Instant::now();
    h.show(live(&[2, 1]));

    let second = h.browser.session_of("Page.startScreencast", 1).await;
    let switched = h.browser.sent("Page.startScreencast")[1].at - changed;
    assert_eq!(h.target_of(&second), "target-2", "the new current tab");
    // claw-server re-read the session every second; the view's change is the wake-up.
    assert!(
        switched < Duration::from_millis(100),
        "switched {switched:?} after the view changed"
    );
    h.browser.frame(&second, b"tab two", Some(1.0));
    let frame = loop {
        let frame = next(&mut frames).await;
        if frame.as_ref().is_none_or(|frame| frame.tab == tab_id(2)) {
            break frame;
        }
    };
    assert_eq!(
        frame.map(|frame| frame.jpeg),
        Some(Bytes::from_static(b"tab two")),
        "frames of the new current tab"
    );

    time::sleep(STOP_GRACE + Duration::from_secs(2)).await;
    let stops = h.browser.wait_for("Page.stopScreencast", 1).await;
    assert_eq!(
        stops[0].session.as_deref(),
        Some(first.as_str()),
        "nobody watches tab 1 any more, so its capture stopped after the grace period"
    );
}

#[tokio::test(start_paused = true)]
async fn streams_end_with_the_session_without_the_grace_period() {
    let h = harness().await;
    h.show(live(&[1]));
    let mut frames = h.screencast(Quality::High, None).await;
    let mut video = h.video(Quality::High, None).await;

    h.show(ended());

    assert!(next(&mut frames).await.is_none(), "the screencast ended");
    assert!(next(&mut video).await.is_none(), "the video ended");
    time::sleep(STOP_GRACE / 4).await;
    assert_eq!(
        h.browser.sent("Page.stopScreencast").len(),
        1,
        "the screencast stopped without its grace period"
    );
    assert_eq!(
        h.browser.sent("Page.stopScreenRecording").len(),
        1,
        "the recording stopped without its grace period"
    );
}

#[tokio::test(start_paused = true)]
async fn a_pinned_stream_ends_when_its_tab_leaves_the_view() {
    let h = harness().await;
    h.show(live(&[1, 2]));
    let mut frames = h.screencast(Quality::High, Some(1)).await;
    h.browser.wait_for("Page.startScreencast", 1).await;

    // The tab closed; the view drops it even if no detach arrives.
    h.show(live(&[2]));

    assert!(next(&mut frames).await.is_none(), "the pinned stream ended");
    assert!(
        h.browser
            .attachments()
            .iter()
            .all(|attachment| attachment.target == "target-1"),
        "it did not jump to the session's other tab"
    );
}

#[tokio::test(start_paused = true)]
async fn shutdown_ends_open_streams() {
    let h = harness().await;
    h.show(live(&[1]));
    let mut frames = h.screencast(Quality::High, None).await;

    h.shutdown.cancel();

    assert!(next(&mut frames).await.is_none(), "the stream ended");
}

#[tokio::test(start_paused = true)]
async fn video_sends_the_init_segment_then_fragments_and_late_viewers_join_at_a_fragment() {
    let h = harness().await;
    h.show(live(&[1]));
    let init = [mp4_box(*b"ftyp", b"av01"), mp4_box(*b"moov", b"tracks")].concat();
    let first = [mp4_box(*b"moof", b"1"), mp4_box(*b"mdat", b"keyframe-1")].concat();
    let second = [mp4_box(*b"moof", b"2"), mp4_box(*b"mdat", b"keyframe-2")].concat();

    let mut early = h.video(Quality::High, None).await;
    let recording = h.browser.session_of("Page.startScreenRecording", 0).await;
    assert_eq!(
        h.browser.sent("Page.startScreenRecording")[0].params,
        json!({ "maxWidth": 1280, "maxHeight": 800, "frameRate": 15 }),
        "the high preset's settings"
    );
    h.browser
        .push_recording(&recording, &[init.clone(), first.clone()].concat());

    assert_eq!(
        next(&mut early).await.as_deref(),
        Some(&init[..]),
        "the init segment first"
    );
    assert_eq!(
        next(&mut early).await.as_deref(),
        Some(&first[..]),
        "then a fragment"
    );

    let mut late = h.video(Quality::High, None).await;
    assert_eq!(
        h.browser.sent("Page.startScreenRecording").len(),
        1,
        "the late viewer shares the recording"
    );
    assert_eq!(
        next(&mut late).await.as_deref(),
        Some(&init[..]),
        "the late viewer gets the kept init segment"
    );
    h.browser.push_recording(&recording, &second);
    assert_eq!(
        next(&mut late).await.as_deref(),
        Some(&second[..]),
        "and joins at the next fragment"
    );
    assert_eq!(
        next(&mut early).await.as_deref(),
        Some(&second[..]),
        "the early viewer gets it too"
    );
}

#[tokio::test(start_paused = true)]
async fn video_waits_out_reads_before_the_recording_exists() {
    let h = harness().await;
    h.show(live(&[1]));
    let init = [mp4_box(*b"ftyp", b"av01"), mp4_box(*b"moov", b"tracks")].concat();

    let mut video = h.video(Quality::Low, None).await;
    let recording = h.browser.session_of("Page.startScreenRecording", 0).await;
    // Chromium answers "Read failed" until the recording's first write.
    h.browser.fail_reads(&recording, 3);
    h.browser.wait_for("IO.read", 3).await;
    h.browser.push_recording(&recording, &init);

    assert_eq!(
        next(&mut video).await.as_deref(),
        Some(&init[..]),
        "the init segment, once it exists"
    );
}

#[tokio::test(start_paused = true)]
async fn screencast_and_video_of_one_tab_use_separate_cdp_sessions() {
    let h = harness().await;
    h.show(live(&[1]));

    let _frames = h.screencast(Quality::High, None).await;
    let _video = h.video(Quality::High, None).await;

    // Chromium refuses a screencast and a screen recording in one session.
    let screencast = h.browser.session_of("Page.startScreencast", 0).await;
    let recording = h.browser.session_of("Page.startScreenRecording", 0).await;
    assert_ne!(screencast, recording, "one session each");
}

#[tokio::test(start_paused = true)]
async fn a_viewer_that_stops_reading_does_not_keep_an_ended_session_capturing() {
    let h = harness().await;
    h.show(live(&[1]));
    let screencast = h.screencast(Quality::High, None).await;
    let video = h.video(Quality::High, None).await;
    let capture = h.browser.session_of("Page.startScreencast", 0).await;
    let recording = h.browser.session_of("Page.startScreenRecording", 0).await;
    let init = [mp4_box(*b"ftyp", b"av01"), mp4_box(*b"moov", b"tracks")].concat();
    let fragments = (0..6)
        .map(|n| [mp4_box(*b"moof", &[n]), mp4_box(*b"mdat", b"frame")].concat())
        .collect::<Vec<_>>();

    // Neither viewer reads anything while the browser keeps producing.
    for n in 0..6 {
        h.browser.frame(&capture, &[n], Some(f64::from(n) + 1.0));
        time::sleep(Duration::from_millis(200)).await;
    }
    h.browser
        .push_recording(&recording, &[init, fragments.concat()].concat());
    time::sleep(Duration::from_secs(1)).await;

    h.show(ended());
    time::sleep(Duration::from_secs(3)).await;

    assert_eq!(
        h.browser.sent("Page.stopScreencast").len(),
        1,
        "the screencast stopped"
    );
    assert_eq!(
        h.browser.sent("Page.stopScreenRecording").len(),
        1,
        "the recording stopped"
    );
    drop((screencast, video));
}

#[tokio::test(start_paused = true)]
async fn an_unpinned_screencast_waits_for_the_sessions_next_tab() {
    let h = harness().await;
    h.show(live(&[1]));
    let mut frames = h.screencast(Quality::High, None).await;
    let first = h.browser.session_of("Page.startScreencast", 0).await;

    // The agent closes its only tab before opening the next one.
    h.show(live(&[]));
    h.browser.detach(&first);
    assert!(
        still_open(&mut frames, Duration::from_secs(3)).await,
        "a live session between tabs keeps its stream open"
    );

    h.show(live(&[2]));
    let second = h.browser.session_of("Page.startScreencast", 1).await;
    h.browser.frame(&second, b"next tab", Some(1.0));
    assert_eq!(
        next(&mut frames).await.map(|frame| (frame.tab, frame.jpeg)),
        Some((tab_id(2), Bytes::from_static(b"next tab"))),
        "the stream moved on to the next tab"
    );
}

#[tokio::test(start_paused = true)]
async fn an_earlier_grace_timer_does_not_cut_a_later_grace_period_short() {
    let h = harness().await;
    h.show(live(&[1]));
    let first = h.screencast(Quality::High, None).await;
    h.browser.wait_for("Page.startScreencast", 1).await;

    drop(first); // t=0: the first grace period starts
    time::sleep(Duration::from_secs(2)).await;
    let second = h.screencast(Quality::High, None).await;
    time::sleep(Duration::from_secs(2)).await;
    drop(second); // t=4: a new grace period, ending at t=9

    time::sleep(Duration::from_millis(1500)).await; // t=5.5
    assert!(
        h.browser.sent("Page.stopScreencast").is_empty(),
        "the first grace timer did not stop the capture"
    );
    time::sleep(STOP_GRACE).await;
    assert_eq!(
        h.browser.sent("Page.stopScreencast").len(),
        1,
        "the second grace period ran out"
    );
    assert_eq!(
        h.browser.sent("Page.startScreencast").len(),
        1,
        "one capture throughout"
    );
}

#[tokio::test(start_paused = true)]
async fn a_slow_tab_switch_does_not_keep_a_departed_viewer_capturing() {
    let h = harness().await;
    h.show(live(&[1, 2]));
    let frames = h.screencast(Quality::High, None).await;
    let first = h.browser.session_of("Page.startScreencast", 0).await;

    // The session moves to tab 2 while the browser is slow to attach.
    h.browser.delay_attaches(Duration::from_mins(1));
    h.show(live(&[2, 1]));
    // Tab 1's presence and capture, then tab 2's presence.
    h.browser.wait_for("Target.attachToTarget", 3).await;
    drop(frames);

    // The viewer left mid-switch, so tab 1's capture stops after its grace period, not after
    // the browser finally answers.
    time::sleep(STOP_GRACE + Duration::from_secs(2)).await;
    let stops = h.browser.sent("Page.stopScreencast");
    assert_eq!(
        stops.first().and_then(|stop| stop.session.clone()),
        Some(first),
        "tab 1's capture stopped"
    );
}

// ----- Neo's rules ----------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn withholding_ends_streams_and_a_waiting_frame_never_leaves() {
    let h = harness().await;
    h.show(live(&[1]));
    let mut frames = h.screencast(Quality::High, None).await;
    let mut video = h.video(Quality::High, None).await;
    let capture = h.browser.session_of("Page.startScreencast", 0).await;
    h.browser.frame(&capture, b"before the secret", Some(1.0));
    // Unread, the frame now waits in the viewer's slot.
    time::sleep(Duration::from_millis(100)).await;

    h.view.send_modify(|view| view.withheld = true);

    assert!(
        next(&mut frames).await.is_none(),
        "no part starts once the view is withheld, not even the one waiting"
    );
    assert!(next(&mut video).await.is_none(), "the video ended");
    time::sleep(STOP_GRACE / 4).await;
    assert_eq!(
        h.browser.sent("Page.stopScreencast").len(),
        1,
        "the capture stopped without its grace period"
    );
    assert_eq!(
        h.hub
            .screencast(h.view.subscribe(), Quality::High, None)
            .await
            .err(),
        Some(CaptureError::Withheld),
        "a new view of a withheld session is refused"
    );
}

#[tokio::test(start_paused = true)]
async fn a_tainted_tabs_captures_end_at_once_for_every_session() {
    let h = harness().await;
    h.show(live(&[1, 2]));
    let mut frames = h.screencast(Quality::High, None).await;
    let first = h.browser.session_of("Page.startScreencast", 0).await;

    // Another session that touched tab 1 was withheld: this view drops the tainted tab, and the
    // host ends the tab's captures.
    let tainted = Instant::now();
    h.show(live(&[2]));
    h.hub.end_captures(&PageId::new("target-1".to_string()));

    let stops = h.browser.wait_for("Page.stopScreencast", 1).await;
    assert_eq!(
        stops[0].session.as_deref(),
        Some(first.as_str()),
        "the tainted tab's capture stopped"
    );
    assert!(
        stops[0].at - tainted < STOP_GRACE / 4,
        "without its grace period"
    );
    let second = h.browser.session_of("Page.startScreencast", 1).await;
    h.browser.frame(&second, b"untainted", Some(1.0));
    assert_eq!(
        next(&mut frames).await.map(|frame| (frame.tab, frame.jpeg)),
        Some((tab_id(2), Bytes::from_static(b"untainted"))),
        "the follower moved on to the tab its view still shows"
    );
}

#[tokio::test(start_paused = true)]
async fn a_stalled_viewer_is_dropped_after_thirty_seconds() {
    let h = harness().await;
    h.show(live(&[1]));
    let mut frames = h.screencast(Quality::High, None).await;
    let capture = h.browser.session_of("Page.startScreencast", 0).await;

    h.browser.frame(&capture, b"never read", Some(1.0));
    time::sleep(VIEWER_STALL.saturating_sub(Duration::from_secs(1))).await;
    assert!(
        h.browser.sent("Page.stopScreencast").is_empty(),
        "a viewer inside the stall window keeps its capture"
    );
    time::sleep(Duration::from_secs(1) + STOP_GRACE + Duration::from_secs(1)).await;

    assert_eq!(
        h.browser.sent("Page.stopScreencast").len(),
        1,
        "the stalled viewer was dropped, and its capture stopped after the grace period"
    );
    assert!(
        next(&mut frames).await.is_none(),
        "the stalled viewer's stream ended without the frame it left waiting"
    );
}

#[tokio::test(start_paused = true)]
async fn a_viewer_with_nothing_waiting_is_never_stalled() {
    let h = harness().await;
    h.show(live(&[1]));
    let mut frames = h.screencast(Quality::High, None).await;
    let capture = h.browser.session_of("Page.startScreencast", 0).await;
    h.browser.frame(&capture, b"static page", Some(1.0));
    assert!(next(&mut frames).await.is_some(), "the one frame was read");

    // A still page sends nothing more, and the viewer has read everything.
    assert!(
        still_open(&mut frames, VIEWER_STALL * 2).await,
        "the stream stays open"
    );
    assert!(
        h.browser.sent("Page.stopScreencast").is_empty(),
        "the capture keeps running"
    );
}

#[tokio::test(start_paused = true)]
async fn frames_without_a_browser_time_or_over_the_cap_are_dropped_and_counted() {
    let h = harness().await;
    h.show(live(&[1]));
    let mut frames = h.screencast(Quality::High, None).await;
    let capture = h.browser.session_of("Page.startScreencast", 0).await;

    h.browser.frame(&capture, b"no timestamp", None);
    h.browser.frame(&capture, b"zero timestamp", Some(0.0));
    h.browser
        .frame(&capture, &vec![0xff; MAX_FRAME_BYTES + 1], Some(1.0));
    h.browser.frame(&capture, b"good", Some(2.0));

    assert_eq!(
        next(&mut frames).await.map(|frame| frame.jpeg),
        Some(Bytes::from_static(b"good")),
        "only the timed frame within the cap reached the viewer"
    );
    assert_eq!(
        h.browser.wait_for("Page.screencastFrameAck", 4).await.len(),
        4,
        "every frame was still acknowledged"
    );
    let drops = h.hub.dropped_frames();
    assert_eq!(
        (drops.untimed(), drops.oversized(), drops.undecodable()),
        (2, 1, 0),
        "the dropped frames were counted by reason"
    );
}

#[tokio::test(start_paused = true)]
async fn the_presence_turns_emulation_on_before_any_capture_and_captures_never_emulate() {
    let h = harness().await;
    h.show(live(&[1, 2]));

    let _screencast = h.screencast(Quality::High, None).await;
    let _video = h.video(Quality::High, Some(2)).await;
    let _low = h.screencast(Quality::Low, Some(1)).await;

    let commands = h.browser.commands();
    // The n-th attach command is answered with the n-th session the fake handed out.
    let attached: Vec<(usize, String)> = commands
        .iter()
        .enumerate()
        .filter(|(_, command)| command.method == "Target.attachToTarget")
        .map(|(index, _)| index)
        .zip(h.browser.attachments())
        .map(|(index, attachment)| (index, attachment.session))
        .collect();
    let emulating: HashSet<String> = commands
        .iter()
        .filter(|command| command.method.starts_with("Emulation."))
        .filter_map(|command| command.session.clone())
        .collect();
    let capturing: HashSet<String> = commands
        .iter()
        .filter(|command| command.method.starts_with("Page."))
        .filter_map(|command| command.session.clone())
        .collect();
    assert_eq!(capturing.len(), 3, "three captures");
    assert!(
        emulating.is_disjoint(&capturing),
        "capture sessions send no Emulation command"
    );
    assert!(
        commands
            .iter()
            .filter(|command| command.method.starts_with("Emulation."))
            .all(
                |command| command.method == "Emulation.setFocusEmulationEnabled"
                    && command.params == json!({ "enabled": true })
            ),
        "emulation is only ever turned on"
    );
    for capture in &capturing {
        let target = h.target_of(capture);
        let attach = attached
            .iter()
            .find(|(_, session)| session == capture)
            .map(|(index, _)| *index);
        let emulation = commands.iter().position(|command| {
            command.method == "Emulation.setFocusEmulationEnabled"
                && command
                    .session
                    .as_deref()
                    .is_some_and(|session| h.target_of(session) == target)
        });
        assert!(
            emulation.is_some() && emulation < attach,
            "{target}'s presence emulated before its capture {capture} attached"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_new_socket_ends_every_capture_and_presence_and_the_screencast_resumes() {
    let h = harness().await;
    h.show(live(&[1]));
    let mut frames = h.screencast(Quality::High, None).await;
    let mut video = h.video(Quality::High, None).await;
    let old: HashSet<String> = h
        .browser
        .attachments()
        .into_iter()
        .map(|attachment| attachment.session)
        .collect();

    h.browser.drop_connection();

    assert!(
        next(&mut video).await.is_none(),
        "the video ended with its capture"
    );
    let resumed = h.browser.session_of("Page.startScreencast", 1).await;
    assert!(
        !old.contains(&resumed),
        "the screencast resumed on a new session"
    );
    h.browser.frame(&resumed, b"after the reconnect", Some(1.0));
    let frame = loop {
        let frame = next(&mut frames).await;
        if frame
            .as_ref()
            .is_none_or(|frame| frame.jpeg == Bytes::from_static(b"after the reconnect"))
        {
            break frame;
        }
    };
    assert!(frame.is_some(), "the follower re-leased on the new socket");
    let stale = h
        .browser
        .commands()
        .into_iter()
        .filter(|command| command.connection > 1)
        .filter(|command| {
            let named = |id: Option<&str>| id.is_some_and(|id| old.contains(id));
            named(command.session.as_deref()) || named(command.params["sessionId"].as_str())
        })
        .count();
    assert_eq!(
        stale, 0,
        "no command named an old session id on the new socket"
    );
    let new_presences = h
        .browser
        .sent("Emulation.setFocusEmulationEnabled")
        .into_iter()
        .filter(|command| command.connection > 1)
        .count();
    assert_eq!(
        new_presences, 1,
        "the tab got a new presence on the new socket"
    );
}

#[tokio::test(start_paused = true)]
async fn a_video_stays_on_its_tab_and_ends_when_the_tab_leaves_the_view() {
    let h = harness().await;
    h.show(live(&[1, 2]));
    let mut video = h.video(Quality::High, None).await;
    h.browser.wait_for("Page.startScreenRecording", 1).await;

    h.show(live(&[2, 1]));
    assert!(
        still_open(&mut video, Duration::from_secs(2)).await,
        "a video does not follow the current tab"
    );
    assert_eq!(
        h.browser.sent("Page.startScreenRecording").len(),
        1,
        "no second recording"
    );

    h.show(live(&[2]));
    assert!(
        next(&mut video).await.is_none(),
        "the video ended when its tab left the view"
    );
}
