use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::process::Git;

const MIN_GIT_VERSION: GitVersion = GitVersion::new(2, 40, 0);

/// Result type returned by the typed git adapter.
pub type GitResult<T> = std::result::Result<T, GitError>;

/// Errors surfaced by typed git plumbing operations.
#[derive(Debug, Error)]
pub enum GitError {
    /// Git is older than the merge-tree floor required by bpatch.
    #[error(
        "git {actual} is too old; bpatch requires git >= {required} because it uses `git merge-tree --write-tree --merge-base`; upgrade git and retry"
    )]
    UnsupportedGitVersion {
        /// Version reported by `git version`.
        actual: GitVersion,
        /// Minimum version accepted by bpatch.
        required: GitVersion,
    },
    /// `git version` output did not contain a parseable semantic version.
    #[error("could not parse git version from {output:?}")]
    InvalidGitVersion {
        /// Raw version output.
        output: String,
    },
    /// A path could not be passed to the low-level runner.
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(PathBuf),
    /// The real index was not at the expected old tree before materialization.
    #[error(
        "index tree does not match expected old tree {expected}; current index is {actual}. Clear staged changes or reset the index before materializing"
    )]
    IndexTreeMismatch {
        /// Tree id required before `read-tree -m -u`.
        expected: String,
        /// Tree id currently represented by the index.
        actual: String,
    },
    /// `merge-tree` exited with a real failure rather than a conflict.
    #[error("git merge-tree failed ({status}): {stderr}")]
    MergeTreeFailed {
        /// Process exit status.
        status: String,
        /// Stderr from git.
        stderr: String,
    },
    /// `diff-index` exited with a real failure rather than a difference.
    #[error("git diff-index failed ({status}): {stderr}")]
    DiffIndexFailed {
        /// Process exit status.
        status: String,
        /// Stderr from git.
        stderr: String,
    },
    /// `merge-tree -z` returned output that did not match git's record format.
    #[error("git merge-tree returned malformed -z output: {0}")]
    MalformedMergeTree(String),
    /// `diff-tree -z` returned output that did not match git's record format.
    #[error("git diff-tree returned malformed -z output: {0}")]
    MalformedDiffTree(String),
    /// A read-side git command returned output that could not be parsed.
    #[error("git returned malformed output: {0}")]
    MalformedGitOutput(String),
    /// Filesystem operation failed while setting up git plumbing.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The low-level process runner reported a git failure.
    #[error(transparent)]
    Git(#[from] anyhow::Error),
}

/// Parsed `git version` value used by preflight checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl GitVersion {
    /// Creates a git version value.
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Returns the major version component.
    pub const fn major(self) -> u64 {
        self.major
    }

    /// Returns the minor version component.
    pub const fn minor(self) -> u64 {
        self.minor
    }

    /// Returns the patch version component.
    pub const fn patch(self) -> u64 {
        self.patch
    }

    /// Parses the stdout produced by `git version`.
    pub fn parse_output(output: &str) -> GitResult<Self> {
        let token = output
            .split_whitespace()
            .find(|part| part.as_bytes().first().is_some_and(u8::is_ascii_digit))
            .ok_or_else(|| GitError::InvalidGitVersion {
                output: output.to_string(),
            })?;
        let version = token
            .trim_end_matches(|ch: char| !ch.is_ascii_digit())
            .split(['.', '-'])
            .take(3)
            .map(str::parse::<u64>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| GitError::InvalidGitVersion {
                output: output.to_string(),
            })?;
        if version.len() < 2 {
            return Err(GitError::InvalidGitVersion {
                output: output.to_string(),
            });
        }
        Ok(Self::new(
            version[0],
            version[1],
            version.get(2).copied().unwrap_or(0),
        ))
    }
}

impl fmt::Display for GitVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Typed adapter over system git plumbing used by bpatch.
#[derive(Clone, Debug)]
pub struct GitAdapter {
    git: Git,
}

