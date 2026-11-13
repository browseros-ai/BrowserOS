//! The tabs that took a host's private fill, kept by target id in a file beside the trace, so a
//! Neo restart while the browser keeps those tabs still takes no picture of them (protected-fill
//! ruling R45; design section 10.3).
//!
//! ```text
//!  a fill, before its first character ─ remember(tab, target) ─ one line, synced; a write that
//!                                        fails refuses the fill, nothing typed
//!  Neo starts, the browser connects ─ live(the browser's own tab list, unfiltered) ─ the tabs
//!                                     whose targets it names are marked in the entry's own
//!                                     set, the rest dropped from the file
//!  a tab closes ─ closed(tab) ─ its target dropped from the file
//! ```
//!
//! Why target ids: a tab id names another tab once the browser restarts, a target id never does.
//! Why the browser's own unfiltered list: core's page listing leaves out extension and devtools
//! pages, and a filled tab can sit on one when Neo starts. Not the trace's retention: a tab can
//! outlive it. Not the trace store: it commits in batches, and this fact must be on disk before
//! the first character.
//!
//! Limits, named: a tab whose target changes while it lives (if the browser ever does that) is not
//! found again after a restart; and a fill refused after its line was written (a lock released,
//! the entry refusing a box) leaves its tab in the file, so after a restart that tab is withheld
//! though nothing was typed, the safe direction.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::{TabId, TargetId};
use tokio::{fs as async_fs, task};

/// The record's file name, in the trace's folder.
pub(crate) const FILLED_TABS_FILE: &str = "filled-tabs";

/// A tab the browser has now: its target and tab ids, from the browser's own tab list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LiveTab {
    pub(crate) target: TargetId,
    pub(crate) tab: TabId,
}

/// The record. One per process, shared by every fill and by the start that reads it back.
pub(crate) struct FilledTabs {
    path: PathBuf,
    /// The map and the appender under one std lock, taken only inside blocking tasks or for a
    /// moment, never across an await: a `remember` holds it through its write and sync, so a
    /// second fill of a tab answers only once the first one's line is on disk, and a rewrite
    /// reads the map at the moment it writes, so no line appended meanwhile is lost.
    state: Arc<Mutex<State>>,
    /// The file existed and could not be read at this start. Which tabs it named is unknown,
    /// so every tab the browser has at this start is treated as having taken a fill, and the
    /// file is never rewritten: it stays for a person to look at.
    unreadable: bool,
}

/// What the lock holds.
#[derive(Default)]
struct State {
    /// Each remembered target, with its tab while this process knows it.
    targets: HashMap<TargetId, Option<TabId>>,
    /// The appender, opened on first use.
    file: Option<File>,
}

