//! The downloads one run was handed, in the order it first saw each: the run's download index.
//!
//! The event ops add a download when a `download` event reaches the run, through the pump (after
//! it sorts a batch by seq) or a `waitForEvent`; the download ops find it here first; the host
//! reads the list once the run's ops have drained, to keep the finished files with the run's
//! record. Holding the table's entries, not guids, keeps each download's state reachable after
//! the table stops listing it.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::DownloadFacts;
use crate::events::download_table::TrackedDownload;

/// Behind `RunState`. A std mutex, held for a list update and never across an await.
#[derive(Debug, Default)]
pub(crate) struct RunDownloads {
    handed: Mutex<Vec<Arc<TrackedDownload>>>,
}

impl RunDownloads {
    /// The run was handed `download`. A download it was handed before keeps its index.
    pub(crate) fn handed(&self, download: Arc<TrackedDownload>) {
        let mut handed = self.lock();
        if !handed.iter().any(|known| known.guid() == download.guid()) {
            handed.push(download);
        }
    }

    /// The download `guid`, if the run was handed it.
    pub(crate) fn find(&self, guid: &str) -> Option<Arc<TrackedDownload>> {
        self.lock()
            .iter()
            .find(|download| download.guid() == guid)
            .cloned()
    }

    /// Each download's facts as of now, by index.
    pub(crate) fn facts(&self) -> Vec<DownloadFacts> {
        self.lock()
            .iter()
            .map(|download| download.facts())
            .collect()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Arc<TrackedDownload>>> {
        self.handed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
