use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use crate::engine::apply::{self as engine_apply, ApplyOptions, ApplyOutcome, BaseMismatch};
use crate::engine::conflict;
use crate::engine::lock::CheckoutLock;
use crate::engine::progress::ProgressEvent;
use crate::engine::state::{DriftFile, DriftSource, StateContext};
use crate::git::GitAdapter;
use crate::store::Store;

/// Serializable apply result for a checkout.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ApplyReport {
    /// Store changes were written and feature commits were authored.
    Applied {
        /// Short store repository revision applied.
        store_rev: String,
        /// Chromium base display string.
        base: String,
        /// Count of files materialized.
        files_changed: usize,
        /// Feature commits authored by this apply.
        commits: Vec<ApplyCommitReport>,
        /// Process exit code for this result.
        exit: i32,
        #[serde(skip)]
        previous_store_rev: Option<String>,
        #[serde(skip)]
        store_managed_files: usize,
    },
    /// Checkout already matches the store target.
    Converged {
        /// Short store repository revision already present.
        store_rev: String,
        /// Count of files materialized.
        files_changed: usize,
        /// Process exit code for this result.
        exit: i32,
    },
    /// Checkout/store base pins differ.
    BaseMismatch {
        /// Base commit from the checkout state.
        checkout_base: String,
        /// Base commit pinned in store.yaml.
        store_base: String,
        /// Process exit code for this result.
        exit: i32,
    },
    /// Store and checkout bases differed and an out-of-worktree merge found conflicts.
    Conflicts {
        /// Human display for the new chromium base.
        base: String,
        /// Clean store-managed files merged without conflicts.
        merged: usize,
        /// Structured conflict list.
        conflicts: Vec<ApplyConflictFile>,
        /// Whether the worktree was touched while producing this report.
        worktree_touched: bool,
        /// Process exit code for this result.
        exit: i32,
    },
    /// Apply refused because a conflict session is already active.
    #[serde(rename = "session-pending")]
    SessionPending {
        /// Unix timestamp recorded when the session was created.
        created_at: u64,
        /// Number of conflicts recorded in the session.
        conflicts: usize,
        /// Process exit code for this result.
        exit: i32,
    },
    /// Store base pin moved while this checkout is still on old bpatch history.
    #[serde(rename = "base-pin-moved")]
    BasePinMoved {
        /// Base commit pinned in store.yaml.
        store_base: String,
        /// Short base commit pinned in store.yaml.
        store_base_short: String,
        /// Human store base display string.
        store_base_display: String,
        /// Base commit from the checkout state.
        checkout_base: String,
        /// Human checkout base display string.
        checkout_base_display: String,
        /// Process exit code for this result.
        exit: i32,
    },
    /// Checkout has drift and was left untouched.
    Drift {
        /// Drift entries that blocked apply.
        files: Vec<ApplyDriftFile>,
        /// Process exit code for this result.
        exit: i32,
    },
    /// Apply could not start or failed before producing a domain outcome.
    Error {
        /// Human-readable failure reason.
        reason: String,
        /// Process exit code for this result.
        exit: i32,
    },
}

/// One apply-authored commit in the report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ApplyCommitReport {
    /// Feature name, or `(unassigned)`.
    pub feature: String,
    /// Feature sequence number.
    pub seq: usize,
    /// Short commit sha.
    pub sha: String,
    /// Commit subject for human rendering.
    #[serde(skip)]
    pub subject: String,
}

/// One drift entry in an apply refusal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ApplyDriftFile {
    /// Repository-relative file path.
    pub path: PathBuf,
    /// Git status code for the drift.
    pub status: String,
    /// Drift source class.
    pub source: ApplyDriftSource,
    /// Human annotation for the drift.
    pub annotation: String,
}

/// One merge conflict in an apply report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ApplyConflictFile {
    /// Repository-relative file path.
    pub file: PathBuf,
    /// Feature owning the path, or `(unassigned)`.
    pub feature: String,
    /// Conflict kind from git.
    pub kind: String,
}

/// Serializable drift source class.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyDriftSource {
    /// Drift is committed on top of the applied tree.
    Committed,
    /// Drift is in the index or worktree.
    Uncommitted,
}

