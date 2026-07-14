use crate::services::{
    browser::BrowserService,
    now_epoch_ms,
    tab_activity::{ScreencastFrame, TabActivityRecord, TabActivityService},
};
use browseros_cdp::CdpEvent;
use browseros_core::{
    BrowserSession, CoreError, PageId, SessionId,
    screencast::{self, SCREENCAST_FRAME_METHOD, ScreencastOptions},
    screenshot::{ScreenshotCaptureOptions, ScreenshotFormat},
};
use serde_json::json;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Mutex, mpsc},
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

const SUPERVISOR_INTERVAL: Duration = Duration::from_secs(1);
const POLL_INTERVAL: Duration = Duration::from_millis(1500);
const FAILURE_BACKOFF_MS: i64 = 5_000;
const FAILURE_BACKOFF_THRESHOLD: u8 = 3;
/// No `/tabs/activity` reads for this long means nobody is watching the
/// cockpit: stop screencasts and skip poll captures until the next read.
const IDLE_AFTER_MS: i64 = 15_000;
/// A screencast that stays frameless this long while its page is still the
/// window's active agent page gets restarted — revives casts silently killed
/// by a CDP reconnect and refreshes the keyframe on static pages.
const FRAMELESS_RESTART_MS: i64 = 5_000;

pub struct ScreencastService {
    inner: Arc<Mutex<ScreencastInner>>,
    casts: Arc<Mutex<HashMap<SessionId, Cast>>>,
    last_read_ms: AtomicI64,
    cancel: CancellationToken,
    capacity: usize,
}

#[derive(Default)]
struct ScreencastInner {
    frames: HashMap<u32, ScreencastFrame>,
    order: VecDeque<u32>,
    failures: HashMap<u32, u8>,
    retry_after: HashMap<u32, i64>,
}

