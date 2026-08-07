use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use crate::engine::conflict;
use crate::engine::state::{DriftSource, StateContext, resolve};

/// Serializable status result for a checkout.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StatusReport {
    /// Stable result discriminator for JSON consumers.
    pub result: StatusResult,
    /// Chromium base display string.
    pub base: String,
    /// Full chromium base commit.
    pub base_sha: String,
    /// Short chromium base commit.
    pub base_short_sha: String,
    /// Store directory path.
    pub store_path: PathBuf,
    /// Current store repository HEAD.
    pub store_rev: String,
    /// Short current store repository HEAD.
    pub store_short_rev: String,
    /// Store commits ahead of the applied store rev.
    pub store_revs_ahead: Option<usize>,
    /// Applied store revision, when present.
    pub applied_store_rev: Option<String>,
    /// Short applied store revision, when present.
    pub applied_store_short_rev: Option<String>,
    /// Materialized checkout tree cache or recovered tree, when present.
    pub applied_tree: Option<String>,
    /// Number of apply-authored feature commits since base.
    pub feature_commits: usize,
    /// Subject of the newest apply-authored feature commit.
    pub last_feature_commit: Option<String>,
    /// Drift entries relative to the newest bpatch-authored commit tree.
    pub drift: Vec<StatusDriftFile>,
    /// Active conflict session, when one is in progress.
    pub conflict_session: Option<StatusConflictSession>,
}

/// Status result discriminator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusResult {
    /// Checkout has no drift.
    Clean,
    /// Checkout differs from its bpatch-authored drift anchor.
    Drift,
}

/// One drift entry in a status report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StatusDriftFile {
    /// Repository-relative drift path.
    pub path: PathBuf,
    /// Git status code for the drift.
    pub status: String,
    /// Drift source class.
    pub source: StatusDriftSource,
    /// Human annotation for the drift.
    pub annotation: String,
}

/// Conflict-session summary included in status.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StatusConflictSession {
    /// Unix timestamp recorded when the session was created.
    pub created_at: u64,
    /// Human display for the new chromium base.
    pub base: String,
    /// Number of conflicts recorded in the session.
    pub conflicts: usize,
}

/// Serializable drift source class.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusDriftSource {
    /// Drift is committed on top of the bpatch-authored drift anchor.
    Committed,
    /// Drift is in the index or worktree.
    Uncommitted,
}

/// Resolves checkout status from history, store state, and drift checks.
pub fn run(ctx: &StateContext) -> Result<StatusReport> {
    let state = resolve(ctx)?;
    let drift = state
        .drift
        .files()
        .iter()
        .map(|file| StatusDriftFile {
            path: file.path.clone(),
            status: file.status.clone(),
            source: match file.source {
                DriftSource::Committed => StatusDriftSource::Committed,
                DriftSource::Uncommitted => StatusDriftSource::Uncommitted,
            },
            annotation: file.annotation.clone(),
        })
        .collect::<Vec<_>>();
    let applied = state.applied.as_ref();
    let conflict_session =
        conflict::load_session(&ctx.checkout)?.map(|session| StatusConflictSession {
            created_at: session.created_at,
            base: session.new_base_display,
            conflicts: session.conflicts.len(),
        });

    Ok(StatusReport {
        result: if drift.is_empty() {
            StatusResult::Clean
        } else {
            StatusResult::Drift
        },
        base: state.base.display,
        base_sha: state.base.sha,
        base_short_sha: state.base.short_sha,
        store_path: state.store.path,
        store_rev: state.store.head_rev,
        store_short_rev: state.store.short_head_rev,
        store_revs_ahead: state.store.revs_ahead,
        applied_store_rev: applied.map(|applied| applied.store_rev.clone()),
        applied_store_short_rev: applied.map(|applied| applied.short_store_rev.clone()),
        applied_tree: applied.map(|applied| applied.tree.clone()),
        feature_commits: applied
            .map(|applied| applied.feature_commit_count)
            .unwrap_or(0),
        last_feature_commit: applied.map(|applied| applied.last_subject.clone()),
        drift,
        conflict_session,
    })
}

/// Renders a human status report.
pub fn render_human(report: &StatusReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "base     {} ({})\n",
        report.base, report.base_short_sha
    ));
    out.push_str(&format!(
        "store    {} @ {}",
        report.store_path.display(),
        report.store_short_rev
    ));
    if let Some(revs_ahead) = report.store_revs_ahead {
        out.push_str(&format!("   ({} {})", revs_ahead, revs_label(revs_ahead)));
    }
    out.push('\n');

    if let Some(applied) = &report.applied_store_short_rev {
        out.push_str(&format!(
            "applied  store @ {}  ·  {} {}  ·  last: {}\n",
            applied,
            report.feature_commits,
            feature_commits_label(report.feature_commits),
            report.last_feature_commit.as_deref().unwrap_or("unknown")
        ));
    } else {
        out.push_str("applied  none\n");
    }

    if report.drift.is_empty() {
        out.push_str("tree     clean — no drift\n");
    } else {
        out.push_str(&format!(
            "tree     drifted — {} {}\n",
            report.drift.len(),
            files_label(report.drift.len())
        ));
        for file in &report.drift {
            out.push_str(&format!(
                "  {:<44} ({})\n",
                file.path.display(),
                file.annotation
            ));
        }
    }
    if let Some(session) = &report.conflict_session {
        out.push_str(&format!(
            "session  conflict session in progress ({} {}) — bpatch continue / bpatch abort\n",
            session.conflicts,
            conflicts_label(session.conflicts)
        ));
    }
    out
}

/// Renders a JSON status report.
pub fn render_json(report: &StatusReport) -> Result<String> {
    Ok(serde_json::to_string(report)?)
}

fn revs_label(count: usize) -> &'static str {
    if count == 1 {
        "rev ahead"
    } else {
        "revs ahead"
    }
}

fn feature_commits_label(count: usize) -> &'static str {
    if count == 1 {
        "feature commit"
    } else {
        "feature commits"
    }
}

fn files_label(count: usize) -> &'static str {
    if count == 1 { "file" } else { "files" }
}

fn conflicts_label(count: usize) -> &'static str {
    if count == 1 { "conflict" } else { "conflicts" }
}
