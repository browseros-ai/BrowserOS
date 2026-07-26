use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use crate::git::{GitAdapter, Trailer, TreeDiffEntry};
use crate::store::Store;

/// Trailer key carrying the store repository commit applied to a checkout.
pub const TRAILER_STORE_REV: &str = "Bpatch-Store-Rev";
/// Trailer key carrying the chromium base commit used for convergence.
pub const TRAILER_BASE: &str = "Bpatch-Base";
/// Trailer key carrying the cached materialized checkout tree.
pub const TRAILER_TREE: &str = "Bpatch-Tree";
/// Trailer key marking commits authored by `bpatch annotate`.
pub const TRAILER_ANNOTATED: &str = "Bpatch-Annotated";
/// Trailer key marking apply commits that only advance store bookkeeping.
pub const TRAILER_STATE_ONLY: &str = "Bpatch-State-Only";

const HISTORY_LIMIT: usize = 512;
const UNASSIGNED_FEATURE: &str = "(unassigned)";

/// Input paths shared by state, status, and diff operations.
#[derive(Clone, Debug)]
pub struct StateContext {
    /// Chromium checkout root.
    pub checkout: PathBuf,
    /// `chromium_patches` store directory.
    pub store_dir: PathBuf,
}

impl StateContext {
    /// Creates a state context from checkout and store paths.
    pub fn new(checkout: impl Into<PathBuf>, store_dir: impl Into<PathBuf>) -> Self {
        Self {
            checkout: checkout.into(),
            store_dir: store_dir.into(),
        }
    }
}

/// Parsed bpatch trailers from an apply-authored commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyTrailers {
    /// Store repository commit applied to the checkout.
    pub store_rev: String,
    /// Chromium base commit used to compute the applied checkout tree.
    pub base: String,
    /// Cached materialized checkout tree, when the commit still carries it.
    pub tree: Option<String>,
    /// Whether the commit advances store state without changing managed files.
    pub state_only: bool,
}

/// Parsed bpatch annotate trailers from a commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnnotateTrailers {
    /// Chromium base commit used while grouping dirty checkout changes.
    pub base: String,
}

/// Trailer kind of the commit used as the committed-drift anchor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BpatchCommitKind {
    /// Commit authored by `bpatch apply`.
    Apply,
    /// Commit authored by `bpatch annotate`.
    Annotate,
}

/// Resolved checkout state derived from history and the store repo.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedState {
    /// Checkout root used for resolution.
    pub checkout: PathBuf,
    /// Store directory used for resolution.
    pub store_dir: PathBuf,
    /// Current checkout HEAD commit.
    pub head_rev: String,
    /// Current checkout HEAD tree.
    pub head_tree: String,
    /// Base commit display data.
    pub base: BaseState,
    /// Current store repository state.
    pub store: StoreRepoState,
    /// Last apply-authored state, if this checkout has one.
    pub applied: Option<AppliedState>,
    /// Kind of the newest bpatch-authored commit, when any.
    pub drift_anchor_kind: Option<BpatchCommitKind>,
    /// Drift from the newest bpatch-authored commit tree.
    pub drift: DriftState,
}

/// Display data for the chromium base commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BaseState {
    /// Full base commit sha.
    pub sha: String,
    /// Short base commit sha.
    pub short_sha: String,
    /// Chromium version string or short sha fallback.
    pub display: String,
}

/// Current store repository state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreRepoState {
    /// Store directory path.
    pub path: PathBuf,
    /// Store repository HEAD sha.
    pub head_rev: String,
    /// Short store repository HEAD sha.
    pub short_head_rev: String,
    /// Number of store revisions ahead of the applied store rev.
    pub revs_ahead: Option<usize>,
}

/// Last apply-authored state discovered in checkout history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedState {
    /// Commit carrying the newest apply trailers.
    pub commit: String,
    /// Short form of the trailer commit.
    pub short_commit: String,
    /// Store revision recorded in the trailer commit.
    pub store_rev: String,
    /// Short form of the recorded store revision.
    pub short_store_rev: String,
    /// Base revision recorded in the trailer commit.
    pub base: String,
    /// Materialized checkout tree resolved from the trailer or recovery fallback.
    pub tree: String,
    /// Count of apply-authored feature commits since the base.
    pub feature_commit_count: usize,
    /// Subject of the newest apply-authored commit.
    pub last_subject: String,
}

/// Drift state relative to the newest bpatch-authored commit tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DriftState {
    /// No committed or tracked uncommitted drift was found.
    Clean,
    /// One or more files differ from the bpatch-authored drift anchor.
    Drifted {
        /// Drift entries grouped by source class.
        files: Vec<DriftFile>,
    },
}