/// A live `Page.startScreencast` on one target session. Keyed in the casts
/// map by the envelope target session id (string), which routes incoming
/// `Page.screencastFrame` events; the integer sessionId inside frame params
/// is only the ack cookie.
struct Cast {
    page_id: u32,
    window_id: i64,
    target_id: String,
    session: browseros_core::ProtocolSession,
    last_frame_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
struct AgentPage {
    page_id: u32,
    target_id: String,
    window_id: Option<i64>,
    status_active: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct RunningCast {
    key: SessionId,
    page_id: u32,
    target_id: String,
    window_id: i64,
    last_frame_at: i64,
}

#[derive(Debug, Default, PartialEq)]
struct TickPlan {
    stop: Vec<SessionId>,
    start: Vec<(u32, i64)>,
    poll: Vec<u32>,
}

impl ScreencastService {
    #[must_use]
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Mutex::new(ScreencastInner::default())),
            casts: Arc::new(Mutex::new(HashMap::new())),
            last_read_ms: AtomicI64::new(0),
            cancel: CancellationToken::new(),
            capacity,
        })
    }

    pub fn start(
        self: Arc<Self>,
        browser: Arc<BrowserService>,
        tab_activity: Arc<TabActivityService>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut supervisor = interval(SUPERVISOR_INTERVAL);
            supervisor.set_missed_tick_behavior(MissedTickBehavior::Delay);
            let mut poller = interval(POLL_INTERVAL);
            poller.set_missed_tick_behavior(MissedTickBehavior::Delay);
            let mut pump: Option<JoinHandle<()>> = None;
            loop {
                tokio::select! {
                    () = self.cancel.cancelled() => {
                        if let Some(pump) = pump.take() {
                            pump.abort();
                        }
                        self.stop_all_casts().await;
                        return;
                    }
                    _ = supervisor.tick() => {
                        self.ensure_pump(&browser, &mut pump).await;
                        self.supervise(&browser, &tab_activity).await;
                    }
                    _ = poller.tick() => {
                        self.poll_uncovered_pages(&browser, &tab_activity).await;
                    }
                }
            }
        })
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }

    pub async fn frame_for(&self, page_id: u32) -> Option<ScreencastFrame> {
        self.inner.lock().await.frames.get(&page_id).cloned()
    }

    /// Record a `/tabs/activity` read for the idle governor.
    pub fn note_read(&self) {
        self.last_read_ms.store(now_epoch_ms(), Ordering::Relaxed);
    }

    fn is_idle(&self, now: i64) -> bool {
        now.saturating_sub(self.last_read_ms.load(Ordering::Relaxed)) > IDLE_AFTER_MS
    }

    async fn ensure_pump(
        self: &Arc<Self>,
        browser: &Arc<BrowserService>,
        pump: &mut Option<JoinHandle<()>>,
    ) {
        if pump.as_ref().is_some_and(|handle| !handle.is_finished()) {
            return;
        }
        let Some(session) = browser.session().await else {
            return;
        };
        // Frames bypass the broadcast ring (they are large and high-rate;
        // ring slots would retain them long after consumption) — the
        // targeted channel frees each frame once handled, and CDP's ack
        // backpressure caps in-flight frames at ~1 per cast.
        let frames = session.cdp_events_targeted(SCREENCAST_FRAME_METHOD);
        let service = self.clone();
        *pump = Some(tokio::spawn(async move {
            service.pump_frames(frames).await;
        }));
    }

    async fn pump_frames(self: Arc<Self>, mut frames: mpsc::UnboundedReceiver<CdpEvent>) {
        loop {
            tokio::select! {
                () = self.cancel.cancelled() => return,
                received = frames.recv() => match received {
                    Some(event) => self.handle_frame(event).await,
                    // Channel replaced or client gone; the supervisor
                    // respawns the pump on its next tick.
                    None => return,
                }
            }
        }
    }

    async fn handle_frame(&self, event: CdpEvent) {
        let CdpEvent {
            params, session_id, ..
        } = event;
        let Some(session_id) = session_id else {
            return;
        };
        let Some(frame) = screencast::parse_frame_event(params) else {
            debug!(%session_id, "ignoring unparseable screencast frame");
            return;
        };
        let owner = {
            let mut casts = self.casts.lock().await;
            let Some(cast) = casts.get_mut(&session_id) else {
                // Frame from a session we do not own — drop it.
                return;
            };
            cast.last_frame_at = now_epoch_ms();
            (cast.page_id, cast.session.clone())
        };
        let (page_id, session) = owner;
        // Ack from a detached task: a wedged ack send must not stall frame
        // routing for the other casts. Per-session ordering is safe — CDP
        // sends the next frame only after this ack lands.
        let ack_id = frame.session_id;
        tokio::spawn(async move {
            if let Err(err) = screencast::ack_frame(&session, ack_id).await {
                debug!(page_id, error = %err, "screencastFrameAck failed");
            }
        });
        self.store_frame(
            page_id,
            ScreencastFrame {
                jpeg_base64: frame.data,
                captured_at: now_epoch_ms(),
            },
        )
        .await;
    }

    async fn supervise(&self, browser: &Arc<BrowserService>, tab_activity: &Arc<TabActivityService>) {
        let Some(session) = browser.session().await else {
            // No connection: target sessions are gone, drop the bookkeeping.
            self.casts.lock().await.clear();
            return;
        };
        let now = now_epoch_ms();
        let idle = self.is_idle(now);
        let topology = if idle {
            Some((Vec::new(), HashMap::new()))
        } else {
            let records = tab_activity.snapshot().await;
            self.resolve_topology(&session, records).await
        };
        let Some((agent_pages, active_target_by_window)) = topology else {
            // Transient CDP failure while listing pages: keep the running
            // casts and retry next tick instead of tearing them all down.
            return;
        };
        let running: Vec<RunningCast> = self
            .casts
            .lock()
            .await
            .iter()
            .map(|(key, cast)| RunningCast {
                key: key.clone(),
                page_id: cast.page_id,
                target_id: cast.target_id.clone(),
                window_id: cast.window_id,
                last_frame_at: cast.last_frame_at,
            })
            .collect();
        let plan = plan_tick(idle, now, &agent_pages, &active_target_by_window, &running);
        self.apply_plan(&session, plan, now).await;
    }

    /// Map agent pages to windows and resolve each relevant window's active
    /// tab via root `Browser.getActiveTab` — no attach, so the user's own
    /// tabs are never touched.
    async fn resolve_topology(
        &self,
        session: &Arc<BrowserSession>,
        records: Vec<TabActivityRecord>,
    ) -> Option<(Vec<AgentPage>, HashMap<i64, String>)> {
        if records.is_empty() {
            return Some((Vec::new(), HashMap::new()));
        }
        let infos = match session.pages.list().await {
            Ok(infos) => infos,
            Err(err) => {
                debug!(error = %err, "screencast page listing failed");
                return None;
            }
        };
        let window_by_page: HashMap<u32, i64> = infos
            .iter()
            .filter_map(|info| {
                info.window_id
                    .as_ref()
                    .map(|window| (info.page_id.0, window.0))
            })
            .collect();
        let agent_pages: Vec<AgentPage> = records
            .into_iter()
            .map(|record| AgentPage {
                window_id: window_by_page.get(&record.page_id).copied(),
                page_id: record.page_id,
                target_id: record.target_id,
                status_active: record.status == "active",
            })
            .collect();
        let windows: HashSet<i64> = agent_pages
            .iter()
            .filter_map(|page| page.window_id)
            .collect();
        let mut active_target_by_window = HashMap::new();
        for window_id in windows {
            match get_active_target(session, window_id).await {
                Ok(Some(target_id)) => {
                    active_target_by_window.insert(window_id, target_id);
                }
                Ok(None) => {}
                Err(err) => debug!(window_id, error = %err, "getActiveTab failed"),
            }
        }
        Some((agent_pages, active_target_by_window))
    }

    async fn apply_plan(&self, session: &Arc<BrowserSession>, plan: TickPlan, now: i64) {
        for key in plan.stop {
            let cast = self.casts.lock().await.remove(&key);
            if let Some(cast) = cast
                && let Err(err) = screencast::stop_screencast(&cast.session).await
            {
                debug!(page_id = cast.page_id, error = %err, "stopScreencast failed");
            }
        }
        for (page_id, window_id) in plan.start {
            if let Err(err) = self.start_cast(session, page_id, window_id, now).await {
                debug!(page_id, error = %err, "startScreencast failed");
            }
        }
    }

    async fn start_cast(
        &self,
        session: &Arc<BrowserSession>,
        page_id: u32,
        window_id: i64,
        now: i64,
    ) -> Result<(), CoreError> {
        let page = session.pages.get_session(PageId(page_id)).await?;
        // Register before starting so the first frame is routable.
        self.casts.lock().await.insert(
            page.session_id.clone(),
            Cast {
                page_id,
                window_id,
                target_id: page.target_id.into_inner(),
                session: page.session.clone(),
                last_frame_at: now,
            },
        );
        if let Err(err) =
            screencast::start_screencast(&page.session, &ScreencastOptions::default()).await
        {
            self.casts.lock().await.remove(&page.session_id);
            return Err(err);
        }
        Ok(())
    }

    async fn stop_all_casts(&self) {
        let casts: Vec<Cast> = {
            let mut guard = self.casts.lock().await;
            guard.drain().map(|(_, cast)| cast).collect()
        };
        for cast in casts {
            if let Err(err) = screencast::stop_screencast(&cast.session).await {
                debug!(page_id = cast.page_id, error = %err, "stopScreencast on teardown failed");
            }
        }
    }

    /// Poll fallback for agent pages CDP will not composite (background
    /// tabs): the pre-screencast captureScreenshot path, unchanged.
    async fn poll_uncovered_pages(
        &self,
        browser: &Arc<BrowserService>,
        tab_activity: &Arc<TabActivityService>,
    ) {
        if self.is_idle(now_epoch_ms()) {
            return;
        }
        let Some(session) = browser.session().await else {
            return;
        };
        let covered: HashSet<u32> = self
            .casts
            .lock()
            .await
            .values()
            .map(|cast| cast.page_id)
            .collect();
        let pages = tab_activity.snapshot().await;
        for record in pages
            .into_iter()
            .filter(|record| record.status == "active" && !covered.contains(&record.page_id))
        {
            if self.is_backing_off(record.page_id).await {
                continue;
            }
            let options = ScreenshotCaptureOptions {
                format: Some(ScreenshotFormat::Jpeg),
                quality: Some(50),
                full_page: Some(false),
                annotate: Some(false),
                // The BrowserOS fork visibly resizes tabs when clip is set.
                clip: None,
            };
            match session.screenshot(PageId(record.page_id), options).await {
                Ok(capture) => {
                    self.store_frame(
                        record.page_id,
                        ScreencastFrame {
                            jpeg_base64: capture.data,
                            captured_at: now_epoch_ms(),
                        },
                    )
                    .await;
                }
                Err(err) => {
                    warn!(page_id = record.page_id, error = %err, "screencast capture failed");
                    self.record_failure(record.page_id).await;
                }
            }
        }
    }

    async fn is_backing_off(&self, page_id: u32) -> bool {
        self.inner
            .lock()
            .await
            .retry_after
            .get(&page_id)
            .copied()
            .map(|retry_after| now_epoch_ms() < retry_after)
            .unwrap_or(false)
    }

    async fn store_frame(&self, page_id: u32, frame: ScreencastFrame) {
        let mut inner = self.inner.lock().await;
        inner.frames.insert(page_id, frame);
        inner.failures.remove(&page_id);
        inner.retry_after.remove(&page_id);
        if let Some(pos) = inner.order.iter().position(|existing| *existing == page_id) {
            inner.order.remove(pos);
        }
        inner.order.push_back(page_id);
        while inner.order.len() > self.capacity {
            if let Some(evicted) = inner.order.pop_front() {
                inner.frames.remove(&evicted);
                inner.failures.remove(&evicted);
                inner.retry_after.remove(&evicted);
            }
        }
    }

    async fn record_failure(&self, page_id: u32) {
        let mut inner = self.inner.lock().await;
        let failures = inner.failures.entry(page_id).or_insert(0);
        *failures = failures.saturating_add(1);
        if *failures >= FAILURE_BACKOFF_THRESHOLD {
            inner
                .retry_after
                .insert(page_id, now_epoch_ms().saturating_add(FAILURE_BACKOFF_MS));
        }
    }
}

