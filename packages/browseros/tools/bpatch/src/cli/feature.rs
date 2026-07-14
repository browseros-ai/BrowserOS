use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::engine::dirty::{classify_dirty, collapse_dirty_paths, scan_dirty};
use crate::engine::lock::CheckoutLock;
use crate::engine::state::{StateContext, parse_apply_trailers};
use crate::git::GitAdapter;
use crate::process::Git;
use crate::store::{FEATURES_FILE, FeatureMatch, Store};

/// Serializable feature command report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum FeatureReport {
    /// Feature inventory for the current store.
    Features {
        /// Features sorted by name.
        features: Vec<FeatureRow>,
        /// Process exit code for this result.
        exit: i32,
    },
    /// A feature block was appended to .features.yaml.
    FeatureAdded {
        /// Feature name.
        name: String,
        /// Owned paths appended or created.
        paths: Vec<String>,
        /// Description written to .features.yaml.
        description: String,
        /// Whether paths are stored as Chromium patch files.
        store: bool,
        /// Whether this command created a new feature block.
        created: bool,
        /// Number of dirty paths newly covered by this command.
        appended: usize,
        /// Number of dirty paths already claimed by another feature or patch.
        skipped: usize,
        /// Number of collapsed directory entries appended to .features.yaml.
        collapsed_dirs: usize,
        /// Number of file entries appended to .features.yaml.
        collapsed_files: usize,
        /// Store commit created by --commit.
        committed: Option<String>,
        /// Whether paths came from --from-dirty.
        from_dirty: bool,
        /// Process exit code for this result.
        exit: i32,
    },
}

/// Feature add behavior selected by the CLI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeatureAddOptions {
    pub paths: Vec<String>,
    pub description: Option<String>,
    pub store: bool,
    pub from_dirty: bool,
    pub commit: bool,
}

/// One row in feature list output.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FeatureRow {
    /// Feature name.
    pub name: String,
    /// Number of store patch paths owned by this feature.
    pub patches: usize,
    /// Highest apply-authored sequence seen since base.
    pub last_sequence: Option<usize>,
    /// Feature description.
    pub description: String,
}

impl FeatureReport {
    /// Returns the process exit code represented by the report.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Features { exit, .. } | Self::FeatureAdded { exit, .. } => *exit,
        }
    }
}

/// Builds the feature inventory report.
pub fn list(ctx: &StateContext) -> Result<FeatureReport> {
    let store = Store::load(&ctx.store_dir)?;
    let patch_counts = patch_counts(&store);
    let last_sequences = last_sequences(ctx)?;
    let features = store
        .features()
        .features
        .iter()
        .map(|(name, feature)| FeatureRow {
            name: name.clone(),
            patches: patch_counts.get(name).copied().unwrap_or(0),
            last_sequence: last_sequences.get(name).copied(),
            description: feature.description.clone(),
        })
        .collect();
    Ok(FeatureReport::Features { features, exit: 0 })
}

/// Appends explicit paths or unclaimed dirty paths to .features.yaml.
pub fn add(
    ctx: &StateContext,
    store_dir: impl Into<PathBuf>,
    name: &str,
    options: FeatureAddOptions,
) -> Result<FeatureReport> {
    if options.paths.is_empty() && !options.from_dirty {
        bail!("feature add requires a path argument, --path <PATH>, or --from-dirty");
    }
    let store_dir = store_dir.into();
    let _store_lock = CheckoutLock::acquire_store_repo(&store_dir)?;
    let description = options
        .description
        .unwrap_or_else(|| format!("feat: {name}"));
    let mut store = Store::load(&store_dir)?;

    let add_plan = if options.from_dirty {
        from_dirty_plan(ctx, &store)?
    } else {
        explicit_plan(&store, name, options.paths)
    };
    let created = !store.features().features.contains_key(name);
    if !add_plan.paths.is_empty() {
        if created {
            store.add_feature_with_store(
                name,
                &description,
                add_plan.paths.clone(),
                options.store,
            )?;
        } else {
            store.append_feature_paths(name, add_plan.paths.clone())?;
        }
        store.save()?;
    }

    let committed = if options.commit {
        commit_store_features(&store_dir)?
    } else {
        None
    };

    Ok(FeatureReport::FeatureAdded {
        name: name.to_string(),
        paths: add_plan.paths,
        description,
        store: options.store,
        created,
        appended: add_plan.appended,
        skipped: add_plan.skipped,
        collapsed_dirs: add_plan.collapsed_dirs,
        collapsed_files: add_plan.collapsed_files,
        committed,
        from_dirty: options.from_dirty,
        exit: 0,
    })
}

struct AddPlan {
    paths: Vec<String>,
    appended: usize,
    skipped: usize,
    collapsed_dirs: usize,
    collapsed_files: usize,
}

fn explicit_plan(store: &Store, name: &str, paths: Vec<String>) -> AddPlan {
    let existing = store
        .features()
        .features
        .get(name)
        .map(|feature| feature.paths.as_slice())
        .unwrap_or(&[]);
    let paths = paths
        .into_iter()
        .filter(|path| !existing.iter().any(|existing| existing == path))
        .collect::<Vec<_>>();
    AddPlan {
        appended: paths.len(),
        skipped: 0,
        collapsed_dirs: paths.iter().filter(|path| path.ends_with('/')).count(),
        collapsed_files: paths.iter().filter(|path| !path.ends_with('/')).count(),
        paths,
    }
}

