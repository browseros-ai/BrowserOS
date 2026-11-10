//! Neo: The `run` tool's artifact directory, `<state_dir>/artifacts/`: one folder shared by every
//! run, so a later run can upload (`setInputFiles`) what an earlier one wrote. neo-runtime
//! writes into it for `path` options (relative, or absolute under `AbsolutePaths::Redirect`);
//! this module names it, bounds it on disk, and tells the agent where each file landed.
//!
//! ```text
//!  page.screenshot({path:'/tmp/x.png'})  ─▶ neo-runtime writes <artifacts>/x.png
//!  RunOutcome.artifacts [{path:'x.png', bytes, requested:'/tmp/x.png'}]
//!    └─ locate(): + file:'<artifacts>/x.png'  (absolute, readable by a local agent)
//!  startup, then at most once a minute after a run that wrote files: prune()
//! ```

use std::cmp::Reverse;
use std::fs;
use std::io;
use std::path::{self, Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;

/// Neo: all runs of this server share this folder under its state directory.
pub(super) const DIR_NAME: &str = "artifacts";
/// Neo: what [`prune`] keeps.
pub(super) const BOUNDS: Bounds = Bounds {
    // A task's screenshots stay through the working day that made them; old sessions' files do
    // not pile up.
    max_age: Duration::from_hours(24),
    // A DPR-2 full-page PNG is about 5-10 MB, so this keeps roughly 50-100 of them.
    max_bytes: 512 * 1024 * 1024,
    // A run's own output is still there when the agent reads the reply that names it.
    min_age: Duration::from_mins(10),
};
/// Neo: folder depth the pruner walks; deeper files are left alone rather than recursing forever.
const MAX_DEPTH: usize = 16;

/// How long and how much [`prune`] keeps.
#[derive(Debug, Clone, Copy)]
pub(super) struct Bounds {
    /// A file untouched this long is removed.
    pub(super) max_age: Duration,
    /// The directory's total size, oldest files removed first.
    pub(super) max_bytes: u64,
    /// A file this young survives the size bound.
    pub(super) min_age: Duration,
}

/// `<state_dir>/artifacts`, absolute, as neo-runtime compares a script's absolute paths with it
/// and as the reply reports files.
#[must_use]
pub(super) fn dir(state_dir: &Path) -> PathBuf {
    let dir = state_dir.join(DIR_NAME);
    path::absolute(&dir).unwrap_or(dir)
}

/// Adds `file`, the absolute path, to each entry of the reply's `artifacts`, and logs one line
/// for every file written somewhere other than the absolute path the script asked for.
pub(super) fn locate(report: &mut Value, dir: &Path) {
    let Some(entries) = report.get_mut("artifacts").and_then(Value::as_array_mut) else {
        return;
    };
    for entry in entries {
        let Some(relative) = entry.get("path").and_then(Value::as_str) else {
            continue;
        };
        let file = dir.join(relative);
        if let Some(requested) = entry.get("requested").and_then(Value::as_str) {
            tracing::info!(
                requested,
                written = %file.display(),
                "run path outside the artifact directory was written inside it"
            );
        }
        entry["file"] = Value::String(file.display().to_string());
    }
}

/// What one [`prune`] removed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Pruned {
    pub(super) files: usize,
    pub(super) bytes: u64,
}

struct Entry {
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}

/// Removes files older than `bounds.max_age`, then the oldest until the rest fit
/// `bounds.max_bytes` (never one younger than `bounds.min_age`), then the folders left empty. Symbolic links are neither
/// followed nor removed. A missing directory is nothing to prune. Blocking: run it off the async
/// workers.
pub(super) fn prune(dir: &Path, now: SystemTime, bounds: Bounds) -> io::Result<Pruned> {
    let mut files = Vec::new();
    let mut folders = Vec::new();
    match collect(dir, 0, &mut files, &mut folders) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Pruned::default()),
        result => result?,
    }
    let age = |entry: &Entry| now.duration_since(entry.modified).unwrap_or_default();
    files.sort_by_key(|entry| entry.modified);
    let mut total: u64 = files.iter().map(|entry| entry.bytes).sum();
    let mut pruned = Pruned::default();
    for entry in &files {
        let stale = age(entry) > bounds.max_age;
        let over = total > bounds.max_bytes && age(entry) >= bounds.min_age;
        if !(stale || over) {
            continue;
        }
        match fs::remove_file(&entry.path) {
            Ok(()) => {
                total = total.saturating_sub(entry.bytes);
                pruned.files += 1;
                pruned.bytes += entry.bytes;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                total = total.saturating_sub(entry.bytes);
            }
            Err(error) => tracing::warn!(%error, file = %entry.path.display(), "artifact prune"),
        }
    }
    // Deepest first, so a parent emptied by its children goes too. `remove_dir` refuses a
    // folder that still holds anything, which is the check.
    folders.sort_by_key(|folder| Reverse(folder.components().count()));
    for folder in folders {
        let _ = fs::remove_dir(folder);
    }
    Ok(pruned)
}

