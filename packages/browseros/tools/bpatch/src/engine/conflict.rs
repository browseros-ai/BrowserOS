use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::engine::apply::{
    AuthorCommitsInput, AuthoredCommit, CommitTrailerMode, SubjectMode, author_feature_commits,
    finalize_head, untracked_add_collisions,
};
use crate::engine::progress::ProgressEvent;
use crate::engine::state::{DriftFile, DriftSource, StateContext};
use crate::git::GitAdapter;
use crate::process::Git;
use crate::store::{FeatureMatch, Store};

const SESSION_FILE: &str = "session.json";

/// Conflict session persisted under `.git/bpatch/session.json`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConflictSession {
    /// New chromium base checked out before apply began.
    pub new_base: String,
    /// Human display for the new base.
    pub new_base_display: String,
    /// Store-pinned old chromium base.
    pub pin_base: String,
    /// Store repository revision used for the merge.
    pub store_rev: String,
    /// Tree produced by git merge-tree, with marker blobs for conflicted paths.
    pub merged_tree: String,
    /// Store target tree computed on the old pinned base.
    pub target_tree: String,
    /// Conflicts found by merge-tree.
    pub conflicts: Vec<ConflictFile>,
    /// HEAD before conflict-session commands mutate anything.
    pub parent_head: String,
    /// Unix timestamp recorded when the session was created.
    pub created_at: u64,
    /// Whether conflicted blobs have been written into the worktree.
    #[serde(default)]
    pub materialized: bool,
}

/// One conflicted file in a persisted session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConflictFile {
    /// Repository-relative path.
    pub file: PathBuf,
    /// Feature owning the path, or `(unassigned)`.
    pub feature: String,
    /// Conflict kind from git.
    pub kind: String,
}

/// Result of beginning an out-of-worktree conflict session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BeginResult {
    /// Human display for the new chromium base.
    pub base_display: String,
    /// Clean store-managed files merged without conflicts.
    pub merged: usize,
    /// Conflicts persisted in the session.
    pub conflicts: Vec<ConflictFile>,
    /// Whether the worktree was touched while beginning the session.
    pub worktree_touched: bool,
}

/// Result of deleting a conflict session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AbortOutcome {
    /// A session existed and was removed.
    Aborted,
    /// No session file existed.
    NoSession,
}

/// Result of continuing a conflict session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContinueOutcome {
    /// Conflict marker blobs were written to conflicted files.
    Materialized(MaterializedConflicts),
    /// Resolved conflicts were folded into the final tree and committed.
    Completed(CompletedConflict),
    /// One or more conflicted files still contain conflict markers.
    Unresolved(UnresolvedConflicts),
    /// Final continue was attempted before marker files were materialized.
    NotMaterialized(NotMaterializedConflicts),
    /// The index or worktree has edits that would be overwritten.
    Drift(ConflictDrift),
    /// No session file existed.
    NoSession,
}

/// Materialize-only continue result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedConflicts {
    /// Number of conflicted files written.
    pub files_written: usize,
    /// Clean store-managed files waiting for convergence.
    pub clean_files: usize,
}

/// Successful final continue result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedConflict {
    /// Human display for the new chromium base.
    pub base_display: String,
    /// Short store revision used in trailers.
    pub store_short_rev: String,
    /// Authored feature commits.
    pub commits: Vec<AuthoredCommit>,
}

/// Unresolved-marker refusal result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnresolvedConflicts {
    /// Conflicted files that still contain marker lines.
    pub files: Vec<PathBuf>,
}

/// Refusal result when a session has not been materialized.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotMaterializedConflicts {
    /// Human-readable recovery guidance.
    pub reason: String,
}

/// Drift refusal result for conflict-session commands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictDrift {
    /// Drift entries that blocked conflict-session writes.
    pub files: Vec<DriftFile>,
}

struct ResolvedBlob {
    path: PathBuf,
    mode: String,
    oid: String,
}

