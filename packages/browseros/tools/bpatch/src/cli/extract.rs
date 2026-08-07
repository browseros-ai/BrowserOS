use anyhow::Result;
use serde::Serialize;

use crate::engine::extract::{
    CreatedFeature, ExtractContext, ExtractOutcome, ExtractSpec, ExtractedFile,
    FeatureDecisionPolicy, FeatureRoute, NetFold, RepinResult,
};
use crate::engine::progress::ProgressEvent;
use crate::process::Git;
use crate::store::FEATURES_FILE;

/// Extract subcommand mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExtractMode {
    Revs {
        spec: ExtractSpec,
        policy: FeatureDecisionPolicy,
    },
    Repin,
}

/// Options shared by extract and repin CLI entry points.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractOptions {
    pub mode: ExtractMode,
    pub commit: bool,
}

/// Serializable extract report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExtractReport {
    pub result: ExtractReportResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patches: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub new_features: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmatched: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rediffed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_changed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_base: Option<String>,
    pub exit: i32,
    #[serde(skip)]
    pub base_version: String,
    #[serde(skip)]
    pub files: Vec<ExtractedFile>,
    #[serde(skip)]
    pub net_folds: Vec<NetFold>,
    #[serde(skip)]
    pub created_features: Vec<CreatedFeature>,
    #[serde(skip)]
    pub commit_message: String,
    #[serde(skip)]
    pub store_commit: Option<String>,
}

/// Extract report result discriminator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum ExtractReportResult {
    #[serde(rename = "extracted")]
    Extracted,
    #[serde(rename = "needs-feature")]
    NeedsFeature,
    #[serde(rename = "repinned")]
    Repinned,
}

/// Runs extract or repin and optionally commits the store repo changes.
pub fn run(
    ctx: &ExtractContext,
    options: &ExtractOptions,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> Result<ExtractReport> {
    match &options.mode {
        ExtractMode::Revs { spec, policy } => {
            let outcome = crate::engine::extract::extract(ctx, spec, policy, progress)?;
            match outcome {
                ExtractOutcome::NeedsFeature(needs) => Ok(ExtractReport {
                    result: ExtractReportResult::NeedsFeature,
                    patches: None,
                    new_features: Vec::new(),
                    unmatched: needs.unmatched,
                    suggestion: Some(needs.suggestion),
                    rediffed: None,
                    content_changed: None,
                    old_base: None,
                    new_base: None,
                    exit: 3,
                    base_version: String::new(),
                    files: Vec::new(),
                    net_folds: Vec::new(),
                    created_features: Vec::new(),
                    commit_message: String::new(),
                    store_commit: None,
                }),
                ExtractOutcome::Extracted(result) => {
                    let store_commit = if options.commit {
                        commit_store(ctx, &result.store_paths_changed, &result.commit_message)?
                    } else {
                        None
                    };
                    Ok(ExtractReport {
                        result: ExtractReportResult::Extracted,
                        patches: Some(result.patches_changed),
                        new_features: result
                            .new_features
                            .iter()
                            .map(|feature| feature.name.clone())
                            .collect(),
                        unmatched: Vec::new(),
                        suggestion: None,
                        rediffed: None,
                        content_changed: None,
                        old_base: None,
                        new_base: None,
                        exit: 0,
                        base_version: result.base_version,
                        files: result.files,
                        net_folds: result.net_folds,
                        created_features: result.new_features,
                        commit_message: result.commit_message,
                        store_commit,
                    })
                }
            }
        }
        ExtractMode::Repin => {
            let result = crate::engine::extract::repin(ctx, progress)?;
            let store_commit = if options.commit {
                commit_store(ctx, &result.store_paths_changed, &result.commit_message)?
            } else {
                None
            };
            Ok(repin_report(result, store_commit))
        }
    }
}

/// Renders a human extract or repin report.
pub fn render_human(report: &ExtractReport) -> String {
    match report.result {
        ExtractReportResult::Extracted => render_extract_human(report),
        ExtractReportResult::NeedsFeature => render_needs_feature_human(report),
        ExtractReportResult::Repinned => render_repin_human(report),
    }
}

/// Renders a JSON extract or repin report.
pub fn render_json(report: &ExtractReport) -> Result<String> {
    Ok(serde_json::to_string(report)?)
}

fn repin_report(result: RepinResult, store_commit: Option<String>) -> ExtractReport {
    ExtractReport {
        result: ExtractReportResult::Repinned,
        patches: None,
        new_features: Vec::new(),
        unmatched: Vec::new(),
        suggestion: None,
        rediffed: Some(result.rediffed),
        content_changed: Some(result.content_changed),
        old_base: Some(result.old_base_version),
        new_base: Some(result.new_base_version.clone()),
        exit: 0,
        base_version: result.new_base_version,
        files: Vec::new(),
        net_folds: Vec::new(),
        created_features: Vec::new(),
        commit_message: result.commit_message,
        store_commit,
    }
}

fn render_extract_human(report: &ExtractReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "extract: {} {} changed vs base {}\n",
        report.files.len(),
        files_label(report.files.len()),
        report.base_version
    ));
    for file in &report.files {
        out.push_str(&format!(
            "  {:<2} {:<54} {}\n",
            file.status,
            file.path,
            route_label(&file.route)
        ));
    }
    for fold in &report.net_folds {
        out.push_str(&format!(
            "net-fold: {} ({}) → no patch\n",
            fold.path, fold.reason
        ));
    }
    for feature in &report.created_features {
        out.push_str(&format!(
            "created feature \"{}\" (path: {}) in {}\n",
            feature.name, feature.path, FEATURES_FILE
        ));
    }

    let patches = report.patches.unwrap_or(0);
    if report.created_features.is_empty() {
        out.push_str(&format!(
            "store: chromium_patches {} {} updated, {} unchanged\n",
            patches,
            patches_label(patches),
            FEATURES_FILE
        ));
    } else {
        out.push_str(&format!(
            "store: {} {} written · {} +{} {}\n",
            patches,
            patches_label(patches),
            FEATURES_FILE,
            report.created_features.len(),
            features_label(report.created_features.len())
        ));
    }

    if let Some(commit) = &report.store_commit {
        out.push_str(&format!(
            "commit: {} \"{}\"\n",
            commit, report.commit_message
        ));
    } else {
        out.push_str(
            "next: bpatch extract --commit to commit the store repo, or commit it yourself\n",
        );
    }
    out
}

