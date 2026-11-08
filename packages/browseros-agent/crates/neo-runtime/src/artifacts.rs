//! Neo: the files a run writes for `path` options, and the size a binary result may have to
//! reach the script.
//!
//! ```text
//!  page.screenshot({ path: 'shots/a.png' })      SDK sends `path` with Playwright's params
//!    └─ capture.rs: Artifacts::check("path", …)  before any page work: refused → Error
//!         └─ screenshot / pdf bytes
//!              ├─ Artifacts::write           <artifact dir>/shots/a.png, listed in RunOutcome
//!              └─ returnable(bytes)           over MAX_BINARY_RESULT_BYTES → Error, file kept
//! ```
//!
//! Playwright's client writes `path` itself with Node's `fs`, anywhere on the host. A Neo script
//! has no file system, and the host running it must not hand an agent's script its own disk, so
//! Rust writes the bytes, only inside the directory the host gave the run
//! ([`crate::RunRequest::artifacts`]), and lists every file in
//! [`crate::RunOutcome::artifacts`] for the host to serve.
//!
//! An absolute `path` is refused unless the host opts its runtime into [`AbsolutePaths::Redirect`]
//! (`Runtime::with_absolute_paths`): then one inside the directory names that file, and a write
//! outside it lands at `<artifact directory>/<file name>` instead, listed with the path the script
//! asked for and noted in the run's logs. Nothing is ever written or read outside the directory.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{self, Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use neo_runtime_spec::Binary;
use serde::Serialize;
use tokio::task;

use crate::progress::PlaywrightError;

/// Deviation: `client/page.ts` `Page.screenshot`: `return result.binary;` returns a buffer of
/// any size. A binary result reaches the script as base64 JSON that the 64 MiB realm holds
/// twice while it parses (probes-bin.md B3: 16 MiB fits, 20 MiB ends the run with
/// `InternalError: out of memory`), so a larger one is refused with an Error naming its size.
pub(crate) const MAX_BINARY_RESULT_BYTES: usize = 16 * 1024 * 1024;

/// One file a run wrote into its artifact directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunArtifact {
    /// Relative to the artifact directory, with `/` separators.
    pub path: String,
    /// Its size after the run's last write to it.
    pub bytes: u64,
    /// The absolute path the script asked for, when it lay outside the artifact directory and
    /// Neo wrote the file at `path` instead ([`AbsolutePaths::Redirect`]). Absent from the JSON
    /// otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested: Option<String>,
}

/// How a run's `path` options treat an absolute path. A host picks one for every run of its
/// runtime (`Runtime::with_absolute_paths`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AbsolutePaths {
    /// Every absolute path is refused with an Error naming the option.
    #[default]
    Refuse,
    /// An absolute path inside the artifact directory names that file, for writes and reads. A
    /// write outside it lands at `<artifact directory>/<file name>`, is listed with
    /// [`RunArtifact::requested`] and noted in the run's logs; a read outside it is refused.
    Redirect,
}

/// A run's artifact directory and what the run wrote into it. Shared by every call of the run
/// through `RunScope`; written from call tasks, read once when the outcome is built.
#[derive(Debug, Default)]
pub(crate) struct Artifacts {
    dir: Option<PathBuf>,
    absolute: AbsolutePaths,
    written: Mutex<Vec<RunArtifact>>,
    /// One line per redirected file, for the run's logs.
    notes: Mutex<Vec<String>>,
}

/// A file or directory a script named for `setInputFiles` or `drop`, found inside the artifact
/// directory: what Playwright's client learns with `fs.promises.stat`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExistingPath {
    /// Absolute, as CDP `DOM.setFileInputFiles` takes it.
    pub(crate) path: String,
    /// As the script named it, with `/` separators, for errors.
    pub(crate) display: String,
    pub(crate) is_dir: bool,
    /// Its size in bytes (`stat.size`).
    pub(crate) size: u64,
    /// Its modification time in ms since the epoch (`stat.mtimeMs`), when the platform has one.
    pub(crate) modified_ms: Option<f64>,
}

/// A `path` option [`Artifacts::check`] accepted: relative to the artifact directory, without
/// `.` or `..`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactPath {
    /// The option's name, for errors.
    option: &'static str,
    /// Normal components only.
    relative: PathBuf,
    /// `relative` with `/` separators, as the outcome lists it.
    display: String,
    /// The script's absolute path, when it lay outside the directory ([`AbsolutePaths::Redirect`]).
    requested: Option<String>,
}

