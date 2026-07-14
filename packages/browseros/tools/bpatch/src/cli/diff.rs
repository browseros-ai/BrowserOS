use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::engine::apply::build_apply_target_trees;
use crate::engine::progress;
use crate::engine::state::{StateContext, resolve, unassigned_feature_name};
use crate::git::GitAdapter;
use crate::store::{FeatureMatch, Store};

/// Serializable diff report for the next apply.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiffReport {
    /// Stable result discriminator for JSON consumers.
    pub result: DiffResult,
    /// Chromium base display string.
    pub base: String,
    /// Full chromium base commit.
    pub base_sha: String,
    /// Current store repository HEAD.
    pub store_rev: String,
    /// Applied store revision, when present.
    pub applied_store_rev: Option<String>,
    /// Number of files that apply would touch.
    pub files_changed: usize,
    /// Number of feature groups touched.
    pub features_changed: usize,
    /// Files grouped by feature ownership.
    pub groups: Vec<DiffFeatureGroup>,
    /// Rebuild-scope hint for BUILD.gn and `.gni` changes.
    pub rebuild_scope: RebuildScope,
}

/// Diff result discriminator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffResult {
    /// Current checkout already matches the store target.
    Converged,
    /// Applying current store patches would touch files.
    Changes,
}

/// One feature group in a diff report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiffFeatureGroup {
    /// Feature name, or `(unassigned)` for unmatched paths.
    pub feature: String,
    /// Changed files owned by this feature.
    pub files: Vec<DiffFile>,
}

/// One file that apply would touch.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiffFile {
    /// Git name-status code such as `M`, `A`, or `D`.
    pub status: String,
    /// Repository-relative file path.
    pub path: PathBuf,
    /// Source path for rename/copy entries.
    pub old_path: Option<PathBuf>,
}

/// BUILD.gn and `.gni` rebuild hint for a diff.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RebuildScope {
    /// Whether any BUILD.gn or `.gni` file is touched.
    pub touches_build_files: bool,
    /// Count of touched BUILD.gn or `.gni` files.
    pub build_files_changed: usize,
    /// Human hint for renderers and JSON consumers.
    pub hint: String,
}

/// Computes the current store target and reports what apply would touch.
pub fn run(ctx: &StateContext) -> Result<DiffReport> {
    let state = resolve(ctx)?;
    let store = Store::load(&ctx.store_dir)?;
    let checkout = GitAdapter::new(&ctx.checkout);
    let target = build_apply_target_trees(
        &checkout,
        &ctx.store_dir,
        &store,
        &state,
        &mut progress::noop(),
    )?;
    let entries = checkout
        .diff_tree_name_status(&state.head_tree, &target.checkout_tree)?
        .into_iter()
        .filter(|entry| {
            entry
                .path
                .to_str()
                .is_none_or(|path| store.stores_path(path))
        })
        .collect::<Vec<_>>();

    let mut groups = BTreeMap::<String, Vec<DiffFile>>::new();
    let mut build_files_changed = 0;
    for entry in entries {
        if is_build_file(&entry.path) {
            build_files_changed += 1;
        }
        let path = entry.path.to_string_lossy();
        let feature = match store.match_path(&path) {
            FeatureMatch::Matched { feature, .. } => feature,
            FeatureMatch::Unmatched { .. } => unassigned_feature_name().to_string(),
        };
        groups.entry(feature).or_default().push(DiffFile {
            status: entry.status,
            path: entry.path,
            old_path: entry.old_path,
        });
    }

    let groups = groups
        .into_iter()
        .map(|(feature, files)| DiffFeatureGroup { feature, files })
        .collect::<Vec<_>>();
    let files_changed = groups.iter().map(|group| group.files.len()).sum();
    let rebuild_scope = rebuild_scope(build_files_changed);

    Ok(DiffReport {
        result: if files_changed == 0 {
            DiffResult::Converged
        } else {
            DiffResult::Changes
        },
        base: state.base.display,
        base_sha: state.base.sha,
        store_rev: state.store.head_rev,
        applied_store_rev: state.applied.map(|applied| applied.store_rev),
        files_changed,
        features_changed: groups.len(),
        groups,
        rebuild_scope,
    })
}

/// Renders a human diff report.
pub fn render_human(report: &DiffReport) -> String {
    let mut out = String::new();
    if report.files_changed == 0 {
        out.push_str("apply would touch 0 files — already converged\n");
        out.push_str(&format!("rebuild scope: {}\n", report.rebuild_scope.hint));
        return out;
    }

    out.push_str(&format!(
        "apply would touch {} {} · {} {}:\n",
        report.files_changed,
        files_label(report.files_changed),
        report.features_changed,
        features_label(report.features_changed)
    ));
    for group in &report.groups {
        for (index, file) in group.files.iter().enumerate() {
            let label = if index == 0 {
                group.feature.as_str()
            } else {
                ""
            };
            out.push_str(&format!(
                "  {:<12} {:<4} {}\n",
                label,
                file.status,
                file.path.display()
            ));
        }
    }
    out.push_str(&format!("rebuild scope: {}\n", report.rebuild_scope.hint));
    out
}

/// Renders a JSON diff report.
pub fn render_json(report: &DiffReport) -> Result<String> {
    Ok(serde_json::to_string(report)?)
}

fn rebuild_scope(build_files_changed: usize) -> RebuildScope {
    if build_files_changed == 0 {
        RebuildScope {
            touches_build_files: false,
            build_files_changed,
            hint: "no BUILD.gn / *.gni / include-fanout files touched → small incremental"
                .to_string(),
        }
    } else {
        RebuildScope {
            touches_build_files: true,
            build_files_changed,
            hint: format!(
                "touches {} BUILD.gn / *.gni files → large rebuild likely",
                build_files_changed
            ),
        }
    }
}

fn is_build_file(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "BUILD.gn")
        || path.extension().is_some_and(|ext| ext == "gni")
}

fn files_label(count: usize) -> &'static str {
    if count == 1 { "file" } else { "files" }
}

fn features_label(count: usize) -> &'static str {
    if count == 1 { "feature" } else { "features" }
}
