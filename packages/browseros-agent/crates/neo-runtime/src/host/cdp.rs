//! The host side's CDP surface in one marked place: every command it sends on its own socket,
//! every event it reads there, and every constant it ports, each with its source. The modules
//! behind it see domain types only.
//!
//! A command is typed through browseros-cdp's surface where the surface has its method; the
//! design's "CDP methods and events" table lists the untyped ones for browseros-cdp's
//! `protocol/surface.json`. Commands on an attached session take its [`Attached`], whose `send`
//! writes at the epoch the session was attached in; browser-level commands that name a session
//! take that epoch explicitly. Either way a command racing a reconnect is refused before it is
//! written, so a session id never reaches a socket it does not exist on.

use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::{DecodeError, Engine as _};
use browseros_cdp::{
    CdpClient, CdpError, CdpEvent, ConnectOptions, EmptyParams, ReconnectPolicy, SessionId,
    browser, page, target,
};
use bytes::Bytes;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::debug;

use crate::host::capture::Preset;
use crate::host::directory::{DirectoryInput, PageTarget, TabEntry};
use crate::host::ids::TabId;
use crate::host::link::{Attached, HostLink, LinkError};
use crate::host::store::CallArtifacts;
use crate::run::PageId;
use tokio::sync::broadcast;

// ----- the tab directory (`directory.rs`) ----------------------------------------------------

/// How long a closed tab's last facts stay readable, for a call whose tab closed before its page
/// facts were read. claw-server keeps destroyed tab mappings as long, for late attribution
/// (`apps/claw-server/src/services/browser/tab_registry.rs:20-21`).
pub(crate) const TAB_TOMBSTONE: Duration = Duration::from_mins(5);

/// One event on the host socket as the tab directory reads it: the `Target.targetCreated`,
/// `Target.targetInfoChanged` and `Target.targetDestroyed` that `Target.setDiscoverTargets`
/// delivers. `None` for every other event, for params that do not parse, and for targets that
/// are not tabs.
pub(crate) fn directory_input(event: &CdpEvent) -> Option<DirectoryInput> {
    match event.method.as_str() {
        "Target.targetCreated" => target::TargetCreatedEvent::deserialize(&event.params)
            .ok()
            .and_then(|created| page_target(created.target_info))
            .map(DirectoryInput::Created),
        "Target.targetInfoChanged" => target::TargetInfoChangedEvent::deserialize(&event.params)
            .ok()
            .and_then(|changed| page_target(changed.target_info))
            .map(DirectoryInput::Changed),
        "Target.targetDestroyed" => target::TargetDestroyedEvent::deserialize(&event.params)
            .ok()
            .map(|destroyed| DirectoryInput::Destroyed(PageId::new(destroyed.target_id))),
        _ => None,
    }
}

/// The `Browser.getTabs` reply as the directory's seed. A tab without a positive tab id is left
/// out, as it would be from discovery.
pub(crate) fn seed(reply: browser::GetTabsResult) -> DirectoryInput {
    let tabs = reply
        .tabs
        .into_iter()
        .filter_map(|info| {
            Some(PageTarget {
                target: PageId::new(info.target_id),
                entry: TabEntry {
                    tab: TabId::parse(info.tab_id)?,
                    url: info.url,
                    title: info.title,
                },
            })
        })
        .collect();
    DirectoryInput::Seeded(tabs)
}

/// Subscribe before discovery; the seed returns an event receiver whose first event is
/// strictly after its reply. This is the freshness boundary described in design §7.
pub(crate) async fn discover_and_seed(
    link: &HostLink,
) -> Result<(u64, DirectoryInput, broadcast::Receiver<CdpEvent>), LinkError> {
    let epoch = link.current()?.epoch();
    let _: target::SetDiscoverTargetsResult = send_at(
        link.client(),
        epoch,
        "Target.setDiscoverTargets",
        target::SetDiscoverTargetsParams {
            discover: true,
            ..target::SetDiscoverTargetsParams::default()
        },
        None,
    )
    .await?;
    let (reply, events) = link
        .client()
        .send_at_epoch_with_events(epoch, "Browser.getTabs", json!({"includeHidden":true}))
        .await?;
    Ok((
        epoch,
        seed(serde_json::from_value(reply).map_err(CdpError::from)?),
        events,
    ))
}