impl Artifacts {
    pub(crate) fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            absolute: AbsolutePaths::Refuse,
            written: Mutex::new(Vec::new()),
            notes: Mutex::new(Vec::new()),
        }
    }

    /// The same directory, treating absolute paths as `absolute` says.
    pub(crate) fn with_absolute_paths(self, absolute: AbsolutePaths) -> Self {
        Self { absolute, ..self }
    }

    /// Accepts `path` for the option `option` (`path` for screenshots and PDFs) or explains
    /// why Neo will not write it, before the call does any page work.
    pub(crate) fn check(
        &self,
        option: &'static str,
        path: &str,
    ) -> Result<ArtifactPath, PlaywrightError> {
        let Some(dir) = &self.dir else {
            return Err(PlaywrightError::error(format!(
                "{option}: this run has no artifact directory, so Neo cannot write \"{path}\"; \
                 the host sets one per run (RunRequest.artifacts)"
            )));
        };
        if self.absolute == AbsolutePaths::Redirect && Path::new(path).is_absolute() {
            return redirected(option, path, dir);
        }
        let Relative {
            path: relative,
            parts,
            directory,
        } = relative_path(option, path)?;
        if parts.is_empty() {
            return Err(refused(option, path, "names no file"));
        }
        // Node's `fs` refuses to write a file at `capture.png/`; Neo refuses before any effect,
        // so an existing `capture.png` stays as it was.
        if directory {
            return Err(refused(option, path, "names a directory, not a file"));
        }
        Ok(ArtifactPath {
            option,
            relative,
            display: parts.join("/"),
            requested: None,
        })
    }

    /// Writes `bytes` to `path` inside the artifact directory, creating its parent directories
    /// as Playwright's `mkdirIfNeeded` does, and lists the file in the outcome. A component that
    /// is a symbolic link is refused, so no write lands outside the directory.
    pub(crate) async fn write(
        &self,
        path: &ArtifactPath,
        bytes: &[u8],
    ) -> Result<(), PlaywrightError> {
        let dir = self.dir_for(path)?;
        let relative = path.relative.clone();
        let contents = bytes.to_vec();
        let written = task::spawn_blocking(move || write_inside(&dir, &relative, &contents))
            .await
            .map_err(|error| io::Error::other(error.to_string()))
            .and_then(|result| result);
        if let Err(error) = written {
            return Err(PlaywrightError::error(format!(
                "{}: writing \"{}\" failed: {error}",
                path.option, path.display
            )));
        }
        self.list(path, u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        Ok(())
    }

    /// Copies the file at `source` (a download the browser saved) to `path` inside the artifact
    /// directory as [`Self::write`] writes bytes, overwriting a file already there as
    /// Playwright's copy does, and lists it in the outcome. Returns its size.
    pub(crate) async fn copy(
        &self,
        path: &ArtifactPath,
        source: &Path,
    ) -> Result<u64, PlaywrightError> {
        let dir = self.dir_for(path)?;
        let relative = path.relative.clone();
        let source = source.to_path_buf();
        let copied =
            task::spawn_blocking(move || fs::copy(&source, inside(&dir, &relative, true)?))
                .await
                .map_err(|error| io::Error::other(error.to_string()))
                .and_then(|result| result)
                .map_err(|error| {
                    PlaywrightError::error(format!(
                        "{}: saving \"{}\" failed: {error}",
                        path.option, path.display
                    ))
                })?;
        self.list(path, copied);
        Ok(copied)
    }

    fn dir_for(&self, path: &ArtifactPath) -> Result<PathBuf, PlaywrightError> {
        self.dir.clone().ok_or_else(|| {
            PlaywrightError::error(format!(
                "{}: this run has no artifact directory",
                path.option
            ))
        })
    }

    /// Lists `path` in the outcome with its size, once, the last write winning, and notes a
    /// redirected one for the run's logs.
    fn list(&self, path: &ArtifactPath, size: u64) {
        if let Ok(mut written) = self.written.lock() {
            match written.iter_mut().find(|file| file.path == path.display) {
                Some(file) => {
                    file.bytes = size;
                    file.requested.clone_from(&path.requested);
                }
                None => written.push(RunArtifact {
                    path: path.display.clone(),
                    bytes: size,
                    requested: path.requested.clone(),
                }),
            }
        }
        if let (Some(requested), Some(dir)) = (&path.requested, &self.dir) {
            let at = absolute_dir(dir);
            let note = format!(
                "neo: {}: \"{requested}\" is outside the run's artifact directory, so the file \
                 was written to \"{}\" instead",
                path.option,
                at.join(&path.relative).display()
            );
            if let Ok(mut notes) = self.notes.lock()
                && !notes.contains(&note)
            {
                notes.push(note);
            }
        }
    }

    /// The notes for redirected files, in the order first written.
    pub(crate) fn notes(&self) -> Vec<String> {
        self.notes
            .lock()
            .map(|notes| notes.clone())
            .unwrap_or_default()
    }

    /// Deviation: `client/elementHandle.ts` `resolvePathsAndDirectoryForInputFiles`: `const stat = await fs.promises.stat(item as string);` Playwright's client reads any path on its host, relative to its working directory. A Neo script has no file system, and the host must not hand an agent's script its disk, so Rust reads a file to upload or drop only inside the run's artifact directory, at a relative path without `..`, refusing a symbolic link on the way, and a run without one names the option.
    ///
    /// The file or directory `path` names for the option `option`, or why Neo will not read it.
    ///
    /// # Errors
    ///
    /// No artifact directory, a path [`Self::check`] refuses, a symbolic link on the way, or a
    /// path that does not exist.
    pub(crate) async fn existing(
        &self,
        option: &'static str,
        path: &str,
    ) -> Result<ExistingPath, PlaywrightError> {
        let Some(dir) = self.dir.clone() else {
            return Err(PlaywrightError::error(format!(
                "{option}: this run has no artifact directory, so Neo cannot read \"{path}\"; \
                 the host sets one per run (RunRequest.artifacts)"
            )));
        };
        // A read may name the directory itself (`.`, a directory upload of the root), which no
        // write may; an empty path names nothing, as Playwright's client stat of it fails.
        if path.is_empty() {
            return Err(refused(option, path, "names no file"));
        }
        let spelled = if self.absolute == AbsolutePaths::Redirect && Path::new(path).is_absolute() {
            match within(&dir, path) {
                Some(rest) => rest,
                None => {
                    return Err(refused(
                        option,
                        path,
                        &format!(
                            "is outside the run's artifact directory \"{}\", the only place Neo \
                             reads files",
                            absolute_dir(&dir).display()
                        ),
                    ));
                }
            }
        } else {
            path.to_string()
        };
        let Relative {
            path: relative,
            parts,
            directory,
        } = relative_path(option, &spelled)?;
        let display = if parts.is_empty() {
            ".".to_string()
        } else {
            parts.join("/")
        };
        let found = task::spawn_blocking(move || stat_inside(&dir, &relative))
            .await
            .map_err(|error| io::Error::other(error.to_string()))
            .and_then(|result| result);
        let (at, metadata) = found.map_err(|error| {
            PlaywrightError::error(format!(
                "{option}: \"{display}\" cannot be read in the run's artifact directory: {error}"
            ))
        })?;
        // Playwright's client stats the path as spelled: `real.txt/` is no file (`ENOTDIR`).
        if directory && !metadata.is_dir() {
            return Err(refused(option, path, "is not a directory"));
        }
        let Some(text) = at.to_str() else {
            return Err(refused(
                option,
                path,
                "is not valid UTF-8 in the artifact directory",
            ));
        };
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|since| since.as_secs_f64() * 1000.0);
        Ok(ExistingPath {
            path: text.to_string(),
            display,
            is_dir: metadata.is_dir(),
            size: metadata.len(),
            modified_ms,
        })
    }

    /// The files written so far, in the order they were first written.
    pub(crate) fn written(&self) -> Vec<RunArtifact> {
        self.written
            .lock()
            .map(|written| written.clone())
            .unwrap_or_default()
    }
}