fn collect(
    dir: &Path,
    depth: usize,
    files: &mut Vec<Entry>,
    folders: &mut Vec<PathBuf>,
) -> io::Result<()> {
    for item in fs::read_dir(dir)? {
        let item = item?;
        let path = item.path();
        let metadata = fs::symlink_metadata(&path)?;
        let kind = metadata.file_type();
        if kind.is_file() {
            files.push(Entry {
                path,
                bytes: metadata.len(),
                modified: metadata.modified()?,
            });
        } else if kind.is_dir() && depth < MAX_DEPTH {
            collect(&path, depth + 1, files, folders)?;
            folders.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn file(dir: &Path, name: &str, bytes: usize, age: Duration, now: SystemTime) -> PathBuf {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![0_u8; bytes]).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(now - age)
            .unwrap();
        path
    }

    #[test]
    fn tests_that_prune_removes_stale_then_oldest_over_the_size_bound() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(DIR_NAME);
        let now = SystemTime::now();
        let hour = Duration::from_hours(1);
        let bounds = Bounds {
            max_bytes: 100,
            ..BOUNDS
        };
        let stale = file(&dir, "old/stale.png", 10, BOUNDS.max_age + hour, now);
        let oldest = file(&dir, "a.png", 50, 3 * hour, now);
        let middle = file(&dir, "b.png", 50, 2 * hour, now);
        let young = file(&dir, "c.png", 50, Duration::from_mins(1), now);
        let pruned = prune(&dir, now, bounds).unwrap();
        assert_eq!(
            pruned,
            Pruned {
                files: 2,
                bytes: 60
            }
        );
        assert!(
            !stale.exists() && !dir.join("old").exists(),
            "stale file and empty folder"
        );
        assert!(!oldest.exists(), "the oldest goes to fit the bound");
        assert!(middle.exists(), "then the rest fit");
        assert!(young.exists(), "a young file survives the size bound");
        assert!(dir.exists(), "the directory itself stays");
        assert_eq!(
            prune(&root.path().join("missing"), now, BOUNDS).unwrap(),
            Pruned::default()
        );
    }

    #[cfg(unix)]
    #[test]
    fn tests_that_prune_neither_follows_nor_removes_symbolic_links() {
        use std::os::unix::fs as unix_fs;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(DIR_NAME);
        let now = SystemTime::now();
        let outside = file(root.path(), "outside/keep.txt", 10, BOUNDS.max_age * 2, now);
        fs::create_dir_all(&dir).unwrap();
        unix_fs::symlink(root.path().join("outside"), dir.join("link")).unwrap();
        prune(&dir, now, BOUNDS).unwrap();
        assert!(outside.exists(), "nothing outside the directory is touched");
        assert!(
            fs::symlink_metadata(dir.join("link")).is_ok(),
            "the link stays"
        );
    }

    #[test]
    fn tests_that_locate_adds_absolute_files() {
        let dir = Path::new("/state/artifacts");
        let mut report = json!({"status":"ok","artifacts":[
            {"path":"shots/a.png","bytes":3},
            {"path":"x.png","bytes":4,"requested":"/tmp/x.png"}
        ]});
        locate(&mut report, dir);
        assert_eq!(
            report["artifacts"][0]["file"],
            "/state/artifacts/shots/a.png"
        );
        assert_eq!(report["artifacts"][1]["file"], "/state/artifacts/x.png");
        assert_eq!(report["artifacts"][1]["requested"], "/tmp/x.png");
        let mut none = json!({"status":"ok"});
        locate(&mut none, dir);
        assert_eq!(none, json!({"status":"ok"}));
    }
}