impl ApplyReport {
    /// Returns the process exit code represented by the report.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Applied { exit, .. }
            | Self::Converged { exit, .. }
            | Self::BaseMismatch { exit, .. }
            | Self::Conflicts { exit, .. }
            | Self::SessionPending { exit, .. }
            | Self::BasePinMoved { exit, .. }
            | Self::Drift { exit, .. }
            | Self::Error { exit, .. } => *exit,
        }
    }
}

/// Runs apply with the checkout lock held for the whole operation.
pub fn run(
    ctx: &StateContext,
    options: ApplyOptions,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> ApplyReport {
    let _lock = match CheckoutLock::acquire(&ctx.checkout) {
        Ok(lock) => lock,
        Err(err) => {
            return ApplyReport::Error {
                reason: err.to_string(),
                exit: 1,
            };
        }
    };

    match conflict::load_session(&ctx.checkout) {
        Ok(Some(session)) => {
            return ApplyReport::SessionPending {
                created_at: session.created_at,
                conflicts: session.conflicts.len(),
                exit: 2,
            };
        }
        Ok(None) => {}
        Err(err) => {
            return ApplyReport::Error {
                reason: err.to_string(),
                exit: 1,
            };
        }
    }

    match engine_apply::apply(ctx, options, progress) {
        Ok(ApplyOutcome::BaseMismatch(mismatch)) => base_mismatch_report(ctx, mismatch, progress),
        Ok(outcome) => report_from_outcome(outcome),
        Err(err) => ApplyReport::Error {
            reason: format!("{err:#}"),
            exit: 1,
        },
    }
}

/// Renders a human apply report.
pub fn render_human(report: &ApplyReport) -> String {
    match report {
        ApplyReport::Applied {
            store_rev,
            files_changed,
            commits,
            previous_store_rev,
            store_managed_files,
            ..
        } => {
            let mut out = String::new();
            match previous_store_rev {
                Some(previous) => out.push_str(&format!(
                    "apply: store {store_rev} (delta vs applied {previous}: {} {})\n",
                    files_changed,
                    files_label(*files_changed)
                )),
                None => out.push_str(&format!(
                    "apply: store {store_rev} (delta vs base: {} {})\n",
                    files_changed,
                    files_label(*files_changed)
                )),
            }
            let untouched = store_managed_files.saturating_sub(*files_changed);
            out.push_str(&format!(
                "  ✓ {} {} written · {} store-managed {} untouched (content + mtime preserved)\n",
                files_changed,
                files_label(*files_changed),
                untouched,
                files_label(untouched)
            ));
            for commit in commits {
                out.push_str(&format!(
                    "  ✓ commit {} \"{}\"   [Bpatch-Store-Rev: {}]\n",
                    commit.sha, commit.subject, store_rev
                ));
            }
            out.push_str("converged. → incremental build will recompile ~1 target dir\n");
            out
        }
        ApplyReport::Converged { store_rev, .. } => {
            format!("already converged at store {store_rev} — nothing to do.\n")
        }
        ApplyReport::BaseMismatch {
            checkout_base,
            store_base,
            ..
        } => format!("base mismatch: checkout base {checkout_base}, store base {store_base}\n"),
        ApplyReport::Conflicts {
            base,
            merged,
            conflicts,
            ..
        } => {
            let mut out = String::new();
            out.push_str(&format!(
                "conflicts on base {base}: {merged} clean {}, {} {}\n",
                files_label(*merged),
                conflicts.len(),
                files_label(conflicts.len())
            ));
            for conflict in conflicts {
                out.push_str(&format!(
                    "  {} ({}, {})\n",
                    conflict.file.display(),
                    conflict.feature,
                    conflict.kind
                ));
            }
            out
        }
        ApplyReport::SessionPending {
            created_at,
            conflicts,
            ..
        } => format!(
            "conflict session in progress (started {}, {} {}) — finish with `bpatch continue` or `bpatch abort`\n",
            created_at,
            conflicts,
            conflicts_label(*conflicts)
        ),
        ApplyReport::BasePinMoved {
            store_base,
            store_base_short,
            store_base_display,
            checkout_base_display,
            ..
        } => format!(
            "store base pin moved to {} ({}) but this checkout is converged on {} — check out the new base first: `git checkout {} && gclient sync`, then `bpatch apply`\n",
            store_base_display, store_base_short, checkout_base_display, store_base
        ),
        ApplyReport::Drift { files, .. } => {
            let mut out = String::new();
            out.push_str(&format!(
                "drift: working tree differs from applied state in {} {}:\n",
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
            out.push_str("  keep the edits →  commit them, then: bpatch extract <rev>   (folds into store)\n");
            out.push_str("  discard them  →  git checkout -- <file>\n");
            out.push_str("exit 3\n");
            out
        }
        ApplyReport::Error { reason, .. } => format!("error: {reason}\n"),
    }
}

/// Renders a JSON apply report.
pub fn render_json(report: &ApplyReport) -> Result<String> {
    Ok(serde_json::to_string(report)?)
}

fn report_from_outcome(outcome: ApplyOutcome) -> ApplyReport {
    match outcome {
        ApplyOutcome::Converged(converged) => ApplyReport::Converged {
            store_rev: converged.store_short_rev,
            files_changed: 0,
            exit: 0,
        },
        ApplyOutcome::Applied(applied) => ApplyReport::Applied {
            store_rev: applied.store_short_rev,
            base: applied.base_display,
            files_changed: applied.files_changed,
            commits: applied
                .commits
                .into_iter()
                .map(|commit| ApplyCommitReport {
                    feature: commit.feature,
                    seq: commit.seq,
                    sha: commit.short_sha,
                    subject: commit.subject,
                })
                .collect(),
            exit: 0,
            previous_store_rev: applied.previous_store_short_rev,
            store_managed_files: applied.store_managed_files,
        },
        ApplyOutcome::BaseMismatch(mismatch) => ApplyReport::BaseMismatch {
            checkout_base: mismatch.checkout_base,
            store_base: mismatch.store_base,
            exit: 2,
        },
        ApplyOutcome::Drift(drift) => ApplyReport::Drift {
            files: apply_drift_files(drift.files),
            exit: 3,
        },
    }
}

fn base_mismatch_report(
    ctx: &StateContext,
    mismatch: BaseMismatch,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> ApplyReport {
    let state = match crate::engine::state::resolve(ctx) {
        Ok(state) => state,
        Err(err) => {
            return ApplyReport::Error {
                reason: format!("{err:#}"),
                exit: 1,
            };
        }
    };
    if !state.drift.is_clean() {
        return drift_report(state.drift.files().to_vec());
    }
    if state.applied.is_some() {
        let store = match Store::load(&ctx.store_dir) {
            Ok(store) => store,
            Err(err) => {
                return ApplyReport::Error {
                    reason: format!("{err:#}"),
                    exit: 1,
                };
            }
        };
        let git = GitAdapter::new(&ctx.checkout);
        return ApplyReport::BasePinMoved {
            store_base_short: git
                .short_rev(&mismatch.store_base)
                .unwrap_or_else(|_| short_sha_fallback(&mismatch.store_base)),
            store_base_display: store.metadata().base_version.clone(),
            store_base: mismatch.store_base,
            checkout_base: state.base.sha,
            checkout_base_display: state.base.display,
            exit: 3,
        };
    }

    match conflict::begin(ctx, progress) {
        Ok(begin) => ApplyReport::Conflicts {
            base: begin.base_display,
            merged: begin.merged,
            conflicts: begin
                .conflicts
                .into_iter()
                .map(|conflict| ApplyConflictFile {
                    file: conflict.file,
                    feature: conflict.feature,
                    kind: conflict.kind,
                })
                .collect(),
            worktree_touched: begin.worktree_touched,
            exit: 2,
        },
        Err(err) => ApplyReport::Error {
            reason: format!("{err:#}"),
            exit: 1,
        },
    }
}

fn drift_report(files: Vec<DriftFile>) -> ApplyReport {
    ApplyReport::Drift {
        files: apply_drift_files(files),
        exit: 3,
    }
}

fn apply_drift_files(files: Vec<DriftFile>) -> Vec<ApplyDriftFile> {
    files
        .into_iter()
        .map(|file| ApplyDriftFile {
            path: file.path,
            status: file.status,
            source: match file.source {
                DriftSource::Committed => ApplyDriftSource::Committed,
                DriftSource::Uncommitted => ApplyDriftSource::Uncommitted,
            },
            annotation: file.annotation,
        })
        .collect()
}

fn short_sha_fallback(sha: &str) -> String {
    sha.chars().take(12).collect()
}

fn files_label(count: usize) -> &'static str {
    if count == 1 { "file" } else { "files" }
}

fn conflicts_label(count: usize) -> &'static str {
    if count == 1 { "conflict" } else { "conflicts" }
}
