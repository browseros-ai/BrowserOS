//! The screenshot jobs between the trace writer and the camera, and how each job's result comes
//! back.
//!
//! ```text
//!  op ends (rule) ─▶ writer ─ ShotJob {op, tab, path, answer} ─▶ ShotJobs ─▶ camera
//!  run ends ─▶ writer: a job per touched tab whose last op got none ─┘          │ writes the
//!                      and that has no job out on it                            │ JPEG at path
//!  writer ◀─ ShotAnswer::send(Shot), or its drop ◀───────────────────────────────┘
//!    stores the answer's state in its next batch, and marks the run complete once the run has
//!    ended and none of its jobs is unanswered
//! ```
//!
//! Every job gets exactly one answer, and the type enforces it: [`ShotAnswer::send`] consumes the
//! answer, and an answer dropped unsent answers [`Shot::Dropped`]. A job the camera drops,
//! gives up on, never reads, or loses to a panic still answers. The writer holds a run
//! incomplete until every job it made for the run is answered, so one lost answer would hold the
//! run incomplete for the life of the process.
//!
//! Answers travel on the writer's own command queue, behind whatever was sent before them, so a
//! flush after an answer finds it stored.

use crate::{OperationId, TraceSink};
use browseros_core::TabId;
use std::fmt;
use std::path::PathBuf;
use tokio::sync::mpsc;

/// The queue between the writer ([`ShotQueue`], given to
/// [`spawn_writer`](crate::spawn_writer)) and the camera ([`ShotJobs`]). Unbounded on purpose:
/// the writer never waits on the camera, and the camera reads every job as soon as it is free,
/// keeping a few per tab.
#[must_use]
pub fn screenshot_queue() -> (ShotQueue, ShotJobs) {
    let (tx, rx) = mpsc::unbounded_channel();
    (ShotQueue(tx), ShotJobs(rx))
}

/// The writer's end of the queue.
pub struct ShotQueue(mpsc::UnboundedSender<ShotJob>);

impl ShotQueue {
    /// Hands `job` to the camera. A camera that is gone drops it, which answers `Dropped`.
    pub(crate) fn send(&self, job: ShotJob) {
        drop(self.0.send(job));
    }
}

/// The camera's end of the queue. Dropping it drops every waiting job, and each answers
/// `Dropped`.
pub struct ShotJobs(mpsc::UnboundedReceiver<ShotJob>);

impl ShotJobs {
    /// The next job; `None` once the writer has stopped.
    pub async fn recv(&mut self) -> Option<ShotJob> {
        self.0.recv().await
    }

    /// A job if one is waiting, so the camera can take in everything queued before it picks.
    pub fn try_recv(&mut self) -> Option<ShotJob> {
        self.0.try_recv().ok()
    }
}

/// One screenshot to take: the page in `tab` as it is now, stored at `path` as `op`'s.
#[derive(Debug)]
pub struct ShotJob {
    /// The operation the screenshot belongs to.
    pub op: OperationId,
    /// The tab to capture.
    pub tab: TabId,
    /// Where the camera writes the JPEG (`screenshots/<run>/<seq>.jpg` beside `trace.db`).
    pub path: PathBuf,
    /// How the camera reports what became of the job.
    pub answer: ShotAnswer,
}

/// What became of one job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shot {
    /// The JPEG is at the job's path, admitted by its capture guard.
    Taken {
        /// Its size, which the disk cap counts.
        bytes_len: u64,
    },
    /// The tab was private before the capture, or went private before the pixels were kept.
    /// Nothing was written. Read back as 410 `withheld`.
    Withheld,
    /// No picture: too many newer jobs waited on its tab, the tab closed, or the capture failed
    /// or timed out on every attempt. Nothing was written, and the operation keeps no screenshot.
    Dropped,
}

/// A job's reply. Exactly one [`Shot`] reaches the writer per answer: the one sent, or `Dropped`
/// when the answer is dropped unsent.
pub struct ShotAnswer {
    op: OperationId,
    /// `None` once answered.
    sink: Option<TraceSink>,
}

impl ShotAnswer {
    pub(crate) fn new(op: OperationId, sink: TraceSink) -> Self {
        Self {
            op,
            sink: Some(sink),
        }
    }

    /// Reports what became of the job.
    pub fn send(mut self, shot: Shot) {
        self.answer(shot);
    }

    fn answer(&mut self, shot: Shot) {
        if let Some(sink) = self.sink.take() {
            sink.screenshot_answered(self.op, shot);
        }
    }
}

impl Drop for ShotAnswer {
    fn drop(&mut self) {
        self.answer(Shot::Dropped);
    }
}

impl fmt::Debug for ShotAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShotAnswer")
            .field("op", &self.op)
            .field("answered", &self.sink.is_none())
            .finish()
    }
}
