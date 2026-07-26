use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::git::GitAdapter;
use crate::store::{FEATURES_FILE, FeatureMatch, FeatureSuggestion, STORE_FILE, Store};

/// One path reported by `git status --porcelain -z`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirtyEntry {
    pub status: String,
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
}

/// Dirty checkout scan split into ordinary entries and unresolved conflicts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirtyScan {
    pub entries: Vec<DirtyEntry>,
    pub conflicts: Vec<DirtyEntry>,
}

/// Dirty path populations after applying bpatch ownership rules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimPlan {
    pub claimed: Vec<PathBuf>,
    pub resources: Vec<PathBuf>,
    pub unclaimed: Vec<UnclaimedPath>,
}

/// Dirty path that bpatch cannot assign to a feature or resource group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnclaimedPath {
    pub path: PathBuf,
    pub suggestion: FeatureSuggestion,
}

/// Scans the checkout's porcelain status without mutating the index.
pub fn scan_dirty(git: &GitAdapter) -> Result<DirtyScan> {
    git.refresh_index()?;
    let mut scan = parse_dirty_status(&git.status_porcelain_z()?)?;
    scan.entries
        .retain(|entry| !entry_has_ignored_checkout_state_path(entry));
    scan.conflicts
        .retain(|entry| !entry_has_ignored_checkout_state_path(entry));
    Ok(scan)
}

/// Classifies dirty entries into stored features, store:false resources, and leftovers.
pub fn classify_dirty(store: &Store, entries: &[DirtyEntry]) -> ClaimPlan {
    let mut claimed = BTreeSet::new();
    let mut resources = BTreeSet::new();
    let mut unclaimed = BTreeMap::new();

    for path in dirty_paths(entries) {
        let Some(path_str) = path.to_str() else {
            unclaimed.insert(
                path.clone(),
                FeatureSuggestion::NewFeature("feature".to_string()),
            );
            continue;
        };
        if is_store_metadata_path(path_str) {
            unclaimed.insert(
                path.clone(),
                FeatureSuggestion::NewFeature("metadata".to_string()),
            );
            continue;
        }

        match store.match_path(path_str) {
            FeatureMatch::Matched { feature, .. } => {
                let stores_path = store
                    .features()
                    .features
                    .get(&feature)
                    .is_none_or(|feature| feature.store);
                if stores_path {
                    claimed.insert(path);
                } else {
                    resources.insert(path);
                }
            }
            FeatureMatch::Unmatched { suggestion } => {
                if store.patches().contains_key(path_str) {
                    claimed.insert(path);
                } else {
                    unclaimed.insert(path, suggestion);
                }
            }
        }
    }

    ClaimPlan {
        claimed: claimed.into_iter().collect(),
        resources: resources.into_iter().collect(),
        unclaimed: unclaimed
            .into_iter()
            .map(|(path, suggestion)| UnclaimedPath { path, suggestion })
            .collect(),
    }
}

/// Writes a tree that starts at `base_tree` and captures selected worktree paths.
pub fn tree_from_worktree_paths(
    git: &GitAdapter,
    base_tree: &str,
    paths: &[PathBuf],
) -> Result<String> {
    if paths.is_empty() {
        return Ok(base_tree.to_string());
    }
    let git_dir = git_dir(git)?;
    let temp = tempfile::Builder::new()
        .prefix("bpatch-dirty-index-")
        .tempfile_in(git_dir)?;
    let index_path = temp.into_temp_path();
    fs::remove_file(&index_path)?;
    let indexed = git
        .process()
        .with_env("GIT_INDEX_FILE", index_path.as_os_str().to_os_string());
    indexed.run(&["read-tree", base_tree])?;
    indexed.run_with_stdin(
        &["add", "-A", "--pathspec-from-file=-", "--pathspec-file-nul"],
        &pathspec_bytes(paths)?,
    )?;
    indexed.run_str(&["write-tree"])
}

/// Updates the real index for committed paths after HEAD moves.
pub fn reset_index_paths_to_head(git: &GitAdapter, paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    git.process().run_with_stdin(
        &[
            "reset",
            "-q",
            "HEAD",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ],
        &pathspec_bytes(paths)?,
    )?;
    git.refresh_index()?;
    Ok(())
}

