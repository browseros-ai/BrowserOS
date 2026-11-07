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

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

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
    /// Relative to the artifact directory, with `/` separators, as the script named it.
    pub path: String,
    /// Its size after the run's last write to it.
    pub bytes: u64,
}

/// A run's artifact directory and what the run wrote into it. Shared by every call of the run
/// through `RunScope`; written from call tasks, read once when the outcome is built.
#[derive(Debug, Default)]
pub(crate) struct Artifacts {
    dir: Option<PathBuf>,
    written: Mutex<Vec<RunArtifact>>,
}

/// A `path` option [`Artifacts::check`] accepted: still relative, without `.` or `..`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactPath {
    /// The option's name, for errors.
    option: &'static str,
    /// Normal components only.
    relative: PathBuf,
    /// `relative` with `/` separators, as the outcome lists it.
    display: String,
}

impl ArtifactPath {
    /// The path as the script named it, relative, with `/` separators.
    pub(crate) fn display(&self) -> &str {
        &self.display
    }
}

impl Artifacts {
    pub(crate) fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            written: Mutex::new(Vec::new()),
        }
    }

    /// Accepts `path` for the option `option` (`path` for screenshots and PDFs) or explains
    /// why Neo will not write it, before the call does any page work.
    pub(crate) fn check(
        &self,
        option: &'static str,
        path: &str,
    ) -> Result<ArtifactPath, PlaywrightError> {
        if self.dir.is_none() {
            return Err(PlaywrightError::error(format!(
                "{option}: this run has no artifact directory, so Neo cannot write \"{path}\"; \
                 the host sets one per run (RunRequest.artifacts)"
            )));
        }
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
        if parts.is_empty() {
            return Err(refused(option, path, "names no file"));
        }
        Ok(ArtifactPath {
            option,
            relative,
            display: parts.join("/"),
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

    /// The bytes of the file at `path` inside the artifact directory, a part of the path that is
    /// a symbolic link refused as [`Self::write`] refuses one. A read lists nothing.
    pub(crate) async fn read(&self, path: &ArtifactPath) -> Result<Vec<u8>, PlaywrightError> {
        let dir = self.dir_for(path)?;
        let relative = path.relative.clone();
        task::spawn_blocking(move || fs::read(inside(&dir, &relative, false)?))
            .await
            .map_err(|error| io::Error::other(error.to_string()))
            .and_then(|result| result)
            .map_err(|error| {
                PlaywrightError::error(format!(
                    "{}: reading \"{}\" failed: {error}",
                    path.option, path.display
                ))
            })
    }

    fn dir_for(&self, path: &ArtifactPath) -> Result<PathBuf, PlaywrightError> {
        self.dir.clone().ok_or_else(|| {
            PlaywrightError::error(format!(
                "{}: this run has no artifact directory",
                path.option
            ))
        })
    }

    /// Lists `path` in the outcome with its size, once, the last size winning.
    fn list(&self, path: &ArtifactPath, size: u64) {
        if let Ok(mut written) = self.written.lock() {
            match written.iter_mut().find(|file| file.path == path.display) {
                Some(file) => file.bytes = size,
                None => written.push(RunArtifact {
                    path: path.display.clone(),
                    bytes: size,
                }),
            }
        }
    }

    /// The files written so far, in the order they were first written.
    pub(crate) fn written(&self) -> Vec<RunArtifact> {
        self.written
            .lock()
            .map(|written| written.clone())
            .unwrap_or_default()
    }
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
                    bytes: 7
                },
                RunArtifact {
                    path: "b.pdf".to_string(),
                    bytes: 5
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

    #[cfg(unix)]
    #[tokio::test]
    async fn tests_that_a_read_finds_what_was_written_and_refuses_a_link_or_a_missing_file() {
        use std::os::unix::fs as unix_fs;

        let (dir, artifacts) = artifacts();
        let path = artifacts.check("path", "state.json").expect("relative");
        artifacts.write(&path, b"{}").await.expect("written");
        assert_eq!(artifacts.read(&path).await.ok(), Some(b"{}".to_vec()));
        let missing = artifacts.check("path", "missing.json").expect("relative");
        assert!(artifacts.read(&missing).await.is_err(), "a missing file");
        let secret = dir.path().join("secret.json");
        fs::write(&secret, "secret").expect("outside");
        unix_fs::symlink(&secret, dir.path().join("run/link.json")).expect("a link");
        let link = artifacts.check("path", "link.json").expect("relative");
        assert!(
            artifacts.read(&link).await.is_err(),
            "a link is not followed"
        );
        assert_eq!(artifacts.written().len(), 1, "a read lists nothing");
        assert!(
            Artifacts::default().read(&path).await.is_err(),
            "a run without a directory reads nothing"
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
}
