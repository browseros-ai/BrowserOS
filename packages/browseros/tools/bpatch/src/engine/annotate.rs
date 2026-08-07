use anyhow::{Context, Result};

use crate::engine::apply::{
    AuthorCommitsInput, AuthoredCommit, CommitTrailerMode, SubjectMode, author_feature_commits,
};
use crate::engine::dirty::{
    ClaimPlan, DirtyEntry, UnclaimedPath, classify_dirty, reset_index_paths_to_head, scan_dirty,
    tree_from_worktree_paths,
};
use crate::engine::lock::CheckoutLock;
use crate::engine::progress::ProgressEvent;
use crate::engine::state::{StateContext, format_annotate_trailers};
use crate::git::{GitAdapter, TreeDiffEntry};
use crate::store::Store;

/// Options controlling dirty-tree annotation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AnnotateOptions {
    pub rest: Option<String>,
}

/// Result of annotating the current dirty checkout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnnotateOutcome {
    Clean,
    Conflicts { files: Vec<DirtyEntry> },
    Annotated(Box<AnnotateResult>),
    Leftovers(Box<AnnotateResult>),
}

/// Commits authored by one annotate run plus any leftover paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnnotateResult {
    pub claimed_files: usize,
    pub resource_files: usize,
    pub rest_files: usize,
    pub feature_commits: Vec<AuthoredCommit>,
    pub resource_commit: Option<NamedCommit>,
    pub rest_commit: Option<NamedCommit>,
    pub unclaimed: Vec<UnclaimedPath>,
}

/// A non-feature commit authored by annotate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamedCommit {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub files: usize,
}

/// Builds an annotate plan without mutating the checkout.
pub fn plan(ctx: &StateContext) -> Result<AnnotatePlan> {
    let git = GitAdapter::new(&ctx.checkout);
    let scan = scan_dirty(&git)?;
    if !scan.conflicts.is_empty() {
        return Ok(AnnotatePlan::Conflicts {
            files: scan.conflicts,
        });
    }
    if scan.entries.is_empty() {
        return Ok(AnnotatePlan::Clean);
    }
    let store = Store::load(&ctx.store_dir)?;
    Ok(AnnotatePlan::Dirty {
        plan: classify_dirty(&store, &scan.entries),
    })
}

/// Read-only annotate classification used by JSON triage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnnotatePlan {
    Clean,
    Conflicts { files: Vec<DirtyEntry> },
    Dirty { plan: ClaimPlan },
}

/// Converts a dirty checkout into feature/resource/rest commits.
pub fn annotate(
    ctx: &StateContext,
    options: &AnnotateOptions,
    progress: &mut dyn FnMut(ProgressEvent<'_>),
) -> Result<AnnotateOutcome> {
    let _lock = CheckoutLock::acquire(&ctx.checkout)?;
    let git = GitAdapter::new(&ctx.checkout);
    let scan = scan_dirty(&git)?;
    if !scan.conflicts.is_empty() {
        return Ok(AnnotateOutcome::Conflicts {
            files: scan.conflicts,
        });
    }
    if scan.entries.is_empty() {
        return Ok(AnnotateOutcome::Clean);
    }

    let state = crate::engine::state::resolve(ctx)?;
    let store = Store::load(&ctx.store_dir)?;
    let claim_plan = classify_dirty(&store, &scan.entries);
    let rest_paths = options
        .rest
        .as_ref()
        .map(|_| {
            claim_plan
                .unclaimed
                .iter()
                .map(|path| path.path.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut committed_paths = Vec::new();
    let feature_tree = tree_from_worktree_paths(&git, &state.head_tree, &claim_plan.claimed)?;
    let feature_delta = git.diff_tree_name_status(&state.head_tree, &feature_tree)?;
    let feature_chain = author_feature_commits(
        AuthorCommitsInput {
            checkout: &ctx.checkout,
            store: &store,
            base: &state.base.sha,
            applied_tree: &state.head_tree,
            target_tree: &feature_tree,
            trailers: CommitTrailerMode::Annotate,
            subject_mode: SubjectMode::FeatureDescription,
            parent_commit: &state.head_rev,
            delta: &feature_delta,
        },
        progress,
    )?;
    if !feature_delta.is_empty() {
        committed_paths.extend(claim_plan.claimed.iter().cloned());
    }

    let mut parent = feature_chain.final_sha.clone();
    let resource_tree = tree_from_worktree_paths(&git, &feature_tree, &claim_plan.resources)?;
    let resource_delta = git.diff_tree_name_status(&feature_tree, &resource_tree)?;
    let resource_commit = if resource_delta.is_empty() {
        None
    } else {
        let commit = commit_named_tree(
            &git,
            &resource_tree,
            &parent,
            "resource: bos_build outputs",
            &state.base.sha,
            &resource_delta,
        )?;
        parent = commit.sha.clone();
        committed_paths.extend(claim_plan.resources.iter().cloned());
        Some(commit)
    };

    let rest_tree = tree_from_worktree_paths(&git, &resource_tree, &rest_paths)?;
    let rest_delta = git.diff_tree_name_status(&resource_tree, &rest_tree)?;
    let rest_commit = if rest_delta.is_empty() {
        None
    } else {
        let subject = format!(
            "wip: {}",
            options
                .rest
                .as_deref()
                .expect("rest paths require rest name")
        );
        let commit = commit_named_tree(
            &git,
            &rest_tree,
            &parent,
            &subject,
            &state.base.sha,
            &rest_delta,
        )?;
        parent = commit.sha.clone();
        committed_paths.extend(rest_paths);
        Some(commit)
    };

    if parent != state.head_rev {
        git.process()
            .run(&["update-ref", "HEAD", &parent, &state.head_rev])
            .with_context(|| {
                format!(
                    "finalizing annotated HEAD failed; recover with `git update-ref HEAD {parent} {}`",
                    state.head_rev
                )
            })?;
        reset_index_paths_to_head(&git, &committed_paths)?;
    }

    let result = AnnotateResult {
        claimed_files: claim_plan.claimed.len(),
        resource_files: claim_plan.resources.len(),
        rest_files: rest_commit.as_ref().map(|commit| commit.files).unwrap_or(0),
        feature_commits: feature_chain.commits,
        resource_commit,
        rest_commit,
        unclaimed: if options.rest.is_some() {
            Vec::new()
        } else {
            claim_plan.unclaimed
        },
    };

    if result.unclaimed.is_empty() {
        Ok(AnnotateOutcome::Annotated(Box::new(result)))
    } else {
        Ok(AnnotateOutcome::Leftovers(Box::new(result)))
    }
}

fn commit_named_tree(
    git: &GitAdapter,
    tree: &str,
    parent: &str,
    subject: &str,
    base: &str,
    delta: &[TreeDiffEntry],
) -> Result<NamedCommit> {
    let mut message = String::new();
    message.push_str(subject);
    message.push_str("\n\n");
    message.push_str(&format_annotate_trailers(base));
    let sha = git
        .process()
        .run_with_stdin(&["commit-tree", tree, "-p", parent], message.as_bytes())?;
    let sha = String::from_utf8(sha)
        .context("commit-tree output was not UTF-8")?
        .trim()
        .to_string();
    Ok(NamedCommit {
        short_sha: git.short_rev(&sha)?,
        sha,
        subject: subject.to_string(),
        files: delta.len(),
    })
}