/// A path a script named, as [`relative_path`] reads it.
struct Relative {
    /// Normal components only.
    path: PathBuf,
    /// Their text.
    parts: Vec<String>,
    /// The spelling names a directory: `.` itself, or a trailing `/` or `/.`, which
    /// `Path::components` drops. Node's `fs` keeps it: `stat('real.txt/')` is `ENOTDIR`.
    directory: bool,
}

/// The normal components of `path` and their text, refusing what leaves the artifact directory:
/// `..`, an absolute path, text that is not UTF-8. `.` components vanish, so `.` names the
/// directory itself (no components); a trailing `/` or `/.` is kept as directory intent.
fn relative_path(option: &str, path: &str) -> Result<Relative, PlaywrightError> {
    let mut relative = PathBuf::new();
    let mut parts = Vec::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => {
                let Some(text) = part.to_str() else {
                    return Err(refused(option, path, "is not valid UTF-8"));
                };
                relative.push(part);
                parts.push(text.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(refused(
                    option,
                    path,
                    "leaves the run's artifact directory (`..`)",
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(refused(
                    option,
                    path,
                    "must be relative to the run's artifact directory",
                ));
            }
        }
    }
    Ok(Relative {
        path: relative,
        parts,
        directory: names_directory(path),
    })
}

/// Deviation: `client/page.ts` `Page.screenshot`: `await fs.promises.writeFile(options.path, result.binary);` Playwright writes an absolute path anywhere on its host. Under [`AbsolutePaths::Redirect`] Neo writes one inside the artifact directory as that file, and one outside it at `<artifact directory>/<file name>`, so an agent's script still never writes the host's disk; the script carries on as Playwright's does, and the outcome lists the file with the path asked for (`RunArtifact::requested`) and a log line says where it went. Two such paths with one file name share the file, the last write winning.
///
/// The write target for the absolute `path`, under the artifact directory `dir`.
fn redirected(
    option: &'static str,
    path: &str,
    dir: &Path,
) -> Result<ArtifactPath, PlaywrightError> {
    // Node's `fs` refuses to write a file at `capture.png/`; so does Neo, wherever it points.
    if names_directory(path) {
        return Err(refused(option, path, "names a directory, not a file"));
    }
    if let Some(rest) = within(dir, path) {
        let Relative {
            path: relative,
            parts,
            ..
        } = relative_path(option, &rest)?;
        if parts.is_empty() {
            return Err(refused(option, path, "names a directory, not a file"));
        }
        return Ok(ArtifactPath {
            option,
            relative,
            display: parts.join("/"),
            requested: None,
        });
    }
    let Some(name) = Path::new(path).file_name().and_then(OsStr::to_str) else {
        return Err(refused(option, path, "names no file"));
    };
    Ok(ArtifactPath {
        option,
        relative: PathBuf::from(name),
        display: name.to_string(),
        requested: Some(path.to_string()),
    })
}

/// `dir` made absolute against the working directory, as an absolute path a script names is
/// compared with it. Lexical only: a symbolic link in `dir`'s own path is not resolved, so a
/// script names the directory as the host reports it.
fn absolute_dir(dir: &Path) -> PathBuf {
    path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf())
}

