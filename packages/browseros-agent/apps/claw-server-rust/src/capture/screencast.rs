use crate::{
    browser::BrowserService,
    clock::now_epoch_ms,
    tabs::activity::{ScreencastFrame, TabActivityRecord, TabActivityService},
};
use browseros_core::{
    BrowserSession, PageId, TargetId,
    screenshot::{ScreenshotCaptureOptions, ScreenshotCaptureResult, ScreenshotFormat},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::Duration,
};
use tokio::{
    task::{JoinHandle, JoinSet},
    time::{MissedTickBehavior, interval, timeout},
};
use tokio_util::sync::CancellationToken;
use tracing::warn;

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(1500);
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_PARALLEL_SHOTS: usize = 8;
const FAILURE_BACKOFF_THRESHOLD: u8 = 3;
const IDLE_AFTER_MS: i64 = 15_000;

pub struct ScreencastService {
    inner: Arc<tokio::sync::Mutex<ScreencastInner>>,
    last_read_ms: AtomicI64,
    tick_running: AtomicBool,
    cancel: CancellationToken,
    capacity: usize,
}

#[derive(Default)]
struct ScreencastInner {
    frames: HashMap<TabIncarnation, ScreencastFrame>,
    order: VecDeque<TabIncarnation>,
    failures: HashMap<TabIncarnation, FailureState>,
    in_flight: HashSet<TabIncarnation>,
    live: HashSet<TabIncarnation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TabIncarnation {
    page_id: u32,
    target_id: String,
}

impl TabIncarnation {
    fn new(page_id: u32, target_id: impl Into<String>) -> Self {
        Self {
            page_id,
            target_id: target_id.into(),
        }
    }

    fn from_record(record: &TabActivityRecord) -> Self {
        Self::new(record.page_id, record.target_id.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FailureState {
    consecutive: u8,
    last_failure_at: i64,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct TickPlan {
    capture: Vec<TabIncarnation>,
    gc: Vec<TabIncarnation>,
}

struct TickGuard<'a>(&'a AtomicBool);

impl Drop for TickGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl ScreencastService {
    #[must_use]
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(tokio::sync::Mutex::new(ScreencastInner::default())),
            last_read_ms: AtomicI64::new(0),
            tick_running: AtomicBool::new(false),
            cancel: CancellationToken::new(),
            capacity,
        })
    }

