//! The camera: takes the screenshots the trace writer asks for, on the agent's own browser
//! connection, in the background.
//!
//! The writer decides which operations get a screenshot and accounts for every answer
//! (`browseros_trace::screenshot_queue`); this loop only takes the pictures. Each capture is
//! claw-server's proven chain (decision 21): an unbound view on the agent's session holds the
//! tab's viewport, then `screenshot_for_target` takes a JPEG of what the tab shows, hiding the
//! agent cursor and never touching emulation. A capture from another DevTools session would
//! show the cursor, and a `clip` from one may reset the agent's emulation.
//!
//! ```text
//!  ShotJobs ─▶ waiting (in order, a few per tab) ─▶ one job at a time:
//!    privacy.capture(tab) ─ None ─────────────────────────────────────────▶ Withheld
//!    tab → page and target ─ tab closed, dialog open ─────────────────────▶ Dropped
//!    hold the viewport, capture ─ error ──────────────────────────────────▶ Dropped
//!      past CAPTURE_TIMEOUT: aborted, then again from the lookup after CAPTURE_BACKOFF,
//!        unless the tab went private meanwhile ───────────────────────────▶ Withheld
//!        CAPTURE_ATTEMPTS all timed out, or CAPTURE_BUDGET spent ─────────▶ Dropped
//!    guard.admit() ─ false ───────────────────────────────────────────────▶ Withheld
//!    write the JPEG at the job's path ────────────────────────────────────▶ Taken
//! ```
//!
//! **Privacy order.** The capture guard is taken before any browser call, so a tab that goes
//! private at any later point (during the lookup, the capture or the decode) fails `admit`, and
//! nothing is written. A tab that is already private costs no browser call at all.
//!
//! **In order, bounded per tab.** Jobs are taken in the order the writer made them, so each
//! write's screenshot is its own capture, taken after it (decision 37). At most
//! `MAX_WAITING_PER_TAB` wait on one tab: a newer job drops the tab's oldest, whose answer
//! reports `Dropped`, so a burst of writes on one tab cannot leave the camera far behind the
//! agent. The writer folds a run-end shot into a job still out on its tab, so a run's end adds
//! no second capture of a tab whose write is still waiting here.
//!
//! **Retries, concurrency 1.** A busy renderer answers late, not never, so an attempt past
//! `CAPTURE_TIMEOUT` is aborted and the job tries again after `CAPTURE_BACKOFF`, up to
//! `CAPTURE_ATTEMPTS` inside `CAPTURE_BUDGET`, before it answers `Dropped` (decision 37). The
//! abort is what makes a retry a fresh capture: core holds a per-session capture lock for the
//! whole capture, so a second request would only queue behind the stuck one. It also releases
//! the tab's viewport at once, and the next job never waits on a capture already given up. A
//! private tab, a closed tab, an open dialog and a failed call are answered at once and never
//! retried, and a tab that goes private between attempts gets no further capture.
//!
//! **Off the agent's path.** The writer hands jobs over an unbounded queue and never waits. A
//! capture holds the tab's viewport shared, as the agent's ordinary operations do, so it delays
//! only what holds the tab alone: a viewport change, or an operation on a tab in a fixed
//! viewport mode, by at most one attempt (`CAPTURE_TIMEOUT`); the pause before a retry holds
//! nothing. A takeover's fitted tab is private, so it is never captured.