/// Collapses dirty path claims to directories when every tracked child is dirty.
pub fn collapse_dirty_paths(git: &GitAdapter, paths: &[PathBuf]) -> Result<Vec<String>> {
    let dirty = paths
        .iter()
        .filter_map(|path| path.to_str().map(ToOwned::to_owned))
        .collect::<BTreeSet<_>>();
    let mut dirs = dirty
        .iter()
        .filter_map(|path| path.rsplit_once('/').map(|(parent, _)| parent.to_string()))
        .collect::<BTreeSet<_>>();
    dirs.retain(|dir| !dir.is_empty());
    let mut collapsible = BTreeSet::new();
    for dir in dirs.iter().rev() {
        let prefix = format!("{dir}/");
        let dirty_under = dirty
            .iter()
            .filter(|path| path.starts_with(&prefix))
            .cloned()
            .collect::<BTreeSet<_>>();
        if dirty_under.len() < 2 {
            continue;
        }
        let tracked = tracked_files_under(git, dir)?;
        if tracked.iter().all(|path| dirty_under.contains(path)) {
            collapsible.insert(format!("{dir}/"));
        }
    }

    let mut out = Vec::new();
    for path in dirty {
        let covered = collapsible
            .iter()
            .find(|dir| path.starts_with(dir.as_str()))
            .cloned();
        if let Some(dir) = covered {
            if !out.contains(&dir) {
                out.push(dir);
            }
        } else {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn parse_dirty_status(bytes: &[u8]) -> Result<DirtyScan> {
    let mut parts = bytes.split(|byte| *byte == 0);
    let mut entries = Vec::new();
    let mut conflicts = Vec::new();
    while let Some(record) = parts.next() {
        if record.is_empty() {
            break;
        }
        let text = std::str::from_utf8(record).context("git status output was not UTF-8")?;
        if text.len() < 4 {
            continue;
        }
        let status = text[..2].to_string();
        let path = PathBuf::from(&text[3..]);
        let old_path = if status.starts_with('R') || status.starts_with('C') {
            parts.next().map(|old| {
                std::str::from_utf8(old)
                    .map(PathBuf::from)
                    .context("git status rename source was not UTF-8")
            })
        } else {
            None
        }
        .transpose()?;
        let entry = DirtyEntry {
            status,
            path,
            old_path,
        };
        if is_conflict_status(&entry.status) {
            conflicts.push(entry);
        } else {
            entries.push(entry);
        }
    }
    Ok(DirtyScan { entries, conflicts })
}

fn dirty_paths(entries: &[DirtyEntry]) -> Vec<PathBuf> {
    let mut paths = BTreeSet::new();
    for entry in entries {
        paths.insert(entry.path.clone());
        if let Some(old_path) = &entry.old_path {
            paths.insert(old_path.clone());
        }
    }
    paths.into_iter().collect()
}

fn tracked_files_under(git: &GitAdapter, dir: &str) -> Result<BTreeSet<String>> {
    let out = git.process().run(&["ls-files", "-z", "--", dir])?;
    out.split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8(field.to_vec()).context("git ls-files path was not UTF-8"))
        .collect::<Result<BTreeSet<_>>>()
}

fn is_conflict_status(status: &str) -> bool {
    let bytes = status.as_bytes();
    bytes.contains(&b'U') || matches!(status, "AA" | "DD")
}

fn is_store_metadata_path(path: &str) -> bool {
    matches!(
        path,
        FEATURES_FILE | STORE_FILE | "features.yaml" | "store.yaml"
    )
}

fn entry_has_ignored_checkout_state_path(entry: &DirtyEntry) -> bool {
    is_ignored_checkout_state_path(&entry.path)
        || entry
            .old_path
            .as_ref()
            .is_some_and(|path| is_ignored_checkout_state_path(path))
}

fn is_ignored_checkout_state_path(path: &Path) -> bool {
    path.to_str().is_some_and(|path| {
        path == ".browseros-patch"
            || path == ".browseros-patch/"
            || path.starts_with(".browseros-patch/")
    })
}

fn pathspec_bytes(paths: &[PathBuf]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for path in paths {
        let path = path
            .to_str()
            .ok_or_else(|| anyhow!("path is not UTF-8: {}", path.display()))?;
        out.extend_from_slice(path.as_bytes());
        out.push(0);
    }
    Ok(out)
}

fn git_dir(git: &GitAdapter) -> Result<PathBuf> {
    let git_dir = PathBuf::from(git.process().run_str(&["rev-parse", "--git-dir"])?);
    if git_dir.is_absolute() {
        Ok(git_dir)
    } else {
        Ok(git.process().repo().join(git_dir))
    }
}
