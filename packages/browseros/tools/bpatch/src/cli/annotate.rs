use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::engine::annotate::{self as engine_annotate, AnnotateOptions, AnnotateOutcome};
use crate::engine::dirty::UnclaimedPath;
use crate::engine::lock::CheckoutLock;
use crate::engine::progress::ProgressEvent;
use crate::engine::state::StateContext;
use crate::git::GitAdapter;
use crate::store::{FeatureSuggestion, Store};

/// Serializable annotate command report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum AnnotateReport {
    Clean {
        exit: i32,
    },
    Annotated {
        claimed: usize,
        resources: usize,
        rest: usize,
        commits: Vec<AnnotateCommitReport>,
        resource_commit: Option<AnnotateCommitReport>,
        rest_commit: Option<AnnotateCommitReport>,
        next: String,
        exit: i32,
    },
    Leftovers {
        claimed: usize,
        resources: usize,
        rest: usize,
        commits: Vec<AnnotateCommitReport>,
        resource_commit: Option<AnnotateCommitReport>,
        rest_commit: Option<AnnotateCommitReport>,
        unclaimed: Vec<UnclaimedReport>,
        next: String,
        exit: i32,
    },
    Conflicts {
        files: Vec<ConflictReport>,
        exit: i32,
    },
    Triage {
        claimed: usize,
        resources: usize,
        unclaimed: Vec<UnclaimedReport>,
        exit: i32,
    },
}

/// One commit authored by annotate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AnnotateCommitReport {
    pub subject: String,
    pub sha: String,
    pub files: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<usize>,
}

/// One path left for feature assignment.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UnclaimedReport {
    pub path: PathBuf,
    pub suggestion: String,
    pub command: String,
}

/// One unresolved conflict path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConflictReport {
    pub path: PathBuf,
    pub status: String,
}

impl AnnotateReport {
    /// Returns the process exit code represented by the report.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Clean { exit }
            | Self::Annotated { exit, .. }
            | Self::Leftovers { exit, .. }
            | Self::Conflicts { exit, .. }
            | Self::Triage { exit, .. } => *exit,
        }
    }
}