fn render_needs_feature_human(report: &ExtractReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "extract: {} unmatched {} need a feature\n",
        report.unmatched.len(),
        files_label(report.unmatched.len())
    ));
    for path in &report.unmatched {
        out.push_str(&format!("  {}  → no feature matches\n", path));
    }
    if let Some(suggestion) = &report.suggestion {
        out.push_str(&format!("suggestion: {}\n", suggestion));
    }
    out
}

fn render_repin_human(report: &ExtractReport) -> String {
    let rediffed = report.rediffed.unwrap_or(0);
    let content_changed = report.content_changed.unwrap_or(0);
    let old_base = report.old_base.as_deref().unwrap_or("unknown");
    let new_base = report.new_base.as_deref().unwrap_or("unknown");
    let mut out = String::new();
    out.push_str(&format!(
        "re-diffed {} {} against base {} ({} content {} from conflict fixes)\n",
        rediffed,
        patches_label(rediffed),
        new_base,
        content_changed,
        changes_label(content_changed)
    ));
    out.push_str(&format!("store base pin: {} → {}\n", old_base, new_base));
    if let Some(commit) = &report.store_commit {
        out.push_str(&format!(
            "commit: {} \"{}\"\n",
            commit, report.commit_message
        ));
    } else {
        out.push_str(&format!(
            "next: bpatch extract --commit   (store repo commit: \"{}\")\n",
            report.commit_message
        ));
    }
    out
}

fn commit_store(ctx: &ExtractContext, paths: &[String], message: &str) -> Result<Option<String>> {
    if paths.is_empty() {
        return Ok(None);
    }
    let git = Git::new(&ctx.store_dir);
    let mut add_args = vec!["add", "--"];
    let path_args = paths.iter().map(String::as_str).collect::<Vec<_>>();
    add_args.extend(path_args);
    git.run(&add_args)?;
    git.run(&["commit", "-m", message])?;
    Ok(Some(git.run_str(&["rev-parse", "--short", "HEAD"])?))
}

fn route_label(route: &FeatureRoute) -> String {
    match route {
        FeatureRoute::Matched { feature, .. } => format!("→ feature: {} (matched)", feature),
        FeatureRoute::AcceptedSuggestion { feature } => {
            format!("→ feature: {} (nearest path)", feature)
        }
        FeatureRoute::Named { feature } => format!("→ feature: {}", feature),
    }
}

fn files_label(count: usize) -> &'static str {
    if count == 1 { "file" } else { "files" }
}

fn patches_label(count: usize) -> &'static str {
    if count == 1 { "patch" } else { "patches" }
}

fn features_label(count: usize) -> &'static str {
    if count == 1 { "feature" } else { "features" }
}

fn changes_label(count: usize) -> &'static str {
    if count == 1 { "change" } else { "changes" }
}
