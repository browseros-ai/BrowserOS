//! Neo's own secret files, which `setInputFiles` never hands a page. The host names them
//! (`PlaywrightHooks::secret_paths`, read once per op), and every path the op would hand
//! Chromium is checked against them before anything reads a file's contents: once when the
//! client stats it, and again right before the send.
//!
//! What is compared is what is on disk, never the strings, so no other name of a secret gets
//! through: a symlink, `..`, a hard link, another case on a case-insensitive disk, a bind mount.
//!
//! Identity cannot speak for `/dev` and `/proc`. Neo resolves a path in its own process, but
//! Chromium opens the string in its process, where `/dev/fd/N`, `/dev/stdin` and
//! `/proc/self/…` name Chromium's own files (its descriptor N can be its Cookies database), and
//! `/proc/<pid>/…` reaches any process's. Those trees hold device and per-process files, never
//! an ordinary upload, so rule ⓪ refuses any path that enters them, and rule ④ any directory
//! upload that holds one, whatever the host named.
//!
//! ```text
//!  entries     each named path that exists now: realpath, stat ─▶ (dev, inode), and the
//!              (dev, inode) of every directory above it; an absent one is skipped
//!  device      `/dev` and `/proc`, where there is one, resolved as an entry is
//!  candidate   the path Chromium would open
//!    ⓪ walked a component at a time as the kernel resolves it (read_link at each symlink, a
//!      relative target from the link's directory), it enters a device tree: by its own
//!      components or by a symlink's target on the way; checked with no entry named too, and
//!      before the caller's own stat of the path, whose error it outranks
//!    then its realpath (its symlinks and `..`), stat'ed, for ① to ④
//!    ① its (dev, inode) is an entry's
//!    ② it, or a directory above its real path, is an entry directory: inside one
//!    ③ a regular file with more than one link whose (dev, inode) is a file's inside an entry
//!      directory (those directories walked once per round of checks, on first need)
//!    ④ a directory (a `webkitdirectory` upload): a directory above an entry's or a device
//!      tree's real path, or an entry, a device tree or a file ③ denies anywhere in its tree,
//!      walked as Chromium enumerates it, symlinks inside skipped as Chromium skips them;
//!      walked with no entry named too
//! ```
//!
//! A check that cannot finish refuses, so Neo uploads only what it could clear: an entry it
//! cannot resolve for a reason other than its absence, a walk that meets a directory it cannot
//! read, or the op's timeout during classify's check. In `ready` the op's timeout ends the op
//! with Playwright's timeout error, as it ends every wait there. A refusal never says which
//! entry matched, nor any path.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, Metadata};
use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use tokio::task;

use crate::op_scope::OpScope;
use crate::{ErrorCode, PlaywrightError};

/// What a host that named no secret files answers an upload from a path. Payloads read no file
/// on Neo's disk, so they still upload, and the message says so.
const NO_SECRETS_NAMED: &str = "Neo uploads no file from its disk until its host names Neo's \
                                own secret files; pass { name, mimeType, buffer } instead";

/// What a script that named one of Neo's secrets gets: never which one, nor where it is.
const SECRET_REFUSED: &str = "Neo does not upload its own secret files";

/// The trees of device and per-process files (rule ⓪). A missing one, as `/proc` on macOS, has
/// nothing in it to reach.
const DEVICE_TREES: [&str; 2] = ["/dev", "/proc"];

/// The most symlinks rule ⓪ follows on one path: Linux's limit (`MAXSYMLINKS`), above macOS's
/// 32. Past it the kernel fails the open too, so nothing past it can be reached.
const MAX_SYMLINK_HOPS: usize = 40;

/// The host's answer to `secret_paths` for one op.
pub(super) struct Secrets {
    /// `None`: the host has not named its secrets, so no path may upload.
    named: Option<Vec<PathBuf>>,
}

impl Secrets {
    /// What the host answered `secret_paths`, asked once for the whole op.
    pub(super) fn named_by(named: Option<Vec<PathBuf>>) -> Self {
        Self { named }
    }

    /// The named entries as they stand on disk now, for one round of checks; `neo / unsupported`
    /// when the host named none, before anything is stat'ed, so such a host's disk is never read.
    pub(super) async fn on_disk(&self, scope: &OpScope<'_>) -> Result<OnDisk, PlaywrightError> {
        let named = self
            .named
            .clone()
            .ok_or_else(|| scope.fail(ErrorCode::Unsupported, NO_SECRETS_NAMED))?;
        let entries =
            blocking(move |_stop| Entries::resolve(&DEVICE_TREES.map(Path::new), &named)).await;
        entries
            .map(|entries| OnDisk(Arc::new(entries)))
            .ok_or_else(|| refused(scope))
    }
}

