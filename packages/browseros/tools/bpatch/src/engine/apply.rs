use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::engine::progress::ProgressEvent;
use crate::engine::state::{
    BpatchCommitKind, DriftFile, DriftSource, ResolvedState, StateContext,
    format_annotate_trailers, format_apply_trailers, format_state_apply_trailers,
    parse_bpatch_authored_base, unassigned_feature_name,
};
use crate::git::{GitAdapter, TreeDiffEntry};
use crate::process::Git;
use crate::store::{FeatureMatch, Store};

/// Options controlling a same-base apply run.
#[derive(Clone, Copy, Debug, Default)]
pub struct ApplyOptions {
    /// Fast-forward the store repository before resolving state.
    pub pull: bool,
}

/// Result of planning and applying the current store state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplyOutcome {
    /// The checkout already matches the current store tree.
    Converged(ConvergedApply),
    /// Store changes were materialized and feature commits were authored.
    Applied(AppliedApply),
    /// Store and checkout base pins differ; Task 6 owns this path.
    BaseMismatch(BaseMismatch),
    /// The checkout has committed or tracked uncommitted drift.
    Drift(DriftApply),
}

/// No-op same-base apply result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConvergedApply {
    /// Store repository HEAD applied by this state.
    pub store_rev: String,
    /// Short store repository HEAD.
    pub store_short_rev: String,
    /// Target tree built from the store patches.
    pub target_tree: String,
}

/// Successful apply result with authored commits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedApply {
    /// Store repository HEAD used for trailers.
    pub store_rev: String,
    /// Short store repository HEAD.
    pub store_short_rev: String,
    /// Chromium base commit used for convergence.
    pub base: String,
    /// Human base display string.
    pub base_display: String,
    /// Applied store revision before this run, when any.
    pub previous_store_short_rev: Option<String>,
    /// Files changed between the current checkout and target tree.
    pub files_changed: usize,
    /// Store-managed file count loaded from the store.
    pub store_managed_files: usize,
    /// Final checkout tree materialized by this apply.
    pub target_tree: String,
    /// Feature commits authored by this run.
    pub commits: Vec<AuthoredCommit>,
}

/// Checkout/store base mismatch details.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BaseMismatch {
    /// Base commit recorded by the checkout's current applied state.
    pub checkout_base: String,
    /// Base commit pinned in .store.yaml.
    pub store_base: String,
}

/// Drift refusal result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DriftApply {
    /// Drift files reported by state resolution.
    pub files: Vec<DriftFile>,
}

/// Input for reusable commit-tree authoring.
pub struct AuthorCommitsInput<'a> {
    /// Chromium checkout root.
    pub checkout: &'a Path,
    /// Loaded patch store used for feature grouping.
    pub store: &'a Store,
    /// Chromium base commit to write into trailers.
    pub base: &'a str,
    /// Tree currently represented by the parent apply state.
    pub applied_tree: &'a str,
    /// Final tree that the authored commit chain must reach.
    pub target_tree: &'a str,
    /// Trailer block written to each authored commit.
    pub trailers: CommitTrailerMode<'a>,
    /// Subject source used when building feature commit messages.
    pub subject_mode: SubjectMode,
    /// Parent commit for the first authored feature commit.
    pub parent_commit: &'a str,
    /// Files changed between `applied_tree` and `target_tree`.
    pub delta: &'a [TreeDiffEntry],
}

/// Trailer style for commit-tree authored bpatch commits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitTrailerMode<'a> {
    /// Apply commits record the store revision and materialized checkout tree.
    Apply { store_rev: &'a str },
    /// Annotate commits record only the base plus an annotation marker.
    Annotate,
}

/// Subject source for grouped feature commits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubjectMode {
    /// Use the stable `feat: <feature>` subject style.
    FeatureName,
    /// Use `.features.yaml` descriptions when present.
    FeatureDescription,
}

/// Commit authored for one feature group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoredCommit {
    /// Feature name, or `(unassigned)` for unmatched files.
    pub feature: String,
    /// Sequence number derived from previous apply-authored commits.
    pub seq: usize,
    /// Full commit sha.
    pub sha: String,
    /// Short commit sha.
    pub short_sha: String,
    /// Commit subject written by the authoring chain.
    pub subject: String,
}

