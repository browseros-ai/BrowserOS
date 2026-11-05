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
//!  ShotJobs ─▶ waiting (one job per tab) ─▶ one job at a time:
//!    privacy.capture(tab) ─ None ─────────────────────────────────────────▶ Withheld
//!    tab → page and target ─ tab closed, dialog open ──────────────────────▶ Dropped
//!    hold the viewport, capture ─ error, or past CAPTURE_TIMEOUT ──────────▶ Dropped
//!    guard.admit() ─ false ────────────────────────────────────────────────▶ Withheld
//!    write the JPEG at the job's path ─────────────────────────────────────▶ Taken
//! ```
//!
//! **Privacy order.** The capture guard is taken before any browser call, so a tab that goes
//! private at any later point (during the lookup, the capture or the decode) fails `admit`, and
//! nothing is written. A tab that is already private costs no browser call at all.
//!
//! **Coalescing.** A newer job on a tab replaces the one waiting for it, and the replaced job's
//! answer reports `Dropped` as it is dropped. The newer job shows the page later, so the camera
//! never falls behind the agent.
//!
//! **Concurrency 1.** A capture past `CAPTURE_TIMEOUT` answers `Dropped` at once, but the next
//! job waits for it up to `STUCK_CAPTURE_CAP`, as claw-server's in-flight slot does. Then it is
//! aborted, which releases the tab's viewport.
//!
//! **Off the agent's path.** The writer hands jobs over an unbounded queue and never waits. A
//! capture holds the tab's viewport shared, as the agent's ordinary operations do, so it delays
//! only what holds the tab alone: a viewport change, or an operation on a tab in a fixed
//! viewport mode, by at most one capture (`STUCK_CAPTURE_CAP` if it is stuck). A takeover's
//! fitted tab is private, so it is never captured.

use crate::limits::{CAPTURE_TIMEOUT, SCREENSHOT_QUALITY, STUCK_CAPTURE_CAP};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use browseros_core::screenshot::{ScreenshotCaptureOptions, ScreenshotFormat};
use browseros_core::viewport::Viewports;
use browseros_core::{BrowserSession, BrowserView, TabId};
use browseros_trace::{OperationId, Privacy, Shot, ShotJob, ShotJobs};
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
        let started = Instant::now();
        let mut capture = tokio::spawn(capture(
            Arc::clone(&self.browser),
            Arc::clone(&self.viewports),
            tab,
        ));
        let pixels = match tokio::time::timeout(CAPTURE_TIMEOUT, &mut capture).await {
            Ok(Ok(pixels)) => pixels,
            Ok(Err(panicked)) => Err(Missed::Failed(panicked.to_string())),
            Err(_) => {
                tracing::warn!(%op, tab = tab.0, "screenshot capture timed out; dropped");
                answer.send(Shot::Dropped);
                if tokio::time::timeout_at(started + STUCK_CAPTURE_CAP, &mut capture)
                    .await
                    .is_err()
                {
                    capture.abort();
                }
                return;
            }
        };
        let shot = match pixels {
            Err(missed) => {
                missed.log(op, tab);
                Shot::Dropped
            }
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
}

/// The jobs not started yet: at most one per tab, oldest tab first.
#[derive(Default)]
struct Waiting(VecDeque<ShotJob>);

impl Waiting {
    /// Queues `job`, replacing the job already waiting on its tab. The replaced job is dropped
    /// here, and its answer reports `Dropped`.
    fn push(&mut self, job: ShotJob) {
        match self.0.iter_mut().find(|waiting| waiting.tab == job.tab) {
            Some(waiting) => *waiting = job,
            None => self.0.push_back(job),
        }
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
}

impl Missed {
    /// The one log line for a dropped job. A closed tab or an open dialog is ordinary.
    fn log(&self, op: OperationId, tab: TabId) {
        match self {
            Self::Closed => tracing::debug!(%op, tab = tab.0, "screenshot dropped: tab closed"),
            Self::Dialog => tracing::debug!(%op, tab = tab.0, "screenshot dropped: dialog open"),
            Self::Failed(error) => {
                tracing::warn!(%op, tab = tab.0, %error, "screenshot capture failed; dropped");
            }
        }
    }
}

/// Finds the tab's page and captures it, as a JPEG at the tab's own size.
async fn capture(
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
