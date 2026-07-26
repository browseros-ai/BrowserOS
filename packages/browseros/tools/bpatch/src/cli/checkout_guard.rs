use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::git::GitAdapter;
use crate::store::Store;

const PATCH_SAMPLE_LIMIT: usize = 8;

/// Refuses status/diff/apply before a non-Chromium repo can masquerade as state.
pub fn ensure_matches_store(checkout: &Path, store_dir: &Path) -> Result<()> {
    let samples = modified_patch_samples(store_dir, PATCH_SAMPLE_LIMIT)?;
    let missing = missing_index_paths(checkout, &samples)?;
    let sample_reason = missing
        .first()
        .map(|path| format!("{} not in index", path.display()));

    if store_inside_checkout(checkout, store_dir)? {
        return Err(super::refusal(mismatch_message(
            checkout,
            store_dir,
            sample_reason
                .as_deref()
                .unwrap_or("store directory is inside checkout"),
        )));
    }

    let base_commit = store_base_commit(store_dir)?;
    if !commit_exists(checkout, &base_commit)? {
        return Err(super::refusal(mismatch_message(
            checkout,
            store_dir,
            sample_reason
                .as_deref()
                .unwrap_or("store base commit is not present in checkout"),
        )));
    }

    if !missing.is_empty() && !index_has_path(checkout, "chrome/VERSION")? {
        return Err(super::refusal(mismatch_message(
            checkout,
            store_dir,
            sample_reason
                .as_deref()
                .expect("missing paths checked above"),
        )));
    }

    Ok(())
}

fn mismatch_message(checkout: &Path, store_dir: &Path, reason: &str) -> String {
    format!(
        "{} does not look like a Chromium checkout for store {} ({}); cd into your chromium checkout and re-run",
        checkout.display(),
        store_dir.display(),
        reason
    )
}

fn store_inside_checkout(checkout: &Path, store_dir: &Path) -> Result<bool> {
    let checkout = fs::canonicalize(checkout)
        .with_context(|| format!("resolving checkout {}", checkout.display()))?;
    let store = fs::canonicalize(store_dir)
        .with_context(|| format!("resolving store {}", store_dir.display()))?;
    Ok(store.starts_with(checkout))
}

fn store_base_commit(store_dir: &Path) -> Result<String> {
    Ok(Store::load(store_dir)?.metadata().base_commit.clone())
}

fn commit_exists(checkout: &Path, rev: &str) -> Result<bool> {
    let spec = format!("{rev}^{{commit}}");
    let output = GitAdapter::new(checkout)
        .process()
        .output(&["cat-file", "-e", &spec])?;
    Ok(output.status.success())
}

fn index_has_path(checkout: &Path, path: &str) -> Result<bool> {
    Ok(missing_index_paths(checkout, &[PathBuf::from(path)])?.is_empty())
}

fn missing_index_paths(checkout: &Path, paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let path_args = paths
        .iter()
        .map(|path| {
            path.to_str()
                .ok_or_else(|| anyhow!("path is not valid UTF-8: {}", path.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut args = vec!["--literal-pathspecs", "ls-files", "-z", "--"];
    args.extend(path_args.iter().copied());

    let tracked = GitAdapter::new(checkout)
        .process()
        .run(&args)?
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8(part.to_vec()).context("git ls-files path was not UTF-8"))
        .collect::<Result<BTreeSet<_>>>()?;

    Ok(paths
        .iter()
        .filter(|path| !tracked.contains(&path.to_string_lossy().to_string()))
        .cloned()
        .collect())
}

fn modified_patch_samples(store_dir: &Path, limit: usize) -> Result<Vec<PathBuf>> {
    let mut samples = Vec::new();
    collect_modified_patch_samples(store_dir, store_dir, limit, &mut samples)?;
    Ok(samples)
}

fn collect_modified_patch_samples(
    root: &Path,
    dir: &Path,
    limit: usize,
    samples: &mut Vec<PathBuf>,
) -> Result<()> {
    if samples.len() >= limit {
        return Ok(());
    }

    let mut entries = fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("reading {}", dir.display()))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        if samples.len() >= limit {
            break;
        }
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_modified_patch_samples(root, &path, limit, samples)?;
        } else if file_type.is_file() && patch_modifies_existing_file(&path)? {
            samples.push(relative_path(root, &path)?);
        }
    }
    Ok(())
}

fn patch_modifies_existing_file(path: &Path) -> Result<bool> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if !bytes.starts_with(b"diff --git ") {
        return Ok(false);
    }

    for line in bytes.split(|byte| *byte == b'\n').take(64) {
        if line.starts_with(b"new file mode ") || line == b"--- /dev/null" {
            return Ok(false);
        }
        if line.starts_with(b"--- a/") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn relative_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let rel = path
        .strip_prefix(root)
        .with_context(|| format!("{} is not under {}", path.display(), root.display()))?;
    let mut out = PathBuf::new();
    for component in rel.components() {
        match component {
            Component::Normal(part) => out.push(part),
            _ => return Err(anyhow!("invalid store path: {}", path.display())),
        }
    }
    Ok(out)
}