    /// Start polling active agent pages and caching their latest screenshots.
    pub fn start(
        self: Arc<Self>,
        browser: Arc<BrowserService>,
        tab_activity: Arc<TabActivityService>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut poller = interval(DEFAULT_POLL_INTERVAL);
            poller.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    () = self.cancel.cancelled() => return,
                    _ = poller.tick() => {
                        let service = self.clone();
                        let browser = browser.clone();
                        let tab_activity = tab_activity.clone();
                        tokio::spawn(async move {
                            service.tick(&browser, &tab_activity).await;
                        });
                    }
                }
            }
        })
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }

    pub async fn frame_for(&self, page_id: u32, target_id: &str) -> Option<ScreencastFrame> {
        self.inner
            .lock()
            .await
            .frames
            .get(&TabIncarnation::new(page_id, target_id))
            .cloned()
    }

    /// Record an `/api/v1/tabs` read for the idle governor.
    pub fn note_read(&self) {
        self.last_read_ms.store(now_epoch_ms(), Ordering::Relaxed);
    }

    fn is_idle(&self, now: i64) -> bool {
        now.saturating_sub(self.last_read_ms.load(Ordering::Relaxed)) > IDLE_AFTER_MS
    }

    fn begin_tick(&self) -> Option<TickGuard<'_>> {
        self.tick_running
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| TickGuard(&self.tick_running))
    }

    async fn tick(
        self: &Arc<Self>,
        browser: &Arc<BrowserService>,
        tab_activity: &Arc<TabActivityService>,
    ) {
        let Some(_tick_guard) = self.begin_tick() else {
            return;
        };
        self.run_tick(browser, tab_activity).await;
    }

    async fn run_tick(
        self: &Arc<Self>,
        browser: &Arc<BrowserService>,
        tab_activity: &Arc<TabActivityService>,
    ) {
        let idle = self.is_idle(now_epoch_ms());
        let session = browser.session().await;
        let records = tab_activity.snapshot(session.as_deref()).await;
        let live = records
            .iter()
            .map(TabIncarnation::from_record)
            .collect::<HashSet<_>>();
        let (failures, in_flight, cached) = {
            let mut inner = self.inner.lock().await;
            inner.live = live;
            (
                inner.failures.clone(),
                inner.in_flight.clone(),
                inner.order.iter().cloned().collect::<Vec<_>>(),
            )
        };
        let plan = plan_tick(idle, &records, &failures, &in_flight, &cached);
        self.gc_incarnations(&plan.gc).await;

        if plan.capture.is_empty() {
            return;
        }
        let Some(session) = session else {
            return;
        };
        for batch in plan.capture.chunks(MAX_PARALLEL_SHOTS) {
            let mut captures = JoinSet::new();
            for incarnation in batch.iter().cloned() {
                let service = self.clone();
                let session = session.clone();
                captures.spawn(async move {
                    service.capture_one(session, incarnation).await;
                });
            }
            while let Some(result) = captures.join_next().await {
                if let Err(err) = result {
                    warn!(error = %err, "screencast capture task failed");
                }
            }
        }
    }

    /// Capture one page while keeping timed-out CDP work guarded until it resolves.
    async fn capture_one(
        self: Arc<Self>,
        session: Arc<BrowserSession>,
        incarnation: TabIncarnation,
    ) {
        if !self.begin_capture(&incarnation).await {
            return;
        }
        let page_id = incarnation.page_id;
        let options = ScreenshotCaptureOptions {
            format: Some(ScreenshotFormat::Jpeg),
            quality: Some(50),
            full_page: Some(false),
            annotate: Some(false),
            // BrowserOS visibly resizes watched tabs when captureScreenshot includes a clip.
            clip: None,
        };
        let target_id = TargetId::from(incarnation.target_id.clone());
        let mut capture = tokio::spawn(async move {
            session
                .screenshot_for_target(PageId(page_id), &target_id, options)
                .await
        });
        let outcome = timeout(SCREENSHOT_TIMEOUT, &mut capture).await;
        match outcome {
            Ok(result) => {
                self.clear_in_flight(&incarnation).await;
                match result {
                    Ok(Ok(Some(capture))) if !capture.data.is_empty() => {
                        self.store_capture(incarnation, capture).await;
                    }
                    Ok(Ok(Some(_))) => self.capture_failed(&incarnation, "empty screenshot").await,
                    Ok(Ok(None)) => {}
                    Ok(Err(err)) => self.capture_failed(&incarnation, &err.to_string()).await,
                    Err(err) => self.capture_failed(&incarnation, &err.to_string()).await,
                }
            }
            Err(_) => {
                let service = self.clone();
                let completed_incarnation = incarnation.clone();
                tokio::spawn(async move {
                    let _ = capture.await;
                    service.clear_in_flight(&completed_incarnation).await;
                });
                self.capture_failed(&incarnation, "screenshot timeout")
                    .await;
            }
        }
    }

    async fn begin_capture(&self, incarnation: &TabIncarnation) -> bool {
        self.inner
            .lock()
            .await
            .in_flight
            .insert(incarnation.clone())
    }

    async fn clear_in_flight(&self, incarnation: &TabIncarnation) {
        self.inner.lock().await.in_flight.remove(incarnation);
    }

    async fn store_capture(&self, incarnation: TabIncarnation, capture: ScreenshotCaptureResult) {
        let mut inner = self.inner.lock().await;
        if !inner.live.contains(&incarnation) {
            return;
        }
        self.insert_frame(
            &mut inner,
            incarnation,
            ScreencastFrame {
                jpeg_base64: capture.data,
                captured_at: now_epoch_ms(),
            },
        );
    }

    /// Public so integration tests can seed preview frames; production
    /// frames arrive via `store_capture` from the poller.
    pub async fn cache_frame(&self, page_id: u32, target_id: &str, frame: ScreencastFrame) {
        let mut inner = self.inner.lock().await;
        self.insert_frame(&mut inner, TabIncarnation::new(page_id, target_id), frame);
    }

    fn insert_frame(
        &self,
        inner: &mut ScreencastInner,
        incarnation: TabIncarnation,
        frame: ScreencastFrame,
    ) {
        inner.frames.remove(&incarnation);
        inner.order.retain(|existing| existing != &incarnation);
        inner.frames.insert(incarnation.clone(), frame);
        inner.order.push_back(incarnation.clone());
        inner.failures.remove(&incarnation);
        while inner.order.len() > self.capacity {
            if let Some(evicted) = inner.order.pop_front() {
                inner.frames.remove(&evicted);
                inner.failures.remove(&evicted);
            }
        }
    }

    async fn capture_failed(&self, incarnation: &TabIncarnation, error: &str) {
        let Some(in_backoff) = self
            .record_failure_if_live(incarnation, now_epoch_ms())
            .await
        else {
            return;
        };
        warn!(page_id = incarnation.page_id, target_id = %incarnation.target_id, error, "screencast capture failed");
        if in_backoff {
            warn!(page_id = incarnation.page_id, target_id = %incarnation.target_id, "screencast page enters backoff");
        }
    }

    async fn record_failure_if_live(&self, incarnation: &TabIncarnation, now: i64) -> Option<bool> {
        let mut inner = self.inner.lock().await;
        if !inner.live.contains(incarnation) {
            return None;
        }
        Some(Self::record_failure_locked(&mut inner, incarnation, now))
    }

    #[cfg(test)]
    async fn record_failure(&self, incarnation: &TabIncarnation, now: i64) -> bool {
        let mut inner = self.inner.lock().await;
        Self::record_failure_locked(&mut inner, incarnation, now)
    }

    fn record_failure_locked(
        inner: &mut ScreencastInner,
        incarnation: &TabIncarnation,
        now: i64,
    ) -> bool {
        let state = inner
            .failures
            .entry(incarnation.clone())
            .or_insert(FailureState {
                consecutive: 0,
                last_failure_at: now,
            });
        state.consecutive = state.consecutive.saturating_add(1);
        state.last_failure_at = now;
        let in_backoff = state.consecutive >= FAILURE_BACKOFF_THRESHOLD;
        if in_backoff {
            inner.frames.remove(incarnation);
            inner.order.retain(|existing| existing != incarnation);
        }
        in_backoff
    }

    async fn gc_incarnations(&self, incarnations: &[TabIncarnation]) {
        if incarnations.is_empty() {
            return;
        }
        let incarnations = incarnations.iter().cloned().collect::<HashSet<_>>();
        let mut inner = self.inner.lock().await;
        for incarnation in &incarnations {
            inner.frames.remove(incarnation);
            inner.failures.remove(incarnation);
        }
        inner
            .order
            .retain(|incarnation| !incarnations.contains(incarnation));
    }
}