fn from_dirty_plan(ctx: &StateContext, store: &Store) -> Result<AddPlan> {
    let git = GitAdapter::new(&ctx.checkout);
    let scan = scan_dirty(&git)?;
    if !scan.conflicts.is_empty() {
        bail!("feature add --from-dirty refuses conflicted trees");
    }
    let plan = classify_dirty(store, &scan.entries);
    let candidates = plan
        .unclaimed
        .iter()
        .map(|path| path.path.clone())
        .collect::<Vec<_>>();
    let paths = collapse_dirty_paths(&git, &candidates)?;
    let dirty_count = scan.entries.len();
    let skipped = dirty_count.saturating_sub(candidates.len());
    Ok(AddPlan {
        appended: candidates.len(),
        skipped,
        collapsed_dirs: paths.iter().filter(|path| path.ends_with('/')).count(),
        collapsed_files: paths.iter().filter(|path| !path.ends_with('/')).count(),
        paths,
    })
}

fn commit_store_features(store_dir: &Path) -> Result<Option<String>> {
    let git = Git::new(store_dir);
    git.run(&[
        "add",
        "-A",
        "--",
        FEATURES_FILE,
        ".store.yaml",
        "features.yaml",
        "store.yaml",
    ])?;
    let diff = git.output(&["diff", "--cached", "--quiet", "--", FEATURES_FILE])?;
    if diff.status.success() {
        return Ok(None);
    }
    git.run(&["commit", "-m", "chore(chromium_patches): update features"])
        .context("committing store feature update")?;
    Ok(Some(git.run_str(&["rev-parse", "HEAD"])?))
}

/// Renders a human feature report.
pub fn render_human(report: &FeatureReport) -> String {
    match report {
        FeatureReport::Features { features, .. } => {
            let mut out = String::new();
            out.push_str(&format!(
                "{:<24} {:>7} {:>5}  {}\n",
                "feature", "patches", "last", "description"
            ));
            for feature in features {
                out.push_str(&format!(
                    "{:<24} {:>7} {:>5}  {}\n",
                    feature.name,
                    feature.patches,
                    feature
                        .last_sequence
                        .map(|seq| seq.to_string())
                        .unwrap_or_default(),
                    feature.description
                ));
            }
            out
        }
        FeatureReport::FeatureAdded {
            name,
            paths,
            created,
            appended,
            skipped,
            collapsed_dirs,
            collapsed_files,
            committed,
            from_dirty,
            ..
        } => {
            let action = if *created { "created" } else { "updated" };
            let mut out = String::new();
            if paths.len() == 1 {
                out.push_str(&format!(
                    "{action} feature \"{name}\" (path: {}) in {FEATURES_FILE}\n",
                    paths[0]
                ));
            } else {
                out.push_str(&format!(
                    "{action} feature \"{name}\" ({} entries) in {FEATURES_FILE}\n",
                    paths.len()
                ));
            }
            if *from_dirty && (*appended > 0 || *skipped > 0) {
                out.push_str(&format!(
                    "scanned {} dirty {} · collapsed to {} dirs + {} files · {} new, {} already claimed\n",
                    appended + skipped,
                    files_label(appended + skipped),
                    collapsed_dirs,
                    collapsed_files,
                    appended,
                    skipped
                ));
            }
            if *from_dirty && committed.is_none() {
                out.push_str("next: --commit to commit the store repo\n");
            }
            out
        }
    }
}

/// Renders a JSON feature report.
pub fn render_json(report: &FeatureReport) -> Result<String> {
    Ok(serde_json::to_string(report)?)
}

fn patch_counts(store: &Store) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for path in store.patches().keys() {
        if !store.stores_path(path) {
            continue;
        }
        if let FeatureMatch::Matched { feature, .. } = store.match_path(path) {
            *counts.entry(feature).or_insert(0) += 1;
        }
    }
    counts
}

fn last_sequences(ctx: &StateContext) -> Result<BTreeMap<String, usize>> {
    let state = crate::engine::state::resolve(ctx)?;
    let git = GitAdapter::new(&ctx.checkout);
    let mut sequences = BTreeMap::new();
    let range = format!("{}..HEAD", state.base.sha);
    for commit in git.first_parent_commits(Some(&range), None)? {
        if parse_apply_trailers(&git.commit_trailers(&commit)?)?
            .is_none_or(|trailers| trailers.state_only)
        {
            continue;
        }
        if let Some((feature, seq)) = subject_sequence(&git.commit_subject(&commit)?) {
            let entry = sequences.entry(feature).or_insert(0);
            *entry = (*entry).max(seq);
        }
    }
    Ok(sequences)
}

fn subject_sequence(subject: &str) -> Option<(String, usize)> {
    let rest = subject.strip_prefix("feat: ")?;
    if let Some((feature, seq)) = rest.rsplit_once(" #") {
        return seq
            .parse::<usize>()
            .ok()
            .map(|seq| (feature.to_string(), seq));
    }
    Some((rest.to_string(), 1))
}

fn files_label(count: usize) -> &'static str {
    if count == 1 { "path" } else { "paths" }
}