impl DriftState {
    /// Returns true when no drift entries were found.
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Clean)
    }

    /// Returns drift entries, or an empty slice when clean.
    pub fn files(&self) -> &[DriftFile] {
        match self {
            Self::Clean => &[],
            Self::Drifted { files } => files,
        }
    }
}

/// One file that differs from the bpatch-authored drift anchor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DriftFile {
    /// Repository-relative file path.
    pub path: PathBuf,
    /// Git status code for the drift.
    pub status: String,
    /// Whether the drift is committed or uncommitted.
    pub source: DriftSource,
    /// Human annotation used by status/apply renderers.
    pub annotation: String,
}

/// Source class for a drift entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DriftSource {
    /// HEAD contains commits after the bpatch-authored drift anchor.
    Committed,
    /// The index or worktree differs from HEAD.
    Uncommitted,
}

/// Read-only handle for the git repository containing `chromium_patches`.
#[derive(Clone, Debug)]
pub struct StoreRepo {
    dir: PathBuf,
    git: GitAdapter,
}

impl StoreRepo {
    /// Creates a store-repo handle rooted at a store directory.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        Self {
            git: GitAdapter::new(&dir),
            dir,
        }
    }

    /// Returns the store directory path.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Resolves the store repository HEAD.
    pub fn head_rev(&self) -> Result<String> {
        Ok(self.git.head_rev()?)
    }

    /// Resolves a short store repository revision.
    pub fn short_rev(&self, rev: &str) -> Result<String> {
        Ok(self.git.short_rev(rev)?)
    }

    /// Counts store repository commits ahead of an applied revision.
    pub fn revs_ahead(&self, applied_rev: &str) -> Result<usize> {
        Ok(self.git.rev_list_count(&format!("{applied_rev}..HEAD"))?)
    }
}

struct TrailerCommit {
    commit: String,
    trailers: ApplyTrailers,
    subject: String,
}

struct BpatchTrailerCommit {
    commit: String,
    base: String,
    subject: String,
    kind: BpatchCommitKind,
}

/// Resolves trailer state, store freshness, base display, and checkout drift.
pub fn resolve(ctx: &StateContext) -> Result<ResolvedState> {
    let checkout = GitAdapter::new(&ctx.checkout);
    let store_repo = StoreRepo::new(&ctx.store_dir);
    let store_model = Store::load(&ctx.store_dir)?;
    let head_rev = checkout.head_rev()?;
    let head_tree = checkout.tree_id("HEAD")?;
    let store_head = store_repo.head_rev()?;
    let latest_apply = find_latest_apply_commit(&checkout)?;
    let latest_feature_subject = find_latest_feature_subject(&checkout)?;
    let latest_bpatch = find_latest_bpatch_commit(&checkout)?;
    let latest_bpatch_tree = latest_bpatch
        .as_ref()
        .map(|entry| checkout.tree_id(&entry.commit))
        .transpose()?;
    let base_sha = latest_bpatch
        .as_ref()
        .map(|entry| entry.base.clone())
        .unwrap_or_else(|| head_rev.clone());
    let base = BaseState {
        short_sha: checkout.short_rev(&base_sha)?,
        display: base_display(&checkout, &base_sha)?,
        sha: base_sha.clone(),
    };

    let applied = latest_apply
        .map(|entry| {
            let tree = applied_tree(&checkout, &ctx.store_dir, &store_head, &entry)?;
            Ok::<_, anyhow::Error>(AppliedState {
                short_commit: checkout.short_rev(&entry.commit)?,
                short_store_rev: store_repo.short_rev(&entry.trailers.store_rev)?,
                feature_commit_count: feature_commit_count(&checkout, &entry.trailers.base)?,
                base: entry.trailers.base,
                store_rev: entry.trailers.store_rev,
                commit: entry.commit,
                tree,
                last_subject: latest_feature_subject.unwrap_or(entry.subject),
            })
        })
        .transpose()?;

    let store = StoreRepoState {
        path: ctx.store_dir.clone(),
        short_head_rev: store_repo.short_rev(&store_head)?,
        revs_ahead: applied
            .as_ref()
            .map(|applied| store_repo.revs_ahead(&applied.store_rev))
            .transpose()?,
        head_rev: store_head,
    };
    let (drift_tree, drift_subject) = latest_bpatch
        .as_ref()
        .zip(latest_bpatch_tree.as_deref())
        .map(|(entry, tree)| (tree, entry.subject.as_str()))
        .unwrap_or((base.sha.as_str(), "base state"));
    let drift = detect_drift(&checkout, &store_model, drift_tree, drift_subject)?;

    Ok(ResolvedState {
        checkout: ctx.checkout.clone(),
        store_dir: ctx.store_dir.clone(),
        head_rev,
        head_tree,
        base,
        store,
        applied,
        drift_anchor_kind: latest_bpatch.as_ref().map(|entry| entry.kind),
        drift,
    })
}