/// A discovered target, if it is a tab: a `page` target carrying BrowserOS's patched `tabId`
/// (probe 6 in the neo-host-events design), positive. Workers, iframes and the browser target
/// are not tabs.
fn page_target(info: target::TargetInfo) -> Option<PageTarget> {
    if info.r#type != "page" {
        return None;
    }
    Some(PageTarget {
        target: PageId::new(info.target_id),
        entry: TabEntry {
            tab: TabId::parse(info.tab_id?)?,
            url: info.url,
            title: info.title,
        },
    })
}

// ----- the host link (`link.rs`): claw-server `services/capture`, unchanged values -----------

/// How often the link checks that its socket is still the incarnation its sessions were
/// attached on. `apps/claw-server/src/services/capture/link.rs:19-20` (origin/main).
pub(crate) const WATCHDOG_INTERVAL: Duration = Duration::from_secs(1);

/// Neo: claw-server's `CdpPortTransport::connect` (`apps/claw-server/src/services/capture/ports.rs`), bounding host connection startup to 2 s per attempt.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Neo: claw-server's `CdpPortTransport::connect` (`apps/claw-server/src/services/capture/ports.rs`), spacing retries so a lost browser does not spin the host socket.
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// The host side's own client: connect within 2 s with one retry, and reconnect every second
/// for as long as it takes. The default policy exits the process when reconnects run out.
/// `apps/claw-server/src/services/capture/ports.rs:122-131` (origin/main).
pub(crate) fn connect_options(port: u16) -> ConnectOptions {
    ConnectOptions {
        connect_timeout: CONNECT_TIMEOUT,
        connect_max_retries: 1,
        reconnect_policy: ReconnectPolicy::KeepTrying,
        reconnect_delay: RECONNECT_DELAY,
        reconnect_max_retries: usize::MAX,
        ..ConnectOptions::new(port)
    }
}

/// The one event the link takes off the shared ring with `events_targeted`: frames are large
/// and frequent, and the ring would copy them once per subscriber and hold thousands
/// (`apps/claw-server/src/services/capture/link.rs:46-48`, origin/main).
pub(crate) const SCREENCAST_FRAME: &str = "Page.screencastFrame";

/// `Target.attachToTarget{targetId, flatten: true}` (typed) on the socket incarnation `epoch`.
pub(crate) async fn attach_to_target(
    client: &CdpClient,
    epoch: u64,
    target: &PageId,
) -> Result<SessionId, CdpError> {
    let attached: target::AttachToTargetResult = send_at(
        client,
        epoch,
        "Target.attachToTarget",
        target::AttachToTargetParams {
            target_id: target.as_str().to_string(),
            flatten: Some(true),
        },
        None,
    )
    .await?;
    Ok(SessionId::from(attached.session_id))
}

/// `Target.detachFromTarget{sessionId}` (typed), on the incarnation `epoch` the session belongs
/// to.
pub(crate) async fn detach_from_target(
    client: &CdpClient,
    epoch: u64,
    session: &SessionId,
) -> Result<(), CdpError> {
    let _: target::DetachFromTargetResult = send_at(
        client,
        epoch,
        "Target.detachFromTarget",
        target::DetachFromTargetParams {
            session_id: Some(session.as_str().to_string()),
            target_id: None,
        },
        None,
    )
    .await?;
    Ok(())
}

/// The session a `Target.detachedFromTarget` (typed) names: the browser closed it, which is how
/// a closed tab shows up on the sessions attached to it.
pub(crate) fn detached_session(event: &CdpEvent) -> Option<SessionId> {
    if event.method != "Target.detachedFromTarget" {
        return None;
    }
    let detached: target::DetachedFromTargetEvent =
        serde_json::from_value(event.params.clone()).ok()?;
    Some(SessionId::from(detached.session_id))
}

async fn send_at<P: Serialize, R: DeserializeOwned>(
    client: &CdpClient,
    epoch: u64,
    method: &str,
    params: P,
    session: Option<&SessionId>,
) -> Result<R, CdpError> {
    let reply = client
        .send_at_epoch(epoch, method, serde_json::to_value(params)?, session)
        .await?;
    Ok(serde_json::from_value(reply)?)
}

