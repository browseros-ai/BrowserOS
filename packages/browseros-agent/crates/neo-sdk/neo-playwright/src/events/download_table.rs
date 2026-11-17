//! Every download the browser reported since the `Playwright` started, and how each ended.
//!
//! ```text
//!  Browser.downloadWillBegin ─▶ begin: InProgress ──▶ the `download` event's record
//!  Browser.downloadProgress  ─▶ progress: completed { filePath } ─▶ Completed(path)
//!                                         canceled ──────────────▶ Failed("canceled")
//!  download.path / saveAs ─── wait on the entry until it leaves InProgress
//!  the event reader ───────── the run that is handed a download keeps its entry (RunState)
//! ```
//!
//! Neo leaves the browser's own download behavior in place (`target_watcher.rs` sends
//! `behavior: 'default'`), so a file lands where the browser saves every download, with the name
//! it chose, and `filePath` on the completed progress event is the only place that name is
//! reported. The watcher is the only writer; ops and the host read entries through an `Arc`, so
//! an entry the table no longer lists still answers the runs that hold it.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::watch;

use crate::{DownloadFacts, DownloadState};

/// How many downloads the table lists. Past it, the oldest finished one goes; a download in
/// flight is never dropped, so it keeps getting its progress, and the table outgrows the bound
/// only while more than this many are in flight at once.
const LISTED: usize = 1024;

/// What `failure()` reads for a download the browser canceled or interrupted, as upstream's.
const CANCELED: &str = "canceled";

/// The downloads the watcher saw, by the browser's guid.
#[derive(Debug, Default)]
pub(crate) struct DownloadTable {
    /// A std mutex, held for a map update and never across an await.
    inner: Mutex<Listed>,
}

#[derive(Debug, Default)]
struct Listed {
    by_guid: HashMap<String, Arc<TrackedDownload>>,
    /// Guids, oldest first, for the bound.
    order: VecDeque<String>,
}

/// One download: what its event reported, and its state, which waiters watch.
#[derive(Debug)]
pub(crate) struct TrackedDownload {
    guid: String,
    url: String,
    suggested_filename: String,
    state: watch::Sender<DownloadState>,
}

impl DownloadTable {
    /// A download began. Listing a guid again keeps the first entry.
    pub(crate) fn begin(&self, guid: String, url: String, suggested_filename: String) {
        let mut listed = self.lock();
        if listed.by_guid.contains_key(&guid) {
            return;
        }
        let download = Arc::new(TrackedDownload {
            guid: guid.clone(),
            url,
            suggested_filename,
            state: watch::Sender::new(DownloadState::InProgress),
        });
        listed.by_guid.insert(guid.clone(), download);
        listed.order.push_back(guid);
        while listed.order.len() > LISTED && listed.drop_oldest_finished() {}
    }

    /// `Browser.downloadProgress` for `guid`. Only the first final state counts; a guid the table
    /// does not list (it began before the watcher did) is ignored.
    pub(crate) fn progress(&self, guid: &str, state: &str, file_path: Option<String>) {
        let next = match state {
            "completed" => match file_path.filter(|path| !path.is_empty()) {
                Some(path) => DownloadState::Completed(PathBuf::from(path)),
                None => DownloadState::Failed(
                    "the browser finished the download without saying where it saved the file"
                        .to_owned(),
                ),
            },
            "canceled" => DownloadState::Failed(CANCELED.to_owned()),
            _ => return,
        };
        let Some(download) = self.get(guid) else {
            return;
        };
        download.state.send_if_modified(|current| {
            if current.is_in_progress() {
                *current = next;
                true
            } else {
                false
            }
        });
    }

    /// The download `guid`, while the table lists it.
    pub(crate) fn get(&self, guid: &str) -> Option<Arc<TrackedDownload>> {
        self.lock().by_guid.get(guid).cloned()
    }