async fn get_active_target(
    session: &Arc<BrowserSession>,
    window_id: i64,
) -> Result<Option<String>, CoreError> {
    let value = session
        .cdp("Browser.getActiveTab", json!({ "windowId": window_id }), None)
        .await?;
    let result: browseros_cdp::browser::GetActiveTabResult =
        serde_json::from_value(value).map_err(|err| CoreError::Message(err.to_string()))?;
    Ok(result.tab.map(|tab| tab.target_id))
}

/// Pure per-tick decision: which casts to stop, which pages to start
/// screencasting (window's active agent page), and which pages the
/// captureScreenshot fallback should poll.
fn plan_tick(
    idle: bool,
    now: i64,
    agent_pages: &[AgentPage],
    active_target_by_window: &HashMap<i64, String>,
    running: &[RunningCast],
) -> TickPlan {
    let mut plan = TickPlan::default();
    if idle {
        plan.stop = running.iter().map(|cast| cast.key.clone()).collect();
        return plan;
    }
    let agent_by_target: HashMap<&str, &AgentPage> = agent_pages
        .iter()
        .map(|page| (page.target_id.as_str(), page))
        .collect();
    let mut covered: HashSet<u32> = HashSet::new();
    for cast in running {
        let still_window_active = active_target_by_window
            .get(&cast.window_id)
            .is_some_and(|target| *target == cast.target_id);
        let still_agent = agent_by_target.contains_key(cast.target_id.as_str());
        if !still_window_active || !still_agent {
            plan.stop.push(cast.key.clone());
            continue;
        }
        if now.saturating_sub(cast.last_frame_at) > FRAMELESS_RESTART_MS {
            plan.stop.push(cast.key.clone());
            plan.start.push((cast.page_id, cast.window_id));
        }
        covered.insert(cast.page_id);
    }
    for (window_id, target_id) in active_target_by_window {
        let Some(page) = agent_by_target.get(target_id.as_str()) else {
            continue;
        };
        if covered.insert(page.page_id) {
            plan.start.push((page.page_id, *window_id));
        }
    }
    for page in agent_pages {
        if page.status_active && !covered.contains(&page.page_id) {
            plan.poll.push(page.page_id);
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::{AgentPage, RunningCast, ScreencastService, plan_tick};
    use crate::services::tab_activity::ScreencastFrame;
    use browseros_core::SessionId;
    use std::collections::HashMap;

    const NOW: i64 = 1_000_000;

    fn page(page_id: u32, target_id: &str, window_id: Option<i64>, status_active: bool) -> AgentPage {
        AgentPage {
            page_id,
            target_id: target_id.to_string(),
            window_id,
            status_active,
        }
    }

    fn cast(key: &str, page_id: u32, target_id: &str, window_id: i64, last_frame_at: i64) -> RunningCast {
        RunningCast {
            key: SessionId::from(key),
            page_id,
            target_id: target_id.to_string(),
            window_id,
            last_frame_at,
        }
    }

    fn active(entries: &[(i64, &str)]) -> HashMap<i64, String> {
        entries
            .iter()
            .map(|(window, target)| (*window, (*target).to_string()))
            .collect()
    }

    #[test]
    fn active_agent_page_starts_screencast_and_is_not_polled() {
        let pages = [page(1, "t1", Some(1), true)];
        let plan = plan_tick(false, NOW, &pages, &active(&[(1, "t1")]), &[]);
        assert_eq!(plan.start, vec![(1, 1)]);
        assert!(plan.stop.is_empty());
        assert!(plan.poll.is_empty());
    }

    #[test]
    fn background_agent_page_polls_instead_of_screencasting() {
        let pages = [page(1, "t1", Some(1), true), page(2, "t2", Some(1), true)];
        let plan = plan_tick(false, NOW, &pages, &active(&[(1, "t1")]), &[]);
        assert_eq!(plan.start, vec![(1, 1)]);
        assert_eq!(plan.poll, vec![2]);
    }

    #[test]
    fn non_agent_active_tab_gets_no_screencast() {
        let pages = [page(1, "t1", Some(1), true)];
        let plan = plan_tick(false, NOW, &pages, &active(&[(1, "user-tab")]), &[]);
        assert!(plan.start.is_empty());
        assert_eq!(plan.poll, vec![1]);
    }

    #[test]
    fn unknown_window_agent_page_still_polls() {
        let pages = [page(1, "t1", None, true)];
        let plan = plan_tick(false, NOW, &pages, &HashMap::new(), &[]);
        assert!(plan.start.is_empty());
        assert_eq!(plan.poll, vec![1]);
    }

    #[test]
    fn idle_agent_page_neither_screencasts_nor_polls_when_backgrounded() {
        let pages = [page(2, "t2", Some(1), false)];
        let plan = plan_tick(false, NOW, &pages, &active(&[(1, "user-tab")]), &[]);
        assert!(plan.start.is_empty());
        assert!(plan.poll.is_empty());
    }

    #[test]
    fn active_page_switch_stops_old_cast_and_starts_new() {
        let pages = [page(1, "t1", Some(1), true), page(2, "t2", Some(1), true)];
        let running = [cast("s1", 1, "t1", 1, NOW)];
        let plan = plan_tick(false, NOW, &pages, &active(&[(1, "t2")]), &running);
        assert_eq!(plan.stop, vec![SessionId::from("s1")]);
        assert_eq!(plan.start, vec![(2, 1)]);
        assert_eq!(plan.poll, vec![1]);
    }

    #[test]
    fn cast_whose_page_left_the_agent_set_stops() {
        let running = [cast("s1", 1, "t1", 1, NOW)];
        let plan = plan_tick(false, NOW, &[], &HashMap::new(), &running);
        assert_eq!(plan.stop, vec![SessionId::from("s1")]);
        assert!(plan.start.is_empty());
    }

    #[test]
    fn idle_governor_stops_all_casts_and_skips_polling() {
        let pages = [page(1, "t1", Some(1), true)];
        let running = [cast("s1", 1, "t1", 1, NOW), cast("s2", 2, "t2", 2, NOW)];
        let plan = plan_tick(true, NOW, &pages, &active(&[(1, "t1")]), &running);
        let mut stopped = plan.stop.clone();
        stopped.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        assert_eq!(stopped, vec![SessionId::from("s1"), SessionId::from("s2")]);
        assert!(plan.start.is_empty());
        assert!(plan.poll.is_empty());
    }

    #[test]
    fn resume_after_read_restarts_within_one_tick() {
        let pages = [page(1, "t1", Some(1), true)];
        let plan = plan_tick(false, NOW, &pages, &active(&[(1, "t1")]), &[]);
        assert_eq!(plan.start, vec![(1, 1)]);
    }

    #[test]
    fn frameless_cast_on_active_agent_page_restarts() {
        let pages = [page(1, "t1", Some(1), true)];
        let running = [cast("s1", 1, "t1", 1, NOW - 6_000)];
        let plan = plan_tick(false, NOW, &pages, &active(&[(1, "t1")]), &running);
        assert_eq!(plan.stop, vec![SessionId::from("s1")]);
        assert_eq!(plan.start, vec![(1, 1)]);
        assert!(plan.poll.is_empty());
    }

    #[test]
    fn cast_with_recent_frame_is_left_alone() {
        let pages = [page(1, "t1", Some(1), true)];
        let running = [cast("s1", 1, "t1", 1, NOW - 1_000)];
        let plan = plan_tick(false, NOW, &pages, &active(&[(1, "t1")]), &running);
        assert!(plan.stop.is_empty());
        assert!(plan.start.is_empty());
        assert!(plan.poll.is_empty());
    }

    #[test]
    fn idle_governor_uses_fifteen_second_window() {
        let service = ScreencastService::new(2);
        assert!(service.is_idle(NOW), "no reads yet means idle");
        service
            .last_read_ms
            .store(NOW, std::sync::atomic::Ordering::Relaxed);
        assert!(!service.is_idle(NOW));
        assert!(!service.is_idle(NOW + 15_000));
        assert!(service.is_idle(NOW + 15_001));
    }

    #[tokio::test]
    async fn frame_cache_is_lru_capped() {
        let service = ScreencastService::new(2);
        service
            .store_frame(
                1,
                ScreencastFrame {
                    jpeg_base64: "a".to_string(),
                    captured_at: 1,
                },
            )
            .await;
        service
            .store_frame(
                2,
                ScreencastFrame {
                    jpeg_base64: "b".to_string(),
                    captured_at: 2,
                },
            )
            .await;
        service
            .store_frame(
                3,
                ScreencastFrame {
                    jpeg_base64: "c".to_string(),
                    captured_at: 3,
                },
            )
            .await;
        assert!(service.frame_for(1).await.is_none());
        assert!(service.frame_for(2).await.is_some());
        assert!(service.frame_for(3).await.is_some());
    }
}
