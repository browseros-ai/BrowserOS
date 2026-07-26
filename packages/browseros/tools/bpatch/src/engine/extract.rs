use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};

use crate::engine::lock::CheckoutLock;
use crate::engine::progress::ProgressEvent;
use crate::engine::state::{self, StateContext};
use crate::process::Git;
use crate::store::{
    FEATURES_FILE, FeatureMatch, FeatureSuggestion, PatchFile, STORE_FILE, Store, StoreMetadata,
};

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Filesystem roots for extract and repin operations.
#[derive(Clone, Debug)]
pub struct ExtractContext {
    pub checkout: PathBuf,
    pub store_dir: PathBuf,
}

impl ExtractContext {
    /// Creates an extract context from checkout and store paths.
    pub fn new(checkout: impl Into<PathBuf>, store_dir: impl Into<PathBuf>) -> Self {
        Self {
            checkout: checkout.into(),
            store_dir: store_dir.into(),
        }
    }
}

/// Revision selector accepted by extract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExtractSpec {
    Rev(String),
    Range { from: String, to: String },
}

impl ExtractSpec {
    /// Parses either `<rev>` or `<rev1>..<rev2>`.
    pub fn parse(spec: &str) -> Result<Self> {
        if let Some((from, to)) = spec.split_once("..") {
            if from.is_empty() || to.is_empty() {
                bail!("extract range must be <rev1>..<rev2>");
            }
            return Ok(Self::Range {
                from: from.to_string(),
                to: to.to_string(),
            });
        }
        if spec.is_empty() {
            bail!("extract revision cannot be empty");
        }
        Ok(Self::Rev(spec.to_string()))
    }
}

/// Policy the CLI uses to resolve files that do not match .features.yaml.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FeatureDecisionPolicy {
    RequireExplicit,
    Named(String),
    AcceptSuggestions,
}

/// Result of extracting commits into the store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExtractOutcome {
    Extracted(Box<ExtractResult>),
    NeedsFeature(NeedsFeature),
}

/// Successful extract result with store write details.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractResult {
    pub base_commit: String,
    pub base_version: String,
    pub target_rev: String,
    pub target_short_rev: String,
    pub files: Vec<ExtractedFile>,
    pub net_folds: Vec<NetFold>,
    pub patches_changed: usize,
    pub patches_written: Vec<String>,
    pub patches_removed: Vec<String>,
    pub new_features: Vec<CreatedFeature>,
    pub store_paths_changed: Vec<String>,
    pub commit_message: String,
}

/// File-level extract routing used by renderers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractedFile {
    pub status: String,
    pub path: String,
    pub route: FeatureRoute,
}

/// Feature routing decision for one extracted file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FeatureRoute {
    Matched {
        feature: String,
        matched_path: String,
    },
    AcceptedSuggestion {
        feature: String,
    },
    Named {
        feature: String,
    },
}

/// Candidate path that folded to no net patch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetFold {
    pub path: String,
    pub reason: String,
}

/// Feature block appended while resolving unmatched files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatedFeature {
    pub name: String,
    pub path: String,
}

/// Feature-routing refusal for non-interactive extraction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NeedsFeature {
    pub unmatched: Vec<String>,
    pub suggestion: String,
}

/// Result of re-pinning the store against a new Chromium base.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepinResult {
    pub old_base_commit: String,
    pub old_base_version: String,
    pub new_base_commit: String,
    pub new_base_version: String,
    pub rediffed: usize,
    pub content_changed: usize,
    pub patches_written: Vec<String>,
    pub patches_removed: Vec<String>,
    pub store_paths_changed: Vec<String>,
    pub commit_message: String,
}

