mod fixtures;

use std::collections::BTreeMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use bpatch::git::GitAdapter;
use fixtures::FixtureRepo;

#[test]
fn builds_tree_from_patch_set_without_touching_real_index_or_worktree() -> Result<()> {
    let repo = FixtureRepo::new()?;
    repo.write_file("chrome/browser/base.cc", "base\n")?;
    repo.write_file("chrome/browser/keep.cc", "keep\n")?;
    let base = repo.commit("base")?;
    repo.mark_base("bpatch/base", &base)?;

    repo.write_file("chrome/browser/base.cc", "patched\n")?;
    repo.write_file("chrome/browser/new.cc", "new\n")?;
    let target = repo.commit("target")?;
    let target_tree = repo.git_adapter().tree_id(&target)?;
    let patch = repo.create_patch(
        "chromium_patches/browser.patch",
        &base,
        &target,
        &["chrome/browser/base.cc", "chrome/browser/new.cc"],
    )?;

    repo.git().run(&["reset", "--hard", &base])?;
    let status_before = repo.status_porcelain()?;
    let index_before = fs::read(repo.path().join(".git/index"))?;
    let worktree_before = worktree_snapshot(repo.path())?;

    let built_tree = repo
        .git_adapter()
        .build_tree_from_patches(&base, &[patch])?;

    assert_eq!(built_tree, target_tree);
    assert_eq!(repo.status_porcelain()?, status_before);
    assert_eq!(fs::read(repo.path().join(".git/index"))?, index_before);
    assert_eq!(worktree_snapshot(repo.path())?, worktree_before);
    Ok(())
}

#[test]
fn merge_tree_reports_conflicts_without_worktree_writes() -> Result<()> {
    let repo = FixtureRepo::new()?;
    repo.write_file("chrome/browser/conflict.cc", "base\n")?;
    let base = repo.commit("base")?;

    repo.git().run(&["checkout", "-b", "ours", &base])?;
    repo.write_file("chrome/browser/conflict.cc", "ours\n")?;
    let ours = repo.commit("ours")?;

    repo.git().run(&["checkout", "-B", "theirs", &base])?;
    repo.write_file("chrome/browser/conflict.cc", "theirs\n")?;
    let theirs = repo.commit("theirs")?;

    let worktree_before = worktree_snapshot(repo.path())?;
    let result = repo.git_adapter().merge_trees(&base, &ours, &theirs)?;

    assert!(!result.merged_tree_sha.is_empty());
    assert_eq!(result.conflicts.len(), 1);
    assert_eq!(
        result.conflicts[0].file,
        PathBuf::from("chrome/browser/conflict.cc")
    );
    assert_eq!(result.conflicts[0].kind, "content");
    assert_eq!(worktree_snapshot(repo.path())?, worktree_before);
    Ok(())
}

#[test]
fn materializes_two_tree_delta_without_rewriting_unchanged_or_untracked_files() -> Result<()> {
    let repo = FixtureRepo::new()?;
    repo.write_file("chrome/browser/change.cc", "old\n")?;
    repo.write_file("chrome/browser/delete.cc", "delete\n")?;
    repo.write_file("chrome/browser/keep.cc", "keep\n")?;
    let old = repo.commit("old")?;
    let old_tree = repo.git_adapter().tree_id(&old)?;

    repo.write_file("chrome/browser/change.cc", "new\n")?;
    repo.write_file("chrome/browser/add.cc", "add\n")?;
    repo.remove_file("chrome/browser/delete.cc")?;
    let new = repo.commit("new")?;
    let new_tree = repo.git_adapter().tree_id(&new)?;

    repo.git().run(&["reset", "--hard", &old])?;
    repo.plant_untracked("out/Default_arm64/local.marker", "do not touch\n")?;
    let keep_mtime = repo.mtime("chrome/browser/keep.cc")?;
    thread::sleep(Duration::from_millis(1100));

    let materialized = repo
        .git_adapter()
        .materialize_tree_delta(&old_tree, &new_tree)?;

    let changed: BTreeMap<_, _> = materialized
        .changed_files
        .iter()
        .map(|entry| (entry.path.as_path(), entry.status.as_str()))
        .collect();
    assert_eq!(changed.len(), 3);
    assert_eq!(
        changed.get(Path::new("chrome/browser/change.cc")).copied(),
        Some("M")
    );
    assert_eq!(
        changed.get(Path::new("chrome/browser/add.cc")).copied(),
        Some("A")
    );
    assert_eq!(
        changed.get(Path::new("chrome/browser/delete.cc")).copied(),
        Some("D")
    );
    assert_eq!(repo.read_file("chrome/browser/change.cc")?, "new\n");
    assert_eq!(repo.read_file("chrome/browser/keep.cc")?, "keep\n");
    assert!(!repo.path().join("chrome/browser/delete.cc").exists());
    assert_eq!(repo.mtime("chrome/browser/keep.cc")?, keep_mtime);
    assert_eq!(
        repo.read_file("out/Default_arm64/local.marker")?,
        "do not touch\n"
    );
    Ok(())
}

#[test]
fn preflight_rejects_git_before_merge_tree_floor_with_actionable_error() {
    let err = GitAdapter::preflight_version_output("git version 2.39.5").unwrap_err();
    let message = err.to_string();
    assert!(message.contains("git 2.39.5 is too old"));
    assert!(message.contains("requires git >= 2.40"));
    assert!(message.contains("merge-tree --write-tree --merge-base"));
    assert!(message.contains("upgrade git"));
}

fn worktree_snapshot(root: &Path) -> Result<u64> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for path in files {
        path.hash(&mut hasher);
        fs::read(root.join(&path))
            .with_context(|| format!("reading {}", root.join(&path).display()))?
            .hash(&mut hasher);
    }
    Ok(hasher.finish())
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == ".git") {
            continue;
        }
        if path.is_dir() {
            collect_files(root, &path, files)?;
        } else if path.is_file() {
            files.push(path.strip_prefix(root)?.to_owned());
        }
    }
    Ok(())
}