use crate::limits::{
    CAPTURE_ATTEMPTS, CAPTURE_BACKOFF, CAPTURE_BUDGET, CAPTURE_TIMEOUT, MAX_WAITING_PER_TAB,
    SCREENSHOT_QUALITY,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use browseros_core::screenshot::{ScreenshotCaptureOptions, ScreenshotFormat};
use browseros_core::viewport::Viewports;
use browseros_core::{BrowserSession, BrowserView, TabId};
use browseros_trace::{CaptureGuard, OperationId, Privacy, Shot, ShotJob, ShotJobs};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use tokio::time::Instant;

/// What the camera captures with: the agent's session and viewports, and the privacy every
/// capture is admitted by.
pub(crate) struct Camera {
    pub(crate) browser: Arc<BrowserSession>,
    pub(crate) viewports: Arc<Viewports>,
    pub(crate) privacy: Arc<Privacy>,
}

impl Camera {
    /// Starts the loop over `jobs`. It ends when the writer stops; jobs still waiting then are
    /// dropped, and each answers `Dropped`.
    pub(crate) fn spawn(self, jobs: ShotJobs) {
        tokio::spawn(self.run(jobs));
    }

    async fn run(self, mut jobs: ShotJobs) {
        let mut waiting = Waiting::default();
        loop {
            if waiting.is_empty() {
                match jobs.recv().await {
                    Some(job) => waiting.push(job),
                    None => return,
                }
            }
            while let Some(job) = jobs.try_recv() {
                waiting.push(job);
            }
            if let Some(job) = waiting.pop() {
                self.take(job).await;
            }
        }
    }

    /// Takes one job's picture and answers it.
    async fn take(&self, job: ShotJob) {
        let ShotJob {
            op,
            tab,
            path,
            answer,
        } = job;
        let Some(guard) = self.privacy.capture(tab) else {
            answer.send(Shot::Withheld);
            return;
        };
        let shot = match self.capture(&guard, op, tab).await {
            Err(missed) => missed.answer(op, tab),
            // After the pixels arrive and before any byte is kept: the tab may have gone private
            // since the guard was taken (D6's epoch rule).
            Ok(_) if !guard.admit() => Shot::Withheld,
            Ok(jpeg) => match write_jpeg(&path, &jpeg).await {
                Ok(()) => Shot::Taken {
                    bytes_len: u64::try_from(jpeg.len()).unwrap_or(u64::MAX),
                },
                Err(error) => {
                    tracing::warn!(%op, %error, "screenshot could not be written; dropped");
                    Shot::Dropped
                }
            },
        };
        answer.send(shot);
    }

    /// Captures `tab`, retrying an attempt that runs past `CAPTURE_TIMEOUT`. Each attempt runs
    /// as its own task, so a panic in it is a failed capture rather than the camera's end.
    async fn capture(
        &self,
        guard: &CaptureGuard,
        op: OperationId,
        tab: TabId,
    ) -> Result<Vec<u8>, Missed> {
        let deadline = Instant::now() + CAPTURE_BUDGET;
        let mut attempts = 1;
        loop {
            let mut running = tokio::spawn(attempt(
                Arc::clone(&self.browser),
                Arc::clone(&self.viewports),
                tab,
            ));
            let limit = deadline.min(Instant::now() + CAPTURE_TIMEOUT);
            let joined = match tokio::time::timeout_at(limit, &mut running).await {
                Ok(joined) => joined,
                // Awaited after the abort, so the attempt's viewport hold and core's capture
                // lock are released before the pause. One that finished as the timeout fired
                // still counts.
                Err(_) => {
                    running.abort();
                    running.await
                }
            };
            match joined {
                Ok(pixels) => return pixels,
                Err(error) if error.is_cancelled() => {}
                Err(panicked) => return Err(Missed::Failed(panicked.to_string())),
            }
            let retry_at = Instant::now() + CAPTURE_BACKOFF;
            if attempts == CAPTURE_ATTEMPTS || retry_at >= deadline {
                return Err(Missed::TimedOut(attempts));
            }
            tracing::debug!(%op, tab = tab.0, attempts, "screenshot capture timed out; retrying");
            tokio::time::sleep_until(retry_at).await;
            if !guard.admit() {
                return Err(Missed::WentPrivate);
            }
            attempts += 1;
        }
    }
}

/// The jobs not started yet, oldest first, at most `MAX_WAITING_PER_TAB` on one tab.
#[derive(Default)]
struct Waiting(VecDeque<ShotJob>);

impl Waiting {
    /// Queues `job` behind every job waiting. On a tab already at the bound, the tab's oldest
    /// waiting job is dropped here, and its answer reports `Dropped`.
    fn push(&mut self, job: ShotJob) {
        let on_tab = self
            .0
            .iter()
            .filter(|waiting| waiting.tab == job.tab)
            .count();
        if on_tab >= MAX_WAITING_PER_TAB
            && let Some(oldest) = self.0.iter().position(|waiting| waiting.tab == job.tab)
        {
            drop(self.0.remove(oldest));
        }
        self.0.push_back(job);
    }

    fn pop(&mut self) -> Option<ShotJob> {
        self.0.pop_front()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Why a capture got no picture.
enum Missed {
    /// The tab closed, or its target changed under the capture.
    Closed,
    /// A JavaScript dialog is open: it stalls the renderer, so a capture would only wait on it
    /// while holding the tab.
    Dialog,
    /// The browser refused or failed, or the capture task panicked.
    Failed(String),
    /// Every attempt ran past `CAPTURE_TIMEOUT`; how many there were.
    TimedOut(u32),
    /// The tab went private while a retry waited, so it got none.
    WentPrivate,
}

impl Missed {
    /// The job's answer, with its one log line. A closed tab or an open dialog is ordinary.
    fn answer(self, op: OperationId, tab: TabId) -> Shot {
        match self {
            Self::WentPrivate => return Shot::Withheld,
            Self::Closed => tracing::debug!(%op, tab = tab.0, "screenshot dropped: tab closed"),
            Self::Dialog => tracing::debug!(%op, tab = tab.0, "screenshot dropped: dialog open"),
            Self::Failed(error) => {
                tracing::warn!(%op, tab = tab.0, %error, "screenshot capture failed; dropped");
            }
            Self::TimedOut(attempts) => {
                tracing::warn!(%op, tab = tab.0, attempts, "screenshot capture timed out; dropped");
            }
        }
        Shot::Dropped
    }
}

/// One attempt: finds the tab's page and captures it, as a JPEG at the tab's own size.
async fn attempt(
    browser: Arc<BrowserSession>,
    viewports: Arc<Viewports>,
    tab: TabId,
) -> Result<Vec<u8>, Missed> {
    let failed = |error: browseros_core::CoreError| Missed::Failed(error.to_string());
    let pages = browser
        .pages
        .resolve_tab_ids(&[tab])
        .await
        .map_err(failed)?;
    let page = pages.get(&tab).cloned().ok_or(Missed::Closed)?;
    let target = browser
        .pages
        .get_info(page.clone())
        .await
        .ok_or(Missed::Closed)?
        .target_id;
    if browser.page_signals.pending_dialog(&page).is_some() {
        return Err(Missed::Dialog);
    }
    // Shared with the agent's operations on the tab and exclusive of a viewport change, so the
    // capture never sees a half-applied mode.
    let _held = BrowserView::unbound(Arc::clone(&browser), None)
        .with_viewports(viewports)
        .hold_viewport(page.clone())
        .await;
    let shot = browser
        .screenshot_for_target(page, &target, capture_options())
        .await
        .map_err(failed)?
        .ok_or(Missed::Closed)?;
    let jpeg = STANDARD
        .decode(shot.data)
        .map_err(|error| Missed::Failed(format!("invalid screenshot data: {error}")))?;
    if jpeg.is_empty() {
        return Err(Missed::Failed("an empty screenshot".into()));
    }
    Ok(jpeg)
}

/// What the tab shows, as JPEG: no full page, no annotations, no clip.
fn capture_options() -> ScreenshotCaptureOptions {
    ScreenshotCaptureOptions {
        format: Some(ScreenshotFormat::Jpeg),
        quality: Some(SCREENSHOT_QUALITY),
        full_page: Some(false),
        annotate: Some(false),
        clip: None,
    }
}

/// Writes the JPEG under a temporary name and renames it into place, so a reader never sees
/// half a file. A crash leaves at most the temporary file in the run's directory, which the
/// retention sweep deletes with the run.
async fn write_jpeg(path: &Path, jpeg: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let partial = path.with_extension("jpg.partial");
    let written = async {
        tokio::fs::write(&partial, jpeg).await?;
        tokio::fs::rename(&partial, path).await
    }
    .await;
    if written.is_err() {
        let _ = tokio::fs::remove_file(&partial).await;
    }
    written
}