/// Extracts a revision or range into the patch store as net state.
pub fn extract(
    ctx: &ExtractContext,
    spec: &ExtractSpec,
    policy: &FeatureDecisionPolicy,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> Result<ExtractOutcome> {
    let _store_lock = CheckoutLock::acquire_store_repo(&ctx.store_dir)?;
    let git = Git::new(&ctx.checkout);
    let mut store = Store::load(&ctx.store_dir)?;
    let base_commit = store.metadata().base_commit.clone();
    let base_version = store.metadata().base_version.clone();
    ensure_tree_exists(&git, &base_commit)?;
    let revs = selected_revs(&git, spec)?;
    let target_rev = revs
        .last()
        .cloned()
        .ok_or_else(|| anyhow!("extract selected no commits"))?;
    let target_short_rev = short_rev(&git, &target_rev)?;

    progress(ProgressEvent::Start {
        phase: "scan",
        total: Some(revs.len()),
    });
    let mut candidates = BTreeSet::new();
    for (index, rev) in revs.iter().enumerate() {
        for path in touched_paths(&git, rev)? {
            candidates.insert(path);
        }
        progress(ProgressEvent::Tick {
            phase: "scan",
            done: index + 1,
            total: Some(revs.len()),
            item: Some(rev.as_str()),
        });
    }
    progress(ProgressEvent::End { phase: "scan" });

    let candidate_paths = candidates
        .into_iter()
        .filter(|path| store.stores_path(path))
        .collect::<Vec<_>>();
    progress(ProgressEvent::Start {
        phase: "diff",
        total: Some(candidate_paths.len()),
    });
    let mut non_empty = Vec::new();
    let mut net_folds = Vec::new();
    for (index, path) in candidate_paths.iter().enumerate() {
        let patch = net_patch_for_path(&git, &base_commit, &target_rev, path)?;
        if let Some(patch) = patch {
            let status = status_for_path(&git, &base_commit, &target_rev, path)?
                .unwrap_or_else(|| "M".to_string());
            non_empty.push((status, patch));
        } else {
            net_folds.push(NetFold {
                path: path.clone(),
                reason: "touched by extracted revs; final diff is empty".to_string(),
            });
        }
        progress(ProgressEvent::Tick {
            phase: "diff",
            done: index + 1,
            total: Some(candidate_paths.len()),
            item: Some(path.as_str()),
        });
    }
    progress(ProgressEvent::End { phase: "diff" });

    let mut files = Vec::new();
    let mut unresolved = Vec::new();
    for (status, patch) in &non_empty {
        match route_for_patch(&store, &patch.path, policy) {
            RouteDecision::Resolved(route) => files.push(ExtractedFile {
                status: status.clone(),
                path: patch.path.clone(),
                route,
            }),
            RouteDecision::Unresolved { suggestion } => {
                unresolved.push((patch.path.clone(), suggestion));
            }
        }
    }

    if !unresolved.is_empty() {
        return Ok(ExtractOutcome::NeedsFeature(NeedsFeature {
            unmatched: unresolved.iter().map(|(path, _)| path.clone()).collect(),
            suggestion: suggestion_for_unmatched(&unresolved),
        }));
    }

    let mut created_features = Vec::new();
    if let FeatureDecisionPolicy::Named(name) = policy
        && !store.features().features.contains_key(name)
    {
        let unmatched_paths = files
            .iter()
            .filter_map(|file| match &file.route {
                FeatureRoute::Named { feature } if feature == name => Some(file.path.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !unmatched_paths.is_empty() {
            let path = deepest_common_directory(&unmatched_paths);
            store.add_feature(name, &format!("feat: {name}"), vec![path.clone()])?;
            created_features.push(CreatedFeature {
                name: name.clone(),
                path,
            });
        }
    }
    if matches!(policy, FeatureDecisionPolicy::AcceptSuggestions) {
        let mut suggested = BTreeMap::<String, Vec<String>>::new();
        for file in &files {
            if let FeatureRoute::AcceptedSuggestion { feature } = &file.route
                && !store.features().features.contains_key(feature)
            {
                suggested
                    .entry(feature.clone())
                    .or_default()
                    .push(file.path.clone());
            }
        }
        for (name, paths) in suggested {
            let path = deepest_common_directory(&paths);
            store.add_feature(&name, &format!("feat: {name}"), vec![path.clone()])?;
            created_features.push(CreatedFeature { name, path });
        }
    }

    let mutation = apply_patch_mutation(&mut store, &non_empty, &net_folds)?;
    let mut store_paths_changed = mutation.store_paths_changed();
    if !created_features.is_empty() {
        store_paths_changed.push(FEATURES_FILE.to_string());
    }

    progress(ProgressEvent::Start {
        phase: "write",
        total: Some(store_paths_changed.len()),
    });
    store.save()?;
    for (index, path) in store_paths_changed.iter().enumerate() {
        progress(ProgressEvent::Tick {
            phase: "write",
            done: index + 1,
            total: Some(store_paths_changed.len()),
            item: Some(path.as_str()),
        });
    }
    progress(ProgressEvent::End { phase: "write" });

    Ok(ExtractOutcome::Extracted(Box::new(ExtractResult {
        base_commit,
        base_version,
        target_rev,
        target_short_rev: target_short_rev.clone(),
        files,
        net_folds,
        patches_changed: mutation.patches_changed(),
        patches_written: mutation.patches_written,
        patches_removed: mutation.patches_removed,
        new_features: created_features,
        store_paths_changed,
        commit_message: format!("feat(chromium_patches): extract {target_short_rev}"),
    })))
}

/// Re-diffs every store patch path against the checkout's resolved base.
pub fn repin(
    ctx: &ExtractContext,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> Result<RepinResult> {
    let _store_lock = CheckoutLock::acquire_store_repo(&ctx.store_dir)?;
    let state = state::resolve(&StateContext::new(&ctx.checkout, &ctx.store_dir))?;
    let git = Git::new(&ctx.checkout);
    let mut store = Store::load(&ctx.store_dir)?;
    let old_metadata = store.metadata().clone();
    ensure_tree_exists(&git, &state.base.sha)?;

    let paths = store
        .patches()
        .keys()
        .filter(|path| store.stores_path(path))
        .cloned()
        .collect::<Vec<_>>();
    progress(ProgressEvent::Start {
        phase: "repin",
        total: Some(paths.len()),
    });
    let mut non_empty = Vec::new();
    let mut removed = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        if let Some(patch) = net_patch_for_path(&git, &state.base.sha, "HEAD", path)? {
            non_empty.push(("M".to_string(), patch));
        } else {
            removed.push(NetFold {
                path: path.clone(),
                reason: "new base already contains this patch".to_string(),
            });
        }
        progress(ProgressEvent::Tick {
            phase: "repin",
            done: index + 1,
            total: Some(paths.len()),
            item: Some(path.as_str()),
        });
    }
    progress(ProgressEvent::End { phase: "repin" });

    let mutation = apply_patch_mutation(&mut store, &non_empty, &removed)?;
    store.set_metadata(StoreMetadata {
        base_commit: state.base.sha.clone(),
        base_version: state.base.display.clone(),
    });

    let mut store_paths_changed = mutation.store_paths_changed();
    if old_metadata.base_commit != state.base.sha || old_metadata.base_version != state.base.display
    {
        store_paths_changed.push(STORE_FILE.to_string());
    }

    progress(ProgressEvent::Start {
        phase: "write",
        total: Some(store_paths_changed.len()),
    });
    store.save()?;
    for (index, path) in store_paths_changed.iter().enumerate() {
        progress(ProgressEvent::Tick {
            phase: "write",
            done: index + 1,
            total: Some(store_paths_changed.len()),
            item: Some(path.as_str()),
        });
    }
    progress(ProgressEvent::End { phase: "write" });

    Ok(RepinResult {
        old_base_commit: old_metadata.base_commit,
        old_base_version: old_metadata.base_version,
        new_base_commit: state.base.sha,
        new_base_version: state.base.display.clone(),
        rediffed: paths.len(),
        content_changed: mutation.patches_changed(),
        patches_written: mutation.patches_written,
        patches_removed: mutation.patches_removed,
        store_paths_changed,
        commit_message: format!("chore: repin to {}", state.base.display),
    })
}

enum RouteDecision {
    Resolved(FeatureRoute),
    Unresolved { suggestion: FeatureSuggestion },
}

struct PatchMutation {
    patches_written: Vec<String>,
    patches_removed: Vec<String>,
}

impl PatchMutation {
    fn patches_changed(&self) -> usize {
        self.patches_written.len() + self.patches_removed.len()
    }

    fn store_paths_changed(&self) -> Vec<String> {
        self.patches_written
            .iter()
            .chain(self.patches_removed.iter())
            .cloned()
            .collect()
    }
}

fn selected_revs(git: &Git, spec: &ExtractSpec) -> Result<Vec<String>> {
    match spec {
        ExtractSpec::Rev(rev) => Ok(vec![
            git.run_str(&["rev-parse", &format!("{rev}^{{commit}}")])
                .with_context(|| format!("resolving extract revision {rev}"))?,
        ]),
        ExtractSpec::Range { from, to } => {
            let range = format!("{from}..{to}");
            let out = git
                .run_str(&["rev-list", "--reverse", &range])
                .with_context(|| format!("resolving extract range {range}"))?;
            let revs = out
                .lines()
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            if revs.is_empty() {
                bail!("extract range {range} selected no commits");
            }
            Ok(revs)
        }
    }
}

fn touched_paths(git: &Git, rev: &str) -> Result<Vec<String>> {
    let parent = first_parent(git, rev)?;
    let out = git.run(&[
        "diff-tree",
        "-r",
        "--name-only",
        "-z",
        "--no-renames",
        &parent,
        rev,
    ])?;
    parse_nul_paths(&out)
}

fn first_parent(git: &Git, rev: &str) -> Result<String> {
    let out = git.output(&["rev-parse", &format!("{rev}^1")])?;
    if out.status.success() {
        return Ok(String::from_utf8(out.stdout)?.trim_end().to_string());
    }
    Ok(EMPTY_TREE.to_string())
}

fn ensure_tree_exists(git: &Git, rev: &str) -> Result<()> {
    git.run_str(&["rev-parse", &format!("{rev}^{{tree}}")])
        .with_context(|| {
            format!(
                "store base commit {rev} does not exist in checkout; fetch or sync the Chromium base pinned by .store.yaml"
            )
        })?;
    Ok(())
}

fn short_rev(git: &Git, rev: &str) -> Result<String> {
    git.run_str(&["rev-parse", "--short", rev])
        .with_context(|| format!("resolving short rev for {rev}"))
}

fn net_patch_for_path(
    git: &Git,
    base: &str,
    target: &str,
    path: &str,
) -> Result<Option<PatchFile>> {
    let diff = git.run(&[
        "diff",
        "--binary",
        "--full-index",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        base,
        target,
        "--",
        path,
    ])?;
    if diff.is_empty() {
        return Ok(None);
    }
    Ok(Some(PatchFile {
        path: path.to_string(),
        contents: diff,
    }))
}

fn status_for_path(git: &Git, base: &str, target: &str, path: &str) -> Result<Option<String>> {
    let out = git.run(&[
        "diff",
        "--name-status",
        "-z",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        base,
        target,
        "--",
        path,
    ])?;
    let fields = out
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    if fields.is_empty() {
        return Ok(None);
    }
    Ok(Some(std::str::from_utf8(fields[0])?.to_string()))
}

fn route_for_patch(store: &Store, path: &str, policy: &FeatureDecisionPolicy) -> RouteDecision {
    match store.match_path(path) {
        FeatureMatch::Matched {
            feature,
            matched_path,
        } => RouteDecision::Resolved(FeatureRoute::Matched {
            feature,
            matched_path,
        }),
        FeatureMatch::Unmatched { suggestion } => match policy {
            FeatureDecisionPolicy::RequireExplicit => RouteDecision::Unresolved { suggestion },
            FeatureDecisionPolicy::Named(feature) => RouteDecision::Resolved(FeatureRoute::Named {
                feature: feature.clone(),
            }),
            FeatureDecisionPolicy::AcceptSuggestions => match suggestion {
                FeatureSuggestion::ExistingFeature(feature)
                | FeatureSuggestion::NewFeature(feature) => {
                    RouteDecision::Resolved(FeatureRoute::AcceptedSuggestion { feature })
                }
            },
        },
    }
}

fn apply_patch_mutation(
    store: &mut Store,
    non_empty: &[(String, PatchFile)],
    folds: &[NetFold],
) -> Result<PatchMutation> {
    let mut patches = store.patches().clone();
    let mut patches_written = Vec::new();
    let mut patches_removed = Vec::new();

    for (_, patch) in non_empty {
        let changed = patches
            .get(&patch.path)
            .is_none_or(|existing| !semantic_eq(&existing.contents, &patch.contents));
        if changed {
            patches.insert(patch.path.clone(), patch.clone());
            patches_written.push(patch.path.clone());
        }
    }

    for fold in folds {
        if patches.remove(&fold.path).is_some() {
            patches_removed.push(fold.path.clone());
        }
    }

    store.set_patches(patches.into_values().collect())?;
    Ok(PatchMutation {
        patches_written,
        patches_removed,
    })
}

fn semantic_eq(left: &[u8], right: &[u8]) -> bool {
    without_index_lines(left) == without_index_lines(right)
}

fn without_index_lines(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if !line.starts_with(b"index ") {
            out.extend_from_slice(line);
        }
    }
    out
}

fn suggestion_for_unmatched(unresolved: &[(String, FeatureSuggestion)]) -> String {
    if let Some((_, suggestion)) = unresolved.first() {
        match suggestion {
            FeatureSuggestion::ExistingFeature(feature)
            | FeatureSuggestion::NewFeature(feature) => {
                return feature.clone();
            }
        }
    }
    let paths = unresolved
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    deepest_common_directory(&paths)
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("feature")
        .to_string()
}

fn deepest_common_directory(paths: &[String]) -> String {
    let Some(first) = paths.first() else {
        return "feature/".to_string();
    };
    let mut common = parent_components(first);
    for path in &paths[1..] {
        let components = parent_components(path);
        let len = common
            .iter()
            .zip(components.iter())
            .take_while(|(left, right)| left == right)
            .count();
        common.truncate(len);
    }
    if common.is_empty() {
        return first
            .rsplit_once('/')
            .map(|(parent, _)| format!("{parent}/"))
            .unwrap_or_else(|| first.to_string());
    }
    format!("{}/", common.join("/"))
}

fn parent_components(path: &str) -> Vec<String> {
    path.rsplit_once('/')
        .map(|(parent, _)| {
            parent
                .split('/')
                .filter(|part| !part.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_nul_paths(bytes: &[u8]) -> Result<Vec<String>> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8(part.to_vec()).context("git path output was not UTF-8"))
        .collect()
}