/// One round's view of the entries: classify's, or one pass of `ready`'s. Every path the round
/// checks is compared with the same resolution, and the entry directories are walked for hard
/// links at most once in it.
pub(super) struct OnDisk(Arc<Entries>);

impl OnDisk {
    /// Rule ⓪ alone, off the runtime's threads: whether resolving `path`, an absolute path,
    /// enters `/dev` or `/proc`. Each check runs it before its own stat of the path: Neo's stat
    /// of such a name says nothing of what Chromium's process opens there (`/dev/fd/N` can fail
    /// here and be Chromium's Cookies there), so its error must not take the refusal's place.
    /// `check` runs it again, with the rest.
    pub(super) async fn enters_device_tree(&self, path: &str) -> bool {
        let entries = Arc::clone(&self.0);
        let path = PathBuf::from(path);
        blocking(move |_stop| entries.enters_device_tree(&path))
            .await
            .unwrap_or(true)
    }

    /// Whether `path` may go to the page, checked off the runtime's threads. The caller bounds
    /// it by the op's timeout; dropping it stops a walk in progress.
    pub(super) async fn check(&self, path: &str) -> Verdict {
        let entries = Arc::clone(&self.0);
        let path = PathBuf::from(path);
        blocking(move |stop| entries.check(&path, stop))
            .await
            .unwrap_or(Verdict::Refused)
    }
}

/// What one check says of a path.
#[derive(Debug)]
pub(super) enum Verdict {
    /// None of Neo's secrets is reachable through it now.
    Clear,
    /// Nothing can be read through it now: resolving it failed with this.
    Missing(io::Error),
    /// One of Neo's secrets, or what Neo could not clear.
    Refused,
}

/// `script / neo_secret_upload_refused`, with nothing sent.
pub(super) fn refused(scope: &OpScope<'_>) -> PlaywrightError {
    scope.fail(ErrorCode::SecretUploadRefused, SECRET_REFUSED)
}

/// A file's identity on its file system, `(st_dev, st_ino)`: the same for every name the file
/// has, so it is what a symlink, a hard link, a path in another case or a bind mount reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct FileId {
    dev: u64,
    ino: u64,
}

/// What a check needs of one `stat`.
struct Stat {
    id: FileId,
    /// A regular file with more than one name, which may be a secret's other name.
    linked_file: bool,
}

impl Stat {
    /// `None` where the platform gives no inode, and a check then refuses.
    #[cfg(unix)]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the same signature is None on platforms without inodes"
    )]
    fn of(metadata: &Metadata) -> Option<Self> {
        use std::os::unix::fs::MetadataExt;

        Some(Self {
            id: FileId {
                dev: metadata.dev(),
                ino: metadata.ino(),
            },
            linked_file: metadata.is_file() && metadata.nlink() > 1,
        })
    }

    #[cfg(not(unix))]
    fn of(_metadata: &Metadata) -> Option<Self> {
        None
    }
}

/// The named entries as they stood on disk when a round began.
struct Entries {
    /// Every entry's id, files and directories.
    ids: HashSet<FileId>,
    /// The entry directories' ids: a path below one of them is inside it.
    directories: HashSet<FileId>,
    /// The entry directories' real paths, walked for hard links.
    roots: Vec<PathBuf>,
    /// The ids of the directories above each entry's real path and each device tree's: a
    /// directory upload whose id is one of them holds one.
    above: HashSet<FileId>,
    /// The ids of the device trees that exist (rules ⓪ and ④).
    device_trees: HashSet<FileId>,
    /// An entry or a device tree Neo could not resolve for a reason other than its absence, so
    /// nothing is cleared.
    unresolved: bool,
    /// The ids of the regular files with more than one link inside the entry directories,
    /// walked when a check first needs them; `None` when the walk could not finish.
    linked: OnceLock<Option<HashSet<FileId>>>,
}

impl Entries {
    /// The device trees (`DEVICE_TREES`, which a test replaces with a tree of its own) and
    /// each named path that exists now, resolved through their symlinks and stat'ed. A path
    /// that does not exist, or runs through a file, names nothing that can be uploaded now, and
    /// is skipped; the list is read again by the next op.
    fn resolve(device_trees: &[&Path], named: &[PathBuf]) -> Self {
        let mut entries = Self {
            ids: HashSet::new(),
            directories: HashSet::new(),
            roots: Vec::new(),
            above: HashSet::new(),
            device_trees: HashSet::new(),
            unresolved: false,
            linked: OnceLock::new(),
        };
        for tree in device_trees {
            if let Some((_, id)) = entries.resolve_one(tree) {
                entries.device_trees.insert(id);
            }
        }
        for path in named {
            let Some((real, id)) = entries.resolve_one(path) else {
                continue;
            };
            entries.ids.insert(id);
            if real.metadata.is_dir() {
                entries.directories.insert(id);
                entries.roots.push(real.path);
            }
        }
        entries
    }