impl GitAdapter {
    /// Creates an adapter rooted at a git worktree.
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        Self {
            git: Git::new(repo),
        }
    }

    /// Wraps an existing low-level git runner.
    pub fn from_process(git: Git) -> Self {
        Self { git }
    }

    /// Returns the underlying low-level git runner.
    pub fn process(&self) -> &Git {
        &self.git
    }

    /// Verifies that system git supports all plumbing bpatch depends on.
    pub fn preflight(&self) -> GitResult<GitVersion> {
        Self::preflight_version_output(&self.git.run_str(&["version"])?)
    }

    /// Verifies injected `git version` stdout, for tests and diagnostics.
    pub fn preflight_version_output(output: &str) -> GitResult<GitVersion> {
        let version = GitVersion::parse_output(output)?;
        if version < MIN_GIT_VERSION {
            return Err(GitError::UnsupportedGitVersion {
                actual: version,
                required: MIN_GIT_VERSION,
            });
        }
        Ok(version)
    }

    /// Resolves a revision to its object id.
    pub fn rev_parse(&self, rev: &str) -> GitResult<String> {
        Ok(self.git.run_str(&["rev-parse", rev])?)
    }

    /// Resolves HEAD to its object id.
    pub fn head_rev(&self) -> GitResult<String> {
        self.rev_parse("HEAD")
    }

    /// Resolves a revision to git's short object id.
    pub fn short_rev(&self, rev: &str) -> GitResult<String> {
        Ok(self.git.run_str(&["rev-parse", "--short", rev])?)
    }

    /// Resolves a commit or tree-ish to its tree id.
    pub fn tree_id(&self, treeish: &str) -> GitResult<String> {
        let rev = format!("{treeish}^{{tree}}");
        Ok(self.git.run_str(&["rev-parse", &rev])?)
    }

    /// Returns a commit's subject line.
    pub fn commit_subject(&self, rev: &str) -> GitResult<String> {
        Ok(self.git.run_str(&["log", "-1", "--format=%s", rev])?)
    }

    /// Returns a file from a tree-ish, or `None` when the path is absent.
    pub fn show_file(&self, treeish: &str, path: &str) -> GitResult<Option<Vec<u8>>> {
        let spec = format!("{treeish}:{path}");
        let out = self.git.output(&["show", &spec])?;
        if out.status.success() {
            Ok(Some(out.stdout))
        } else {
            Ok(None)
        }
    }

    /// Lists first-parent commits for a range, bounded when `max_count` is set.
    pub fn first_parent_commits(
        &self,
        range: Option<&str>,
        max_count: Option<usize>,
    ) -> GitResult<Vec<String>> {
        let mut args = vec![
            "log".to_string(),
            "--first-parent".to_string(),
            "--format=%H".to_string(),
        ];
        if let Some(max_count) = max_count {
            args.push(format!("--max-count={max_count}"));
        }
        if let Some(range) = range {
            args.push(range.to_string());
        }
        let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        Ok(self
            .git
            .run_str(&refs)?
            .lines()
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect())
    }

    /// Counts revisions in a rev-list range.
    pub fn rev_list_count(&self, range: &str) -> GitResult<usize> {
        self.git
            .run_str(&["rev-list", "--count", range])?
            .parse()
            .map_err(|_| GitError::MalformedGitOutput(format!("invalid rev-list count {range}")))
    }

    /// Refreshes the index stat cache before worktree drift checks.
    pub fn refresh_index(&self) -> GitResult<()> {
        self.git.run(&["update-index", "-q", "--refresh"])?;
        Ok(())
    }

    /// Returns `git status --porcelain -z` for exact uncommitted paths.
    pub fn status_porcelain_z(&self) -> GitResult<Vec<u8>> {
        Ok(self.git.run(&["status", "--porcelain", "-z", "-uall"])?)
    }

    /// Builds a tree by applying patches to a base in a temporary index.
    pub fn build_tree_from_patches<P>(&self, base: &str, patches: &[P]) -> GitResult<String>
    where
        P: AsRef<Path>,
    {
        let git_dir = self.git_dir()?;
        let temp = tempfile::Builder::new()
            .prefix("bpatch-index-")
            .tempfile_in(git_dir)?;
        let index_path = temp.into_temp_path();
        fs::remove_file(&index_path)?;

        let indexed = self
            .git
            .with_env("GIT_INDEX_FILE", index_path.as_os_str().to_os_string());
        indexed.run(&["read-tree", base])?;
        for patch in patches {
            let patch = path_arg(patch.as_ref())?;
            indexed.run(&["apply", "--cached", "--whitespace=nowarn", patch])?;
        }
        Ok(indexed.run_str(&["write-tree"])?)
    }

    /// Merges two trees with an explicit base and returns git's merged tree plus conflicts.
    pub fn merge_trees(&self, base: &str, ours: &str, theirs: &str) -> GitResult<MergeTreeResult> {
        let output = self.git.output(&[
            "merge-tree",
            "--write-tree",
            "-z",
            "--merge-base",
            base,
            ours,
            theirs,
        ])?;
        match output.status.code() {
            Some(0 | 1) => parse_merge_tree_output(&output.stdout),
            _ => Err(GitError::MergeTreeFailed {
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            }),
        }
    }

    /// Materializes a two-tree update after verifying the index is at the old tree.
    pub fn materialize_tree_delta(
        &self,
        old_tree: &str,
        new_tree: &str,
    ) -> GitResult<MaterializeResult> {
        let expected = self.tree_id(old_tree)?;
        let actual = self.git.run_str(&["write-tree"])?;
        if actual != expected {
            return Err(GitError::IndexTreeMismatch { expected, actual });
        }

        let changed_files = self.diff_tree_name_status(old_tree, new_tree)?;
        self.git
            .run(&["read-tree", "-m", "-u", old_tree, new_tree])?;
        Ok(MaterializeResult { changed_files })
    }

    /// Lists path-level name/status changes between two trees.
    pub fn diff_tree_name_status(
        &self,
        old_tree: &str,
        new_tree: &str,
    ) -> GitResult<Vec<TreeDiffEntry>> {
        let out = self
            .git
            .run(&["diff-tree", "-r", "--name-status", "-z", old_tree, new_tree])?;
        parse_diff_tree_name_status(&out)
    }

    /// Reports whether the worktree or index differs from a tree.
    pub fn diff_index_has_changes(&self, tree: &str) -> GitResult<bool> {
        let output = self.git.output(&["diff-index", "--quiet", tree, "--"])?;
        match output.status.code() {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(GitError::DiffIndexFailed {
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            }),
        }
    }

    /// Parses trailers from a commit message using git's trailer rules.
    pub fn commit_trailers(&self, rev: &str) -> GitResult<Vec<Trailer>> {
        let message = self.git.run(&["log", "-1", "--format=%B", rev])?;
        let parsed = self
            .git
            .run_with_stdin(&["interpret-trailers", "--parse"], &message)?;
        let text = String::from_utf8_lossy(&parsed);
        Ok(text
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once(':')?;
                Some(Trailer {
                    key: key.trim().to_string(),
                    value: value.trim().to_string(),
                })
            })
            .collect())
    }

    fn git_dir(&self) -> GitResult<PathBuf> {
        let git_dir = PathBuf::from(self.git.run_str(&["rev-parse", "--git-dir"])?);
        if git_dir.is_absolute() {
            Ok(git_dir)
        } else {
            Ok(self.git.repo().join(git_dir))
        }
    }
}