impl FilledTabs {
    /// The record at `folder/filled-tabs`, read back. A file that exists and cannot be read
    /// fails closed ([`FilledTabs::live`]) and is left for a person.
    pub(crate) async fn open(folder: &Path) -> Arc<Self> {
        let path = folder.join(FILLED_TABS_FILE);
        let (text, unreadable) = match async_fs::read(&path).await {
            Ok(bytes) => (String::from_utf8_lossy(&bytes).into_owned(), false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (String::new(), false),
            Err(error) => {
                tracing::error!(%error, file = %path.display(), "cannot read the tabs that took a private fill; every tab open at this start is treated as having taken one");
                (String::new(), true)
            }
        };
        // One target a line; a torn line names a target no tab has, and the next start drops it.
        let targets = text
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .map(|target| (TargetId::from(target.to_owned()), None))
            .collect();
        Arc::new(Self {
            path,
            state: Arc::new(Mutex::new(State {
                targets,
                file: None,
            })),
            unreadable,
        })
    }

    /// Records that `tab`, the browser's `target`, takes a fill. Synced to disk before it
    /// returns; call it before the fill's first character. `Err` means nothing may be typed. A
    /// second call for the same target waits for the first and answers only once its line is
    /// on disk.
    pub(crate) async fn remember(&self, tab: TabId, target: TargetId) -> io::Result<()> {
        let state = Arc::clone(&self.state);
        let path = self.path.clone();
        task::spawn_blocking(move || {
            let mut state = lock(&state);
            if let Some(known) = state.targets.get_mut(&target) {
                *known = Some(tab);
                return Ok(());
            }
            append(&mut state.file, &path, target.0.as_str())?;
            state.targets.insert(target, Some(tab));
            Ok(())
        })
        .await
        .map_err(io::Error::other)?
    }

    /// The tabs among `live` (the browser's own tab list, unfiltered) whose targets the record
    /// names, for the entry's own set; every target the browser no longer has is dropped from
    /// the file. A record that could not be read names every live tab.
    pub(crate) async fn live(&self, live: &[LiveTab]) -> Vec<TabId> {
        if self.unreadable {
            return live.iter().map(|tab| tab.tab).collect();
        }
        let (marked, dropped) = {
            let mut state = lock(&self.state);
            let before = state.targets.len();
            state
                .targets
                .retain(|target, _| live.iter().any(|tab| tab.target == *target));
            let mut marked = Vec::new();
            for tab in live {
                if let Some(known) = state.targets.get_mut(&tab.target) {
                    *known = Some(tab.tab);
                    marked.push(tab.tab);
                }
            }
            (marked, state.targets.len() < before)
        };
        if dropped {
            self.rewrite().await;
        }
        marked
    }

    /// Drops `tab`'s target once the tab closed: no picture of it can be taken any more.
    pub(crate) async fn closed(&self, tab: TabId) {
        if self.unreadable {
            return;
        }
        let dropped = {
            let mut state = lock(&self.state);
            let before = state.targets.len();
            state.targets.retain(|_, known| *known != Some(tab));
            state.targets.len() < before
        };
        if dropped {
            self.rewrite().await;
        }
    }

    /// Whether the record names no tab, so a start has nothing to mark. A record that could not
    /// be read is never empty: it may have named any tab.
    pub(crate) fn is_empty(&self) -> bool {
        !self.unreadable && lock(&self.state).targets.is_empty()
    }

    /// Rewrites the file with the targets remembered at the moment it writes, under the lock. A
    /// failure leaves the old file, which only names more tabs than needed. Never while the
    /// record was unreadable at start.
    async fn rewrite(&self) {
        let state = Arc::clone(&self.state);
        let path = self.path.clone();
        let written = task::spawn_blocking(move || {
            let mut state = lock(&state);
            let mut lines = String::new();
            for target in state.targets.keys() {
                lines.push_str(target.0.as_str());
                lines.push('\n');
            }
            let temporary = path.with_extension("tmp");
            let mut out = File::create(&temporary)?;
            out.write_all(lines.as_bytes())?;
            out.sync_all()?;
            drop(out);
            fs::rename(&temporary, &path)?;
            // The rename replaced the file the appender held: the next append opens afresh.
            state.file = None;
            sync_folder(&path)
        })
        .await
        .map_err(io::Error::other)
        .and_then(|done| done);
        if let Err(error) = written {
            tracing::warn!(%error, "cannot rewrite the tabs that took a private fill; the old record stays");
        }
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Appends one line and syncs it, opening the file (and syncing its folder) on first use. Each
/// line starts on a line of its own, so one that an earlier write left half done costs only
/// itself. Blocking: call it on a blocking thread.
fn append(file: &mut Option<File>, path: &Path, line: &str) -> io::Result<()> {
    if file.is_none() {
        let created = !path.exists();
        *file = Some(OpenOptions::new().create(true).append(true).open(path)?);
        // A new file's name lives in its folder: synced too, or a power cut can lose it.
        if created {
            sync_folder(path)?;
        }
    }
    let Some(open) = file.as_mut() else {
        return Err(io::Error::other("the record file did not open"));
    };
    let written = open
        .write_all(format!("\n{line}\n").as_bytes())
        .and_then(|()| open.sync_data());
    if written.is_err() {
        // A half-written line may be in the file; it names no tab. The next append opens a
        // fresh handle and starts on a line of its own.
        *file = None;
    }
    written
}

/// Syncs the folder that holds `path`, so a file created or renamed there survives a power cut.
fn sync_folder(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(folder) => File::open(folder)?.sync_all(),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;

    use browseros_core::{TabId, TargetId};

    use super::{FILLED_TABS_FILE, FilledTabs, LiveTab};

    fn target(id: &str) -> TargetId {
        TargetId::from(id.to_owned())
    }

    fn live(id: &str, tab: i64) -> LiveTab {
        LiveTab {
            target: target(id),
            tab: TabId(tab),
        }
    }

    #[tokio::test]
    async fn a_restart_knows_the_tabs_the_browser_kept_and_forgets_the_rest()
    -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        {
            let record = FilledTabs::open(folder.path()).await;
            record.remember(TabId(3), target("A3")).await?;
            record.remember(TabId(4), target("B4")).await?;
            record.remember(TabId(3), target("A3")).await?;
        }
        let text = fs::read_to_string(folder.path().join(FILLED_TABS_FILE))?;
        assert_eq!(
            text, "\nA3\n\nB4\n",
            "one synced line per target, never a value"
        );
        // After the restart the browser still has A3, now tab 7, and not B4; a torn line names
        // nothing.
        fs::write(folder.path().join(FILLED_TABS_FILE), format!("{text}C9"))?;
        let record = FilledTabs::open(folder.path()).await;
        let tabs = record.live(&[live("A3", 7), live("D8", 8)]).await;
        assert_eq!(tabs, vec![TabId(7)]);
        let text = fs::read_to_string(folder.path().join(FILLED_TABS_FILE))?;
        assert_eq!(
            text, "A3\n",
            "a target the browser no longer has is dropped"
        );
        record.closed(TabId(7)).await;
        let text = fs::read_to_string(folder.path().join(FILLED_TABS_FILE))?;
        assert_eq!(text, "", "a closed tab's target is dropped");
        Ok(())
    }

    #[tokio::test]
    async fn a_line_appended_after_a_torn_one_is_known_after_a_restart()
    -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        // A power cut left a half-written last line, with no newline.
        fs::write(folder.path().join(FILLED_TABS_FILE), "\nA3\nB")?;
        let record = FilledTabs::open(folder.path()).await;
        record.remember(TabId(5), target("C5")).await?;
        drop(record);
        let record = FilledTabs::open(folder.path()).await;
        let tabs = record.live(&[live("A3", 7), live("C5", 8)]).await;
        assert_eq!(
            tabs,
            vec![TabId(7), TabId(8)],
            "the new line starts on its own, not glued to the torn one"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_record_that_cannot_be_read_marks_every_tab_and_is_never_rewritten()
    -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        // A directory where the file should be: it exists and cannot be read.
        fs::create_dir(folder.path().join(FILLED_TABS_FILE))?;
        let record = FilledTabs::open(folder.path()).await;
        let tabs = record.live(&[live("A7", 7), live("B8", 8)]).await;
        assert_eq!(tabs, vec![TabId(7), TabId(8)], "fails closed");
        record.closed(TabId(7)).await;
        assert!(
            folder.path().join(FILLED_TABS_FILE).is_dir(),
            "left as it was"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_record_that_cannot_be_written_refuses_and_remembers_nothing()
    -> Result<(), Box<dyn Error>> {
        let folder = tempfile::tempdir()?;
        // A directory where the file should be: it cannot be opened for appending.
        fs::create_dir(folder.path().join(FILLED_TABS_FILE))?;
        let record = FilledTabs::open(folder.path()).await;
        assert!(
            record.remember(TabId(3), target("A3")).await.is_err(),
            "the fill that asked is refused, nothing typed"
        );
        assert!(
            record.remember(TabId(3), target("A3")).await.is_err(),
            "a second fill of the tab is refused too: nothing was written for the first"
        );
        Ok(())
    }
}