/// Plan active captures and stale-frame GC without mutating service state.
fn plan_tick(
    idle: bool,
    records: &[TabActivityRecord],
    failures: &HashMap<TabIncarnation, FailureState>,
    in_flight: &HashSet<TabIncarnation>,
    cached: &[TabIncarnation],
) -> TickPlan {
    let live = records
        .iter()
        .map(TabIncarnation::from_record)
        .collect::<HashSet<_>>();
    let capture = if idle {
        Vec::new()
    } else {
        records
            .iter()
            .filter(|record| record.status == "active")
            .filter_map(|record| {
                let incarnation = TabIncarnation::from_record(record);
                if in_flight.contains(&incarnation)
                    || failures.get(&incarnation).is_some_and(|failure| {
                        failure.consecutive >= FAILURE_BACKOFF_THRESHOLD
                            && record.last_tool_at <= failure.last_failure_at
                    })
                {
                    return None;
                }
                Some(incarnation)
            })
            .collect()
    };
    let mut known = cached.to_vec();
    for incarnation in failures.keys() {
        if !known.contains(incarnation) {
            known.push(incarnation.clone());
        }
    }
    let gc = known
        .into_iter()
        .filter(|incarnation| !live.contains(incarnation))
        .collect();
    TickPlan { capture, gc }
}

#[cfg(test)]
mod tests {
    use super::{FailureState, ScreencastService, TabIncarnation, TickPlan, plan_tick};
    use crate::tabs::activity::{ScreencastFrame, TabActivityRecord};
    use std::collections::{HashMap, HashSet};