/// Runs annotate with the checkout lock held by the engine.
pub fn run(
    ctx: &StateContext,
    options: &AnnotateOptions,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> Result<AnnotateReport> {
    Ok(match engine_annotate::annotate(ctx, options, progress)? {
        AnnotateOutcome::Clean => AnnotateReport::Clean { exit: 0 },
        AnnotateOutcome::Conflicts { files } => AnnotateReport::Conflicts {
            files: files
                .into_iter()
                .map(|file| ConflictReport {
                    path: file.path,
                    status: file.status,
                })
                .collect(),
            exit: 2,
        },
        AnnotateOutcome::Annotated(result) => AnnotateReport::Annotated {
            claimed: result.claimed_files,
            resources: result.resource_files,
            rest: result.rest_files,
            commits: feature_commits(&result.feature_commits),
            resource_commit: result.resource_commit.as_ref().map(named_commit),
            rest_commit: result.rest_commit.as_ref().map(named_commit),
            next: next_extract(),
            exit: 0,
        },
        AnnotateOutcome::Leftovers(result) => AnnotateReport::Leftovers {
            claimed: result.claimed_files,
            resources: result.resource_files,
            rest: result.rest_files,
            commits: feature_commits(&result.feature_commits),
            resource_commit: result.resource_commit.as_ref().map(named_commit),
            rest_commit: result.rest_commit.as_ref().map(named_commit),
            unclaimed: unclaimed_reports(&result.unclaimed),
            next: next_extract(),
            exit: 3,
        },
    })
}

/// Builds a no-mutation triage report for JSON consumers.
pub fn triage(ctx: &StateContext) -> Result<AnnotateReport> {
    Ok(match engine_annotate::plan(ctx)? {
        engine_annotate::AnnotatePlan::Clean => AnnotateReport::Clean { exit: 0 },
        engine_annotate::AnnotatePlan::Conflicts { files } => AnnotateReport::Conflicts {
            files: files
                .into_iter()
                .map(|file| ConflictReport {
                    path: file.path,
                    status: file.status,
                })
                .collect(),
            exit: 2,
        },
        engine_annotate::AnnotatePlan::Dirty { plan } => AnnotateReport::Triage {
            claimed: plan.claimed.len(),
            resources: plan.resources.len(),
            unclaimed: unclaimed_reports(&plan.unclaimed),
            exit: if plan.unclaimed.is_empty() { 0 } else { 3 },
        },
    })
}

/// Runs human triage through $EDITOR, applies saved feature paths, then annotates.
pub fn triage_editor(
    ctx: &StateContext,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> Result<AnnotateReport> {
    let plan = engine_annotate::plan(ctx)?;
    let engine_annotate::AnnotatePlan::Dirty { plan } = plan else {
        return triage(ctx);
    };
    if plan.unclaimed.is_empty() {
        return run(ctx, &AnnotateOptions::default(), progress);
    }

    let path = triage_path(&ctx.checkout)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let file = TriageFile {
        features: triage_features(&plan.unclaimed),
    };
    fs::write(&path, serde_yaml::to_string(&file)?)
        .with_context(|| format!("writing {}", path.display()))?;
    open_editor(&path)?;
    let edited =
        fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let edited: TriageFile =
        serde_yaml::from_str(&edited).with_context(|| format!("parsing {}", path.display()))?;
    apply_triage_file(ctx, edited)?;
    run(ctx, &AnnotateOptions::default(), progress)
}

/// Renders a human annotate report.
pub fn render_human(report: &AnnotateReport) -> String {
    match report {
        AnnotateReport::Clean { .. } => "annotate: clean — nothing to do.\n".to_string(),
        AnnotateReport::Conflicts { files, .. } => {
            let mut out = String::new();
            out.push_str(&format!(
                "annotate: refusing conflicted tree ({} {})\n",
                files.len(),
                files_label(files.len())
            ));
            for file in files {
                out.push_str(&format!("  {:<4} {}\n", file.status, file.path.display()));
            }
            out
        }
        AnnotateReport::Annotated {
            claimed,
            resources,
            rest,
            commits,
            resource_commit,
            rest_commit,
            next,
            ..
        } => render_committed(
            *claimed,
            *resources,
            *rest,
            commits,
            resource_commit,
            rest_commit,
            next,
        ),
        AnnotateReport::Leftovers {
            claimed,
            resources,
            rest,
            commits,
            resource_commit,
            rest_commit,
            unclaimed,
            next,
            ..
        } => {
            let mut out = render_committed(
                *claimed,
                *resources,
                *rest,
                commits,
                resource_commit,
                rest_commit,
                next,
            );
            out.push_str(&format!(
                "unclaimed {} (left in working tree):\n",
                unclaimed.len()
            ));
            for path in unclaimed {
                out.push_str(&format!(
                    "  {:<56} suggest: {}\n",
                    path.path.display(),
                    path.command
                ));
            }
            out.push_str("exit 3\n");
            out
        }
        AnnotateReport::Triage {
            claimed,
            resources,
            unclaimed,
            ..
        } => {
            let mut out = String::new();
            out.push_str(&format!(
                "triage: claimed {}, resources {}, unclaimed {}\n",
                claimed,
                resources,
                unclaimed.len()
            ));
            for path in unclaimed {
                out.push_str(&format!(
                    "  {:<56} suggest: {}\n",
                    path.path.display(),
                    path.command
                ));
            }
            out
        }
    }
}

/// Renders a JSON annotate report.
pub fn render_json(report: &AnnotateReport) -> Result<String> {
    Ok(serde_json::to_string(report)?)
}

fn render_committed(
    claimed: usize,
    resources: usize,
    rest: usize,
    commits: &[AnnotateCommitReport],
    resource_commit: &Option<AnnotateCommitReport>,
    rest_commit: &Option<AnnotateCommitReport>,
    next: &str,
) -> String {
    let mut out = String::new();
    if !commits.is_empty() || claimed > 0 {
        out.push_str(&format!(
            "claimed {} -> {} feature {}\n",
            claimed,
            commits.len(),
            commits_label(commits.len())
        ));
        for commit in commits {
            out.push_str(&format!(
                "  ✓ commit {} \"{}\"\n",
                commit.sha, commit.subject
            ));
        }
    }
    if let Some(commit) = resource_commit {
        out.push_str(&format!(
            "resources {} -> 1 commit \"{}\"\n",
            resources, commit.subject
        ));
    }
    if let Some(commit) = rest_commit {
        out.push_str(&format!(
            "rest {} -> 1 commit \"{}\"\n",
            rest, commit.subject
        ));
    }
    if commits.is_empty() && resource_commit.is_none() && rest_commit.is_none() {
        out.push_str("annotate: no claimable paths committed\n");
    }
    out.push_str(next);
    out.push('\n');
    out
}

fn feature_commits(commits: &[crate::engine::apply::AuthoredCommit]) -> Vec<AnnotateCommitReport> {
    commits
        .iter()
        .map(|commit| AnnotateCommitReport {
            subject: commit.subject.clone(),
            sha: commit.short_sha.clone(),
            files: 0,
            feature: Some(commit.feature.clone()),
            seq: Some(commit.seq),
        })
        .collect()
}

fn named_commit(commit: &crate::engine::annotate::NamedCommit) -> AnnotateCommitReport {
    AnnotateCommitReport {
        subject: commit.subject.clone(),
        sha: commit.short_sha.clone(),
        files: commit.files,
        feature: None,
        seq: None,
    }
}

fn unclaimed_reports(paths: &[UnclaimedPath]) -> Vec<UnclaimedReport> {
    paths
        .iter()
        .map(|path| {
            let (suggestion, command) = suggestion_command(path);
            UnclaimedReport {
                path: path.path.clone(),
                suggestion,
                command,
            }
        })
        .collect()
}

fn suggestion_command(path: &UnclaimedPath) -> (String, String) {
    let path_text = path.path.to_string_lossy();
    match &path.suggestion {
        FeatureSuggestion::ExistingFeature(feature) => (
            format!("nearest feature \"{feature}\""),
            format!("bpatch feature add {feature} {}", shell_word(&path_text)),
        ),
        FeatureSuggestion::NewFeature(feature) => {
            let owned_path = parent_dir(&path_text).unwrap_or_else(|| path_text.to_string());
            (
                feature.clone(),
                format!(
                    "bpatch feature add {feature} {} --desc {}",
                    shell_word(&owned_path),
                    shell_word(&format!("feat: {feature}"))
                ),
            )
        }
    }
}

fn parent_dir(path: &str) -> Option<String> {
    path.rsplit_once('/')
        .map(|(parent, _)| format!("{parent}/"))
}

#[derive(Debug, Deserialize, Serialize)]
struct TriageFile {
    features: Vec<TriageFeature>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TriageFeature {
    name: String,
    description: String,
    #[serde(default)]
    store: Option<bool>,
    paths: Vec<String>,
}

fn triage_features(paths: &[UnclaimedPath]) -> Vec<TriageFeature> {
    let mut grouped = BTreeMap::<String, TriageFeature>::new();
    for path in paths {
        let path_text = path.path.to_string_lossy();
        let (name, description, owned_path) = match &path.suggestion {
            FeatureSuggestion::ExistingFeature(feature) => (
                feature.clone(),
                format!("feat: {feature}"),
                path_text.to_string(),
            ),
            FeatureSuggestion::NewFeature(feature) => (
                feature.clone(),
                format!("feat: {feature}"),
                parent_dir(&path_text).unwrap_or_else(|| path_text.to_string()),
            ),
        };
        grouped
            .entry(name.clone())
            .or_insert_with(|| TriageFeature {
                name,
                description,
                store: None,
                paths: Vec::new(),
            })
            .paths
            .push(owned_path);
    }
    grouped.into_values().collect()
}

fn apply_triage_file(ctx: &StateContext, file: TriageFile) -> Result<()> {
    let _store_lock = CheckoutLock::acquire_store_repo(&ctx.store_dir)?;
    let mut store = Store::load(&ctx.store_dir)?;
    for feature in file.features {
        if feature.paths.is_empty() {
            continue;
        }
        if store.features().features.contains_key(&feature.name) {
            store.append_feature_paths(&feature.name, feature.paths)?;
        } else {
            store.add_feature_with_store(
                &feature.name,
                &feature.description,
                feature.paths,
                feature.store.unwrap_or(true),
            )?;
        }
    }
    store.save()
}

fn open_editor(path: &Path) -> Result<()> {
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let status = Command::new(&editor)
        .arg(path)
        .status()
        .with_context(|| format!("opening {editor}"))?;
    if !status.success() {
        bail!("{editor} exited with {status}");
    }
    Ok(())
}

fn triage_path(checkout: &Path) -> Result<PathBuf> {
    let git = GitAdapter::new(checkout);
    let git_dir = PathBuf::from(git.process().run_str(&["rev-parse", "--git-dir"])?);
    let git_dir = if git_dir.is_absolute() {
        git_dir
    } else {
        checkout.join(git_dir)
    };
    Ok(git_dir.join("bpatch/triage.yaml"))
}

fn shell_word(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-'))
    {
        value.to_string()
    } else {
        format!("{value:?}")
    }
}

fn next_extract() -> String {
    "next: bpatch extract <annotated-rev-range> to fold feature commits into the store".to_string()
}

fn files_label(count: usize) -> &'static str {
    if count == 1 { "file" } else { "files" }
}

fn commits_label(count: usize) -> &'static str {
    if count == 1 { "commit" } else { "commits" }
}