/// Begins a base-bump conflict session without writing the worktree.
pub fn begin(
    ctx: &StateContext,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> Result<BeginResult> {
    if session_path(&ctx.checkout)?.exists() {
        bail!("conflict session already exists");
    }

    let state = crate::engine::state::resolve(ctx)?;
    let store = Store::load(&ctx.store_dir)?;
    if store.metadata().base_commit == state.base.sha {
        bail!("store base matches checkout base; no conflict session needed");
    }
    if state.applied.is_some() {
        let git = GitAdapter::new(&ctx.checkout);
        let store_short = git.short_rev(&store.metadata().base_commit)?;
        bail!(
            "store base pin moved to {} ({}) but this checkout is converged on {} — check out the new base first: `git checkout {} && gclient sync`, then `bpatch apply`",
            store.metadata().base_version,
            store_short,
            state.base.display,
            store.metadata().base_commit
        );
    }

    let git = GitAdapter::new(&ctx.checkout);
    let patches = store
        .patches()
        .values()
        .map(|patch| ctx.store_dir.join(&patch.path))
        .collect::<Vec<_>>();

    progress(ProgressEvent::Start {
        phase: "merge",
        total: Some(store.patches().len()),
    });
    let target_tree = git
        .build_tree_from_patches(&store.metadata().base_commit, &patches)
        .context("building target tree from store patches for base-bump merge")?;
    let target_commit = commit_tree(
        git.process(),
        &target_tree,
        &[store.metadata().base_commit.as_str()],
        "bpatch target tree for base-bump merge",
    )?;
    let merge = git.merge_trees(
        &store.metadata().base_commit,
        &state.head_rev,
        &target_commit,
    )?;
    progress(ProgressEvent::End { phase: "merge" });

    let conflicts = merge
        .conflicts
        .into_iter()
        .map(|conflict| {
            let feature = feature_for_path(&store, &conflict.file);
            ConflictFile {
                file: conflict.file,
                feature,
                kind: conflict.kind,
            }
        })
        .collect::<Vec<_>>();
    let merged = store.patches().len().saturating_sub(conflicts.len());
    let session = ConflictSession {
        new_base: state.head_rev.clone(),
        new_base_display: state.base.display.clone(),
        pin_base: store.metadata().base_commit.clone(),
        store_rev: state.store.head_rev,
        merged_tree: merge.merged_tree_sha,
        target_tree,
        conflicts: conflicts.clone(),
        parent_head: state.head_rev.clone(),
        created_at: now_epoch_seconds(),
        materialized: false,
    };
    save_session(&ctx.checkout, &session)?;

    Ok(BeginResult {
        base_display: state.base.display,
        merged,
        conflicts,
        worktree_touched: false,
    })
}

/// Deletes the conflict session file if present.
pub fn abort(checkout: impl AsRef<Path>) -> Result<AbortOutcome> {
    let path = session_path(checkout)?;
    if !path.exists() {
        return Ok(AbortOutcome::NoSession);
    }
    fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    Ok(AbortOutcome::Aborted)
}

/// Continues a conflict session, either materializing markers or completing convergence.
pub fn continue_session(
    ctx: &StateContext,
    materialize: bool,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> Result<ContinueOutcome> {
    let Some(session) = load_session(&ctx.checkout)? else {
        return Ok(ContinueOutcome::NoSession);
    };

    if materialize {
        return materialize_conflicts(ctx, &session, progress);
    }

    if !session.materialized {
        return Ok(ContinueOutcome::NotMaterialized(NotMaterializedConflicts {
            reason: "conflicts were never materialized — run `bpatch continue --materialize`, resolve, then continue".to_string(),
        }));
    }

    let unresolved = unresolved_marker_files(&ctx.checkout, &session)?;
    if !unresolved.is_empty() {
        return Ok(ContinueOutcome::Unresolved(UnresolvedConflicts {
            files: unresolved,
        }));
    }

    let git = GitAdapter::new(&ctx.checkout);
    let store = Store::load(&ctx.store_dir)?;
    let resolved = resolved_blobs(&git, &ctx.checkout, &session)?;
    let final_tree = final_tree_with_resolutions(&git, &session, &resolved)?;

    let new_base_tree = git.tree_id(&session.new_base)?;
    let delta = git.diff_tree_name_status(&new_base_tree, &final_tree)?;
    let collisions = untracked_add_collisions(&git, &delta)?;
    if !collisions.is_empty() {
        return Ok(ContinueOutcome::Drift(ConflictDrift { files: collisions }));
    }

    let chain = author_feature_commits(
        AuthorCommitsInput {
            checkout: &ctx.checkout,
            store: &store,
            base: &session.new_base,
            applied_tree: &new_base_tree,
            target_tree: &final_tree,
            trailers: CommitTrailerMode::Apply {
                store_rev: &session.store_rev,
            },
            subject_mode: SubjectMode::FeatureName,
            parent_commit: &session.parent_head,
            delta: &delta,
        },
        progress,
    )?;

    update_real_index_with_resolutions(git.process(), &resolved)
        .context("staging resolved conflict files failed; recover with `git update-index --index-info` for the resolved paths")?;
    let old_prime = git.process().run_str(&["write-tree"])?;

    let clean_delta = git.diff_tree_name_status(&old_prime, &final_tree)?;
    progress(ProgressEvent::Start {
        phase: "materialize",
        total: Some(clean_delta.len()),
    });
    git.materialize_tree_delta(&old_prime, &final_tree)
        .with_context(|| {
            format!(
                "materializing clean conflict remainder failed; recover with `git read-tree -m -u {old_prime} {final_tree}`"
            )
        })?;
    progress(ProgressEvent::End {
        phase: "materialize",
    });
    finalize_head(
        &ctx.checkout,
        &session.parent_head,
        &chain.final_sha,
        &final_tree,
    )?;

    let store_short_rev = GitAdapter::new(&ctx.store_dir).short_rev(&session.store_rev)?;
    abort(&ctx.checkout)?;

    Ok(ContinueOutcome::Completed(CompletedConflict {
        base_display: session.new_base_display,
        store_short_rev,
        commits: chain.commits,
    }))
}

/// Returns the persisted conflict session, if any.
pub fn load_session(checkout: impl AsRef<Path>) -> Result<Option<ConflictSession>> {
    let path = session_path(checkout)?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Some(
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?,
    ))
}

/// Returns `.git/bpatch/session.json` for a checkout.
pub fn session_path(checkout: impl AsRef<Path>) -> Result<PathBuf> {
    Ok(bpatch_dir(checkout.as_ref())?.join(SESSION_FILE))
}

fn save_session(checkout: &Path, session: &ConflictSession) -> Result<()> {
    let path = session_path(checkout)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&path, serde_json::to_vec_pretty(session)?)
        .with_context(|| format!("writing {}", path.display()))
}