/// Result of `merge-tree --write-tree`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeTreeResult {
    /// Tree id produced by git, even when conflicts are present.
    pub merged_tree_sha: String,
    /// Structured conflict records parsed from merge-tree messages.
    pub conflicts: Vec<MergeConflict>,
}

/// One conflicted path reported by `merge-tree`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeConflict {
    /// Repository-relative conflicted path.
    pub file: PathBuf,
    /// Normalized conflict kind such as `content`.
    pub kind: String,
}

/// Files changed between two trees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializeResult {
    /// Name/status entries that git says differ between old and new trees.
    pub changed_files: Vec<TreeDiffEntry>,
}

/// One `diff-tree --name-status` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeDiffEntry {
    /// Name/status code such as `M`, `A`, `D`, or `R100`.
    pub status: String,
    /// New path for normal entries, or destination path for rename/copy entries.
    pub path: PathBuf,
    /// Source path for rename/copy entries.
    pub old_path: Option<PathBuf>,
}

/// One parsed commit trailer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trailer {
    /// Trailer key without the colon.
    pub key: String,
    /// Trailer value after trimming surrounding whitespace.
    pub value: String,
}

fn parse_merge_tree_output(output: &[u8]) -> GitResult<MergeTreeResult> {
    let mut fields = output.split(|byte| *byte == 0);
    let merged_tree_sha = next_utf8(&mut fields, "missing merged tree id")?.to_string();
    if merged_tree_sha.is_empty() {
        return Err(GitError::MalformedMergeTree(
            "missing merged tree id".to_string(),
        ));
    }

    for field in fields.by_ref() {
        if field.is_empty() {
            break;
        }
    }

    let mut conflicts = Vec::new();
    while let Some(count_field) = fields.next() {
        if count_field.is_empty() {
            break;
        }
        let path_count = utf8(count_field, "message path count")?
            .parse::<usize>()
            .map_err(|_| GitError::MalformedMergeTree("invalid message path count".to_string()))?;
        let mut paths = Vec::with_capacity(path_count);
        for _ in 0..path_count {
            paths.push(PathBuf::from(next_utf8(
                &mut fields,
                "missing message path",
            )?));
        }
        let label = next_utf8(&mut fields, "missing message label")?;
        let _message = next_utf8(&mut fields, "missing message text")?;
        if let Some(kind) = conflict_kind(label) {
            for file in paths {
                if !conflicts
                    .iter()
                    .any(|conflict: &MergeConflict| conflict.file == file && conflict.kind == kind)
                {
                    conflicts.push(MergeConflict {
                        file,
                        kind: kind.clone(),
                    });
                }
            }
        }
    }

    Ok(MergeTreeResult {
        merged_tree_sha,
        conflicts,
    })
}