    /// `path` as it resolves now and its id, with the ids of the directories above its real
    /// path added to `above`. `None` when it names nothing now, and when it cannot be resolved,
    /// which leaves the round unresolved.
    fn resolve_one(&mut self, path: &Path) -> Option<(Real, FileId)> {
        let real = match Real::of(path) {
            Ok(real) => real,
            Err(error) => {
                self.unresolved |= !absent(&error);
                return None;
            }
        };
        let id = Stat::of(&real.metadata).map(|stat| stat.id);
        let above: Option<Vec<FileId>> = real
            .path
            .ancestors()
            .skip(1)
            .map(|above| {
                let metadata = fs::metadata(above).ok()?;
                Stat::of(&metadata).map(|stat| stat.id)
            })
            .collect();
        let (Some(id), Some(above)) = (id, above) else {
            self.unresolved = true;
            return None;
        };
        self.above.extend(above);
        Some((real, id))
    }

    /// The rules in the module's diagram, on what `path`, an absolute path, names now.
    fn check(&self, path: &Path, stop: &AtomicBool) -> Verdict {
        if self.unresolved || self.enters_device_tree(path) {
            return Verdict::Refused;
        }
        let real = match Real::of(path) {
            Ok(real) => real,
            Err(error) => return Verdict::Missing(error),
        };
        let Some(stat) = Stat::of(&real.metadata) else {
            return Verdict::Refused;
        };
        let secret = self.is_secret(stat.id, stat.linked_file, stop)
            || self.under_entry_directory(&real.path)
            || (real.metadata.is_dir()
                && (self.above.contains(&stat.id)
                    || self.tree_holds_secret(&real.path, stat.id, stop)));
        if secret {
            Verdict::Refused
        } else {
            Verdict::Clear
        }
    }

    /// Rule ⓪: whether resolving `path` as the kernel does, one component at a time, steps into
    /// a device tree. Each component is `lstat`ed: a symlink's target takes its place (from `/`
    /// when absolute, else from the link's directory, so a `..` in it leaves that directory),
    /// and a real directory is compared with the device trees by identity, so `/DEV` on a
    /// case-insensitive disk and a bind mount of `/dev` count. A component that cannot be
    /// `lstat`ed ends the walk with no: the kernel cannot pass it either, in any process, and
    /// what is left is the realpath's to report.
    fn enters_device_tree(&self, path: &Path) -> bool {
        let mut real = PathBuf::from("/");
        let mut pending = Vec::new();
        push_components(&mut pending, path);
        let mut hops = 0;
        while let Some(name) = pending.pop() {
            if name == ".." {
                real.pop();
                continue;
            }
            let next = real.join(&name);
            let Ok(metadata) = fs::symlink_metadata(&next) else {
                return false;
            };
            if metadata.file_type().is_symlink() {
                hops += 1;
                if hops > MAX_SYMLINK_HOPS {
                    return false;
                }
                let Ok(target) = fs::read_link(&next) else {
                    return false;
                };
                if target.has_root() {
                    real = PathBuf::from("/");
                }
                push_components(&mut pending, &target);
                continue;
            }
            if Stat::of(&metadata).is_some_and(|stat| self.device_trees.contains(&stat.id)) {
                return true;
            }
            real = next;
        }
        false
    }

    /// Rules ① and ③ on one file or directory's identity: an entry itself, or a regular file
    /// with more than one link that is also a file inside an entry directory.
    fn is_secret(&self, id: FileId, linked_file: bool, stop: &AtomicBool) -> bool {
        self.ids.contains(&id) || (linked_file && self.linked_inside(id, stop))
    }