    fn lock(&self) -> MutexGuard<'_, Listed> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Listed {
    /// Unlists the oldest finished download; false when every one is in flight.
    fn drop_oldest_finished(&mut self) -> bool {
        let finished = self.order.iter().position(|guid| {
            self.by_guid
                .get(guid)
                .is_none_or(|download| !download.state.borrow().is_in_progress())
        });
        let Some(guid) = finished.and_then(|place| self.order.remove(place)) else {
            return false;
        };
        self.by_guid.remove(&guid);
        true
    }
}

impl TrackedDownload {
    /// The browser's guid, which the script's `Download` names it by.
    pub(crate) fn guid(&self) -> &str {
        &self.guid
    }

    /// Its facts as of now.
    pub(crate) fn facts(&self) -> DownloadFacts {
        DownloadFacts {
            url: self.url.clone(),
            suggested_filename: self.suggested_filename.clone(),
            state: self.state.borrow().clone(),
        }
    }

    /// Resolves once the download has finished or failed. It has no timeout of its own: the op
    /// that waits races it with its own, and the host drops the op when the run stops.
    pub(crate) async fn finished(&self) -> DownloadState {
        let mut state = self.state.subscribe();
        // The sender lives in `self`, so the wait cannot see it dropped.
        let finished = state
            .wait_for(|state| !state.is_in_progress())
            .await
            .map(|state| state.clone());
        finished.unwrap_or_else(|_closed| self.state.borrow().clone())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use tokio::time;

    use super::{DownloadTable, LISTED};
    use crate::DownloadState;

    fn begin(table: &DownloadTable, guid: &str) {
        table.begin(
            guid.to_owned(),
            format!("https://example.com/{guid}"),
            format!("{guid}.txt"),
        );
    }

    #[tokio::test]
    async fn tests_that_a_wait_resolves_with_the_saved_path() -> Result<(), String> {
        let table = DownloadTable::default();
        begin(&table, "a");
        let download = table.get("a").ok_or("listed")?;
        let waiting = tokio::spawn(async move { download.finished().await });
        table.progress("a", "inProgress", None);
        table.progress("a", "completed", Some("/data/downloads/a.txt".to_owned()));
        let state = time::timeout(Duration::from_secs(1), waiting)
            .await
            .map_err(|_elapsed| "the wait resolves")?
            .map_err(|error| error.to_string())?;
        assert_eq!(
            state,
            DownloadState::Completed(PathBuf::from("/data/downloads/a.txt"))
        );
        Ok(())
    }

    #[test]
    fn tests_that_the_first_final_state_counts() -> Result<(), String> {
        let table = DownloadTable::default();
        begin(&table, "a");
        table.progress("a", "canceled", None);
        table.progress("a", "completed", Some("/data/downloads/a.txt".to_owned()));
        let facts = table.get("a").ok_or("listed")?.facts();
        assert_eq!(facts.state, DownloadState::Failed("canceled".to_owned()));
        assert_eq!(facts.suggested_filename, "a.txt");
        begin(&table, "b");
        table.progress("b", "completed", None);
        let unsaved = table.get("b").ok_or("listed")?.facts();
        assert!(
            matches!(unsaved.state, DownloadState::Failed(_)),
            "a completed download with no path cannot be read: {unsaved:?}"
        );
        Ok(())
    }

    #[test]
    fn tests_that_the_bound_unlists_only_finished_downloads() {
        let table = DownloadTable::default();
        begin(&table, "running");
        begin(&table, "done");
        table.progress("done", "completed", Some("/d/done".to_owned()));
        for index in 0..LISTED {
            begin(&table, &format!("x{index}"));
        }
        assert!(table.get("running").is_some(), "a download in flight stays");
        assert!(table.get("done").is_none(), "the finished one went");
        assert!(
            table.get(&format!("x{}", LISTED - 1)).is_some(),
            "past the bound with every download in flight, the table grows"
        );
    }
}