// ----- presences (`presence.rs`) -------------------------------------------------------------

/// `Emulation.setFocusEmulationEnabled{enabled: true}` (untyped: browseros-cdp's surface has no
/// Emulation domain), on a tab's presence alone. Never `false`: see `presence.rs`.
pub(crate) async fn emulate_focus(session: &Attached) -> Result<(), CdpError> {
    session
        .send(
            "Emulation.setFocusEmulationEnabled",
            json!({ "enabled": true }),
        )
        .await?;
    Ok(())
}

// ----- after-call artifacts (`after_call.rs`) ------------------------------------------------

/// Screenshot concurrency and timeout copied from claw-server's cockpit visual worker:
/// `apps/claw-server/src/services/cockpit/visual.rs:25-26,305-322` (origin/main).
pub(crate) const SCREENSHOT_SLOTS: usize = 2;
pub(crate) const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(2);
/// Neo policy: Zuzu accepts at most 64 KiB JPEGs in audit records (design §8).
pub(crate) const MAX_STILL_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy)]
pub(crate) struct StillRung {
    width: u32,
    height: u32,
    quality: i64,
}

/// Neo's bounded audit-image ladder, rather than full viewport images that Zuzu discards.
pub(crate) const STILL_LADDER: [StillRung; 3] = [
    StillRung {
        width: 800,
        height: 500,
        quality: 50,
    },
    StillRung {
        width: 640,
        height: 400,
        quality: 50,
    },
    StillRung {
        width: 640,
        height: 400,
        quality: 30,
    },
];