    const NOW: i64 = 1_000_000;

    fn record(page_id: u32, status: &'static str, last_tool_at: i64) -> TabActivityRecord {
        TabActivityRecord {
            target_id: format!("target-{page_id}"),
            tab_id: i64::from(page_id) + 100,
            page_id,
            url: format!("https://example.com/{page_id}"),
            title: format!("Page {page_id}"),
            session_id: "session-1".to_string(),
            agent_id: "agent".to_string(),
            slug: "codex".to_string(),
            first_tool_at: last_tool_at,
            last_tool_at,
            last_tool_name: "tabs".to_string(),
            tool_count: 1,
            recent_tools: Vec::new(),
            status,
        }
    }

    fn failure(consecutive: u8, last_failure_at: i64) -> FailureState {
        FailureState {
            consecutive,
            last_failure_at,
        }
    }

    fn key(page_id: u32) -> TabIncarnation {
        TabIncarnation::new(page_id, format!("target-{page_id}"))
    }

    fn target_key(page_id: u32, target_id: &str) -> TabIncarnation {
        TabIncarnation::new(page_id, target_id)
    }

    fn frame(data: &str, captured_at: i64) -> ScreencastFrame {
        ScreencastFrame {
            jpeg_base64: data.to_string(),
            captured_at,
        }
    }

    #[test]
    fn planner_captures_only_active_tabs() {
        let records = [record(1, "active", NOW), record(2, "idle", NOW)];
        let plan = plan_tick(false, &records, &HashMap::new(), &HashSet::new(), &[]);
        assert_eq!(plan.capture, vec![key(1)]);
        assert!(plan.gc.is_empty());
    }

    #[test]
    fn planner_retries_before_failure_threshold() {
        let records = [record(1, "active", NOW)];
        let failures = HashMap::from([(key(1), failure(2, NOW))]);
        let plan = plan_tick(false, &records, &failures, &HashSet::new(), &[]);
        assert_eq!(plan.capture, vec![key(1)]);
    }

    #[test]
    fn planner_backs_off_after_three_failures() {
        let records = [record(1, "active", NOW)];
        let failures = HashMap::from([(key(1), failure(3, NOW))]);
        let plan = plan_tick(false, &records, &failures, &HashSet::new(), &[]);
        assert!(plan.capture.is_empty());
    }

    #[test]
    fn planner_lifts_backoff_after_new_tool_activity() {
        let records = [record(1, "active", NOW + 1)];
        let failures = HashMap::from([(key(1), failure(3, NOW))]);
        let plan = plan_tick(false, &records, &failures, &HashSet::new(), &[]);
        assert_eq!(plan.capture, vec![key(1)]);
    }

    #[test]
    fn planner_skips_pages_with_capture_in_flight() {
        let records = [record(1, "active", NOW), record(2, "active", NOW)];
        let in_flight = HashSet::from([key(1)]);
        let plan = plan_tick(false, &records, &HashMap::new(), &in_flight, &[]);
        assert_eq!(plan.capture, vec![key(2)]);
    }

    #[test]
    fn planner_garbage_collects_closed_page_frames() {
        let records = [record(2, "idle", NOW), record(3, "active", NOW)];
        let plan = plan_tick(
            false,
            &records,
            &HashMap::new(),
            &HashSet::new(),
            &[key(1), key(2)],
        );
        assert_eq!(plan.capture, vec![key(3)]);
        assert_eq!(plan.gc, vec![key(1)]);
    }

    #[test]
    fn idle_planner_skips_capture_but_still_collects_stale_state() {
        let records = [record(1, "active", NOW)];
        let plan = plan_tick(true, &records, &HashMap::new(), &HashSet::new(), &[key(2)]);
        assert_eq!(
            plan,
            TickPlan {
                capture: Vec::new(),
                gc: vec![key(2)],
            }
        );
    }

    #[test]
    fn planner_collects_failure_only_state_and_old_page_incarnations() {
        let records = [record(1, "active", NOW)];
        let old = target_key(1, "target-old");
        let closed = key(2);
        let failures = HashMap::from([
            (old.clone(), failure(3, NOW)),
            (closed.clone(), failure(1, NOW)),
        ]);

        let plan = plan_tick(
            true,
            &records,
            &failures,
            &HashSet::new(),
            std::slice::from_ref(&old),
        );

        assert_eq!(plan.capture, Vec::<TabIncarnation>::new());
        assert_eq!(plan.gc, vec![old, closed]);
    }

