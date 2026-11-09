//! Neo: After-call work, off the run path. One scheduler coalesces facts by target and keeps one
//! pending still per session. Work's drop marker settles every exit path, including refusal
//! and shutdown; artifact writes carry that marker back through the host writer first.

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::host::SessionKey;
use crate::host::cdp;
use crate::host::link::HostLink;
use crate::host::presence::Presences;
use crate::host::sessions::JobKind;
use crate::host::store::CallArtifacts;
use crate::host::task::{Artifacts, Jobs, Work};
use crate::run::{PageId, RunId};

/// Neo: at most 256 waiting rows per target bounds a burst's memory without blocking the run path (design §6.5).
const WAITING_PER_TARGET: usize = 256;
/// Neo: at most 1024 waiting targets bounds scheduler memory when many tabs finish calls at once.
const WAITING_TARGETS: usize = 1024;
/// Neo: eight concurrent facts reads keep browser work bounded while unrelated targets can progress.
const FACTS_IN_FLIGHT: usize = 8;

/// Neo: share screenshot slots process-wide, including other runtimes and commands whose caller timed out, so timed-out work cannot exceed the browser concurrency bound.
static STILL_SLOTS: Semaphore = Semaphore::const_new(cdp::SCREENSHOT_SLOTS);

type Row = (RunId, Option<u64>);
type Read = (PageId, Vec<Work>, Result<Option<CallArtifacts>, ()>);

#[derive(Default)]
struct Target {
    waiting: HashMap<Row, Vec<Work>>,
    reading: bool,
}

struct Still {
    work: Work,
    facts: CallArtifacts,
}

struct Worker {
    link: Arc<HostLink>,
    presences: Arc<Presences>,
    targets: HashMap<PageId, Target>,
    reads: FuturesUnordered<BoxFuture<'static, Read>>,
    pending: HashMap<SessionKey, Still>,
    stills: FuturesUnordered<BoxFuture<'static, Artifacts>>,
    shutdown: CancellationToken,
    refused: u64,
    // The result sender closes last: the actor uses its closure as proof that every Work
    // guard in targets, reads, pending and stills has already enqueued its settlement.
    jobs: Jobs,
}

pub(super) fn spawn(
    link: Arc<HostLink>,
    presences: Arc<Presences>,
    jobs: Jobs,
    shutdown: CancellationToken,
) {
    let owner = Arc::clone(&link);
    owner.spawn(
        Worker {
            link,
            presences,
            jobs,
            targets: HashMap::new(),
            reads: FuturesUnordered::new(),
            pending: HashMap::new(),
            stills: FuturesUnordered::new(),
            shutdown,
            refused: 0,
        }
        .run(),
    );
}

impl Worker {
    async fn run(mut self) {
        loop {
            self.start_reads();
            // A newer still or a privacy revocation finalizes old work before taking a slot.
            self.pending.retain(|_, still| {
                !still.work.revoked.is_cancelled()
                    && !still.work.newest.is_cancelled()
                    && !still.work.tainted()
            });
            tokio::select! {
                () = self.shutdown.cancelled() => return,
                work = self.jobs.waiting.recv() => match work {
                    Some(work) => {
                        self.enqueue(work);
                        // Coalesce the burst already accepted at the tap, rather than issuing
                        // one browser read per immediately ready receive.
                        while let Ok(work) = self.jobs.waiting.try_recv() { self.enqueue(work); }
                    },
                    None => return,
                },
                Some(read) = self.reads.next(), if !self.reads.is_empty() => self.read_done(read).await,
                permit = STILL_SLOTS.acquire(), if !self.pending.is_empty() => {
                    let Ok(permit) = permit else { return; };
                    if let Some(key) = self.pending.keys().next().cloned()
                        && let Some(still) = self.pending.remove(&key) {
                        self.stills.push(Box::pin(screenshot(Arc::clone(&self.presences), still, permit)));
                    }
                },
                Some(result) = self.stills.next(), if !self.stills.is_empty() => { let _ = self.jobs.results.send(result).await; },
            }
        }
    }

    fn enqueue(&mut self, work: Work) {
        if work.job.kind == JobKind::Still
            && (work.revoked.is_cancelled() || work.newest.is_cancelled() || work.tainted())
        {
            return;
        }
        if !self.targets.contains_key(&work.job.page) && self.targets.len() >= WAITING_TARGETS {
            self.refused += 1;
            warn!(
                refused = self.refused,
                "host after-call target bound reached; finalizing absence"
            );
            return;
        }
        let target = self.targets.entry(work.job.page.clone()).or_default();
        let row = (work.job.run.clone(), work.job.call_id);
        if !target.waiting.contains_key(&row) && target.waiting.len() >= WAITING_PER_TARGET {
            self.refused += 1;
            warn!(
                refused = self.refused,
                "host after-call per-target bound reached; finalizing absence"
            );
            return;
        }
        target.waiting.entry(row).or_default().push(work);
    }

