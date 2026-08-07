use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use crate::engine::conflict::{self, ContinueOutcome};
use crate::engine::lock::CheckoutLock;
use crate::engine::progress::ProgressEvent;
use crate::engine::state::{DriftSource, StateContext};

/// Options controlling conflict-session continue.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ContinueOptions {
    /// Materialize marker files instead of finishing convergence.
    pub materialize: bool,
}

/// Serializable continue result for a conflict session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ContinueReport {
    /// Marker blobs were written to conflicted files.
    Materialized {
        /// Number of conflicted files written.
        files_written: usize,
        /// Number of clean store-managed files staged for convergence.
        clean_files: usize,
        /// Process exit code for this result.
        exit: i32,
    },
    /// Convergence completed and feature commits were authored.
    Completed {
        /// Human display for the new chromium base.
        base: String,
        /// Short store revision written to trailers.
        store_rev: String,
        /// Number of feature commits authored.
        commits_authored: usize,
        /// Process exit code for this result.
        exit: i32,
    },
    /// Conflicted files still contain marker lines.
    Unresolved {
        /// Files still carrying conflict markers.
        files: Vec<PathBuf>,
        /// Process exit code for this result.
        exit: i32,
    },
    /// Conflicts were not materialized before final continue.
    NotMaterialized {
        /// Human-readable recovery guidance.
        reason: String,
        /// Process exit code for this result.
        exit: i32,
    },
    /// Continue refused to overwrite uncommitted worktree or index changes.
    Drift {
        /// Drift entries that blocked continue.
        files: Vec<ContinueDriftFile>,
        /// Process exit code for this result.
        exit: i32,
    },
    /// No conflict session exists.
    NoSession {
        /// Human-readable reason.
        reason: String,
        /// Process exit code for this result.
        exit: i32,
    },
    /// Continue could not acquire the lock or failed unexpectedly.
    Error {
        /// Human-readable failure reason.
        reason: String,
        /// Process exit code for this result.
        exit: i32,
    },
}

/// One drift entry in a continue refusal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContinueDriftFile {
    /// Repository-relative file path.
    pub path: PathBuf,
    /// Git status code for the drift.
    pub status: String,
    /// Drift source class.
    pub source: ContinueDriftSource,
    /// Human annotation for the drift.
    pub annotation: String,
}

/// Serializable drift source class.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinueDriftSource {
    /// The index or worktree differs from HEAD.
    Uncommitted,
}

impl ContinueReport {
    /// Returns the process exit code represented by the report.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Materialized { exit, .. }
            | Self::Completed { exit, .. }
            | Self::Unresolved { exit, .. }
            | Self::NotMaterialized { exit, .. }
            | Self::Drift { exit, .. }
            | Self::NoSession { exit, .. }
            | Self::Error { exit, .. } => *exit,
        }
    }
}

/// Runs continue with the checkout lock held.
pub fn run(
    ctx: &StateContext,
    options: ContinueOptions,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> ContinueReport {
    let _lock = match CheckoutLock::acquire(&ctx.checkout) {
        Ok(lock) => lock,
        Err(err) => {
            return ContinueReport::Error {
                reason: err.to_string(),
                exit: 1,
            };
        }
    };

    match conflict::continue_session(ctx, options.materialize, progress) {
        Ok(ContinueOutcome::Materialized(materialized)) => ContinueReport::Materialized {
            files_written: materialized.files_written,
            clean_files: materialized.clean_files,
            exit: 0,
        },
        Ok(ContinueOutcome::Completed(completed)) => ContinueReport::Completed {
            base: completed.base_display,
            store_rev: completed.store_short_rev,
            commits_authored: completed.commits.len(),
            exit: 0,
        },
        Ok(ContinueOutcome::Unresolved(unresolved)) => ContinueReport::Unresolved {
            files: unresolved.files,
            exit: 2,
        },
        Ok(ContinueOutcome::NotMaterialized(not_materialized)) => ContinueReport::NotMaterialized {
            reason: not_materialized.reason,
            exit: 2,
        },
        Ok(ContinueOutcome::Drift(drift)) => ContinueReport::Drift {
            files: drift
                .files
                .into_iter()
                .map(|file| ContinueDriftFile {
                    path: file.path,
                    status: file.status,
                    source: match file.source {
                        DriftSource::Committed => ContinueDriftSource::Uncommitted,
                        DriftSource::Uncommitted => ContinueDriftSource::Uncommitted,
                    },
                    annotation: file.annotation,
                })
                .collect(),
            exit: 3,
        },
        Ok(ContinueOutcome::NoSession) => ContinueReport::NoSession {
            reason: "no conflict session".to_string(),
            exit: 1,
        },
        Err(err) => ContinueReport::Error {
            reason: err.to_string(),
            exit: 1,
        },
    }
}

/// Renders a human continue report.
pub fn render_human(report: &ContinueReport) -> String {
    match report {
        ContinueReport::Materialized {
            files_written,
            clean_files,
            ..
        } => format!(
            "{} {} written with conflict markers; {} clean {} staged for convergence\n",
            files_written,
            files_label(*files_written),
            format_count(*clean_files),
            files_label(*clean_files)
        ),
        ContinueReport::Completed {
            base,
            store_rev,
            commits_authored,
            ..
        } => format!(
            "  ✓ converged on base {} · {} {} authored\n    [Bpatch-Store-Rev: {} · Bpatch-Base: {}]\n",
            base,
            commits_authored,
            feature_commits_label(*commits_authored),
            store_rev,
            base
        ),
        ContinueReport::Unresolved { files, .. } => {
            let mut out = String::new();
            out.push_str(&format!(
                "{} {} still contain conflict markers:\n",
                files.len(),
                files_label(files.len())
            ));
            for file in files {
                out.push_str(&format!("  {}\n", file.display()));
            }
            out
        }
        ContinueReport::NotMaterialized { reason, .. } => format!("error: {reason}\n"),
        ContinueReport::Drift { files, .. } => {
            let mut out = String::new();
            out.push_str(&format!(
                "drift: working tree has {} {} that would be overwritten:\n",
                files.len(),
                files_label(files.len())
            ));
            for file in files {
                out.push_str(&format!(
                    "  {:<44} ({})\n",
                    file.path.display(),
                    file.annotation
                ));
            }
            out.push_str("refusing to touch a drifted tree.\n");
            out
        }
        ContinueReport::NoSession { reason, .. } | ContinueReport::Error { reason, .. } => {
            format!("error: {reason}\n")
        }
    }
}

/// Renders a JSON continue report.
pub fn render_json(report: &ContinueReport) -> Result<String> {
    Ok(serde_json::to_string(report)?)
}

fn files_label(count: usize) -> &'static str {
    if count == 1 { "file" } else { "files" }
}

fn feature_commits_label(count: usize) -> &'static str {
    if count == 1 {
        "feature commit"
    } else {
        "feature commits"
    }
}

fn format_count(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}