fn materialize_conflicts(
    ctx: &StateContext,
    session: &ConflictSession,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> Result<ContinueOutcome> {
    let git = GitAdapter::new(&ctx.checkout);
    let drift = uncommitted_conflict_drift(&git, session)?;
    if !drift.is_empty() {
        return Ok(ContinueOutcome::Drift(ConflictDrift { files: drift }));
    }

    progress(ProgressEvent::Start {
        phase: "materialize",
        total: Some(session.conflicts.len()),
    });
    for (index, conflict) in session.conflicts.iter().enumerate() {
        let file = conflict
            .file
            .to_str()
            .ok_or_else(|| anyhow!("conflict path is not UTF-8: {}", conflict.file.display()))?;
        let Some(bytes) = git.show_file(&session.merged_tree, file)? else {
            bail!("merged tree has no conflicted file {file}");
        };
        let path = ctx.checkout.join(&conflict.file);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
        progress(ProgressEvent::Tick {
            phase: "materialize",
            done: index + 1,
            total: Some(session.conflicts.len()),
            item: Some(file),
        });
    }
    progress(ProgressEvent::End {
        phase: "materialize",
    });

    let mut materialized = session.clone();
    materialized.materialized = true;
    save_session(&ctx.checkout, &materialized)?;

    Ok(ContinueOutcome::Materialized(MaterializedConflicts {
        files_written: session.conflicts.len(),
        clean_files: clean_file_count(ctx, session)?,
    }))
}

fn unresolved_marker_files(checkout: &Path, session: &ConflictSession) -> Result<Vec<PathBuf>> {
    let mut unresolved = Vec::new();
    for conflict in &session.conflicts {
        let path = checkout.join(&conflict.file);
        let contents = fs::read_to_string(&path).unwrap_or_default();
        if contents.lines().any(is_marker_line) {
            unresolved.push(conflict.file.clone());
        }
    }
    Ok(unresolved)
}

fn is_marker_line(line: &str) -> bool {
    line.starts_with("<<<<<<<") || line.starts_with(">>>>>>>")
}

fn uncommitted_conflict_drift(
    git: &GitAdapter,
    session: &ConflictSession,
) -> Result<Vec<DriftFile>> {
    let conflict_paths = session
        .conflicts
        .iter()
        .map(|conflict| conflict.file.clone())
        .collect::<BTreeSet<_>>();
    if conflict_paths.is_empty() {
        return Ok(Vec::new());
    }

    git.refresh_index()?;
    let status = git.status_porcelain_z()?;
    let mut parts = status.split(|byte| *byte == 0);
    let mut files = Vec::new();
    let mut seen = BTreeSet::new();
    while let Some(record) = parts.next() {
        if record.is_empty() {
            break;
        }
        let text = std::str::from_utf8(record)?;
        if text.len() < 4 {
            continue;
        }
        let status = &text[..2];
        let path = PathBuf::from(&text[3..]);
        if status.starts_with('R') || status.starts_with('C') {
            let _old_path = parts.next();
        }
        if conflict_paths.contains(&path) && seen.insert(path.clone()) {
            files.push(DriftFile {
                path,
                status: status.trim().to_string(),
                source: DriftSource::Uncommitted,
                annotation: "modified, uncommitted".to_string(),
            });
        }
    }
    Ok(files)
}

fn resolved_blobs(
    git: &GitAdapter,
    checkout: &Path,
    session: &ConflictSession,
) -> Result<Vec<ResolvedBlob>> {
    session
        .conflicts
        .iter()
        .map(|conflict| {
            let conflict_path = path_arg(&conflict.file)?;
            let mode = tree_file_mode(git.process(), &session.merged_tree, conflict_path)?;
            let full_path = checkout.join(&conflict.file);
            let full_arg = path_arg(&full_path)?;
            let oid = git.process().run_str(&["hash-object", "-w", full_arg])?;
            Ok(ResolvedBlob {
                path: conflict.file.clone(),
                mode,
                oid,
            })
        })
        .collect()
}

fn final_tree_with_resolutions(
    git: &GitAdapter,
    session: &ConflictSession,
    resolved: &[ResolvedBlob],
) -> Result<String> {
    let git_dir = git_dir(git.process())?;
    let temp = tempfile::Builder::new()
        .prefix("bpatch-conflict-index-")
        .tempfile_in(git_dir)?;
    let index_path = temp.into_temp_path();
    fs::remove_file(&index_path)?;
    let indexed = git
        .process()
        .with_env("GIT_INDEX_FILE", index_path.as_os_str().to_os_string());
    indexed.run(&["read-tree", &session.merged_tree])?;
    indexed.run_with_stdin(
        &["update-index", "--index-info"],
        index_info(resolved)?.as_bytes(),
    )?;
    indexed.run_str(&["write-tree"])
}

fn update_real_index_with_resolutions(git: &Git, resolved: &[ResolvedBlob]) -> Result<()> {
    git.run_with_stdin(
        &["update-index", "--index-info"],
        index_info(resolved)?.as_bytes(),
    )?;
    Ok(())
}

fn index_info(resolved: &[ResolvedBlob]) -> Result<String> {
    let mut out = String::new();
    for blob in resolved {
        out.push_str(&blob.mode);
        out.push(' ');
        out.push_str(&blob.oid);
        out.push('\t');
        out.push_str(path_arg(&blob.path)?);
        out.push('\n');
    }
    Ok(out)
}

fn clean_file_count(ctx: &StateContext, session: &ConflictSession) -> Result<usize> {
    let git = GitAdapter::new(&ctx.checkout);
    let target_delta = git.diff_tree_name_status(&session.pin_base, &session.target_tree)?;
    Ok(target_delta.len().saturating_sub(session.conflicts.len()))
}

fn feature_for_path(store: &Store, path: &Path) -> String {
    let path = path.to_string_lossy();
    match store.match_path(&path) {
        FeatureMatch::Matched { feature, .. } => feature,
        FeatureMatch::Unmatched { .. } => {
            crate::engine::state::unassigned_feature_name().to_string()
        }
    }
}

fn commit_tree(git: &Git, tree: &str, parents: &[&str], subject: &str) -> Result<String> {
    let mut args = vec!["commit-tree".to_string(), tree.to_string()];
    for parent in parents {
        args.push("-p".to_string());
        args.push((*parent).to_string());
    }
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let sha = git.run_with_stdin(&refs, format!("{subject}\n").as_bytes())?;
    Ok(String::from_utf8(sha)
        .context("commit-tree output was not UTF-8")?
        .trim()
        .to_string())
}

fn tree_file_mode(git: &Git, tree: &str, path: &str) -> Result<String> {
    let raw = git.run(&["ls-tree", "-z", tree, "--", path])?;
    let first = raw
        .split(|byte| *byte == 0)
        .find(|field| !field.is_empty())
        .ok_or_else(|| anyhow!("ls-tree returned no record for {path}"))?;
    let record = std::str::from_utf8(first).context("ls-tree output was not UTF-8")?;
    let (metadata, _) = record
        .split_once('\t')
        .ok_or_else(|| anyhow!("malformed ls-tree record for {path}"))?;
    metadata
        .split_whitespace()
        .next()
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("missing mode in ls-tree record for {path}"))
}

fn git_dir(git: &Git) -> Result<PathBuf> {
    let git_dir = PathBuf::from(git.run_str(&["rev-parse", "--git-dir"])?);
    if git_dir.is_absolute() {
        Ok(git_dir)
    } else {
        Ok(git.repo().join(git_dir))
    }
}

fn bpatch_dir(checkout: &Path) -> Result<PathBuf> {
    Ok(git_dir(&Git::new(checkout))?.join("bpatch"))
}

fn path_arg(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| anyhow!("path is not UTF-8: {}", path.display()))
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