    /// Rule ④'s walk of a directory upload's tree under `root` (whose id is `root_id`), as
    /// Chromium enumerates it: symlinks inside are skipped, as Chromium drops them (measured on
    /// macOS and in Neo's Linux image), and every other entry is held to rules ① and ③, a
    /// directory too, which catches a bind mount of an entry, and is compared with the device
    /// trees, which catches a bind mount of `/dev` or `/proc`. A directory met twice is walked
    /// once. Yes when the walk cannot finish: a directory it cannot read, or `stop` raised.
    fn tree_holds_secret(&self, root: &Path, root_id: FileId, stop: &AtomicBool) -> bool {
        let mut walked = HashSet::from([root_id]);
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            let Ok(listing) = fs::read_dir(&directory) else {
                return true;
            };
            for entry in listing {
                if stop.load(Ordering::Relaxed) {
                    return true;
                }
                let Ok(entry) = entry else {
                    return true;
                };
                let Ok(kind) = entry.file_type() else {
                    return true;
                };
                if kind.is_symlink() {
                    continue;
                }
                let Some(stat) = entry
                    .metadata()
                    .ok()
                    .and_then(|metadata| Stat::of(&metadata))
                else {
                    return true;
                };
                if self.is_secret(stat.id, stat.linked_file, stop)
                    || self.device_trees.contains(&stat.id)
                {
                    return true;
                }
                if kind.is_dir() && walked.insert(stat.id) {
                    pending.push(entry.path());
                }
            }
        }
        false
    }

    /// Whether a directory above `real`, a real path, is an entry directory, compared by
    /// identity, so another case or a bind mount of the directory counts. A directory above it
    /// that cannot be stat'ed counts too.
    fn under_entry_directory(&self, real: &Path) -> bool {
        if self.directories.is_empty() {
            return false;
        }
        real.ancestors().skip(1).any(|above| {
            fs::metadata(above)
                .ok()
                .and_then(|metadata| Stat::of(&metadata))
                .is_none_or(|stat| self.directories.contains(&stat.id))
        })
    }

    /// Whether `id`, a regular file with more than one link, is also a file inside an entry
    /// directory, where its other name may be a secret's. The directories are walked once per
    /// round, on first need; a walk that cannot finish answers yes.
    fn linked_inside(&self, id: FileId, stop: &AtomicBool) -> bool {
        if self.roots.is_empty() {
            return false;
        }
        self.linked
            .get_or_init(|| linked_files(&self.roots, stop))
            .as_ref()
            .is_none_or(|linked| linked.contains(&id))
    }
}

/// Pushes `path`'s names onto `pending`, a stack, so the first is popped first: a name, or `..`
/// for a parent. Its root and its `.`s go: the caller resets to `/` for an absolute path.
fn push_components(pending: &mut Vec<OsString>, path: &Path) {
    let names: Vec<OsString> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_owned()),
            Component::ParentDir => Some(OsString::from("..")),
            Component::RootDir | Component::CurDir | Component::Prefix(_) => None,
        })
        .collect();
    pending.extend(names.into_iter().rev());
}

/// The ids of the regular files with more than one link under `roots`, or `None` when the walk
/// cannot finish: a directory it cannot read, or `stop` raised. Symlinks are not followed, and
/// a directory met twice (a bind mount of one above it) is walked once.
fn linked_files(roots: &[PathBuf], stop: &AtomicBool) -> Option<HashSet<FileId>> {
    let mut linked = HashSet::new();
    let mut walked = HashSet::new();
    let mut pending = roots.to_vec();
    while let Some(directory) = pending.pop() {
        let stat = Stat::of(&fs::metadata(&directory).ok()?)?;
        if !walked.insert(stat.id) {
            continue;
        }
        for entry in fs::read_dir(&directory).ok()? {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            let entry = entry.ok()?;
            let kind = entry.file_type().ok()?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                let stat = Stat::of(&entry.metadata().ok()?)?;
                if stat.linked_file {
                    linked.insert(stat.id);
                }
            }
        }
    }
    Some(linked)
}

/// A path as it resolves on Neo's disk now, and what is there.
struct Real {
    /// Its real path (`realpath`: its symlinks followed, `..` folded on the way).
    path: PathBuf,
    metadata: Metadata,
}

impl Real {
    fn of(path: &Path) -> io::Result<Self> {
        let path = fs::canonicalize(path)?;
        let metadata = fs::metadata(&path)?;
        Ok(Self { path, metadata })
    }
}

/// Whether a failed resolution means the path names nothing now.
fn absent(error: &io::Error) -> bool {
    matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory)
}

/// Runs `work` on tokio's blocking pool, so a walk of a large directory holds no runtime
/// thread. Dropping the returned future (the op's timeout, or the host dropping the op) raises
/// the flag `work` is handed, which the walks read between entries, so an abandoned walk ends
/// soon instead of running on. `None` if the work panicked.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce(&AtomicBool) -> T + Send + 'static,
) -> Option<T> {
    let stop = Arc::new(AtomicBool::new(false));
    let _raise = RaiseOnDrop(Arc::clone(&stop));
    task::spawn_blocking(move || work(&stop)).await.ok()
}

/// Raises its flag when dropped.
struct RaiseOnDrop(Arc<AtomicBool>);

impl Drop for RaiseOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests;
