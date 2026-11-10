//! Where screenshot files live: one directory per run and one JPEG per operation,
//! `screenshots/<run>/<seq>.jpg` beside `trace.db`.
//!
//! This is the one place the layout is written down. The writer puts each job's path in the
//! job, the camera writes the file there, the store reads it back, and the retention sweep
//! deletes whole run directories, so a run's files go with its rows. Every path is built from a
//! typed id, never from text read back from the file, so no stored value can point a delete
//! outside the directory.

use crate::{OperationId, RunId};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The screenshot directory of one trace file.
#[derive(Clone, Debug)]
pub(crate) struct ShotFiles {
    root: Arc<Path>,
}

impl ShotFiles {
    /// The `screenshots` directory beside the trace file at `db`.
    pub(crate) fn beside(db: &Path) -> Self {
        let dir = db.parent().unwrap_or_else(|| Path::new("."));
        Self {
            root: Arc::from(dir.join("screenshots")),
        }
    }

    /// Where `op`'s screenshot is written and read.
    pub(crate) fn path(&self, op: OperationId) -> PathBuf {
        self.run_dir(op.run).join(format!("{}.jpg", op.seq))
    }

    /// The directory holding every screenshot of `run`.
    pub(crate) fn run_dir(&self, run: RunId) -> PathBuf {
        self.root.join(run.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::ShotFiles;
    use crate::{OperationId, RunId};
    use std::path::{Path, PathBuf};

    #[test]
    fn a_screenshot_lives_in_its_runs_directory_beside_the_trace_file() {
        let files = ShotFiles::beside(Path::new("/data/claw-sdk/trace.db"));
        let run = RunId::new();
        assert_eq!(
            files.path(OperationId::new(run, 7)),
            PathBuf::from(format!("/data/claw-sdk/screenshots/{run}/7.jpg"))
        );
        assert_eq!(
            files.run_dir(run),
            PathBuf::from(format!("/data/claw-sdk/screenshots/{run}"))
        );
    }
}