fn parse_diff_tree_name_status(output: &[u8]) -> GitResult<Vec<TreeDiffEntry>> {
    let mut fields = output.split(|byte| *byte == 0);
    let mut entries = Vec::new();
    while let Some(status_field) = fields.next() {
        if status_field.is_empty() {
            break;
        }
        let status = utf8_diff(status_field, "diff-tree status")?.to_string();
        if status.starts_with('R') || status.starts_with('C') {
            let old_path = PathBuf::from(next_diff_utf8(&mut fields, "missing source path")?);
            let path = PathBuf::from(next_diff_utf8(&mut fields, "missing destination path")?);
            entries.push(TreeDiffEntry {
                status,
                path,
                old_path: Some(old_path),
            });
        } else {
            let path = PathBuf::from(next_diff_utf8(&mut fields, "missing path")?);
            entries.push(TreeDiffEntry {
                status,
                path,
                old_path: None,
            });
        }
    }
    Ok(entries)
}

fn conflict_kind(label: &str) -> Option<String> {
    let raw = label
        .strip_prefix("CONFLICT (")?
        .split_once(')')?
        .0
        .to_ascii_lowercase();
    Some(match raw.as_str() {
        "contents" => "content".to_string(),
        other => other.to_string(),
    })
}

fn path_arg(path: &Path) -> GitResult<&str> {
    path.to_str()
        .ok_or_else(|| GitError::NonUtf8Path(path.to_owned()))
}

fn next_utf8<'a>(
    fields: &mut std::slice::Split<'a, u8, impl FnMut(&u8) -> bool>,
    context: &str,
) -> GitResult<&'a str> {
    let field = fields
        .next()
        .ok_or_else(|| GitError::MalformedMergeTree(context.to_string()))?;
    utf8(field, context)
}

fn next_diff_utf8<'a>(
    fields: &mut std::slice::Split<'a, u8, impl FnMut(&u8) -> bool>,
    context: &str,
) -> GitResult<&'a str> {
    let field = fields
        .next()
        .ok_or_else(|| GitError::MalformedDiffTree(context.to_string()))?;
    utf8_diff(field, context)
}

fn utf8<'a>(field: &'a [u8], context: &str) -> GitResult<&'a str> {
    std::str::from_utf8(field)
        .map_err(|_| GitError::MalformedMergeTree(format!("{context} is not UTF-8")))
}

fn utf8_diff<'a>(field: &'a [u8], context: &str) -> GitResult<&'a str> {
    std::str::from_utf8(field)
        .map_err(|_| GitError::MalformedDiffTree(format!("{context} is not UTF-8")))
}