    #[test]
    fn tick_overlap_guard_resets_when_tick_finishes() {
        let service = ScreencastService::new(2);
        let Some(guard) = service.begin_tick() else {
            panic!("first tick should start");
        };
        assert!(service.begin_tick().is_none());
        drop(guard);
        assert!(service.begin_tick().is_some());
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
    async fn frame_cache_is_lru_capped_and_updates_recency() {
        let service = ScreencastService::new(2);
        service.cache_frame(1, "target-1", frame("a", 1)).await;
        service.cache_frame(2, "target-2", frame("b", 2)).await;
        service.cache_frame(1, "target-1", frame("new-a", 3)).await;
        service.cache_frame(3, "target-3", frame("c", 4)).await;

        let Some(refreshed) = service.frame_for(1, "target-1").await else {
            panic!("missing page 1 frame");
        };
        assert_eq!(refreshed.jpeg_base64, "new-a");
        assert_eq!(refreshed.captured_at, 3);
        assert!(service.frame_for(2, "target-2").await.is_none());
        let Some(newest) = service.frame_for(3, "target-3").await else {
            panic!("missing page 3 frame");
        };
        assert_eq!(newest.jpeg_base64, "c");
        assert_eq!(newest.captured_at, 4);
    }

    #[tokio::test]
    async fn entering_backoff_drops_frame_but_keeps_failure_state() {
        let service = ScreencastService::new(2);
        let incarnation = key(1);
        service.cache_frame(1, "target-1", frame("stale", 1)).await;
        assert!(!service.record_failure(&incarnation, NOW - 2).await);
        assert!(!service.record_failure(&incarnation, NOW - 1).await);
        assert!(service.record_failure(&incarnation, NOW).await);

        let inner = service.inner.lock().await;
        assert!(!inner.frames.contains_key(&incarnation));
        assert_eq!(inner.failures.get(&incarnation), Some(&failure(3, NOW)));
    }

    #[tokio::test]
    async fn successful_capture_clears_failure_state() {
        let service = ScreencastService::new(2);
        let incarnation = key(1);
        assert!(!service.record_failure(&incarnation, NOW - 2).await);
        assert!(!service.record_failure(&incarnation, NOW - 1).await);
        assert!(service.record_failure(&incarnation, NOW).await);
        service
            .cache_frame(1, "target-1", frame("fresh", NOW + 1))
            .await;

        assert!(
            !service
                .inner
                .lock()
                .await
                .failures
                .contains_key(&incarnation)
        );
    }

    #[tokio::test]
    async fn per_page_in_flight_guard_clears_only_on_completion() {
        let service = ScreencastService::new(2);
        let old = target_key(1, "target-old");
        let replacement = target_key(1, "target-new");
        assert!(service.begin_capture(&old).await);
        assert!(!service.begin_capture(&old).await);
        assert!(service.begin_capture(&replacement).await);
        service.clear_in_flight(&old).await;
        assert!(service.begin_capture(&old).await);
    }

    #[tokio::test]
    async fn garbage_collection_drops_frame_and_failure_state() {
        let service = ScreencastService::new(2);
        let incarnation = key(1);
        service.cache_frame(1, "target-1", frame("stale", 1)).await;
        service.record_failure(&incarnation, NOW).await;
        service
            .gc_incarnations(std::slice::from_ref(&incarnation))
            .await;

        let inner = service.inner.lock().await;
        assert!(!inner.frames.contains_key(&incarnation));
        assert!(!inner.failures.contains_key(&incarnation));
        assert!(!inner.order.contains(&incarnation));
    }

    #[tokio::test]
    async fn frame_lookup_never_crosses_a_reused_page_id() {
        let service = ScreencastService::new(2);
        service
            .cache_frame(1, "target-old", frame("old", NOW))
            .await;

        assert!(service.frame_for(1, "target-new").await.is_none());
        assert_eq!(
            service
                .frame_for(1, "target-old")
                .await
                .map(|frame| frame.jpeg_base64)
                .as_deref(),
            Some("old")
        );
    }
}