    fn start_reads(&mut self) {
        for (page, target) in &mut self.targets {
            if self.reads.len() >= FACTS_IN_FLIGHT {
                break;
            }
            if target.reading || target.waiting.is_empty() {
                continue;
            }
            target.reading = true;
            let work = target.waiting.drain().flat_map(|(_, work)| work).collect();
            let page = page.clone();
            let link = Arc::clone(&self.link);
            self.reads.push(Box::pin(async move {
                let facts = cdp::tab_facts(&link, &page).await;
                match facts {
                    Ok(facts) => (page, work, Ok(facts)),
                    Err(error) => {
                        debug!(%error, "host page facts read failed");
                        (page, work, Err(()))
                    }
                }
            }));
        }
    }

    async fn read_done(&mut self, (page, jobs, facts): Read) {
        if let Some(target) = self.targets.get_mut(&page) {
            target.reading = false;
            if target.waiting.is_empty() {
                self.targets.remove(&page);
            }
        }
        for work in jobs {
            let value = match &facts {
                Ok(Some(facts)) => Some(facts.clone()),
                Ok(None) => work.fallback.clone(),
                Err(()) => None,
            };
            match (work.job.kind, value) {
                (JobKind::Still, Some(facts))
                    if !work.revoked.is_cancelled()
                        && !work.newest.is_cancelled()
                        && !work.tainted() =>
                {
                    self.pending
                        .insert(work.job.session.clone(), Still { work, facts });
                }
                (JobKind::Facts, value) => {
                    let _ = self.jobs.results.send(Artifacts { work, value }).await;
                }
                _ => {}
            }
        }
    }
}

