//! A run's downloads, kept with its record when it ends (zuzu gap G2).
//!
//! ```text
//!  run ends ─ ops drained ─ RunState::downloads(): what its `download` events handed the script
//!     ─ each completed one copied to outputs/<run>/downloads/<index>   (RunOutputs, beside trace.db)
//!     ─ the list written to outputs/<run>/downloads.json
//!     ─ then the run's end record and its reply
//! ```
//!
//! `download.path()` answers the browser's own file, which stays where the browser saved it. The
//! copy here is what the host reads at `GET /v1/runs/{id}/downloads/{index}`, so the bytes it gets
//! are the ones the run downloaded, and they go when the retention sweep removes the run. The run
//! service calls this before it answers, so a host that reads a download right after the reply
//! finds it.

use neo_playwright::{DownloadFacts, DownloadState, RunState};
use neo_trace::{DownloadStatus, RunDownload, RunOutputs};

use crate::dispatch::{self, RunCtx, TEXT_CHARS};

/// Keeps `run`'s downloads in `outputs`. A copy or the list that cannot be written is logged and
/// recorded as the download's failure; the run's reply does not depend on it.
pub(crate) async fn keep_downloads(run: &RunCtx, outputs: &RunOutputs) {
    let handed = run.slot::<RunState>().downloads();
    if handed.is_empty() {
        return;
    }
    let mut kept = Vec::with_capacity(handed.len());
    for (index, facts) in (0_u32..).zip(handed) {
        kept.push(keep_one(run, outputs, index, facts).await);
    }
    if let Err(error) = outputs.record_downloads(run.id(), &kept).await {
        tracing::warn!(%error, run = %run.id(), "could not record the run's downloads; its record lists none");
    }
}

async fn keep_one(
    run: &RunCtx,
    outputs: &RunOutputs,
    index: u32,
    facts: DownloadFacts,
) -> RunDownload {
    let DownloadFacts {
        url,
        suggested_filename,
        state,
        ..
    } = facts;
    let mut download = RunDownload {
        index,
        // A `data:` URL can be the whole file; the record keeps a bounded one.
        url: dispatch::clip(url, TEXT_CHARS),
        suggested_filename,
        path: None,
        size: None,
        state: DownloadStatus::InProgress,
        failure: None,
    };
    match state {
        DownloadState::InProgress => {}
        DownloadState::Failed(failure) => {
            download.state = DownloadStatus::Failed;
            download.failure = Some(failure);
        }
        DownloadState::Completed(path) => {
            download.path = Some(path.to_string_lossy().into_owned());
            match outputs.keep_download(run.id(), index, &path).await {
                Ok(size) => {
                    download.state = DownloadStatus::Completed;
                    download.size = Some(size);
                }
                Err(error) => {
                    tracing::warn!(%error, run = %run.id(), index, "could not keep a copy of a finished download");
                    download.state = DownloadStatus::Failed;
                    download.failure = Some(format!(
                        "the browser saved the file, but Neo could not keep a copy: {error}"
                    ));
                }
            }
        }
    }
    download
}
