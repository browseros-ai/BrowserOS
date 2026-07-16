use crate::services::{
    browser::BrowserService,
    now_epoch_ms,
    tab_activity::{ScreencastFrame, TabActivityRecord, TabActivityService},
};
use browseros_core::{
    BrowserSession, PageId,
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
    frames: HashMap<u32, ScreencastFrame>,
    order: VecDeque<u32>,
    failures: HashMap<u32, FailureState>,
    in_flight: HashSet<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FailureState {
    consecutive: u8,
    last_failure_at: i64,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct TickPlan {
    capture: Vec<u32>,
    gc: Vec<u32>,
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
        let records = tab_activity.snapshot().await;
        let (failures, in_flight, cached_page_ids) = {
            let inner = self.inner.lock().await;
            (
                inner.failures.clone(),
                inner.in_flight.clone(),
                inner.order.iter().copied().collect::<Vec<_>>(),
            )
        };
        let plan = plan_tick(idle, &records, &failures, &in_flight, &cached_page_ids);
        self.gc_pages(&plan.gc).await;

        if plan.capture.is_empty() {
            return;
        }
        let Some(session) = browser.session().await else {
            return;
        };
        for batch in plan.capture.chunks(MAX_PARALLEL_SHOTS) {
            let mut captures = JoinSet::new();
            for page_id in batch.iter().copied() {
                let service = self.clone();
                let session = session.clone();
                captures.spawn(async move {
                    service.capture_one(session, page_id).await;
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
    async fn capture_one(self: Arc<Self>, session: Arc<BrowserSession>, page_id: u32) {
        if !self.begin_capture(page_id).await {
            return;
        }
        let options = ScreenshotCaptureOptions {
            format: Some(ScreenshotFormat::Jpeg),
            quality: Some(50),
            full_page: Some(false),
            annotate: Some(false),
            // BrowserOS visibly resizes watched tabs when captureScreenshot includes a clip.
            clip: None,
        };
        let mut capture =
            tokio::spawn(async move { session.screenshot(PageId(page_id), options).await });
        let outcome = timeout(SCREENSHOT_TIMEOUT, &mut capture).await;
        match outcome {
            Ok(result) => {
                self.clear_in_flight(page_id).await;
                match result {
                    Ok(Ok(capture)) if !capture.data.is_empty() => {
                        self.store_capture(page_id, capture).await;
                    }
                    Ok(Ok(_)) => self.capture_failed(page_id, "empty screenshot").await,
                    Ok(Err(err)) => self.capture_failed(page_id, &err.to_string()).await,
                    Err(err) => self.capture_failed(page_id, &err.to_string()).await,
                }
            }
            Err(_) => {
                let service = self.clone();
                tokio::spawn(async move {
                    let _ = capture.await;
                    service.clear_in_flight(page_id).await;
                });
                self.capture_failed(page_id, "screenshot timeout").await;
            }
        }
    }

    async fn begin_capture(&self, page_id: u32) -> bool {
        self.inner.lock().await.in_flight.insert(page_id)
    }

    async fn clear_in_flight(&self, page_id: u32) {
        self.inner.lock().await.in_flight.remove(&page_id);
    }

    async fn store_capture(&self, page_id: u32, capture: ScreenshotCaptureResult) {
        self.store_frame(
            page_id,
            ScreencastFrame {
                jpeg_base64: capture.data,
                captured_at: now_epoch_ms(),
            },
        )
        .await;
    }

    async fn store_frame(&self, page_id: u32, frame: ScreencastFrame) {
        let mut inner = self.inner.lock().await;
        inner.frames.remove(&page_id);
        inner.order.retain(|existing| *existing != page_id);
        inner.frames.insert(page_id, frame);
        inner.order.push_back(page_id);
        inner.failures.remove(&page_id);
        while inner.order.len() > self.capacity {
            if let Some(evicted) = inner.order.pop_front() {
                inner.frames.remove(&evicted);
            }
        }
    }

    async fn capture_failed(&self, page_id: u32, error: &str) {
        warn!(page_id, error, "screencast capture failed");
        if self.record_failure(page_id, now_epoch_ms()).await {
            warn!(page_id, "screencast page enters backoff");
        }
    }

    async fn record_failure(&self, page_id: u32, now: i64) -> bool {
        let mut inner = self.inner.lock().await;
        let state = inner.failures.entry(page_id).or_insert(FailureState {
            consecutive: 0,
            last_failure_at: now,
        });
        state.consecutive = state.consecutive.saturating_add(1);
        state.last_failure_at = now;
        let in_backoff = state.consecutive >= FAILURE_BACKOFF_THRESHOLD;
        if in_backoff {
            inner.frames.remove(&page_id);
            inner.order.retain(|existing| *existing != page_id);
        }
        in_backoff
    }

    async fn gc_pages(&self, page_ids: &[u32]) {
        if page_ids.is_empty() {
            return;
        }
        let page_ids: HashSet<u32> = page_ids.iter().copied().collect();
        let mut inner = self.inner.lock().await;
        for page_id in &page_ids {
            inner.frames.remove(page_id);
            inner.failures.remove(page_id);
        }
        inner.order.retain(|page_id| !page_ids.contains(page_id));
    }
}

/// Plan active captures and stale-frame GC without mutating service state.
fn plan_tick(
    idle: bool,
    records: &[TabActivityRecord],
    failures: &HashMap<u32, FailureState>,
    in_flight: &HashSet<u32>,
    cached_page_ids: &[u32],
) -> TickPlan {
    if idle {
        return TickPlan::default();
    }
    let live_page_ids: HashSet<u32> = records.iter().map(|record| record.page_id).collect();
    let capture = records
        .iter()
        .filter(|record| record.status == "active")
        .filter(|record| !in_flight.contains(&record.page_id))
        .filter(|record| {
            !failures.get(&record.page_id).is_some_and(|failure| {
                failure.consecutive >= FAILURE_BACKOFF_THRESHOLD
                    && record.last_tool_at <= failure.last_failure_at
            })
        })
        .map(|record| record.page_id)
        .collect();
    let gc = cached_page_ids
        .iter()
        .copied()
        .filter(|page_id| !live_page_ids.contains(page_id))
        .collect();
    TickPlan { capture, gc }
}

#[cfg(test)]
mod tests {
    use super::{FailureState, ScreencastService, TickPlan, plan_tick};
    use crate::services::tab_activity::{ScreencastFrame, TabActivityRecord};
    use std::collections::{HashMap, HashSet};

    const NOW: i64 = 1_000_000;

    fn record(page_id: u32, status: &'static str, last_tool_at: i64) -> TabActivityRecord {
        TabActivityRecord {
            target_id: format!("target-{page_id}"),
            page_id,
            url: format!("https://example.com/{page_id}"),
            title: format!("Page {page_id}"),
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
        assert_eq!(plan.capture, vec![1]);
        assert!(plan.gc.is_empty());
    }

    #[test]
    fn planner_retries_before_failure_threshold() {
        let records = [record(1, "active", NOW)];
        let failures = HashMap::from([(1, failure(2, NOW))]);
        let plan = plan_tick(false, &records, &failures, &HashSet::new(), &[]);
        assert_eq!(plan.capture, vec![1]);
    }

    #[test]
    fn planner_backs_off_after_three_failures() {
        let records = [record(1, "active", NOW)];
        let failures = HashMap::from([(1, failure(3, NOW))]);
        let plan = plan_tick(false, &records, &failures, &HashSet::new(), &[]);
        assert!(plan.capture.is_empty());
    }

    #[test]
    fn planner_lifts_backoff_after_new_tool_activity() {
        let records = [record(1, "active", NOW + 1)];
        let failures = HashMap::from([(1, failure(3, NOW))]);
        let plan = plan_tick(false, &records, &failures, &HashSet::new(), &[]);
        assert_eq!(plan.capture, vec![1]);
    }

    #[test]
    fn planner_skips_pages_with_capture_in_flight() {
        let records = [record(1, "active", NOW), record(2, "active", NOW)];
        let in_flight = HashSet::from([1]);
        let plan = plan_tick(false, &records, &HashMap::new(), &in_flight, &[]);
        assert_eq!(plan.capture, vec![2]);
    }

    #[test]
    fn planner_garbage_collects_closed_page_frames() {
        let records = [record(2, "idle", NOW), record(3, "active", NOW)];
        let plan = plan_tick(false, &records, &HashMap::new(), &HashSet::new(), &[1, 2]);
        assert_eq!(plan.capture, vec![3]);
        assert_eq!(plan.gc, vec![1]);
    }

    #[test]
    fn idle_planner_is_a_no_op() {
        let records = [record(1, "active", NOW)];
        let plan = plan_tick(true, &records, &HashMap::new(), &HashSet::new(), &[2]);
        assert_eq!(plan, TickPlan::default());
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
        service.store_frame(1, frame("a", 1)).await;
        service.store_frame(2, frame("b", 2)).await;
        service.store_frame(1, frame("new-a", 3)).await;
        service.store_frame(3, frame("c", 4)).await;

        let Some(refreshed) = service.frame_for(1).await else {
            panic!("missing page 1 frame");
        };
        assert_eq!(refreshed.jpeg_base64, "new-a");
        assert_eq!(refreshed.captured_at, 3);
        assert!(service.frame_for(2).await.is_none());
        let Some(newest) = service.frame_for(3).await else {
            panic!("missing page 3 frame");
        };
        assert_eq!(newest.jpeg_base64, "c");
        assert_eq!(newest.captured_at, 4);
    }

    #[tokio::test]
    async fn entering_backoff_drops_frame_but_keeps_failure_state() {
        let service = ScreencastService::new(2);
        service.store_frame(1, frame("stale", 1)).await;
        assert!(!service.record_failure(1, NOW - 2).await);
        assert!(!service.record_failure(1, NOW - 1).await);
        assert!(service.record_failure(1, NOW).await);

        let inner = service.inner.lock().await;
        assert!(!inner.frames.contains_key(&1));
        assert_eq!(inner.failures.get(&1), Some(&failure(3, NOW)));
    }

    #[tokio::test]
    async fn successful_capture_clears_failure_state() {
        let service = ScreencastService::new(2);
        assert!(!service.record_failure(1, NOW - 2).await);
        assert!(!service.record_failure(1, NOW - 1).await);
        assert!(service.record_failure(1, NOW).await);
        service.store_frame(1, frame("fresh", NOW + 1)).await;

        assert!(!service.inner.lock().await.failures.contains_key(&1));
    }

    #[tokio::test]
    async fn per_page_in_flight_guard_clears_only_on_completion() {
        let service = ScreencastService::new(2);
        assert!(service.begin_capture(1).await);
        assert!(!service.begin_capture(1).await);
        service.clear_in_flight(1).await;
        assert!(service.begin_capture(1).await);
    }

    #[tokio::test]
    async fn garbage_collection_drops_frame_and_failure_state() {
        let service = ScreencastService::new(2);
        service.store_frame(1, frame("stale", 1)).await;
        service.record_failure(1, NOW).await;
        service.gc_pages(&[1]).await;

        let inner = service.inner.lock().await;
        assert!(!inner.frames.contains_key(&1));
        assert!(!inner.failures.contains_key(&1));
        assert!(!inner.order.contains(&1));
    }
}