/// The absolute `path` relative to `dir` (`/`-separated), when it lies inside it: it starts
/// with `dir`'s components and only normal or `.` components follow.
fn within(dir: &Path, path: &str) -> Option<String> {
    let rest = Path::new(path).strip_prefix(absolute_dir(dir)).ok()?;
    let mut parts = Vec::new();
    for component in rest.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    // Keep directory intent (`…/shots/`), which `relative_path` reads from the spelling.
    let mut rest = if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    };
    if names_directory(path) && !parts.is_empty() {
        rest.push('/');
    }
    Some(rest)
}

/// The spelling names a directory: `.` itself, or a trailing `/` or `/.`.
fn names_directory(path: &str) -> bool {
    path == "." || path.ends_with('/') || path.ends_with("/.")
}

fn refused(option: &str, path: &str, why: &str) -> PlaywrightError {
    PlaywrightError::error(format!("{option}: \"{path}\" {why}"))
}

/// `relative` under `dir`, each part checked on the way down: none, the file included, may be a
/// symbolic link, so nothing read or written lands outside `dir`. With `create`, `dir` and the
/// missing parents are made, as Playwright's `mkdirIfNeeded` does.
fn inside(dir: &Path, relative: &Path, create: bool) -> io::Result<PathBuf> {
    if create {
        fs::create_dir_all(dir)?;
    }
    let mut at = dir.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        at.push(component);
        match fs::symlink_metadata(&at) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(io::Error::other(format!(
                    "{} is a symbolic link",
                    at.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if create && components.peek().is_some() {
                    fs::create_dir(&at)?;
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(at)
}

/// Creates `relative`'s parents under `dir` and writes the file ([`inside`]).
fn write_inside(dir: &Path, relative: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::write(inside(dir, relative, true)?, bytes)
}

/// Walks `relative` under `dir` one component at a time, refusing any that is a symbolic link,
/// and answers its absolute path and metadata.
fn stat_inside(dir: &Path, relative: &Path) -> io::Result<(PathBuf, fs::Metadata)> {
    let mut at = path::absolute(dir)?;
    let mut metadata = None;
    for component in relative.components() {
        at.push(component);
        let found = fs::symlink_metadata(&at)?;
        if found.file_type().is_symlink() {
            return Err(io::Error::other(format!(
                "{} is a symbolic link",
                at.display()
            )));
        }
        metadata = Some(found);
    }
    // No components: the artifact directory itself.
    let metadata = match metadata {
        Some(metadata) => metadata,
        None => fs::metadata(&at)?,
    };
    Ok((at, metadata))
}

/// `bytes` as a call's `binary` result, unless it is too large for the script's realm
/// ([`MAX_BINARY_RESULT_BYTES`]). `what` names the result in the error; `saved` is the file the
/// call already wrote, which the error then names.
pub(crate) fn returnable(
    what: &str,
    bytes: Vec<u8>,
    saved: Option<&ArtifactPath>,
) -> Result<Binary, PlaywrightError> {
    if bytes.len() <= MAX_BINARY_RESULT_BYTES {
        return Ok(Binary(bytes));
    }
    let saved = saved.map_or_else(String::new, |path| {
        format!("; it was written in full to \"{}\"", path.display)
    });
    Err(PlaywrightError::error(format!(
        "{what} is {} bytes, over the {MAX_BINARY_RESULT_BYTES} bytes Neo returns to a script \
         at once{saved}",
        bytes.len()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifacts() -> (tempfile::TempDir, Artifacts) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let artifacts = Artifacts::new(Some(dir.path().join("run")));
        (dir, artifacts)
    }

    #[test]
    fn tests_that_paths_outside_the_artifact_directory_are_refused() {
        let (_dir, artifacts) = artifacts();
        for (path, why) in [
            ("/etc/passwd", "must be relative"),
            ("../escape.png", "leaves the run's artifact directory"),
            (
                "shots/../../escape.png",
                "leaves the run's artifact directory",
            ),
            ("", "names no file"),
            (".", "names no file"),
        ] {
            let error = artifacts.check("path", path).expect_err(path);
            assert!(error.message.starts_with("path: "), "{error:?}");
            assert!(error.message.contains(why), "{path}: {error:?}");
        }
        let accepted = artifacts.check("path", "./shots//a.png").expect("relative");
        assert_eq!(accepted.display, "shots/a.png");
    }

    #[test]
    fn tests_that_a_run_without_a_directory_names_the_option() {
        let error = Artifacts::new(None)
            .check("path", "a.png")
            .expect_err("no directory");
        assert!(error.message.starts_with("path: "), "{error:?}");
        assert!(error.message.contains("no artifact directory"), "{error:?}");
    }

    #[tokio::test]
    async fn tests_that_writes_create_parents_and_are_listed_once_per_path() {
        let (dir, artifacts) = artifacts();
        let path = artifacts
            .check("path", "shots/day/a.png")
            .expect("relative");
        artifacts.write(&path, b"first").await.expect("written");
        artifacts.write(&path, b"second!").await.expect("rewritten");
        let other = artifacts.check("path", "b.pdf").expect("relative");
        artifacts.write(&other, b"%PDF-").await.expect("written");
        assert_eq!(
            fs::read(dir.path().join("run/shots/day/a.png")).expect("the file"),
            b"second!"
        );
        assert_eq!(
            artifacts.written(),
            [
                RunArtifact {
                    path: "shots/day/a.png".to_string(),
                    bytes: 7,
                    requested: None,
                },
                RunArtifact {
                    path: "b.pdf".to_string(),
                    bytes: 5,
                    requested: None,
                },
            ]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn tests_that_a_symbolic_link_inside_the_directory_is_not_followed() {
        use std::os::unix::fs as unix_fs;

        let (dir, artifacts) = artifacts();
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).expect("outside");
        fs::create_dir_all(dir.path().join("run")).expect("run");
        unix_fs::symlink(&outside, dir.path().join("run/link")).expect("a link");
        let path = artifacts.check("path", "link/a.png").expect("relative");
        let error = artifacts.write(&path, b"x").await.expect_err("refused");
        assert!(error.message.contains("symbolic link"), "{error:?}");
        assert!(!outside.join("a.png").exists(), "nothing written outside");
        assert!(artifacts.written().is_empty(), "nothing listed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn tests_that_a_copy_makes_its_folders_overwrites_is_listed_and_refuses_a_link() {
        use std::os::unix::fs as unix_fs;

        let (dir, artifacts) = artifacts();
        let source = dir.path().join("download.txt");
        fs::write(&source, "hello data").expect("the source");
        let path = artifacts.check("path", "a/b/saved.txt").expect("relative");
        assert_eq!(artifacts.copy(&path, &source).await.ok(), Some(10));
        fs::write(&source, "hello again").expect("the source again");
        assert_eq!(
            artifacts.copy(&path, &source).await.ok(),
            Some(11),
            "a second save overwrites"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("run/a/b/saved.txt"))
                .ok()
                .as_deref(),
            Some("hello again")
        );
        assert!(source.exists(), "the browser's file stays");
        assert_eq!(
            artifacts.written(),
            vec![RunArtifact {
                path: "a/b/saved.txt".to_string(),
                bytes: 11,
                requested: None,
            }],
            "listed once, with its last size"
        );
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).expect("outside");
        unix_fs::symlink(&outside, dir.path().join("run/link")).expect("a link");
        let through = artifacts.check("path", "link/x.txt").expect("relative");
        assert!(
            artifacts.copy(&through, &source).await.is_err(),
            "through a symbolic link"
        );
        assert!(!outside.join("x.txt").exists(), "nothing landed outside");
    }

    /// Upload paths resolve inside the directory only: a file and a directory are found with
    /// their kind, a missing file and a run without a directory name the option, and a symbolic
    /// link on the way is refused.
    #[tokio::test]
    async fn tests_that_existing_paths_resolve_inside_the_artifact_directory_only() {
        let (dir, artifacts) = artifacts();
        fs::create_dir_all(dir.path().join("run/uploads")).expect("uploads");
        fs::write(dir.path().join("run/uploads/a.txt"), b"a").expect("a file");
        let file = artifacts
            .existing("files", "uploads/a.txt")
            .await
            .expect("found");
        assert!(
            !file.is_dir && file.path.ends_with("run/uploads/a.txt"),
            "{file:?}"
        );
        assert!(file.modified_ms.is_some_and(|ms| ms > 0.0), "{file:?}");
        let folder = artifacts
            .existing("files", "./uploads")
            .await
            .expect("found");
        assert!(folder.is_dir, "{folder:?}");
        for (path, why) in [
            ("uploads/missing.txt", "cannot be read"),
            ("../escape.txt", "leaves the run's artifact directory"),
            ("/etc/hosts", "must be relative"),
        ] {
            let error = artifacts.existing("files", path).await.expect_err(path);
            assert!(error.message.starts_with("files: "), "{error:?}");
            assert!(error.message.contains(why), "{path}: {error:?}");
        }
        // The directory itself is a read (a directory upload of the root), never a write.
        for root in [".", "./", "uploads/.."] {
            let found = artifacts.existing("files", root).await;
            if root == "uploads/.." {
                assert!(
                    found.is_err_and(|error| error.message.contains("`..`")),
                    "{root}"
                );
                continue;
            }
            let found = found.expect(root);
            assert!(
                found.is_dir && found.path.ends_with("run"),
                "{root}: {found:?}"
            );
            assert_eq!(found.display, ".");
        }
        let error = artifacts.existing("files", "").await.expect_err("empty");
        assert!(error.message.contains("names no file"), "{error:?}");
        assert!(
            artifacts.check("path", ".").is_err(),
            "a write needs a file name"
        );
        // A trailing `/` or `/.` names a directory: a read must find one, a write refuses it.
        let folder = artifacts
            .existing("files", "uploads/")
            .await
            .expect("a folder");
        assert!(folder.is_dir, "{folder:?}");
        for file in ["uploads/a.txt/", "uploads/a.txt/."] {
            let error = artifacts.existing("files", file).await.expect_err(file);
            assert!(
                error.message.contains("is not a directory"),
                "{file}: {error:?}"
            );
        }
        for target in ["shot.png/", "shot.png/.", "shots/."] {
            let error = artifacts.check("path", target).expect_err(target);
            assert!(
                error.message.contains("names a directory"),
                "{target}: {error:?}"
            );
        }
        let error = Artifacts::new(None)
            .existing("files", "a.txt")
            .await
            .expect_err("no directory");
        assert!(error.message.contains("cannot read \"a.txt\""), "{error:?}");
        #[cfg(unix)]
        {
            use std::os::unix::fs as unix_fs;

            unix_fs::symlink("/etc", dir.path().join("run/link")).expect("a link");
            let error = artifacts
                .existing("files", "link/hosts")
                .await
                .expect_err("a link");
            assert!(error.message.contains("symbolic link"), "{error:?}");
        }
    }

    fn redirecting() -> (tempfile::TempDir, PathBuf, Artifacts) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("run");
        let artifacts =
            Artifacts::new(Some(root.clone())).with_absolute_paths(AbsolutePaths::Redirect);
        (dir, root, artifacts)
    }

    /// Under `Redirect`, an absolute path inside the directory names that file; one outside it
    /// lands at `<dir>/<file name>`, listed with what was asked for and noted once; nothing is
    /// written outside.
    #[tokio::test]
    async fn tests_that_redirected_absolute_paths_write_only_inside_the_directory() {
        let (dir, root, artifacts) = redirecting();
        let inside = root.join("shots/a.png");
        let path = artifacts
            .check("path", inside.to_str().expect("UTF-8"))
            .expect("inside");
        assert_eq!(
            (path.display.as_str(), &path.requested),
            ("shots/a.png", &None)
        );
        artifacts.write(&path, b"in").await.expect("written");
        assert_eq!(fs::read(&inside).expect("the file"), b"in");

        let outside = dir.path().join("elsewhere/columns.png");
        let asked = outside.to_str().expect("UTF-8");
        for _ in 0..2 {
            let path = artifacts.check("path", asked).expect("redirected");
            artifacts.write(&path, b"out").await.expect("written");
        }
        assert!(!outside.exists(), "nothing written where the script asked");
        assert_eq!(
            fs::read(root.join("columns.png")).expect("the file"),
            b"out"
        );
        assert_eq!(
            artifacts.written(),
            [
                RunArtifact {
                    path: "shots/a.png".to_string(),
                    bytes: 2,
                    requested: None,
                },
                RunArtifact {
                    path: "columns.png".to_string(),
                    bytes: 3,
                    requested: Some(asked.to_string()),
                },
            ]
        );
        let notes = artifacts.notes();
        assert_eq!(notes.len(), 1, "one note per redirected file: {notes:?}");
        assert!(
            notes[0].contains(asked)
                && notes[0].contains(&format!("{}", root.join("columns.png").display())),
            "{notes:?}"
        );
    }

    /// Redirect keeps the write refusals: a directory spelling, no file name, `..` inside the
    /// directory that climbs out (then redirected by file name), and relative paths as before.
    #[test]
    fn tests_that_redirect_keeps_directory_and_climb_refusals() {
        let (_dir, root, artifacts) = redirecting();
        let text = |path: PathBuf| path.to_str().expect("UTF-8").to_string();
        for (path, why) in [
            ("/tmp/shots/".to_string(), "names a directory"),
            ("/tmp/shots/.".to_string(), "names a directory"),
            ("/".to_string(), "names a directory"),
            ("/tmp/..".to_string(), "names no file"),
            (text(root.clone()), "names a directory"),
            (format!("{}/", text(root.clone())), "names a directory"),
            (
                "../escape.png".to_string(),
                "leaves the run's artifact directory",
            ),
        ] {
            let error = artifacts.check("path", &path).expect_err(&path);
            assert!(error.message.contains(why), "{path}: {error:?}");
        }
        let climbed = artifacts
            .check("path", &format!("{}/../escape.png", text(root.clone())))
            .expect("redirected");
        assert_eq!(climbed.display, "escape.png");
        assert!(climbed.requested.is_some(), "{climbed:?}");
        let refused = Artifacts::new(Some(root))
            .check("path", "/tmp/a.png")
            .expect_err("the default refuses absolute paths");
        assert!(refused.message.contains("must be relative"), "{refused:?}");
    }

    /// Reads under `Redirect`: an absolute path inside the directory is found; outside it is
    /// refused naming the directory.
    #[tokio::test]
    async fn tests_that_redirect_reads_absolute_paths_inside_only() {
        let (dir, root, artifacts) = redirecting();
        fs::create_dir_all(root.join("uploads")).expect("uploads");
        fs::write(root.join("uploads/a.txt"), b"a").expect("a file");
        let inside = root.join("uploads/a.txt");
        let found = artifacts
            .existing("files", inside.to_str().expect("UTF-8"))
            .await
            .expect("found");
        assert_eq!(found.display, "uploads/a.txt");
        assert!(!found.is_dir);
        fs::write(dir.path().join("secret.txt"), b"s").expect("outside");
        let outside = dir.path().join("secret.txt");
        let error = artifacts
            .existing("files", outside.to_str().expect("UTF-8"))
            .await
            .expect_err("outside");
        assert!(
            error
                .message
                .contains("outside the run's artifact directory"),
            "{error:?}"
        );
    }

    #[test]
    fn tests_that_results_past_the_realm_cap_are_refused_with_their_size() {
        let fits = returnable("screenshot", vec![1; 8], None).expect("small");
        assert_eq!(fits.as_bytes(), [1; 8]);
        let (_dir, artifacts) = artifacts();
        let path = artifacts.check("path", "big.png").expect("relative");
        let error = returnable(
            "screenshot",
            vec![0; MAX_BINARY_RESULT_BYTES + 1],
            Some(&path),
        )
        .expect_err("too large");
        assert!(error.message.contains("16777217 bytes"), "{error:?}");
        assert!(error.message.contains("\"big.png\""), "{error:?}");
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn review_shared_path_rules_keep_reads_and_writes_inside_the_root() {
        use std::os::unix::fs as unix_fs;
        let (dir, artifacts) = artifacts();
        let root = dir.path().join("run");
        let outside_dir = dir.path().join("outside-dir");
        fs::create_dir_all(root.join("a/subdir")).unwrap();
        fs::create_dir_all(&outside_dir).unwrap();
        fs::write(root.join("safe.txt"), b"safe").unwrap();
        fs::write(outside_dir.join("outside.txt"), b"outside").unwrap();
        let outside_file = dir.path().join("outside.txt");
        fs::write(&outside_file, b"outside").unwrap();
        unix_fs::symlink(&outside_file, root.join("file-link")).unwrap();
        unix_fs::symlink(&outside_dir, root.join("dir-link")).unwrap();
        let absolute_inside = root.join("safe.txt").to_str().unwrap().to_string();
        for spelling in [
            "./../outside.txt",
            "a/../../outside.txt",
            absolute_inside.as_str(),
            "",
        ] {
            assert!(
                artifacts.existing("files", spelling).await.is_err(),
                "read {spelling}"
            );
            assert!(
                artifacts.check("path", spelling).is_err(),
                "write {spelling}"
            );
        }
        for spelling in ["file-link", "dir-link", "dir-link/", "dir-link/outside.txt"] {
            assert!(
                artifacts.existing("files", spelling).await.is_err(),
                "read {spelling}"
            );
            // Refused by `check` (a directory spelling) or by the write (a link on the way).
            if let Ok(write) = artifacts.check("path", spelling) {
                assert!(
                    artifacts.write(&write, b"changed").await.is_err(),
                    "write {spelling}"
                );
            }
        }
        for spelling in [".", "./", ".//."] {
            assert!(
                artifacts.existing("files", spelling).await.unwrap().is_dir,
                "read root {spelling}"
            );
            assert!(
                artifacts.check("path", spelling).is_err(),
                "write root {spelling}"
            );
        }
        for spelling in ["a", "a/", "a/subdir/"] {
            assert!(
                artifacts.existing("files", spelling).await.unwrap().is_dir,
                "read directory {spelling}"
            );
            // Refused by `check` (a directory spelling) or by the write (an actual directory).
            if let Ok(write) = artifacts.check("path", spelling) {
                assert!(
                    artifacts.write(&write, b"changed").await.is_err(),
                    "write directory {spelling}"
                );
            }
        }
        assert!(root.is_dir());
        assert_eq!(fs::read(&outside_file).unwrap(), b"outside");
        assert_eq!(
            fs::read(outside_dir.join("outside.txt")).unwrap(),
            b"outside"
        );
        assert_eq!(fs::read(root.join("safe.txt")).unwrap(), b"safe");
        assert!(artifacts.written().is_empty());
    }
}
