#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use bpatch::git::GitAdapter;
use bpatch::process::Git;
use tempfile::TempDir;

/// Scripted temporary chromium-like repository for integration tests.
pub struct FixtureRepo {
    dir: TempDir,
    git: Git,
}

impl FixtureRepo {
    /// Creates a temporary git repo with deterministic local author config.
    pub fn new() -> Result<Self> {
        let dir = tempfile::tempdir().context("creating fixture repo")?;
        let git = Git::new(dir.path());
        git.run(&["init", "-b", "main"])?;
        git.run(&["config", "user.name", "Bpatch Test"])?;
        git.run(&["config", "user.email", "bpatch@example.com"])?;
        git.run(&["config", "commit.gpgsign", "false"])?;
        Ok(Self { dir, git })
    }

    /// Returns the repository root.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Returns the low-level git runner for custom fixture setup.
    pub fn git(&self) -> &Git {
        &self.git
    }

    /// Returns the typed git adapter pointed at this repo.
    pub fn git_adapter(&self) -> GitAdapter {
        GitAdapter::new(self.path())
    }

    /// Writes a file under the repository root, creating parent dirs.
    pub fn write_file(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<()> {
        let path = self.path().join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating parent dir for {}", path.display()))?;
        }
        fs::write(&path, contents).with_context(|| format!("writing {}", path.display()))
    }

    /// Removes a file under the repository root.
    pub fn remove_file(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = self.path().join(path);
        fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))
    }

    /// Reads a UTF-8 file under the repository root.
    pub fn read_file(&self, path: impl AsRef<Path>) -> Result<String> {
        let path = self.path().join(path);
        fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
    }

    /// Commits the current fixture repo contents and returns the commit sha.
    pub fn commit(&self, message: &str) -> Result<String> {
        self.git.run(&["add", "-A"])?;
        self.git.run(&["commit", "-m", message])?;
        self.git.run_str(&["rev-parse", "HEAD"])
    }

    /// Commits all current changes with an explicit trailer block.
    pub fn commit_with_trailers(&self, subject: &str, trailers: &[(&str, &str)]) -> Result<String> {
        self.git.run(&["add", "-A"])?;
        let mut message = String::new();
        message.push_str(subject);
        message.push_str("\n\n");
        for (key, value) in trailers {
            message.push_str(key);
            message.push_str(": ");
            message.push_str(value);
            message.push('\n');
        }
        self.git
            .run_with_stdin(&["commit", "-F", "-"], message.as_bytes())?;
        self.git.run_str(&["rev-parse", "HEAD"])
    }

    /// Tags a base commit name and returns the resolved sha.
    pub fn mark_base(&self, name: &str, rev: &str) -> Result<String> {
        let sha = self.git.run_str(&["rev-parse", rev])?;
        self.git.run(&["tag", "-f", name, &sha])?;
        Ok(sha)
    }

    /// Writes a unified diff patch from base to target into a store-like path.
    pub fn create_patch(
        &self,
        patch_path: impl AsRef<Path>,
        base: &str,
        target: &str,
        paths: &[&str],
    ) -> Result<PathBuf> {
        let mut args = vec!["diff", "--binary", base, target, "--"];
        args.extend(paths.iter().copied());
        let diff = self.git.run(&args)?;
        if diff.is_empty() {
            bail!("empty fixture patch for {}..{}", base, target);
        }
        let patch_path = self.path().join(patch_path);
        if let Some(parent) = patch_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating parent dir for {}", patch_path.display()))?;
        }
        fs::write(&patch_path, diff)
            .with_context(|| format!("writing {}", patch_path.display()))?;
        Ok(patch_path)
    }

    /// Plants an untracked file under the repository root.
    pub fn plant_untracked(
        &self,
        path: impl AsRef<Path>,
        contents: impl AsRef<[u8]>,
    ) -> Result<()> {
        self.write_file(path, contents)
    }

    /// Reads the last-modified time for a file under the repository root.
    pub fn mtime(&self, path: impl AsRef<Path>) -> Result<SystemTime> {
        let path = self.path().join(path);
        fs::metadata(&path)
            .with_context(|| format!("stat {}", path.display()))?
            .modified()
            .with_context(|| format!("mtime {}", path.display()))
    }

    /// Returns `git status --porcelain` for the fixture repo.
    pub fn status_porcelain(&self) -> Result<String> {
        self.git.run_str(&["status", "--porcelain"])
    }
}