/// Parses apply trailers from a git trailer list.
pub fn parse_apply_trailers(trailers: &[Trailer]) -> Result<Option<ApplyTrailers>> {
    let mut store_rev = None;
    let mut base = None;
    let mut tree = None;
    let mut state_only = false;
    for trailer in trailers {
        match trailer.key.as_str() {
            TRAILER_STORE_REV => store_rev = Some(trailer.value.clone()),
            TRAILER_BASE => base = Some(trailer.value.clone()),
            TRAILER_TREE => tree = Some(trailer.value.clone()),
            TRAILER_STATE_ONLY => state_only = trailer.value.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }

    let Some(store_rev) = store_rev else {
        return Ok(None);
    };
    let base =
        base.ok_or_else(|| anyhow!("{TRAILER_STORE_REV} commit is missing {TRAILER_BASE}"))?;
    Ok(Some(ApplyTrailers {
        store_rev,
        base,
        tree,
        state_only,
    }))
}

/// Parses annotate trailers from a git trailer list.
pub fn parse_annotate_trailers(trailers: &[Trailer]) -> Result<Option<AnnotateTrailers>> {
    let mut annotated = false;
    let mut base = None;
    for trailer in trailers {
        match trailer.key.as_str() {
            TRAILER_ANNOTATED => annotated = true,
            TRAILER_BASE => base = Some(trailer.value.clone()),
            _ => {}
        }
    }
    if !annotated {
        return Ok(None);
    }
    let base =
        base.ok_or_else(|| anyhow!("{TRAILER_ANNOTATED} commit is missing {TRAILER_BASE}"))?;
    Ok(Some(AnnotateTrailers { base }))
}

/// Returns the Chromium base from any bpatch-authored commit trailer block.
pub fn parse_bpatch_authored_base(trailers: &[Trailer]) -> Result<Option<String>> {
    if let Some(apply) = parse_apply_trailers(trailers)? {
        return Ok(Some(apply.base));
    }
    Ok(parse_annotate_trailers(trailers)?.map(|annotate| annotate.base))
}

/// Formats apply trailers as a commit-message trailer block.
pub fn format_apply_trailers(store_rev: &str, base: &str, tree: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str(TRAILER_STORE_REV);
    out.push_str(": ");
    out.push_str(store_rev);
    out.push('\n');
    out.push_str(TRAILER_BASE);
    out.push_str(": ");
    out.push_str(base);
    out.push('\n');
    if let Some(tree) = tree {
        out.push_str(TRAILER_TREE);
        out.push_str(": ");
        out.push_str(tree);
        out.push('\n');
    }
    out
}

/// Formats apply trailers for a commit that only advances store bookkeeping.
pub fn format_state_apply_trailers(store_rev: &str, base: &str, tree: &str) -> String {
    let mut out = format_apply_trailers(store_rev, base, Some(tree));
    out.push_str(TRAILER_STATE_ONLY);
    out.push_str(": true\n");
    out
}

/// Formats the trailer block for commits created by `bpatch annotate`.
pub fn format_annotate_trailers(base: &str) -> String {
    let mut out = String::new();
    out.push_str(TRAILER_BASE);
    out.push_str(": ");
    out.push_str(base);
    out.push('\n');
    out.push_str(TRAILER_ANNOTATED);
    out.push_str(": true\n");
    out
}

/// Returns the conventional fallback group for unowned paths.
pub fn unassigned_feature_name() -> &'static str {
    UNASSIGNED_FEATURE
}

fn find_latest_apply_commit(git: &GitAdapter) -> Result<Option<TrailerCommit>> {
    for commit in git.first_parent_commits(None, Some(HISTORY_LIMIT))? {
        let trailers = git.commit_trailers(&commit)?;
        if let Some(trailers) = parse_apply_trailers(&trailers)? {
            return Ok(Some(TrailerCommit {
                subject: git.commit_subject(&commit)?,
                trailers,
                commit,
            }));
        }
    }
    Ok(None)
}

fn find_latest_bpatch_commit(git: &GitAdapter) -> Result<Option<BpatchTrailerCommit>> {
    for commit in git.first_parent_commits(None, Some(HISTORY_LIMIT))? {
        let trailers = git.commit_trailers(&commit)?;
        let (base, kind) = if let Some(apply) = parse_apply_trailers(&trailers)? {
            (apply.base, BpatchCommitKind::Apply)
        } else if let Some(annotate) = parse_annotate_trailers(&trailers)? {
            (annotate.base, BpatchCommitKind::Annotate)
        } else {
            continue;
        };
        return Ok(Some(BpatchTrailerCommit {
            subject: git.commit_subject(&commit)?,
            commit,
            base,
            kind,
        }));
    }
    Ok(None)
}

