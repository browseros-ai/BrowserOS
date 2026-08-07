//! Low-level system-git runner shared by every layer.
//!
//! Forces a C locale and scrubs inherited GIT_* vars so parsed output is
//! stable and a stray GIT_INDEX_FILE can never leak into an unrelated
//! command; per-call env (e.g. a temp index) is opt-in via [`Git::with_env`].

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, anyhow, bail};

const SCRUBBED: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_NAMESPACE",
    "GIT_CEILING_DIRECTORIES",
];

#[derive(Clone, Debug)]
pub struct Git {
    repo: PathBuf,
    env: Vec<(OsString, OsString)>,
}

impl Git {
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        Self {
            repo: repo.into(),
            env: Vec::new(),
        }
    }

    /// Derived runner with one extra env var applied to every call — the
    /// mechanism for scoping a temp GIT_INDEX_FILE to one tree-construction
    /// sequence without mutating global state.
    pub fn with_env(&self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        let mut derived = self.clone();
        derived.env.push((key.into(), value.into()));
        derived
    }

    pub fn repo(&self) -> &Path {
        &self.repo
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.repo).args(args);
        for var in SCRUBBED {
            cmd.env_remove(var);
        }
        cmd.env("LC_ALL", "C").env("GIT_TERMINAL_PROMPT", "0");
        for (key, value) in &self.env {
            cmd.env(key, value);
        }
        cmd
    }

    /// Runs git, treating any nonzero exit as an error; returns stdout bytes.
    pub fn run(&self, args: &[&str]) -> Result<Vec<u8>> {
        let out = self.output(args)?;
        if !out.status.success() {
            bail!(
                "git {} failed ({}): {}",
                args.join(" "),
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(out.stdout)
    }

    /// Trimmed UTF-8 stdout for single-value plumbing like rev-parse.
    pub fn run_str(&self, args: &[&str]) -> Result<String> {
        let out = self.run(args)?;
        Ok(String::from_utf8(out)
            .with_context(|| format!("git {}: stdout not UTF-8", args.join(" ")))?
            .trim_end()
            .to_string())
    }

    /// Raw [`Output`] for callers that interpret exit codes themselves
    /// (`merge-tree` exits 1 on conflicts, `diff --quiet` on differences).
    pub fn output(&self, args: &[&str]) -> Result<Output> {
        self.command(args)
            .output()
            .with_context(|| format!("failed to spawn git {}", args.join(" ")))
    }

    /// Like [`Git::run`] but feeds `input` on stdin. Stdin is written from a
    /// thread so a command that also fills stdout/stderr cannot deadlock.
    pub fn run_with_stdin(&self, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn git {}", args.join(" ")))?;
        let mut stdin = child.stdin.take().expect("stdin piped");
        let buf = input.to_vec();
        let writer = std::thread::spawn(move || stdin.write_all(&buf));
        let out = child.wait_with_output().context("waiting for git")?;
        writer
            .join()
            .map_err(|_| anyhow!("stdin writer panicked"))?
            .context("writing to git stdin")?;
        if !out.status.success() {
            bail!(
                "git {} failed ({}): {}",
                args.join(" "),
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(out.stdout)
    }
}