async fn screenshot(
    presences: Arc<Presences>,
    still: Still,
    permit: SemaphorePermit<'static>,
) -> Artifacts {
    let Still { work, mut facts } = still;
    let revoked = work.revoked.clone();
    let newest = work.newest.clone();
    let stop = CancellationToken::new();
    let stopping = stop.clone();
    let page = work.job.page.clone();
    let taint = Arc::clone(&work.taint);
    // The browser may finish a command after its caller timed out. This detached operation
    // retains the process-wide slot until that reply, preventing timeout storms from exceeding
    // two actual screenshot commands.
    // Neo: claw-server's `SessionVisualService::capture_with_session` (`apps/claw-server/src/services/cockpit/visual.rs`) retains the worker's slot beyond its caller's timeout; otherwise timeout storms bypass the concurrency cap.
    let mut changes = work.taint.changes();
    let owner = Arc::clone(&presences);
    let mut operation = owner.spawn(async move {
        let _permit = permit;
        let presence = presences.ensure(&page).await.ok()?;
        let viewport = cdp::screenshot_viewport(presence.session()).await.ok()?;
        for rung in cdp::STILL_LADDER {
            if revoked.is_cancelled()
                || newest.is_cancelled()
                || stopping.is_cancelled()
                || taint.contains(&page)
            {
                return None;
            }
            let jpeg = cdp::capture_still(presence.session(), &viewport, rung)
                .await
                .ok()?;
            if jpeg.len() <= cdp::MAX_STILL_BYTES {
                return Some(jpeg);
            }
        }
        None
    });
    let deadline = Instant::now() + cdp::SCREENSHOT_TIMEOUT;
    let image = loop {
        if work.tainted() {
            break None;
        }
        tokio::select! {
            () = work.revoked.cancelled() => break None,
            () = work.newest.cancelled() => break None,
            _ = changes.changed() => {},
            result = time::timeout_at(deadline, &mut operation) => break result.ok().and_then(Result::ok).flatten(),
        }
    };
    stop.cancel();
    facts.screenshot = image;
    let value = (facts.screenshot.is_some() && !work.tainted()).then_some(facts);
    Artifacts { work, value }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;
    use tokio::sync::{Mutex, mpsc};

    use super::*;
    use crate::fake_host_cdp::FakeHostCdp;
    use crate::host::sessions::Job;

    static ONE_WORKER: Mutex<()> = Mutex::const_new(());

    struct Harness {
        browser: FakeHostCdp,
        link: Arc<HostLink>,
        input: mpsc::Sender<Work>,
        output: mpsc::Receiver<Artifacts>,
        settled: mpsc::UnboundedSender<SessionKey>,
        settlements: mpsc::UnboundedReceiver<SessionKey>,
        shutdown: CancellationToken,
    }

    impl Harness {
        async fn new() -> Self {
            let browser = FakeHostCdp::start().await;
            browser.set_tabs(vec![FakeHostCdp::tab("one", 1, "https://one.test/")]);
            let shutdown = CancellationToken::new();
            let link = HostLink::connect(browser.port(), shutdown.clone())
                .await
                .unwrap();
            let presences = Arc::new(Presences::new(Arc::clone(&link)));
            let (input, waiting) = mpsc::channel(1024);
            let (results, output) = mpsc::channel(1024);
            let (settled, settlements) = mpsc::unbounded_channel();
            spawn(
                Arc::clone(&link),
                presences,
                Jobs { waiting, results },
                shutdown.clone(),
            );
            Self {
                browser,
                link,
                input,
                output,
                settled,
                settlements,
                shutdown,
            }
        }
        fn work(&self, kind: JobKind, call: u64, page: &str) -> Work {
            Work::testing(
                Job {
                    kind,
                    session: SessionKey::new("session"),
                    run: RunId::mint(),
                    call_id: Some(call),
                    page: PageId::new(page.to_string()),
                },
                self.settled.clone(),
            )
        }
        async fn result(&mut self) -> Artifacts {
            time::timeout(Duration::from_secs(10), self.output.recv())
                .await
                .unwrap()
                .unwrap()
        }
        async fn settled(&mut self, count: usize) {
            for _ in 0..count {
                time::timeout(Duration::from_secs(10), self.settlements.recv())
                    .await
                    .unwrap()
                    .unwrap();
            }
            assert!(
                self.settlements.try_recv().is_err(),
                "each accepted job settles exactly once"
            );
        }
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            self.shutdown.cancel();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn jobs_during_a_read_share_the_next_read() {
        let _turn = ONE_WORKER.lock().await;
        let mut h = Harness::new().await;
        h.browser.delay_facts(Duration::from_secs(1));
        h.input
            .send(h.work(JobKind::Facts, 1, "one"))
            .await
            .ok()
            .unwrap();
        h.browser.wait_for("Browser.getTabInfo", 1).await;
        for call in 2..=20 {
            h.input
                .send(h.work(JobKind::Facts, call, "one"))
                .await
                .ok()
                .unwrap();
        }
        for _ in 0..20 {
            let result = h.result().await;
            assert_eq!(result.value.as_ref().map(|value| value.tab.get()), Some(1));
            drop(result);
        }
        assert_eq!(h.browser.sent("Browser.getTabInfo").len(), 2);
        h.settled(20).await;
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_fitting_ladder_rung_uses_the_viewport_clip() {
        let _turn = ONE_WORKER.lock().await;
        let mut h = Harness::new().await;
        h.browser.screenshot_sizes(&[70_000, 65_000, 1024]);
        h.input
            .send(h.work(JobKind::Still, 1, "one"))
            .await
            .ok()
            .unwrap();
        let result = h.result().await;
        assert_eq!(
            result
                .value
                .as_ref()
                .and_then(|value| value.screenshot.as_ref())
                .map(Bytes::len),
            Some(65_000)
        );
        let shots = h.browser.sent("Page.captureScreenshot");
        assert_eq!(shots.len(), 2);
        assert_eq!(
            shots[0].params["clip"],
            serde_json::json!({"x":20.0,"y":30.0,"width":1280.0,"height":800.0,"scale":0.625})
        );
        assert_eq!(shots[1].params["clip"]["scale"], 0.5);
        assert_eq!(shots[1].params["quality"], 50);
        let emulation = h.browser.sent("Emulation.setFocusEmulationEnabled");
        assert_eq!(
            emulation[0].session, shots[0].session,
            "only the presence captures screenshots"
        );
        let commands = h.browser.commands();
        assert!(
            commands
                .iter()
                .position(|cmd| cmd.method == "Emulation.setFocusEmulationEnabled")
                < commands
                    .iter()
                    .position(|cmd| cmd.method == "Page.captureScreenshot")
        );
        drop(result);
        h.settled(1).await;
    }

    #[tokio::test(start_paused = true)]
    async fn third_rung_and_total_overflow_are_bounded_jpeg_results() {
        let _turn = ONE_WORKER.lock().await;
        let mut h = Harness::new().await;
        h.browser.screenshot_sizes(&[70_000, 70_000, 60_000]);
        h.input
            .send(h.work(JobKind::Still, 1, "one"))
            .await
            .ok()
            .unwrap();
        let result = h.result().await;
        assert_eq!(
            result
                .value
                .as_ref()
                .and_then(|value| value.screenshot.as_ref())
                .map(Bytes::len),
            Some(60_000)
        );
        assert_eq!(
            h.browser.sent("Page.captureScreenshot")[2].params["quality"],
            30
        );
        drop(result);
        h.browser.screenshot_sizes(&[70_000, 70_000, 70_000]);
        h.input
            .send(h.work(JobKind::Still, 2, "one"))
            .await
            .ok()
            .unwrap();
        let result = h.result().await;
        assert!(result.value.is_none());
        drop(result);
        h.settled(2).await;
    }

    #[tokio::test(start_paused = true)]
    async fn newest_and_revoked_in_flight_images_are_discarded_and_all_jobs_settle() {
        let _turn = ONE_WORKER.lock().await;
        let mut h = Harness::new().await;
        h.browser.delay_screenshots(Duration::from_secs(1));
        let old = h.work(JobKind::Still, 1, "one");
        let old_gate = old.newest.clone();
        h.input.send(old).await.ok().unwrap();
        h.browser.wait_for("Page.captureScreenshot", 1).await;
        old_gate.cancel();
        let newer = h.work(JobKind::Still, 2, "one");
        let revoke = newer.revoked.clone();
        h.input.send(newer).await.ok().unwrap();
        let old = h.result().await;
        assert!(old.value.is_none());
        drop(old);
        h.browser.wait_for("Page.captureScreenshot", 2).await;
        revoke.cancel();
        let newer = h.result().await;
        assert!(newer.value.is_none());
        drop(newer);
        h.settled(2).await;
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_retains_the_slot_until_the_browser_finishes() {
        let _turn = ONE_WORKER.lock().await;
        let mut h = Harness::new().await;
        h.browser.delay_screenshots(Duration::from_secs(3));
        h.input
            .send(h.work(JobKind::Still, 1, "one"))
            .await
            .ok()
            .unwrap();
        let result = h.result().await;
        assert!(result.value.is_none());
        drop(result);
        assert_eq!(
            STILL_SLOTS.available_permits(),
            1,
            "the timed-out remote command still owns its slot"
        );
        time::sleep(Duration::from_secs(2)).await;
        assert_eq!(STILL_SLOTS.available_permits(), 2);
        h.settled(1).await;
    }

    #[tokio::test(start_paused = true)]
    async fn refused_rows_and_targets_settle_as_permanent_absence() {
        let _turn = ONE_WORKER.lock().await;
        let mut h = Harness::new().await;
        let (_input, waiting) = mpsc::channel(1);
        let (results, _output) = mpsc::channel(1);
        let mut worker = Worker {
            link: Arc::clone(&h.link),
            presences: Arc::new(Presences::new(Arc::clone(&h.link))),
            jobs: Jobs { waiting, results },
            targets: HashMap::new(),
            reads: FuturesUnordered::new(),
            pending: HashMap::new(),
            stills: FuturesUnordered::new(),
            shutdown: h.shutdown.clone(),
            refused: 0,
        };
        for call in 0..=256 {
            worker.enqueue(h.work(JobKind::Facts, call, "one"));
        }
        assert_eq!(
            worker.targets[&PageId::new("one".to_string())]
                .waiting
                .len(),
            256
        );
        for target in 1..=1024 {
            worker.enqueue(h.work(JobKind::Facts, 1, &format!("target-{target}")));
        }
        assert_eq!(worker.targets.len(), 1024);
        assert_eq!(worker.refused, 2);
        worker.start_reads();
        assert_eq!(worker.reads.len(), 8);
        assert_eq!(
            h.browser.sent("Browser.getTabInfo").len(),
            0,
            "nothing replays refused work as directory facts"
        );
        drop(worker);
        h.settled(1281).await;
    }
    #[tokio::test(start_paused = true)]
    async fn shared_taint_discards_an_in_flight_image_without_a_task_update() {
        let _turn = ONE_WORKER.lock().await;
        let mut h = Harness::new().await;
        h.browser.delay_screenshots(Duration::from_secs(1));
        let work = h.work(JobKind::Still, 1, "one");
        let taint = Arc::clone(&work.taint);
        let old_gate = work.revoked.clone();
        h.input.send(work).await.ok().unwrap();
        h.browser.wait_for("Page.captureScreenshot", 1).await;
        taint.insert(PageId::new("one".to_string()));
        let result = h.result().await;
        assert!(
            !old_gate.is_cancelled(),
            "no cached actor revocation was needed"
        );
        assert!(
            result.value.is_none(),
            "the worker reads the shared set before store handoff"
        );
        drop(result);
        h.settled(1).await;
        time::sleep(Duration::from_secs(2)).await;
    }
}