fn find_latest_feature_subject(git: &GitAdapter) -> Result<Option<String>> {
    for commit in git.first_parent_commits(None, Some(HISTORY_LIMIT))? {
        if parse_apply_trailers(&git.commit_trailers(&commit)?)?
            .is_some_and(|trailers| !trailers.state_only)
        {
            return Ok(Some(git.commit_subject(&commit)?));
        }
    }
    Ok(None)
}

fn feature_commit_count(git: &GitAdapter, base: &str) -> Result<usize> {
    let range = format!("{base}..HEAD");
    let mut count = 0;
    for commit in git.first_parent_commits(Some(&range), None)? {
        if parse_apply_trailers(&git.commit_trailers(&commit)?)?
            .is_some_and(|trailers| !trailers.state_only)
        {
            count += 1;
        }
    }
    Ok(count)
}

fn applied_tree(
    git: &GitAdapter,
    store_dir: &Path,
    store_head: &str,
    entry: &TrailerCommit,
) -> Result<String> {
    if let Some(tree) = &entry.trailers.tree {
        return Ok(tree.clone());
    }
    if store_head == entry.trailers.store_rev {
        let store = Store::load(store_dir)?;
        let patches = store
            .patches()
            .values()
            .filter(|patch| store.stores_path(&patch.path))
            .map(|patch| store_dir.join(&patch.path))
            .collect::<Vec<_>>();
        return Ok(git.build_tree_from_patches(&entry.trailers.base, &patches)?);
    }

    // Old patch bytes may no longer exist after store history moved; the trailer
    // commit tree keeps hand-amended histories inspectable even without exact recompute.
    Ok(git.tree_id(&entry.commit)?)
}

fn detect_drift(
    git: &GitAdapter,
    store: &Store,
    baseline_tree: &str,
    baseline_subject: &str,
) -> Result<DriftState> {
    let mut files = Vec::new();
    for entry in git.diff_tree_name_status(baseline_tree, "HEAD^{tree}")? {
        if stores_path(store, &entry.path) {
            files.push(committed_drift(entry, baseline_subject));
        }
    }

    git.refresh_index()?;
    if git.diff_index_has_changes("HEAD")? {
        files.extend(parse_porcelain_drift(store, &git.status_porcelain_z()?)?);
    }

    if files.is_empty() {
        Ok(DriftState::Clean)
    } else {
        Ok(DriftState::Drifted { files })
    }
}

fn stores_path(store: &Store, path: &Path) -> bool {
    path.to_str().is_none_or(|path| store.stores_path(path))
}

fn committed_drift(entry: TreeDiffEntry, subject: &str) -> DriftFile {
    DriftFile {
        path: entry.path,
        status: entry.status,
        source: DriftSource::Committed,
        annotation: format!("modified since {subject}"),
    }
}

fn parse_porcelain_drift(store: &Store, bytes: &[u8]) -> Result<Vec<DriftFile>> {
    let mut parts = bytes.split(|byte| *byte == 0);
    let mut files = Vec::new();
    let mut seen = BTreeSet::new();
    while let Some(record) = parts.next() {
        if record.is_empty() {
            break;
        }
        let text = std::str::from_utf8(record)?;
        if text.len() < 4 {
            continue;
        }
        let status = &text[..2];
        let path = &text[3..];
        if status == "??" {
            continue;
        }
        if status.starts_with('R') || status.starts_with('C') {
            let _old_path = parts.next();
        }
        if store.stores_path(path) && seen.insert(path.to_string()) {
            files.push(DriftFile {
                path: PathBuf::from(path),
                status: status.trim().to_string(),
                source: DriftSource::Uncommitted,
                annotation: "modified, uncommitted".to_string(),
            });
        }
    }
    Ok(files)
}

fn base_display(git: &GitAdapter, base: &str) -> Result<String> {
    if let Some(bytes) = git.show_file(base, "chrome/VERSION")?
        && let Some(version) = chromium_version(&bytes)
    {
        return Ok(version);
    }
    git.short_rev(base).map_err(Into::into)
}

fn chromium_version(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let find = |key: &str| -> Option<&str> {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
    };
    Some(format!(
        "{}.{}.{}.{}",
        find("MAJOR")?,
        find("MINOR")?,
        find("BUILD")?,
        find("PATCH")?
    ))
}