/// One browser-level page-facts read. `None` means the tab closed; only that case may use the
/// directory's tombstone. Other failures are permanent absence, never filled from live state.
/// The after-call read ports `apps/claw-server/src/api/mcp/observers/audit.rs:117-121` (origin/main).
pub(crate) async fn tab_facts(
    link: &HostLink,
    target: &PageId,
) -> Result<Option<CallArtifacts>, LinkError> {
    let epoch = link.current()?.epoch();
    let result: Result<browser::GetTabInfoResult, _> = send_at(
        link.client(),
        epoch,
        "Browser.getTabInfo",
        browser::GetTabInfoParams {
            target_id: Some(target.as_str().to_string()),
            tab_id: None,
        },
        None,
    )
    .await;
    match result {
        Ok(info) => {
            let tab = TabId::parse(info.tab.tab_id).ok_or_else(|| {
                CdpError::Transport("page facts carried an invalid tab id".to_string())
            })?;
            Ok(Some(CallArtifacts {
                tab,
                url: info.tab.url,
                title: info.tab.title,
                screenshot: None,
            }))
        }
        Err(CdpError::Protocol { message, .. }) if message.contains("No target with given id") => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

/// The geometry Playwright reads before taking a viewport screenshot. The clip uses document
/// coordinates, enclosing integer size, current page scale and device-scale correction:
/// `packages/playwright-core/src/server/chromium/crPage.ts:275-296` at 26137face.
pub(crate) struct ScreenshotViewport {
    metrics: page::GetLayoutMetricsResult,
}

pub(crate) async fn screenshot_viewport(
    session: &Attached,
) -> Result<ScreenshotViewport, CdpError> {
    let metrics = send_on(session, "Page.getLayoutMetrics", EmptyParams {}).await?;
    Ok(ScreenshotViewport { metrics })
}

pub(crate) async fn capture_still(
    session: &Attached,
    viewport: &ScreenshotViewport,
    rung: StillRung,
) -> Result<Bytes, CdpError> {
    let metrics = &viewport.metrics;
    let visual = &metrics.visual_viewport;
    if !visual.scale.is_finite()
        || visual.scale <= 0.0
        || visual.client_width <= 0.0
        || visual.client_height <= 0.0
    {
        return Err(CdpError::Transport(
            "invalid screenshot viewport".to_string(),
        ));
    }
    let fit = (f64::from(rung.width) / visual.client_width)
        .min(f64::from(rung.height) / visual.client_height)
        .min(1.0);
    let device_scale = metrics.content_size.width / metrics.css_content_size.width;
    let device_scale = if device_scale.is_finite() && device_scale > 0.0 {
        device_scale
    } else {
        1.0
    };
    let clip = page::Viewport {
        x: visual.page_x,
        y: visual.page_y,
        width: (visual.client_width / visual.scale).ceil(),
        height: (visual.client_height / visual.scale).ceil(),
        scale: visual.scale * fit / device_scale,
    };
    let reply: page::CaptureScreenshotResult = send_on(
        session,
        "Page.captureScreenshot",
        page::CaptureScreenshotParams {
            format: Some("jpeg".to_string()),
            quality: Some(rung.quality),
            clip: Some(clip),
            capture_beyond_viewport: Some(false),
            ..page::CaptureScreenshotParams::default()
        },
    )
    .await?;
    let data = reply
        .data
        .as_str()
        .ok_or_else(|| CdpError::Transport("screenshot had no base64 data".to_string()))?;
    BASE64
        .decode(data)
        .map(Bytes::from)
        .map_err(|error| CdpError::Transport(error.to_string()))
}

// ----- live capture (`capture.rs`): claw-server `services/capture`, unchanged values ---------
// The quality presets live verbatim in `capture/presets.rs`, which ports their table whole.

/// How long a capture outlives its last viewer, so a reconnecting viewer or a follow switch
/// back does not restart the browser side.
/// `apps/claw-server/src/services/capture/tab_capture.rs:31-32` (origin/main).
pub(crate) const STOP_GRACE: Duration = Duration::from_secs(5);

/// How often a recording's IO stream is drained. Chromium writes a fragment per keyframe
/// interval (about 2 s), so this only bounds how late a finished fragment is forwarded.
/// `apps/claw-server/src/services/capture/tab_capture.rs:33-35` (origin/main).
pub(crate) const RECORDING_READ_INTERVAL: Duration = Duration::from_millis(250);

/// `apps/claw-server/src/services/capture/tab_capture.rs:36` (origin/main).
pub(crate) const RECORDING_READ_SIZE: u64 = 1 << 20;

/// Reads that may fail before the recording's first bytes exist: Chromium creates the stream's
/// file on its first write and answers earlier reads with "Read failed".
/// `apps/claw-server/src/services/capture/tab_capture.rs:37-39` (origin/main).
pub(crate) const EARLY_READ_FAILURES: u32 = 20;

/// Fragments a slow video viewer may fall behind before it skips ahead.
/// `apps/claw-server/src/services/capture/tab_capture.rs:40-41` (origin/main).
pub(crate) const FRAGMENT_BACKLOG: usize = 8;

/// One unacknowledged frame at a time: while it is out, Chromium drops newer frames before it
/// copies them on its UI thread, which is what makes ack pacing cheap.
/// `apps/claw-server/src/services/capture/tab_capture.rs:298-301` (origin/main).
pub(crate) const MAX_FRAMES_IN_FLIGHT: i64 = 1;

/// How long a followed screencast waits for the browser to start capturing its new tab.
/// `apps/claw-server/src/services/capture/mod.rs:50-52` (origin/main).
pub(crate) const SWITCH_TIMEOUT: Duration = Duration::from_secs(5);

/// How soon a follower whose switch failed tries again, if nothing else changed first:
/// claw-server's session check interval, which drove its retries
/// (`apps/claw-server/src/services/capture/mod.rs:47-49`, origin/main).
pub(crate) const SWITCH_RETRY: Duration = Duration::from_secs(1);

/// Video fragments queued for one viewer. One that falls further behind skips whole fragments,
/// which is safe because every fragment starts on a keyframe.
/// `apps/claw-server/src/services/capture/mod.rs:53-55` (origin/main).
pub(crate) const VIDEO_QUEUE: usize = 4;

// ----- live capture: Neo policy --------------------------------------------------------------

/// A viewer that leaves a waiting frame or fragment unread this long is dropped: its lease
/// would otherwise keep the capture running until TCP gives up on it.
pub(crate) const VIEWER_STALL: Duration = Duration::from_secs(30);

/// The largest frame forwarded: the main host's reader rejects a part over 512 KiB (Zuzu's
/// `cell/mirrord/internal/neo/frames.go:54-66`, per the neo-host-events design §7). The `high`
/// preset measured 90-149 KB (probe 7).
pub(crate) const MAX_FRAME_BYTES: usize = 512 * 1024;

/// `Page.startScreencast{format: "jpeg", quality, maxWidth, maxHeight, maxFramesInFlight: 1}`
/// (typed). Playwright allows 5 or 10 frames in flight and acknowledges each as soon as it is
/// handed on (`packages/playwright-core/src/server/chromium/crPage.ts:321-329,993-1003` at
/// 26137face); one in flight and a delayed ack is claw-server's measured cheaper pacing.
pub(crate) async fn start_screencast(session: &Attached, preset: Preset) -> Result<(), CdpError> {
    let params = page::StartScreencastParams {
        format: Some("jpeg".to_string()),
        quality: Some(i64::from(preset.jpeg_quality)),
        max_width: Some(i64::from(preset.max_width)),
        max_height: Some(i64::from(preset.max_height)),
        max_frames_in_flight: Some(MAX_FRAMES_IN_FLIGHT),
        ..page::StartScreencastParams::default()
    };
    let _: page::StartScreencastResult = send_on(session, "Page.startScreencast", params).await?;
    Ok(())
}

/// `Page.screencastFrameAck{sessionId}` (typed). Its `sessionId` is the frame's number, not a
/// CDP session id (`apps/claw-server/src/services/capture/tab_capture.rs:324`, origin/main).
pub(crate) async fn ack_frame(session: &Attached, number: i64) -> Result<(), CdpError> {
    let _: page::ScreencastFrameAckResult = send_on(
        session,
        "Page.screencastFrameAck",
        page::ScreencastFrameAckParams { session_id: number },
    )
    .await?;
    Ok(())
}

/// `Page.stopScreencast` (typed), best effort: the tab may be gone.
pub(crate) async fn stop_screencast(session: &Attached) {
    let stopped: Result<page::StopScreencastResult, _> =
        send_on(session, "Page.stopScreencast", EmptyParams {}).await;
    if let Err(error) = stopped {
        debug!(session = %session.session, %error, "stopping a screencast failed");
    }
}

/// `Page.startScreenRecording{maxWidth, maxHeight, frameRate}` (untyped) → the `IO` stream
/// handle. Chromium encodes AV1 into a fragmented MP4 it writes as it goes, a fragment about
/// every 2 s (probe 5).
pub(crate) async fn start_recording(
    session: &Attached,
    preset: Preset,
) -> Result<String, CdpError> {
    #[derive(Deserialize)]
    struct Started {
        stream: String,
    }
    let reply = session
        .send(
            "Page.startScreenRecording",
            json!({
                "maxWidth": preset.max_width,
                "maxHeight": preset.max_height,
                "frameRate": preset.video_fps,
            }),
        )
        .await?;
    let started: Started = serde_json::from_value(reply)?;
    Ok(started.stream)
}

/// `IO.read{handle, size}` (untyped) on a recording's stream.
pub(crate) async fn read_recording(
    session: &Attached,
    handle: &str,
) -> Result<RecordingChunk, CdpError> {
    let reply = session
        .send(
            "IO.read",
            json!({ "handle": handle, "size": RECORDING_READ_SIZE }),
        )
        .await?;
    Ok(serde_json::from_value(reply)?)
}

/// `Page.stopScreenRecording`, then `IO.close{handle}` (both untyped), best effort.
pub(crate) async fn stop_recording(session: &Attached, handle: &str) {
    let stops = [
        ("Page.stopScreenRecording", json!({})),
        ("IO.close", json!({ "handle": handle })),
    ];
    for (method, params) in stops {
        if let Err(error) = session.send(method, params).await {
            debug!(session = %session.session, %error, "{method} failed");
        }
    }
}

/// One `IO.read` reply. While recording, an empty chunk only means "nothing more yet"; its
/// `eof` is set then too, so it is not read.
#[derive(Deserialize)]
pub(crate) struct RecordingChunk {
    data: String,
    #[serde(rename = "base64Encoded", default)]
    base64_encoded: bool,
}

impl RecordingChunk {
    pub(crate) fn bytes(&self) -> Result<Vec<u8>, DecodeError> {
        if self.base64_encoded {
            BASE64.decode(&self.data)
        } else {
            Ok(self.data.as_bytes().to_vec())
        }
    }
}

// ----- live capture's events ----------------------------------------------------------------

/// One `Page.screencastFrame`, decoded.
pub(crate) struct ScreencastFrame {
    /// What its acknowledgement echoes. Chromium sends nothing more until the ack, so a frame
    /// whose event does not parse is still acknowledged when this can be read.
    pub(crate) number: Option<i64>,
    /// `None` when the event or its base64 data does not parse.
    pub(crate) jpeg: Option<Vec<u8>>,
    /// `metadata.timestamp`: the browser's frame-swap time, epoch seconds (probe 8).
    pub(crate) timestamp: Option<f64>,
}

/// Decodes `Page.screencastFrame` params (typed).
pub(crate) fn screencast_frame(params: &Value) -> ScreencastFrame {
    match page::ScreencastFrameEvent::deserialize(params) {
        Ok(event) => ScreencastFrame {
            number: Some(event.session_id),
            jpeg: event
                .data
                .as_str()
                .and_then(|data| BASE64.decode(data).ok()),
            timestamp: event.metadata.timestamp,
        },
        Err(_) => ScreencastFrame {
            number: params.get("sessionId").and_then(Value::as_i64),
            jpeg: None,
            timestamp: None,
        },
    }
}

async fn send_on<P: Serialize, R: DeserializeOwned>(
    session: &Attached,
    method: &str,
    params: P,
) -> Result<R, CdpError> {
    let reply = session.send(method, serde_json::to_value(params)?).await?;
    Ok(serde_json::from_value(reply)?)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use serde_json::json;

    use super::*;

    // Recorded from BrowserOS neo 155.0.8318.26 (headless): what `Target.setDiscoverTargets` and
    // `Browser.getTabs` delivered around a tab made with `Target.createTarget` and then closed.
    const CREATED: &str = r#"{"targetInfo":{"targetId":"7EC663DA0290AB032344B3FF32267FE0","type":"page","title":"","url":"data:text/html,<title>Probe</title>hi","attached":false,"canAccessOpener":false,"browserContextId":"27C255E9D02F509F66CFFEB8A45DAA16","tabId":196019726}}"#;
    const CHANGED: &str = r#"{"targetInfo":{"targetId":"7EC663DA0290AB032344B3FF32267FE0","type":"page","title":"data:text/html,<title>Probe</title>hi","url":"data:text/html,<title>Probe</title>hi","attached":false,"canAccessOpener":false,"browserContextId":"27C255E9D02F509F66CFFEB8A45DAA16","tabId":196019726,"windowId":196019724}}"#;
    const DESTROYED: &str = r#"{"targetId":"7EC663DA0290AB032344B3FF32267FE0"}"#;
    const OMNIBOX: &str = r#"{"targetInfo":{"targetId":"52AB4B1B326CFE1C51BA6619E281A86A","type":"browser_ui","title":"Omnibox Popup","url":"chrome://omnibox-popup.top-chrome/","attached":false,"canAccessOpener":false,"browserContextId":"27C255E9D02F509F66CFFEB8A45DAA16"}}"#;
    const WORKER: &str = r#"{"targetInfo":{"targetId":"5CCF81BD9E22C9F4AF3E8B1CB1276785","type":"service_worker","title":"Service Worker chrome-extension://adlpneommgkgeanpaekgoaolcpncohkf/background.js","url":"chrome-extension://adlpneommgkgeanpaekgoaolcpncohkf/background.js","attached":false,"canAccessOpener":false,"browserContextId":"27C255E9D02F509F66CFFEB8A45DAA16"}}"#;
    const TABS: &str = r#"{"tabs":[{"tabId":196019725,"targetId":"7A20C3F0317E37E3F8E43ED16F70A196","url":"about:blank","title":"about:blank","isActive":false,"isLoading":false,"loadProgress":0,"isPinned":false,"isHidden":false,"windowId":196019724,"index":0,"browserContextId":"Default","browserProfileId":"BF7F0FC3C4674C40D1A19F8E41A061F4","liveBrowserContextId":"27C255E9D02F509F66CFFEB8A45DAA16"},{"tabId":196019726,"targetId":"7EC663DA0290AB032344B3FF32267FE0","url":"data:text/html,<title>Probe</title>hi","title":"Probe","isActive":true,"isLoading":false,"loadProgress":0,"isPinned":false,"isHidden":false,"windowId":196019724,"index":1,"browserContextId":"Default","browserProfileId":"BF7F0FC3C4674C40D1A19F8E41A061F4","liveBrowserContextId":"27C255E9D02F509F66CFFEB8A45DAA16"}]}"#;
    const PROBE_URL: &str = "data:text/html,<title>Probe</title>hi";

    fn recorded(params: &str) -> Value {
        serde_json::from_str(params).expect("recorded params are JSON")
    }

    fn event(method: &str, params: Value) -> CdpEvent {
        CdpEvent {
            method: method.to_string(),
            params,
            session_id: None,
        }
    }

    fn tab(target_id: &str, tab: i64, url: &str, title: &str) -> PageTarget {
        PageTarget {
            target: PageId::new(target_id.to_string()),
            entry: TabEntry {
                tab: TabId::parse(tab).expect("a positive tab id"),
                url: url.to_string(),
                title: title.to_string(),
            },
        }
    }

    fn probe_tab(title: &str) -> PageTarget {
        tab(
            "7EC663DA0290AB032344B3FF32267FE0",
            196_019_726,
            PROBE_URL,
            title,
        )
    }

    #[test]
    fn discovery_events_become_directory_inputs() {
        assert_eq!(
            directory_input(&event("Target.targetCreated", recorded(CREATED))),
            Some(DirectoryInput::Created(probe_tab(""))),
            "a new tab as discovery first reports it: a tab id, no window or title yet"
        );
        assert_eq!(
            directory_input(&event("Target.targetInfoChanged", recorded(CHANGED))),
            Some(DirectoryInput::Changed(probe_tab(PROBE_URL))),
            "a tab whose title changed"
        );
        assert_eq!(
            directory_input(&event("Target.targetDestroyed", recorded(DESTROYED))),
            Some(DirectoryInput::Destroyed(PageId::new(
                "7EC663DA0290AB032344B3FF32267FE0".to_string()
            ))),
            "a destroyed target, whatever it was"
        );
    }

    #[test]
    fn only_tabs_enter_the_directory() {
        let with = |field: &str, value: Value| {
            let mut params = recorded(CREATED);
            params["targetInfo"][field] = value;
            params
        };
        let mut untabbed = recorded(CREATED);
        untabbed["targetInfo"]
            .as_object_mut()
            .expect("an object")
            .remove("tabId");
        let not_tabs = [
            (recorded(OMNIBOX), "browser UI"),
            (recorded(WORKER), "an extension's service worker"),
            (
                with("type", json!("iframe")),
                "an iframe, even with a tab id",
            ),
            (untabbed, "a page without BrowserOS's tab id"),
            (with("tabId", json!(0)), "a page with tab id 0"),
            (with("tabId", json!(-1)), "a page with a negative tab id"),
            (
                json!({ "targetInfo": { "type": "page" } }),
                "a TargetInfo that does not parse",
            ),
        ];
        for (params, what) in not_tabs {
            assert_eq!(
                directory_input(&event("Target.targetCreated", params)),
                None,
                "{what} is not a tab"
            );
        }
        assert_eq!(
            directory_input(&event("Target.attachedToTarget", recorded(CREATED))),
            None,
            "the directory reads only discovery events"
        );
    }

    #[test]
    fn the_tab_list_seeds_the_directory() {
        let mut reply = recorded(TABS);
        let mut untabbed = reply["tabs"][0].clone();
        untabbed["tabId"] = json!(0);
        reply["tabs"]
            .as_array_mut()
            .expect("an array")
            .push(untabbed);
        let reply: browser::GetTabsResult =
            serde_json::from_value(reply).expect("a Browser.getTabs reply");
        assert_eq!(
            seed(reply),
            DirectoryInput::Seeded(vec![
                tab(
                    "7A20C3F0317E37E3F8E43ED16F70A196",
                    196_019_725,
                    "about:blank",
                    "about:blank"
                ),
                probe_tab("Probe"),
            ]),
            "every tab with a positive tab id seeds the directory"
        );
    }
}