/// Object-only authored commit chain ready to become HEAD.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoredCommitChain {
    /// Feature commits authored by this run.
    pub commits: Vec<AuthoredCommit>,
    /// Full sha of the last authored commit.
    pub final_sha: String,
}

struct CommitGroup {
    feature: String,
    seq: usize,
    subject: String,
    files: Vec<TreeDiffEntry>,
}

pub(crate) struct ApplyTargetTrees {
    pub checkout_tree: String,
    pub store_delta: Vec<TreeDiffEntry>,
}

struct StoreRevisionSnapshot {
    _temp: tempfile::TempDir,
    dir: PathBuf,
}

const STATE_COMMIT_SUBJECT: &str = "chore: advance bpatch store state";
const STATE_COMMIT_FEATURE: &str = "(state)";

/// Runs same-base convergence against the current store.
pub fn apply(
    ctx: &StateContext,
    options: ApplyOptions,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> Result<ApplyOutcome> {
    if options.pull {
        pull_store(&ctx.store_dir, progress)?;
    }

    let state = crate::engine::state::resolve(ctx)?;
    let store = Store::load(&ctx.store_dir)?;
    if store.metadata().base_commit != state.base.sha {
        return Ok(ApplyOutcome::BaseMismatch(BaseMismatch {
            checkout_base: state.base.sha,
            store_base: store.metadata().base_commit.clone(),
        }));
    }

    let checkout = GitAdapter::new(&ctx.checkout);
    let target = build_apply_target_trees(&checkout, &ctx.store_dir, &store, &state, progress)?;

    checkout.refresh_index()?;
    let has_uncommitted_drift = state
        .drift
        .files()
        .iter()
        .any(|file| matches!(file.source, DriftSource::Uncommitted));
    let checkout_matches_target = state.head_tree == target.checkout_tree;
    let store_revision_changed = state
        .applied
        .as_ref()
        .is_some_and(|applied| applied.store_rev != state.store.head_rev);
    let applied_checkout_matches_head = state.applied.as_ref().is_some_and(|applied| {
        applied.store_rev == state.store.head_rev && applied.tree == state.head_tree
    });
    let committed_drift_is_store_delta =
        committed_drift_covered_by_store_delta(state.drift.files(), &target.store_delta);
    let target_is_safe = state.drift.is_clean()
        || (state.drift_anchor_kind == Some(BpatchCommitKind::Apply)
            && checkout_matches_target
            && !has_uncommitted_drift
            && committed_drift_is_store_delta);
    if checkout_matches_target
        && !store_revision_changed
        && (target.store_delta.is_empty() || applied_checkout_matches_head)
        && target_is_safe
    {
        return Ok(ApplyOutcome::Converged(ConvergedApply {
            store_rev: state.store.head_rev,
            store_short_rev: state.store.short_head_rev,
            target_tree: target.checkout_tree,
        }));
    }

    if !target_is_safe {
        return Ok(ApplyOutcome::Drift(DriftApply {
            files: state.drift.files().to_vec(),
        }));
    }

    let delta = checkout.diff_tree_name_status(&state.head_tree, &target.checkout_tree)?;
    let collisions = untracked_add_collisions(&checkout, &delta)?;
    if !collisions.is_empty() {
        return Ok(ApplyOutcome::Drift(DriftApply { files: collisions }));
    }
    let commit_delta = if delta.is_empty() {
        &target.store_delta
    } else {
        &delta
    };

    let chain = if target.store_delta.is_empty() && store_revision_changed {
        author_state_commit(
            &checkout,
            &state.head_rev,
            &state.head_tree,
            &state.store.head_rev,
            &state.base.sha,
            progress,
        )?
    } else {
        author_feature_commits(
            AuthorCommitsInput {
                checkout: &ctx.checkout,
                store: &store,
                base: &state.base.sha,
                applied_tree: &state.head_tree,
                target_tree: &target.checkout_tree,
                trailers: CommitTrailerMode::Apply {
                    store_rev: &state.store.head_rev,
                },
                subject_mode: SubjectMode::FeatureName,
                parent_commit: &state.head_rev,
                delta: commit_delta,
            },
            progress,
        )?
    };

    progress(ProgressEvent::Start {
        phase: "materialize",
        total: Some(delta.len()),
    });
    checkout
        .materialize_tree_delta(&state.head_tree, &target.checkout_tree)
        .with_context(|| {
            format!(
                "materializing target tree failed; recover with `git read-tree -m -u {} {}`",
                state.head_tree, target.checkout_tree
            )
        })?;
    progress(ProgressEvent::End {
        phase: "materialize",
    });
    finalize_head(
        &ctx.checkout,
        &state.head_rev,
        &chain.final_sha,
        &target.checkout_tree,
    )?;

    Ok(ApplyOutcome::Applied(AppliedApply {
        store_rev: state.store.head_rev,
        store_short_rev: state.store.short_head_rev,
        base: state.base.sha,
        base_display: state.base.display,
        previous_store_short_rev: state.applied.map(|applied| applied.short_store_rev),
        files_changed: delta.len(),
        store_managed_files: store
            .patches()
            .keys()
            .filter(|path| store.stores_path(path))
            .count(),
        target_tree: target.checkout_tree,
        commits: chain.commits,
    }))
}

fn committed_drift_covered_by_store_delta(
    drift: &[DriftFile],
    store_delta: &[TreeDiffEntry],
) -> bool {
    let mut changed_paths = BTreeSet::new();
    for entry in store_delta {
        changed_paths.insert(entry.path.as_path());
        if let Some(old_path) = entry.old_path.as_deref() {
            changed_paths.insert(old_path);
        }
    }
    drift
        .iter()
        .filter(|file| matches!(file.source, DriftSource::Committed))
        .all(|file| changed_paths.contains(file.path.as_path()))
}

/// Authors an unchanged-tree commit when only the store revision advanced.
fn author_state_commit(
    git: &GitAdapter,
    parent_commit: &str,
    tree: &str,
    store_rev: &str,
    base: &str,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> Result<AuthoredCommitChain> {
    progress(ProgressEvent::Start {
        phase: "commit",
        total: Some(1),
    });
    let mut message = String::from(STATE_COMMIT_SUBJECT);
    message.push_str("\n\n");
    message.push_str(&format_state_apply_trailers(store_rev, base, tree));
    let sha = git.process().run_with_stdin(
        &["commit-tree", tree, "-p", parent_commit],
        message.as_bytes(),
    )?;
    let sha = String::from_utf8(sha)
        .context("commit-tree output was not UTF-8")?
        .trim()
        .to_string();
    let short_sha = git.short_rev(&sha)?;
    progress(ProgressEvent::Tick {
        phase: "commit",
        done: 1,
        total: Some(1),
        item: Some(STATE_COMMIT_FEATURE),
    });
    progress(ProgressEvent::End { phase: "commit" });
    Ok(AuthoredCommitChain {
        commits: vec![AuthoredCommit {
            feature: STATE_COMMIT_FEATURE.to_string(),
            seq: 0,
            sha: sha.clone(),
            short_sha,
            subject: STATE_COMMIT_SUBJECT.to_string(),
        }],
        final_sha: sha,
    })
}

/// Authors grouped feature commits with commit-tree without moving refs.
pub fn author_feature_commits(
    input: AuthorCommitsInput<'_>,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> Result<AuthoredCommitChain> {
    if input.delta.is_empty() {
        return Ok(AuthoredCommitChain {
            commits: Vec::new(),
            final_sha: input.parent_commit.to_string(),
        });
    }

    let git = GitAdapter::new(input.checkout);
    let groups = plan_commit_groups(
        &git,
        input.store,
        input.base,
        input.parent_commit,
        input.delta,
        input.subject_mode,
    )?;
    let git_dir = git_dir(git.process())?;
    let temp = tempfile::Builder::new()
        .prefix("bpatch-author-index-")
        .tempfile_in(git_dir)?;
    let index_path = temp.into_temp_path();
    fs::remove_file(&index_path)?;
    let indexed = git
        .process()
        .with_env("GIT_INDEX_FILE", index_path.as_os_str().to_os_string());

    progress(ProgressEvent::Start {
        phase: "commit",
        total: Some(groups.len()),
    });

    let mut authored = Vec::with_capacity(groups.len());
    let mut current_tree = input.applied_tree.to_string();
    let mut parent = input.parent_commit.to_string();
    let last_index = groups.len().saturating_sub(1);

    for (index, group) in groups.iter().enumerate() {
        indexed.run(&["read-tree", &current_tree])?;
        let index_info = index_info_for_group(&git, input.target_tree, &group.files)?;
        indexed.run_with_stdin(&["update-index", "--index-info"], index_info.as_bytes())?;
        let next_tree = indexed.run_str(&["write-tree"])?;
        let tree_trailer = (index == last_index).then_some(input.target_tree);
        let message = commit_message(&group.subject, input.trailers, input.base, tree_trailer);
        let sha = git.process().run_with_stdin(
            &["commit-tree", &next_tree, "-p", &parent],
            message.as_bytes(),
        )?;
        let sha = String::from_utf8(sha)
            .context("commit-tree output was not UTF-8")?
            .trim()
            .to_string();
        let short_sha = git.short_rev(&sha)?;
        authored.push(AuthoredCommit {
            feature: group.feature.clone(),
            seq: group.seq,
            sha: sha.clone(),
            short_sha,
            subject: group.subject.clone(),
        });
        parent = sha;
        current_tree = next_tree;
        progress(ProgressEvent::Tick {
            phase: "commit",
            done: index + 1,
            total: Some(groups.len()),
            item: Some(&group.feature),
        });
    }

    if current_tree != input.target_tree {
        bail!(
            "authored commit chain ended at tree {current_tree}, expected {}",
            input.target_tree
        );
    }
    progress(ProgressEvent::End { phase: "commit" });

    Ok(AuthoredCommitChain {
        commits: authored,
        final_sha: parent,
    })
}

/// Moves HEAD to an already-authored chain tip and syncs the real index.
pub fn finalize_head(
    checkout: impl AsRef<Path>,
    old_head: &str,
    final_sha: &str,
    final_tree: &str,
) -> Result<()> {
    let git = GitAdapter::new(checkout.as_ref());
    git.process()
        .run(&["update-ref", "HEAD", final_sha, old_head])
        .with_context(|| {
            format!(
                "finalizing HEAD failed; recover with `git update-ref HEAD {final_sha} {old_head}`"
            )
        })?;
    git.process()
        .run(&["read-tree", final_tree])
        .with_context(|| {
            format!("syncing index failed; recover with `git read-tree {final_tree}`")
        })?;
    git.refresh_index()
        .context("refreshing index failed; recover with `git update-index -q --refresh`")?;
    Ok(())
}

pub(crate) fn untracked_add_collisions(
    git: &GitAdapter,
    delta: &[TreeDiffEntry],
) -> Result<Vec<DriftFile>> {
    let added = delta
        .iter()
        .filter(|entry| entry.status == "A")
        .map(|entry| entry.path.clone())
        .collect::<BTreeSet<_>>();
    if added.is_empty() {
        return Ok(Vec::new());
    }

    git.refresh_index()?;
    let untracked = untracked_paths(&git.status_porcelain_z()?)?;
    Ok(added
        .into_iter()
        .filter(|path| untracked.contains(path))
        .map(|path| DriftFile {
            path,
            status: "??".to_string(),
            source: DriftSource::Uncommitted,
            annotation: "untracked, would be overwritten".to_string(),
        })
        .collect())
}

fn untracked_paths(bytes: &[u8]) -> Result<BTreeSet<PathBuf>> {
    let mut parts = bytes.split(|byte| *byte == 0);
    let mut paths = BTreeSet::new();
    while let Some(record) = parts.next() {
        if record.is_empty() {
            break;
        }
        let text = std::str::from_utf8(record)?;
        if text.len() < 4 {
            continue;
        }
        let status = &text[..2];
        let path = &text[3..];
        if status == "??" {
            paths.insert(PathBuf::from(path));
        } else if status.starts_with('R') || status.starts_with('C') {
            let _old_path = parts.next();
        }
    }
    Ok(paths)
}

fn pull_store(store_dir: &Path, progress: &mut dyn FnMut(ProgressEvent<'_>)) -> Result<()> {
    progress(ProgressEvent::Start {
        phase: "pull",
        total: None,
    });
    Git::new(store_dir).run(&["pull", "--ff-only"])?;
    progress(ProgressEvent::End { phase: "pull" });
    Ok(())
}

/// Builds the raw store tree and exact overlaid checkout tree for an apply plan.
pub(crate) fn build_apply_target_trees(
    git: &GitAdapter,
    store_dir: &Path,
    store: &Store,
    state: &ResolvedState,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> Result<ApplyTargetTrees> {
    let store_target_tree =
        build_store_target_tree(git, store_dir, store, &state.base.sha, progress)
            .context("building target tree from store patches")?;
    let applied_matches_store_worktree = state
        .applied
        .as_ref()
        .is_some_and(|applied| applied.store_rev == state.store.head_rev)
        && Git::new(store_dir)
            .run(&["status", "--porcelain", "-z", "--", "."])?
            .is_empty();
    let applied_store_tree = match &state.applied {
        Some(_) if applied_matches_store_worktree => store_target_tree.clone(),
        Some(applied) => {
            let snapshot = materialize_store_revision(store_dir, &applied.store_rev)?;
            let applied_store = Store::load(&snapshot.dir).with_context(|| {
                format!(
                    "loading store revision {} from {}",
                    applied.store_rev,
                    snapshot.dir.display()
                )
            })?;
            if applied_store.metadata().base_commit != applied.base {
                // Base-bump continue records the pre-repin store revision with its resolved new-base tree.
                applied.tree.clone()
            } else {
                build_store_target_tree(git, &snapshot.dir, &applied_store, &applied.base, progress)
                    .context("building previous target tree from applied store revision")?
            }
        }
        None => state.base.sha.clone(),
    };
    let store_delta = git
        .diff_tree_name_status(&applied_store_tree, &store_target_tree)?
        .into_iter()
        .filter(|entry| stores_entry(store, entry))
        .collect::<Vec<_>>();
    let checkout_tree =
        build_tree_from_source_entries(git, &state.head_tree, &store_target_tree, &store_delta)?;
    Ok(ApplyTargetTrees {
        checkout_tree,
        store_delta,
    })
}

/// Materializes one historical store subtree without changing the store worktree.
fn materialize_store_revision(store_dir: &Path, revision: &str) -> Result<StoreRevisionSnapshot> {
    let store_dir = fs::canonicalize(store_dir)
        .with_context(|| format!("resolving store path {}", store_dir.display()))?;
    let store_git = Git::new(&store_dir);
    let repo_root = fs::canonicalize(store_git.run_str(&["rev-parse", "--show-toplevel"])?)
        .context("resolving store repository root")?;
    let repo_git = Git::new(&repo_root);
    let store_prefix = store_dir.strip_prefix(&repo_root).with_context(|| {
        format!(
            "store path {} is outside repository {}",
            store_dir.display(),
            repo_root.display()
        )
    })?;
    let paths = if store_prefix.as_os_str().is_empty() {
        repo_git.run(&["ls-tree", "-r", "-z", "--name-only", revision])?
    } else {
        repo_git.run(&[
            "ls-tree",
            "-r",
            "-z",
            "--name-only",
            revision,
            "--",
            path_arg(store_prefix)?,
        ])?
    };
    if paths.is_empty() {
        bail!(
            "store revision {revision} has no files under {}",
            store_prefix.display()
        );
    }

    let temp = tempfile::tempdir().context("creating historical store snapshot")?;
    let indexed = repo_git.with_env("GIT_INDEX_FILE", temp.path().join("index").into_os_string());
    indexed.run(&["read-tree", revision])?;
    let prefix = format!("--prefix={}/", path_arg(temp.path())?.trim_end_matches('/'));
    indexed.run_with_stdin(
        &["checkout-index", "--force", "--stdin", "-z", &prefix],
        &paths,
    )?;
    let dir = temp.path().join(store_prefix);
    Ok(StoreRevisionSnapshot { _temp: temp, dir })
}

fn build_store_target_tree(
    git: &GitAdapter,
    store_dir: &Path,
    store: &Store,
    base: &str,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> Result<String> {
    let git_dir = git_dir(git.process())?;
    let temp = tempfile::Builder::new()
        .prefix("bpatch-tree-index-")
        .tempfile_in(git_dir)?;
    let index_path = temp.into_temp_path();
    fs::remove_file(&index_path)?;
    let indexed = git
        .process()
        .with_env("GIT_INDEX_FILE", index_path.as_os_str().to_os_string());
    indexed.run(&["read-tree", base])?;

    progress(ProgressEvent::Start {
        phase: "tree",
        total: Some(
            store
                .patches()
                .keys()
                .filter(|path| store.stores_path(path))
                .count(),
        ),
    });
    let patches = store
        .patches()
        .values()
        .filter(|patch| store.stores_path(&patch.path))
        .collect::<Vec<_>>();
    for (index, patch) in patches.iter().enumerate() {
        let patch_path = store_dir.join(&patch.path);
        let patch_arg = path_arg(&patch_path)?;
        indexed.run(&["apply", "--cached", "--whitespace=nowarn", patch_arg])?;
        progress(ProgressEvent::Tick {
            phase: "tree",
            done: index + 1,
            total: Some(patches.len()),
            item: Some(&patch.path),
        });
    }
    let tree = indexed.run_str(&["write-tree"])?;
    progress(ProgressEvent::End { phase: "tree" });
    Ok(tree)
}

/// Builds a tree by copying selected entries from a source tree onto a base tree.
pub fn build_tree_from_source_entries(
    git: &GitAdapter,
    base_tree: &str,
    source_tree: &str,
    entries: &[TreeDiffEntry],
) -> Result<String> {
    if entries.is_empty() {
        return Ok(base_tree.to_string());
    }
    let git_dir = git_dir(git.process())?;
    let temp = tempfile::Builder::new()
        .prefix("bpatch-overlay-index-")
        .tempfile_in(git_dir)?;
    let index_path = temp.into_temp_path();
    fs::remove_file(&index_path)?;
    let indexed = git
        .process()
        .with_env("GIT_INDEX_FILE", index_path.as_os_str().to_os_string());
    indexed.run(&["read-tree", base_tree])?;
    let index_info = index_info_for_group(git, source_tree, entries)?;
    indexed.run_with_stdin(&["update-index", "--index-info"], index_info.as_bytes())?;
    indexed.run_str(&["write-tree"])
}

fn plan_commit_groups(
    git: &GitAdapter,
    store: &Store,
    base: &str,
    parent_commit: &str,
    delta: &[TreeDiffEntry],
    subject_mode: SubjectMode,
) -> Result<Vec<CommitGroup>> {
    let mut grouped = BTreeMap::<String, Vec<TreeDiffEntry>>::new();
    for entry in delta {
        let path = entry
            .path
            .to_str()
            .ok_or_else(|| anyhow!("diff path is not UTF-8: {}", entry.path.display()))?;
        let feature = match store.match_path(path) {
            FeatureMatch::Matched { feature, .. } => feature,
            FeatureMatch::Unmatched { .. } => unassigned_feature_name().to_string(),
        };
        grouped.entry(feature).or_default().push(entry.clone());
    }

    let existing = existing_subject_counts(git, base, parent_commit)?;
    grouped
        .into_iter()
        .map(|(feature, files)| {
            let subject_base = subject_base(store, &feature, subject_mode);
            let seq = existing.get(&subject_base).copied().unwrap_or(0) + 1;
            let subject = if seq == 1 {
                subject_base.clone()
            } else {
                format!("{subject_base} #{seq}")
            };
            Ok(CommitGroup {
                feature,
                seq,
                subject,
                files,
            })
        })
        .collect()
}

fn existing_subject_counts(
    git: &GitAdapter,
    base: &str,
    parent_commit: &str,
) -> Result<BTreeMap<String, usize>> {
    let mut counts = BTreeMap::new();
    let range = format!("{base}..{parent_commit}");
    for commit in git.first_parent_commits(Some(&range), None)? {
        if parse_bpatch_authored_base(&git.commit_trailers(&commit)?)?.is_none() {
            continue;
        }
        let subject = git.commit_subject(&commit)?;
        if let Some(base_subject) = apply_subject_base(&subject) {
            *counts.entry(base_subject).or_insert(0) += 1;
        }
    }
    Ok(counts)
}

fn subject_base(store: &Store, feature: &str, subject_mode: SubjectMode) -> String {
    if feature == unassigned_feature_name() {
        "chore: unassigned store patches".to_string()
    } else if subject_mode == SubjectMode::FeatureDescription {
        store
            .features()
            .features
            .get(feature)
            .map(|feature| feature.description.trim())
            .filter(|description| !description.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("feat: {feature}"))
    } else {
        format!("feat: {feature}")
    }
}

fn apply_subject_base(subject: &str) -> Option<String> {
    let without_digits = subject.trim_end_matches(|ch: char| ch.is_ascii_digit());
    let base = without_digits.strip_suffix(" #").unwrap_or(subject);
    (!base.trim().is_empty()).then(|| base.to_string())
}

fn index_info_for_group(
    git: &GitAdapter,
    target_tree: &str,
    files: &[TreeDiffEntry],
) -> Result<String> {
    let mut out = String::new();
    for entry in files {
        if let Some(old_path) = &entry.old_path {
            append_index_info_line(&mut out, git, target_tree, old_path)?;
        }
        append_index_info_line(&mut out, git, target_tree, &entry.path)?;
    }
    Ok(out)
}

fn append_index_info_line(
    out: &mut String,
    git: &GitAdapter,
    target_tree: &str,
    path: &Path,
) -> Result<()> {
    let path_arg = path_arg(path)?;
    let raw = git
        .process()
        .run(&["ls-tree", "-z", target_tree, "--", path_arg])?;
    if raw.is_empty() {
        out.push_str("0 0000000000000000000000000000000000000000\t");
        out.push_str(path_arg);
        out.push('\n');
        return Ok(());
    }

    let first = raw
        .split(|byte| *byte == 0)
        .find(|field| !field.is_empty())
        .ok_or_else(|| anyhow!("ls-tree returned empty record for {path_arg}"))?;
    let record = std::str::from_utf8(first).context("ls-tree output was not UTF-8")?;
    let (metadata, _) = record
        .split_once('\t')
        .ok_or_else(|| anyhow!("malformed ls-tree record for {path_arg}"))?;
    let mut parts = metadata.split_whitespace();
    let mode = parts
        .next()
        .ok_or_else(|| anyhow!("missing mode in ls-tree record for {path_arg}"))?;
    let _kind = parts
        .next()
        .ok_or_else(|| anyhow!("missing kind in ls-tree record for {path_arg}"))?;
    let oid = parts
        .next()
        .ok_or_else(|| anyhow!("missing object id in ls-tree record for {path_arg}"))?;
    out.push_str(mode);
    out.push(' ');
    out.push_str(oid);
    out.push('\t');
    out.push_str(path_arg);
    out.push('\n');
    Ok(())
}

fn commit_message(
    subject: &str,
    trailers: CommitTrailerMode<'_>,
    base: &str,
    tree: Option<&str>,
) -> String {
    let mut message = String::new();
    message.push_str(subject);
    message.push_str("\n\n");
    match trailers {
        CommitTrailerMode::Apply { store_rev } => {
            message.push_str(&format_apply_trailers(store_rev, base, tree));
        }
        CommitTrailerMode::Annotate => {
            message.push_str(&format_annotate_trailers(base));
        }
    }
    message
}

fn stores_entry(store: &Store, entry: &TreeDiffEntry) -> bool {
    entry
        .path
        .to_str()
        .is_none_or(|path| store.stores_path(path))
}

fn git_dir(git: &Git) -> Result<PathBuf> {
    let git_dir = PathBuf::from(git.run_str(&["rev-parse", "--git-dir"])?);
    if git_dir.is_absolute() {
        Ok(git_dir)
    } else {
        Ok(git.repo().join(git_dir))
    }
}

fn path_arg(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| anyhow!("path is not UTF-8: {}", path.display()))
}
