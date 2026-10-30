use crate::{
    CoreError, FrameId, PageId, ProtocolSession, Ref,
    frames::FrameRegistry,
    pages::{PageManager, PageSession},
    snapshot::{
        AxNode, DiffOptions, DocumentId, FrameError, FrameOwner, FrameReadStage, FrameSpan,
        RefEntry, RefMap, RenderOptions, SnapshotCoverage, SnapshotDiff, SnapshotObservation,
        SnapshotOptions, apply_snapshot_options, diff_snapshot_observations, ref_line,
        render_snapshot,
    },
    target::page_access::PageAccess,
};
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::Mutex;

mod acquisition;
mod clickable;
use acquisition::{
    AcquiredFrame, CaptureTrace, SnapshotBudget, SnapshotStage, log_frame_error, trace_stage,
};

const MAX_FRAME_DEPTH: usize = 5;
const MAX_STABLE_CAPTURE_ATTEMPTS: usize = 3;

#[derive(Debug, Clone)]
pub struct SnapshotResult {
    pub text: String,
    pub refs: RefMap,
    pub url: String,
    /// What this capture read: complete unless some frame failed, whose refs are then absent
    /// while every other frame's refs stay usable.
    pub coverage: SnapshotCoverage,
    /// The child frames whose contents this capture read.
    read_frames: Vec<FrameId>,
}

impl SnapshotResult {
    /// The ref and line this capture shows for DOM node `node` in `frame`. A filled field is found
    /// by its node, not by the ref the caller used: refs restart on a new document, where the
    /// same ref names another control, and a field the page re-rendered before the fill was
    /// filled under the old ref but carries a new one now. Read it from a full capture: card
    /// numbers and security codes are masked by their form's context, which one node lacks.
    #[must_use]
    pub fn line_for(&self, node: i64, frame: Option<&FrameId>) -> Option<(&Ref, &str)> {
        let entry = self
            .refs
            .entries_in_order()
            .into_iter()
            .find(|entry| entry.backend_node_id == node && entry.frame_id.as_ref() == frame)?;
        let line = ref_line(&self.text, entry.ref_id.as_str())?;
        Some((&entry.ref_id, line))
    }

    /// Whether `frame`'s contents may be missing from this capture: a child frame it did not
    /// read while some frame's read failed, which it may sit inside. A node of such a frame that
    /// the capture lacks is unknown, not gone.
    #[must_use]
    pub fn may_lack_frame(&self, frame: Option<&FrameId>) -> bool {
        frame.is_some_and(|frame| {
            !self.read_frames.contains(frame)
                && self
                    .coverage
                    .frame_errors
                    .iter()
                    .any(FrameError::contents_missing)
        })
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedElement {
    pub session: ProtocolSession,
    pub backend_node_id: i64,
    pub entry: RefEntry,
}

#[derive(Debug, Clone)]
struct RefScope {
    document_id: DocumentId,
    url: String,
}

#[derive(Debug, Clone)]
struct MainFrameState {
    url: String,
    document_id: Option<DocumentId>,
    frame_documents: HashMap<Option<FrameId>, DocumentId>,
}

#[derive(Debug, Clone)]
struct CaptureResult {
    text: String,
    refs: RefMap,
    url: String,
    scope: Option<RefScope>,
    coverage: SnapshotCoverage,
    frames: Vec<FrameSpan>,
}

impl CaptureResult {
    fn observation(&self) -> SnapshotObservation {
        SnapshotObservation {
            text: self.text.clone(),
            url: Some(self.url.clone()),
            coverage: self.coverage.clone(),
            frames: self.frames.clone(),
        }
    }

    fn into_snapshot(self) -> SnapshotResult {
        SnapshotResult {
            read_frames: self
                .frames
                .iter()
                .filter_map(|span| span.frame_id.clone())
                .collect(),
            text: self.text,
            refs: self.refs,
            url: self.url,
            coverage: self.coverage,
        }
    }
}

/// One frame's rendered lines with each child frame it read stitched in after its `- iframe`
/// line, where those children's contents sit, and what it and they failed to read.
struct Assembled {
    lines: Vec<String>,
    frames: Vec<FrameSpan>,
    frame_errors: Vec<FrameError>,
}

#[derive(Clone)]
struct CaptureContext {
    root_session: ProtocolSession,
    frame_documents: HashMap<Option<FrameId>, DocumentId>,
    /// One DOM capture per session for this attempt, shared by its same-process frames.
    dom_snapshots: clickable::DomSnapshots,
    budget: SnapshotBudget,
    trace: CaptureTrace,
}

#[derive(Debug, Default)]
struct ObserverState {
    baseline: Option<SnapshotObservation>,
    /// The arrangement committed alongside `baseline`, so a later diff can say
    /// whether any control actually moved rather than whether any text did.
    baseline_arrangement: Option<u64>,
    refs: RefMap,
    ref_scope: Option<RefScope>,
}

/// Shared ref/diff state for one reusable page, with an optional captured acquisition capability.
/// Bound wrappers share the cache while keeping per-call scope and incarnation admission private.
pub struct Observer {
    pages: Arc<PageManager>,
    frames: Arc<FrameRegistry>,
    page_id: PageId,
    state: Arc<Mutex<ObserverState>>,
    access: Option<Arc<PageAccess>>,
}

impl Observer {
    #[must_use]
    pub fn new(pages: Arc<PageManager>, frames: Arc<FrameRegistry>, page_id: PageId) -> Self {
        Self {
            pages,
            frames,
            page_id,
            state: Arc::new(Mutex::new(ObserverState::default())),
            access: None,
        }
    }

    pub(crate) fn with_access(&self, access: Arc<PageAccess>) -> Self {
        Self {
            pages: self.pages.clone(),
            frames: self.frames.clone(),
            page_id: self.page_id.clone(),
            state: self.state.clone(),
            access: Some(access),
        }
    }

    async fn page_session(&self) -> Result<PageSession, CoreError> {
        match &self.access {
            Some(access) => access.session().await,
            None => self.pages.get_session(self.page_id.clone()).await,
        }
    }

    pub async fn snapshot(&self) -> Result<SnapshotResult, CoreError> {
        self.snapshot_with_options(SnapshotOptions::default()).await
    }

    pub async fn snapshot_with_options(
        &self,
        options: SnapshotOptions,
    ) -> Result<SnapshotResult, CoreError> {
        let result = self.capture().await?;
        // Options filter only the returned presentation; commit keeps the full
        // capture as the next diff baseline.
        let text = apply_snapshot_options(&result.text, options);
        self.commit(&result).await;
        Ok(SnapshotResult {
            text,
            ..result.into_snapshot()
        })
    }

    pub async fn diff(&self) -> Result<SnapshotDiff, CoreError> {
        Ok(self.diff_with_capture().await?.0)
    }

    /// A diff plus the full capture it compared against the baseline, for a caller that reads
    /// that same capture again rather than whatever a later call commits.
    pub async fn diff_with_capture(&self) -> Result<(SnapshotDiff, SnapshotResult), CoreError> {
        let (before, before_arrangement) = {
            let state = self.state.lock().await;
            (state.baseline.clone(), state.baseline_arrangement)
        };
        let result = self.capture().await?;
        let after_arrangement = result.refs.arrangement();
        self.commit(&result).await;
        let mut diff = diff_snapshot_observations(
            before.as_ref(),
            &result.observation(),
            DiffOptions::default(),
        );
        // A url change already decided it, and that rule wins: an SPA can swap
        // the whole page behind an identical set of controls. A frame only one
        // side read leaves it unknown, since its refs exist on that side alone.
        if diff.arrangement_changed.is_none() && diff.uncompared.is_empty() {
            diff.arrangement_changed = before_arrangement.map(|before| before != after_arrangement);
        }
        Ok((diff, result.into_snapshot()))
    }

    pub async fn last_refs(&self) -> RefMap {
        self.state.lock().await.refs.clone()
    }

    /// The structural fingerprint of the last capture.
    ///
    /// Cheap, and the right pre-check before asking for a diff: an unchanged
    /// arrangement means no control moved, appeared or went away, whatever the
    /// page's countdowns and carousels did meanwhile.
    pub async fn arrangement(&self) -> u64 {
        self.state.lock().await.refs.arrangement()
    }

    pub async fn resolve_ref(&self, ref_id: &Ref) -> Result<ResolvedElement, CoreError> {
        let entry = self
            .state
            .lock()
            .await
            .refs
            .get(ref_id)
            .cloned()
            .ok_or_else(|| CoreError::UnknownRef(ref_id.clone()))?;
        let _page_session = self.page_session().await?;
        let mut target = self
            .frames
            .resolve_frame_target_with_epoch(
                self.page_id.clone(),
                entry.frame_id.clone(),
                None,
                false,
                self.access
                    .as_ref()
                    .map(|access| access.epoch_guard())
                    .transpose()?
                    .flatten(),
            )
            .await?;
        if let Some(access) = &self.access {
            target.session = access
                .frame_session(target.session, entry.frame_id.clone())
                .await?;
        }
        let mut entry_for_resolution = entry.clone();
        let resolved =
            resolve_ref_entry(&target.session, &mut entry_for_resolution, target.ax_params).await?;
        if entry_for_resolution.backend_node_id != entry.backend_node_id
            && let Some(stored) = self.state.lock().await.refs.get_mut(ref_id)
        {
            stored.backend_node_id = entry_for_resolution.backend_node_id;
        }
        if let Some(access) = &self.access {
            access.fresh().await?;
        }
        Ok(resolved)
    }

    async fn capture(&self) -> Result<CaptureResult, CoreError> {
        let capture_started = Instant::now();
        let initial_trace = CaptureTrace::new(self.page_id.clone(), 1);
        let page_session = match self.page_session().await {
            Ok(page_session) => page_session,
            Err(error) => {
                trace_stage(
                    &initial_trace,
                    None,
                    SnapshotStage::Capture,
                    capture_started,
                    "failure",
                );
                return Err(error);
            }
        };
        let budget = SnapshotBudget::new();
        for attempt in 1..=MAX_STABLE_CAPTURE_ATTEMPTS {
            let attempt_started = Instant::now();
            let trace = CaptureTrace::new(self.page_id.clone(), attempt);
            let before = self
                .read_main_frame_state(&page_session.session, &budget)
                .await;
            let refs = self.refs_for_capture(&before).await;
            let context = CaptureContext {
                root_session: page_session.session.clone(),
                frame_documents: before.frame_documents.clone(),
                dom_snapshots: clickable::DomSnapshots::default(),
                budget: budget.clone(),
                trace: trace.clone(),
            };
            let mut refs = refs;
            let frame_result = self.capture_root(&mut refs, context).await;
            let assembled = match frame_result {
                Ok(assembled) => assembled,
                Err(error) => {
                    trace_stage(
                        &trace,
                        None,
                        SnapshotStage::Capture,
                        capture_started,
                        "failure",
                    );
                    return Err(error);
                }
            };
            let after = self
                .read_main_frame_state(&page_session.session, &budget)
                .await;
            if !known_main_frame_changed(&before, &after) {
                trace_stage(
                    &trace,
                    None,
                    SnapshotStage::Capture,
                    capture_started,
                    "success",
                );
                if let Some(access) = &self.access {
                    access.fresh().await?;
                }
                return Ok(CaptureResult {
                    text: assembled.lines.join("\n"),
                    refs,
                    url: after.url.clone(),
                    scope: ref_scope_from(&after),
                    coverage: SnapshotCoverage {
                        frame_errors: assembled.frame_errors,
                    },
                    frames: assembled.frames,
                });
            }
            trace_stage(
                &trace,
                None,
                SnapshotStage::Retry,
                attempt_started,
                "document_changed",
            );
        }
        let trace = CaptureTrace::new(self.page_id.clone(), MAX_STABLE_CAPTURE_ATTEMPTS);
        trace_stage(
            &trace,
            None,
            SnapshotStage::Capture,
            capture_started,
            "document_changed",
        );
        Err(CoreError::DocumentChanged)
    }

    /// The main frame, which a capture needs: its read failing fails the capture.
    async fn capture_root(
        &self,
        refs: &mut RefMap,
        context: CaptureContext,
    ) -> Result<Assembled, CoreError> {
        let acquired = self
            .acquire_frame(None, None, None, false, &context)
            .await
            .map_err(|(_stage, error)| error)?;
        Ok(self
            .assemble_acquired_frame(acquired, None, refs, 0, Vec::new(), context)
            .await)
    }

    fn assemble_acquired_frame<'a>(
        &'a self,
        acquired: AcquiredFrame,
        owner: Option<FrameOwner>,
        refs: &'a mut RefMap,
        base_depth: usize,
        visited: Vec<FrameId>,
        context: CaptureContext,
    ) -> BoxFuture<'a, Assembled> {
        Box::pin(async move {
            let assembly_started = Instant::now();
            let AcquiredFrame {
                frame_id,
                target,
                nodes,
                cursor_hits,
                cursor_error,
                document_id,
            } = acquired;
            // A frame reports before the frames inside it, so errors read in document order.
            let mut frame_errors = Vec::new();
            if let Some(reason) = cursor_error {
                let error = FrameError {
                    frame_id: frame_id.clone(),
                    owner,
                    stage: FrameReadStage::CursorScan,
                    reason,
                };
                frame_errors.push(log_frame_error(
                    &context.trace,
                    error,
                    &"clickable scan failed",
                ));
            }
            let mut render_opts = RenderOptions {
                refs: &mut *refs,
                frame_id: frame_id.clone(),
                document_id,
                cursor_hits: Some(cursor_hits),
                base_depth,
            };
            let rendered = render_snapshot(&nodes, &mut render_opts);
            let lines = if rendered.text.is_empty() {
                Vec::new()
            } else {
                rendered
                    .text
                    .split('\n')
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            };
            if rendered.iframes.is_empty() || base_depth >= MAX_FRAME_DEPTH {
                trace_stage(
                    &context.trace,
                    frame_id.as_ref(),
                    SnapshotStage::Assembly,
                    assembly_started,
                    "success",
                );
                return Assembled {
                    lines,
                    frames: Vec::new(),
                    frame_errors,
                };
            }

            let children = self
                .acquire_child_frames(
                    &target,
                    frame_id.as_ref(),
                    &rendered.iframes,
                    &visited,
                    &context,
                )
                .await;
            // `acquire_child_frames` restores original stitch order after unordered completion.
            // Assemble in reverse because refs have historically been minted in reverse iframe
            // order; changing that would renumber stable public refs.
            let mut stitched = Vec::with_capacity(children.len());
            for child in children.into_iter().rev() {
                let assembled = match child.result {
                    Ok(acquired_child) => {
                        let child_frame_id = acquired_child.frame_id.clone();
                        let mut child_visited = visited.clone();
                        child_visited.extend(child_frame_id.clone());
                        let assembled = self
                            .assemble_acquired_frame(
                                acquired_child,
                                Some(child.owner.clone()),
                                refs,
                                child.stitch.depth + 1,
                                child_visited,
                                context.clone(),
                            )
                            .await;
                        Ok((child_frame_id, assembled))
                    }
                    Err(error) => Err(error),
                };
                stitched.push((child.stitch, child.owner, assembled));
            }
            stitched.reverse();

            let mut out = Vec::with_capacity(lines.len());
            let mut frames = Vec::new();
            let mut parent_lines = lines.into_iter();
            let mut taken = 0;
            for (stitch, owner, child) in stitched {
                // Each child's contents go right after its own `- iframe` line.
                let through_iframe_line = stitch.line_index + 1;
                out.extend(
                    parent_lines
                        .by_ref()
                        .take(through_iframe_line.saturating_sub(taken)),
                );
                taken = through_iframe_line;
                match child {
                    Err(error) => frame_errors.push(error),
                    Ok((frame_id, assembled)) => {
                        let start = out.len();
                        frames.push(FrameSpan {
                            frame_id,
                            owner,
                            lines: start..start + assembled.lines.len(),
                        });
                        frames.extend(assembled.frames.into_iter().map(|span| span.shifted(start)));
                        frame_errors.extend(assembled.frame_errors);
                        out.extend(assembled.lines);
                    }
                }
            }
            out.extend(parent_lines);
            trace_stage(
                &context.trace,
                frame_id.as_ref(),
                SnapshotStage::Assembly,
                assembly_started,
                "success",
            );
            Assembled {
                lines: out,
                frames,
                frame_errors,
            }
        })
    }

    async fn commit(&self, result: &CaptureResult) {
        let mut state = self.state.lock().await;
        state.baseline = Some(result.observation());
        state.baseline_arrangement = Some(result.refs.arrangement());
        state.refs = result.refs.clone();
        state.ref_scope = result.scope.clone();
    }

    async fn refs_for_capture(&self, state: &MainFrameState) -> RefMap {
        let current = self.state.lock().await;
        if should_reset_refs(current.ref_scope.as_ref(), state) {
            RefMap::new()
        } else {
            current.refs.fork_for_snapshot()
        }
    }

    async fn read_main_frame_state(
        &self,
        session: &ProtocolSession,
        budget: &SnapshotBudget,
    ) -> MainFrameState {
        let result = budget
            .send::<GetFrameTreeResult>(session, "Page.getFrameTree", json!({}))
            .await;
        if let Ok(result) = result {
            return MainFrameState {
                url: frame_url(&result.frame_tree.frame),
                document_id: frame_document_id(&result.frame_tree.frame),
                frame_documents: collect_frame_documents(&result.frame_tree),
            };
        }
        MainFrameState {
            url: self.read_registry_url().await,
            document_id: None,
            frame_documents: HashMap::new(),
        }
    }

    async fn read_registry_url(&self) -> String {
        self.pages
            .refresh(self.page_id.clone())
            .await
            .ok()
            .flatten()
            .map(|info| info.url)
            .unwrap_or_else(|| "unknown".to_string())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetFrameTreeResult {
    frame_tree: FrameTreeNode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameTreeNode {
    frame: Frame,
    child_frames: Option<Vec<FrameTreeNode>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Frame {
    id: String,
    loader_id: Option<String>,
    url: Option<String>,
    url_fragment: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AxTreeResult {
    nodes: Vec<AxNode>,
}

#[derive(Debug, Deserialize)]
struct ResolveNodeResult {
    object: Option<RemoteObject>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteObject {
    object_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DescribeNodeResult {
    node: DescribedNode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedNode {
    backend_node_id: Option<i64>,
    frame_id: Option<String>,
    content_document: Option<Box<DescribedNode>>,
}

pub async fn resolve_ref_entry(
    session: &ProtocolSession,
    entry: &mut RefEntry,
    ax_params: Value,
) -> Result<ResolvedElement, CoreError> {
    if is_live(session, entry.backend_node_id).await {
        return Ok(ResolvedElement {
            session: session.clone(),
            backend_node_id: entry.backend_node_id,
            entry: entry.clone(),
        });
    }

    let fresh = find_by_role_name_nth(&fetch_ax_tree(session, ax_params).await?, entry);
    let Some(fresh) = fresh else {
        return Err(CoreError::StaleRef {
            ref_id: entry.ref_id.clone(),
            role: entry.role.clone(),
            name: entry.name.clone(),
        });
    };
    entry.backend_node_id = fresh;
    Ok(ResolvedElement {
        session: session.clone(),
        backend_node_id: fresh,
        entry: entry.clone(),
    })
}

async fn is_live(session: &ProtocolSession, backend_node_id: i64) -> bool {
    let resolved = session
        .send::<_, ResolveNodeResult>(
            "DOM.resolveNode",
            json!({ "backendNodeId": backend_node_id }),
        )
        .await;
    let Ok(resolved) = resolved else {
        return false;
    };
    let object_id = resolved.object.and_then(|object| object.object_id);
    if let Some(object_id) = object_id {
        let _ = session
            .send::<_, Value>("Runtime.releaseObject", json!({ "objectId": object_id }))
            .await;
        true
    } else {
        false
    }
}

fn find_by_role_name_nth(nodes: &[AxNode], entry: &RefEntry) -> Option<i64> {
    let by_id = nodes
        .iter()
        .map(|node| (node.node_id.clone(), node))
        .collect::<HashMap<_, _>>();
    let roots = nodes
        .iter()
        .filter(|node| {
            role_of(node).is_some_and(|role| crate::snapshot::roles::is_root_role(&role))
        })
        .map(|node| node.node_id.clone())
        .collect::<Vec<_>>();
    let start = if roots.is_empty() {
        nodes
            .first()
            .map(|node| vec![node.node_id.clone()])
            .unwrap_or_default()
    } else {
        roots
    };

    let mut count = 0;
    let mut found = None;
    for id in start {
        visit_match(&by_id, &id, entry, &mut count, &mut found);
        if found.is_some() {
            break;
        }
    }
    found
}

fn visit_match(
    by_id: &HashMap<String, &AxNode>,
    id: &str,
    entry: &RefEntry,
    count: &mut usize,
    found: &mut Option<i64>,
) {
    if found.is_some() {
        return;
    }
    let Some(node) = by_id.get(id).copied() else {
        return;
    };
    if !node.ignored.unwrap_or(false)
        && node.backend_dom_node_id.is_some()
        && role_of(node).as_deref() == Some(entry.role.as_str())
        && name_of(node) == entry.name
    {
        if *count == entry.nth {
            *found = node.backend_dom_node_id;
            return;
        }
        *count += 1;
    }
    for child_id in node.child_ids.as_deref().unwrap_or(&[]) {
        visit_match(by_id, child_id, entry, count, found);
    }
}

async fn fetch_ax_tree(
    session: &ProtocolSession,
    ax_params: Value,
) -> Result<Vec<AxNode>, CoreError> {
    let result: AxTreeResult = session
        .send("Accessibility.getFullAXTree", ax_params)
        .await?;
    Ok(result.nodes)
}

fn known_main_frame_changed(before: &MainFrameState, after: &MainFrameState) -> bool {
    if known_urls_differ(&before.url, &after.url) {
        return true;
    }
    before.document_id.is_some()
        && after.document_id.is_some()
        && before.document_id != after.document_id
}

fn known_urls_differ(before: &str, after: &str) -> bool {
    before != "unknown" && after != "unknown" && before != after
}

fn should_reset_refs(current: Option<&RefScope>, next: &MainFrameState) -> bool {
    let Some(current) = current else {
        return true;
    };
    let Some(next_document_id) = &next.document_id else {
        return true;
    };
    current.document_id != *next_document_id || known_urls_differ(&current.url, &next.url)
}

fn ref_scope_from(state: &MainFrameState) -> Option<RefScope> {
    state.document_id.as_ref().map(|document_id| RefScope {
        document_id: document_id.clone(),
        url: state.url.clone(),
    })
}

fn collect_frame_documents(tree: &FrameTreeNode) -> HashMap<Option<FrameId>, DocumentId> {
    let mut documents = HashMap::new();
    visit_frame_documents(tree, true, &mut documents);
    documents
}

fn visit_frame_documents(
    node: &FrameTreeNode,
    is_root: bool,
    documents: &mut HashMap<Option<FrameId>, DocumentId>,
) {
    if let Some(document_id) = frame_document_id(&node.frame) {
        let frame_id = FrameId(node.frame.id.clone());
        documents.insert(
            if is_root {
                None
            } else {
                Some(frame_id.clone())
            },
            document_id.clone(),
        );
        documents.insert(Some(frame_id), document_id);
    }
    for child in node.child_frames.as_deref().unwrap_or(&[]) {
        visit_frame_documents(child, false, documents);
    }
}

/// The main frame's current document identity, as snapshot refs carry it.
pub(crate) async fn main_document_id(
    session: &ProtocolSession,
) -> Result<Option<DocumentId>, CoreError> {
    let result: GetFrameTreeResult = session.send("Page.getFrameTree", json!({})).await?;
    Ok(frame_document_id(&result.frame_tree.frame))
}

fn frame_document_id(frame: &Frame) -> Option<DocumentId> {
    frame
        .loader_id
        .as_ref()
        .map(|loader_id| format!("{}:{loader_id}", frame.id))
}

fn frame_url(frame: &Frame) -> String {
    let Some(url) = &frame.url else {
        return "unknown".to_string();
    };
    format!("{}{}", url, frame.url_fragment.as_deref().unwrap_or(""))
}

fn role_of(node: &AxNode) -> Option<String> {
    node.role
        .as_ref()
        .and_then(|value| value.value.as_ref())
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn name_of(node: &AxNode) -> String {
    node.name
        .as_ref()
        .and_then(|value| value.value.as_ref())
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::resolve_ref_entry;
    use crate::{
        BrowserSession, BrowserSessionHooks, CoreError, FrameId, ProtocolSession,
        connection::CdpConnection,
        snapshot::{
            AxNode, AxValue, FrameError, FrameOwner, FrameReadReason, FrameReadStage, SnapshotMode,
            SnapshotOptions, refs::MintRef,
        },
    };
    use browseros_cdp::{CdpError, CdpEvent};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use std::{
        collections::HashSet,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
        },
    };
    use tokio::sync::{Notify, Semaphore, broadcast};

    struct MockConnection {
        live: HashSet<i64>,
        ax_tree: Vec<AxNode>,
        releases: Mutex<Vec<String>>,
    }

    impl CdpConnection for MockConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            _session: Option<&'a crate::SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                match method {
                    "DOM.resolveNode" => {
                        let backend = params
                            .get("backendNodeId")
                            .and_then(Value::as_i64)
                            .unwrap_or_default();
                        if self.live.contains(&backend) {
                            Ok(json!({ "object": { "objectId": format!("obj-{backend}") } }))
                        } else {
                            Err(CdpError::Protocol {
                                code: -32000,
                                message: "No node with given id".to_string(),
                            })
                        }
                    }
                    "Accessibility.getFullAXTree" => Ok(json!({ "nodes": self.ax_tree })),
                    "Runtime.releaseObject" => {
                        if let Some(object_id) = params.get("objectId").and_then(Value::as_str)
                            && let Ok(mut releases) = self.releases.lock()
                        {
                            releases.push(object_id.to_string());
                        }
                        Ok(json!({}))
                    }
                    _ => Ok(json!({})),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a crate::SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_string()) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            let (_tx, rx) = broadcast::channel(1);
            rx
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    fn ax_button(node_id: &str, name: &str, backend_id: i64) -> AxNode {
        AxNode {
            node_id: node_id.to_string(),
            role: Some(AxValue::role("button")),
            name: Some(AxValue::string(name)),
            backend_dom_node_id: Some(backend_id),
            ..AxNode::default()
        }
    }

    fn ax_named(node_id: &str, role: &str, name: &str, children: &[&str]) -> AxNode {
        AxNode {
            node_id: node_id.to_string(),
            role: Some(AxValue::role(role)),
            name: (!name.is_empty()).then(|| AxValue::string(name)),
            child_ids: (!children.is_empty())
                .then(|| children.iter().map(|child| (*child).to_string()).collect()),
            ..AxNode::default()
        }
    }

    #[tokio::test]
    async fn resolve_ref_tier_one_returns_cached_backend_when_live() -> Result<(), CoreError> {
        let connection = Arc::new(MockConnection {
            live: HashSet::from([10]),
            ax_tree: Vec::new(),
            releases: Mutex::new(Vec::new()),
        });
        let session = ProtocolSession::root(connection.clone());
        let mut refs = crate::snapshot::RefMap::new();
        let ref_id = refs.mint(MintRef {
            backend_node_id: 10,
            role: "button",
            name: "OK",
            document_id: None,
            frame_id: None,
        });
        let mut entry = match refs.get(&ref_id).cloned() {
            Some(entry) => entry,
            None => return Err(CoreError::Message("missing ref".to_string())),
        };
        let resolved = resolve_ref_entry(&session, &mut entry, json!({})).await?;
        assert_eq!(resolved.backend_node_id, 10);
        let releases = match connection.releases.lock() {
            Ok(releases) => releases.clone(),
            Err(_err) => Vec::new(),
        };
        assert_eq!(releases, vec!["obj-10"]);
        Ok(())
    }

    #[tokio::test]
    async fn resolve_ref_tier_two_requeries_by_role_name_nth() -> Result<(), CoreError> {
        let ax_tree = vec![
            AxNode {
                node_id: "root".to_string(),
                role: Some(AxValue::role("RootWebArea")),
                child_ids: Some(vec!["a".to_string(), "b".to_string()]),
                ..AxNode::default()
            },
            ax_button("a", "OK", 20),
            ax_button("b", "OK", 21),
        ];
        let connection = Arc::new(MockConnection {
            live: HashSet::from([20, 21]),
            ax_tree,
            releases: Mutex::new(Vec::new()),
        });
        let session = ProtocolSession::root(connection);
        let mut refs = crate::snapshot::RefMap::new();
        refs.mint(MintRef {
            backend_node_id: 10,
            role: "button",
            name: "OK",
            document_id: None,
            frame_id: None,
        });
        let second = refs.mint(MintRef {
            backend_node_id: 11,
            role: "button",
            name: "OK",
            document_id: None,
            frame_id: None,
        });
        let mut entry = match refs.get(&second).cloned() {
            Some(entry) => entry,
            None => return Err(CoreError::Message("missing ref".to_string())),
        };
        let resolved = resolve_ref_entry(&session, &mut entry, json!({})).await?;
        assert_eq!(resolved.backend_node_id, 21);
        assert_eq!(entry.backend_node_id, 21);
        Ok(())
    }

    #[tokio::test]
    async fn resolve_ref_errors_when_stale_ref_cannot_be_refound() -> Result<(), CoreError> {
        let connection = Arc::new(MockConnection {
            live: HashSet::new(),
            ax_tree: Vec::new(),
            releases: Mutex::new(Vec::new()),
        });
        let session = ProtocolSession::root(connection);
        let mut refs = crate::snapshot::RefMap::new();
        let ref_id = refs.mint(MintRef {
            backend_node_id: 10,
            role: "button",
            name: "Gone",
            document_id: None,
            frame_id: None,
        });
        let mut entry = match refs.get(&ref_id).cloned() {
            Some(entry) => entry,
            None => return Err(CoreError::Message("missing ref".to_string())),
        };
        let result = resolve_ref_entry(&session, &mut entry, json!({})).await;
        assert!(matches!(result, Err(CoreError::StaleRef { .. })));
        Ok(())
    }

    #[derive(Clone)]
    struct HarnessState {
        loader_id: String,
        url: String,
        nodes: Vec<AxNode>,
        child_loader_id: Option<String>,
        child_nodes: Vec<AxNode>,
        fail_ax_tree: bool,
        fail_dom_capture: bool,
        frame_tree_reads: usize,
        ax_tree_reads: usize,
        change_child_loader_on_second_read: bool,
        main_loader_changes_remaining: usize,
    }

    struct HarnessConnection {
        state: Mutex<HarnessState>,
    }

    impl CdpConnection for HarnessConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            _session: Option<&'a crate::SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                let mut state = match self.state.lock() {
                    Ok(state) => state,
                    Err(_err) => {
                        return Err(CdpError::Protocol {
                            code: -1,
                            message: "poisoned test state".to_string(),
                        });
                    }
                };
                match method {
                    "Browser.getTabs" => Ok(json!({
                        "tabs": [tab_value(&state.url)]
                    })),
                    "Browser.getTabInfo" => Ok(json!({ "tab": tab_value(&state.url) })),
                    "Target.attachToTarget" => Ok(json!({ "sessionId": "session-1" })),
                    "Page.enable"
                    | "DOM.enable"
                    | "Runtime.enable"
                    | "Accessibility.enable"
                    | "Runtime.runIfWaitingForDebugger"
                    | "Target.setAutoAttach" => Ok(json!({})),
                    "Page.getFrameTree" => {
                        state.frame_tree_reads += 1;
                        if state.change_child_loader_on_second_read && state.frame_tree_reads == 2 {
                            state.child_loader_id = Some("child-loader-2".to_string());
                        }
                        if state.frame_tree_reads % 2 == 0
                            && state.main_loader_changes_remaining > 0
                        {
                            state.loader_id = format!("loader-{}", state.frame_tree_reads);
                            state.main_loader_changes_remaining -= 1;
                        }
                        Ok(json!({ "frameTree": frame_tree_value(&state) }))
                    }
                    "Accessibility.getFullAXTree" => {
                        state.ax_tree_reads += 1;
                        if state.fail_ax_tree {
                            return Err(CdpError::Protocol {
                                code: -32000,
                                message: "AX tree failed".to_string(),
                            });
                        }
                        if params.get("frameId").and_then(Value::as_str) == Some("child") {
                            Ok(json!({ "nodes": state.child_nodes }))
                        } else {
                            Ok(json!({ "nodes": state.nodes }))
                        }
                    }
                    "Runtime.evaluate" => Ok(json!({ "result": { "value": [] } })),
                    "DOMSnapshot.captureSnapshot" if state.fail_dom_capture => {
                        Err(CdpError::Timeout {
                            method: method.to_string(),
                        })
                    }
                    "DOMSnapshot.captureSnapshot" => Ok(dom_capture([CHILD_DOCUMENT])),
                    "DOM.describeNode" => Ok(json!({ "node": { "contentDocument": {
                        "frameId": "child",
                        "backendNodeId": CHILD_DOCUMENT
                    } } })),
                    _ => Ok(json!({})),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a crate::SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_string()) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            let (_tx, rx) = broadcast::channel(1);
            rx
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    struct SiblingConnection {
        started: AtomicU8,
        started_changed: Notify,
        gates: [Arc<Semaphore>; 2],
        completions: Mutex<Vec<usize>>,
        completed: Notify,
        fail_second_child: bool,
        cycle_first_child: AtomicBool,
        root_ax_reads: AtomicUsize,
        main_loader: Mutex<String>,
    }

    impl SiblingConnection {
        fn new(fail_second_child: bool) -> Self {
            Self {
                started: AtomicU8::new(0),
                started_changed: Notify::new(),
                gates: [Arc::new(Semaphore::new(0)), Arc::new(Semaphore::new(0))],
                completions: Mutex::new(Vec::new()),
                completed: Notify::new(),
                fail_second_child,
                cycle_first_child: AtomicBool::new(false),
                root_ax_reads: AtomicUsize::new(0),
                main_loader: Mutex::new("main-loader".to_string()),
            }
        }

        async fn wait_for_both_children(&self) {
            loop {
                let notified = self.started_changed.notified();
                if self.started.load(Ordering::SeqCst) == 0b11 {
                    return;
                }
                notified.await;
            }
        }

        async fn wait_for_completions(&self, target: usize) {
            loop {
                let notified = self.completed.notified();
                if self
                    .completions
                    .lock()
                    .map(|completions| completions.len())
                    .unwrap_or_default()
                    >= target
                {
                    return;
                }
                notified.await;
            }
        }
    }

    impl CdpConnection for SiblingConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            _session: Option<&'a crate::SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                match method {
                    "Browser.getTabs" => Ok(json!({
                        "tabs": [tab_value("https://example.com/")]
                    })),
                    "Browser.getTabInfo" => Ok(json!({
                        "tab": tab_value("https://example.com/")
                    })),
                    "Target.attachToTarget" => Ok(json!({ "sessionId": "session-1" })),
                    "Page.enable"
                    | "DOM.enable"
                    | "Runtime.enable"
                    | "Accessibility.enable"
                    | "Runtime.runIfWaitingForDebugger"
                    | "Target.setAutoAttach" => Ok(json!({})),
                    "Page.getFrameTree" => {
                        let loader_id = self
                            .main_loader
                            .lock()
                            .map(|loader| loader.clone())
                            .unwrap_or_else(|_error| "main-loader".to_string());
                        Ok(json!({
                            "frameTree": {
                                "frame": {
                                    "id": "main",
                                    "loaderId": loader_id,
                                    "url": "https://example.com/"
                                },
                                "childFrames": [
                                    {
                                        "frame": {
                                            "id": "child-a",
                                            "parentId": "main",
                                            "loaderId": "loader-a",
                                            "url": "https://example.com/a"
                                        }
                                    },
                                    {
                                        "frame": {
                                            "id": "child-b",
                                            "parentId": "main",
                                            "loaderId": "loader-b",
                                            "url": "https://example.com/b"
                                        }
                                    }
                                ]
                            }
                        }))
                    }
                    "Accessibility.getFullAXTree" => {
                        let frame_id = params.get("frameId").and_then(Value::as_str);
                        let Some(index) = (match frame_id {
                            Some("child-a") => Some(0),
                            Some("child-b") => Some(1),
                            _ => None,
                        }) else {
                            self.root_ax_reads.fetch_add(1, Ordering::SeqCst);
                            return Ok(json!({
                                "nodes": [
                                    root_with(&["frame-a", "frame-b"]),
                                    iframe_node("frame-a", 10),
                                    iframe_node("frame-b", 20)
                                ]
                            }));
                        };
                        self.started.fetch_or(1 << index, Ordering::SeqCst);
                        self.started_changed.notify_waiters();
                        let permit =
                            self.gates[index]
                                .clone()
                                .acquire_owned()
                                .await
                                .map_err(|error| CdpError::Protocol {
                                    code: -1,
                                    message: error.to_string(),
                                })?;
                        permit.forget();
                        if let Ok(mut completions) = self.completions.lock() {
                            completions.push(index);
                        }
                        self.completed.notify_waiters();
                        if index == 1 && self.fail_second_child {
                            return Err(CdpError::Protocol {
                                code: -32000,
                                message: "child capture failed".to_string(),
                            });
                        }
                        if index == 0 && self.cycle_first_child.load(Ordering::SeqCst) {
                            return Ok(json!({
                                "nodes": [
                                    root_with(&["cycle-a"]),
                                    iframe_node("cycle-a", 10)
                                ]
                            }));
                        }
                        let (node_id, name, backend_id) = if index == 0 {
                            ("button-a", "A", 101)
                        } else {
                            ("button-b", "B", 201)
                        };
                        Ok(json!({
                            "nodes": [
                                root_with(&[node_id]),
                                ax_button(node_id, name, backend_id)
                            ]
                        }))
                    }
                    "Runtime.evaluate" => Ok(json!({ "result": { "value": [] } })),
                    "Runtime.releaseObjectGroup" => Ok(json!({})),
                    "DOMSnapshot.captureSnapshot" => Ok(dom_capture([1010, 1020])),
                    "DOM.describeNode" => {
                        let backend = params.get("backendNodeId").and_then(Value::as_i64);
                        let frame_id = match backend {
                            Some(10) => Some("child-a"),
                            Some(20) => Some("child-b"),
                            _ => None,
                        };
                        Ok(json!({
                            "node": {
                                "contentDocument": {
                                    "frameId": frame_id,
                                    "backendNodeId": backend.map(|backend| backend + 1000)
                                }
                            }
                        }))
                    }
                    _ => Ok(json!({})),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a crate::SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_string()) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            let (_tx, rx) = broadcast::channel(1);
            rx
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    struct DepthConnection {
        max_child_requested: AtomicUsize,
    }

    impl CdpConnection for DepthConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            _session: Option<&'a crate::SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                match method {
                    "Browser.getTabs" => Ok(json!({
                        "tabs": [tab_value("https://example.com/")]
                    })),
                    "Browser.getTabInfo" => Ok(json!({
                        "tab": tab_value("https://example.com/")
                    })),
                    "Target.attachToTarget" => Ok(json!({ "sessionId": "session-1" })),
                    "Page.enable"
                    | "DOM.enable"
                    | "Runtime.enable"
                    | "Accessibility.enable"
                    | "Runtime.runIfWaitingForDebugger"
                    | "Target.setAutoAttach" => Ok(json!({})),
                    "Page.getFrameTree" => Ok(json!({
                        "frameTree": depth_frame_tree()
                    })),
                    "Accessibility.getFullAXTree" => {
                        let depth = params
                            .get("frameId")
                            .and_then(Value::as_str)
                            .and_then(|frame_id| frame_id.strip_prefix("child-"))
                            .and_then(|depth| depth.parse::<usize>().ok());
                        if let Some(depth) = depth {
                            self.max_child_requested.fetch_max(depth, Ordering::SeqCst);
                            return Ok(json!({
                                "nodes": [
                                    root_with(&["nested-frame"]),
                                    iframe_node("nested-frame", 100 + depth as i64)
                                ]
                            }));
                        }
                        Ok(json!({
                            "nodes": [
                                root_with(&["nested-frame"]),
                                iframe_node("nested-frame", 100)
                            ]
                        }))
                    }
                    "Runtime.evaluate" => Ok(json!({ "result": { "value": [] } })),
                    "Runtime.releaseObjectGroup" => Ok(json!({})),
                    "DOMSnapshot.captureSnapshot" => Ok(dom_capture(1100..1110)),
                    "DOM.describeNode" => {
                        let backend = params.get("backendNodeId").and_then(Value::as_i64);
                        let frame_id =
                            backend.map(|backend_id| format!("child-{}", backend_id - 99));
                        Ok(json!({
                            "node": {
                                "contentDocument": {
                                    "frameId": frame_id,
                                    "backendNodeId": backend.map(|backend| backend + 1000)
                                }
                            }
                        }))
                    }
                    _ => Ok(json!({})),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a crate::SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_string()) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            let (_tx, rx) = broadcast::channel(1);
            rx
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    async fn observer_harness(
        state: HarnessState,
    ) -> Result<(Arc<HarnessConnection>, Arc<super::Observer>), CoreError> {
        let connection = Arc::new(HarnessConnection {
            state: Mutex::new(state),
        });
        let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
        let pages = session.pages.list().await?;
        let Some(page) = pages.first() else {
            return Err(CoreError::Message("missing test page".to_string()));
        };
        let observer = session.observe(page.page_id.clone()).await;
        Ok((connection, observer))
    }

    async fn sibling_observer(
        connection: Arc<SiblingConnection>,
    ) -> Result<Arc<super::Observer>, CoreError> {
        let session = BrowserSession::new(connection, BrowserSessionHooks::default());
        let pages = session.pages.list().await?;
        let Some(page) = pages.first() else {
            return Err(CoreError::Message("missing test page".to_string()));
        };
        Ok(session.observe(page.page_id.clone()).await)
    }

    async fn depth_observer(
        connection: Arc<DepthConnection>,
    ) -> Result<Arc<super::Observer>, CoreError> {
        let session = BrowserSession::new(connection, BrowserSessionHooks::default());
        let pages = session.pages.list().await?;
        let Some(page) = pages.first() else {
            return Err(CoreError::Message("missing test page".to_string()));
        };
        Ok(session.observe(page.page_id.clone()).await)
    }

    /// The backend id of the harness child frame's document node.
    const CHILD_DOCUMENT: i64 = 5000;

    /// A session's DOM capture: the root document, then one document per child document node id,
    /// none of them holding a clickable element.
    fn dom_capture(child_documents: impl IntoIterator<Item = i64>) -> Value {
        let documents = std::iter::once(1)
            .chain(child_documents)
            .map(|document| json!({ "nodes": { "backendNodeId": [document] } }))
            .collect::<Vec<_>>();
        json!({ "documents": documents, "strings": [] })
    }

    fn tab_value(url: &str) -> Value {
        json!({
            "tabId": 101,
            "targetId": "target-1",
            "url": url,
            "title": "Test",
            "isActive": true,
            "isLoading": false,
            "loadProgress": 1,
            "isPinned": false,
            "isHidden": false,
            "windowId": 1
        })
    }

    fn depth_frame_tree() -> Value {
        let mut child = None;
        for depth in (1..=7).rev() {
            let mut node = json!({
                "frame": {
                    "id": format!("child-{depth}"),
                    "parentId": if depth == 1 {
                        "main".to_string()
                    } else {
                        format!("child-{}", depth - 1)
                    },
                    "loaderId": format!("loader-{depth}"),
                    "url": format!("https://example.com/{depth}")
                }
            });
            if let Some(nested) = child {
                node["childFrames"] = json!([nested]);
            }
            child = Some(node);
        }
        let mut root = json!({
            "frame": {
                "id": "main",
                "loaderId": "main-loader",
                "url": "https://example.com/"
            }
        });
        if let Some(child) = child {
            root["childFrames"] = json!([child]);
        }
        root
    }

    fn frame_tree_value(state: &HarnessState) -> Value {
        let mut tree = json!({
            "frame": {
                "id": "main",
                "loaderId": state.loader_id,
                "url": state.url
            }
        });
        if let Some(child_loader_id) = &state.child_loader_id {
            tree["childFrames"] = json!([
                {
                    "frame": {
                        "id": "child",
                        "parentId": "main",
                        "loaderId": child_loader_id,
                        "url": format!("{}frame", state.url)
                    }
                }
            ]);
        }
        tree
    }

    fn root_with(children: &[&str]) -> AxNode {
        AxNode {
            node_id: "1".to_string(),
            role: Some(AxValue::role("RootWebArea")),
            child_ids: Some(children.iter().map(|child| (*child).to_string()).collect()),
            ..AxNode::default()
        }
    }

    fn iframe_node(node_id: &str, backend_id: i64) -> AxNode {
        AxNode {
            node_id: node_id.to_string(),
            role: Some(AxValue::role("Iframe")),
            backend_dom_node_id: Some(backend_id),
            ..AxNode::default()
        }
    }

    fn harness_state(nodes: Vec<AxNode>) -> HarnessState {
        HarnessState {
            loader_id: "loader-1".to_string(),
            url: "https://example.com/".to_string(),
            nodes,
            child_loader_id: None,
            child_nodes: Vec::new(),
            fail_ax_tree: false,
            fail_dom_capture: false,
            frame_tree_reads: 0,
            ax_tree_reads: 0,
            change_child_loader_on_second_read: false,
            main_loader_changes_remaining: 0,
        }
    }

    #[tokio::test]
    async fn observer_diff_keeps_stable_refs_after_insertion() -> Result<(), CoreError> {
        let state = harness_state(vec![root_with(&["2", "3"]), ax_button("2", "A", 1), {
            let mut node = ax_button("3", "B", 2);
            node.role = Some(AxValue::role("link"));
            node
        }]);
        let (connection, observer) = observer_harness(state).await?;
        let _ = observer.snapshot().await?;
        if let Ok(mut state) = connection.state.lock() {
            state.nodes = vec![
                root_with(&["4", "2", "3"]),
                ax_button("4", "X", 3),
                ax_button("2", "A", 1),
                {
                    let mut node = ax_button("3", "B", 2);
                    node.role = Some(AxValue::role("link"));
                    node
                },
            ];
        }
        let diff = observer.diff().await?;
        assert_eq!(diff.added, 1);
        assert_eq!(diff.removed, 0);
        assert!(diff.text.contains("+ button \"X\" [ref=e3]"));
        Ok(())
    }

    #[tokio::test]
    async fn observer_reload_resets_public_ref_namespace() -> Result<(), CoreError> {
        let state = harness_state(vec![
            root_with(&["2", "3"]),
            ax_button("2", "A", 1),
            ax_button("3", "B", 2),
        ]);
        let (connection, observer) = observer_harness(state).await?;
        let _ = observer.snapshot().await?;
        if let Ok(mut state) = connection.state.lock() {
            state.loader_id = "loader-2".to_string();
            state.nodes = vec![root_with(&["4"]), ax_button("4", "Reloaded", 10)];
        }
        let snapshot = observer.snapshot().await?;
        assert_eq!(snapshot.text, "- button \"Reloaded\" [ref=e1]");
        Ok(())
    }

    #[tokio::test]
    async fn observer_failed_capture_does_not_replace_committed_refs() -> Result<(), CoreError> {
        let state = harness_state(vec![root_with(&["2"]), ax_button("2", "A", 1)]);
        let (connection, observer) = observer_harness(state).await?;
        let _ = observer.snapshot().await?;
        if let Ok(mut state) = connection.state.lock() {
            state.fail_ax_tree = true;
        }
        let result = observer.snapshot().await;
        assert!(result.is_err());
        let refs = observer.last_refs().await;
        assert_eq!(
            refs.get(&crate::Ref("e1".to_string()))
                .map(|entry| entry.backend_node_id),
            Some(1)
        );
        Ok(())
    }

    #[tokio::test]
    async fn observer_retries_once_when_main_document_changes() -> Result<(), CoreError> {
        let mut state = harness_state(vec![root_with(&["2"]), ax_button("2", "A", 1)]);
        state.main_loader_changes_remaining = 1;
        let (connection, observer) = observer_harness(state).await?;

        let snapshot = observer.snapshot().await?;

        assert_eq!(snapshot.text, "- button \"A\" [ref=e1]");
        let (frame_tree_reads, ax_tree_reads) = connection
            .state
            .lock()
            .map(|state| (state.frame_tree_reads, state.ax_tree_reads))
            .unwrap_or_default();
        assert_eq!(frame_tree_reads, 4);
        assert_eq!(ax_tree_reads, 2);
        Ok(())
    }

    #[tokio::test]
    async fn observer_document_change_exhaustion_keeps_committed_refs() -> Result<(), CoreError> {
        let state = harness_state(vec![root_with(&["2"]), ax_button("2", "A", 1)]);
        let (connection, observer) = observer_harness(state).await?;
        let _snapshot = observer.snapshot().await?;
        if let Ok(mut state) = connection.state.lock() {
            state.nodes = vec![root_with(&["3"]), ax_button("3", "B", 2)];
            state.frame_tree_reads = 0;
            state.ax_tree_reads = 0;
            state.main_loader_changes_remaining = 3;
        }

        let result = observer.snapshot().await;

        assert!(matches!(result, Err(CoreError::DocumentChanged)));
        let refs = observer.last_refs().await;
        assert_eq!(
            refs.get(&crate::Ref("e1".to_string()))
                .map(|entry| entry.backend_node_id),
            Some(1)
        );
        let ax_tree_reads = connection
            .state
            .lock()
            .map(|state| state.ax_tree_reads)
            .unwrap_or_default();
        assert_eq!(ax_tree_reads, 3);
        Ok(())
    }

    #[tokio::test]
    async fn observer_child_frame_document_churn_falls_back() -> Result<(), CoreError> {
        let mut state = harness_state(vec![root_with(&["2", "3"]), ax_button("2", "Outer", 1), {
            let mut iframe = AxNode {
                node_id: "3".to_string(),
                role: Some(AxValue::role("Iframe")),
                backend_dom_node_id: Some(2),
                ..AxNode::default()
            };
            iframe.name = None;
            iframe
        }]);
        state.child_loader_id = Some("child-loader-1".to_string());
        state.child_nodes = vec![
            root_with(&["child-button"]),
            ax_button("child-button", "Inner", 1),
        ];
        state.change_child_loader_on_second_read = true;
        let (_connection, observer) = observer_harness(state).await?;
        let snapshot = observer.snapshot().await?;
        assert_eq!(
            snapshot.text,
            [
                "- button \"Outer\" [ref=e1]",
                "- iframe",
                "  - button \"Inner\" [ref=e2]"
            ]
            .join("\n")
        );
        Ok(())
    }

    #[tokio::test]
    async fn observer_acquires_siblings_out_of_order_but_assembles_reverse_stitch_order()
    -> Result<(), CoreError> {
        let connection = Arc::new(SiblingConnection::new(false));
        let observer = sibling_observer(connection.clone()).await?;
        let task_observer = observer.clone();
        let task = tokio::spawn(async move { task_observer.snapshot().await });

        let overlap = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            connection.wait_for_both_children(),
        )
        .await;
        if overlap.is_err() {
            connection.gates[0].add_permits(1);
            connection.gates[1].add_permits(1);
            let _result = task
                .await
                .map_err(|error| CoreError::Message(error.to_string()))?;
            return Err(CoreError::Message(
                "sibling acquisition did not overlap".to_string(),
            ));
        }
        connection.gates[1].add_permits(1);
        connection.wait_for_completions(1).await;
        connection.gates[0].add_permits(1);
        let first = task
            .await
            .map_err(|error| CoreError::Message(error.to_string()))??;

        assert_eq!(
            first.text,
            [
                "- iframe",
                "  - button \"A\" [ref=e2]",
                "- iframe",
                "  - button \"B\" [ref=e1]"
            ]
            .join("\n")
        );
        connection.gates[0].add_permits(1);
        connection.gates[1].add_permits(1);
        let second = observer.snapshot().await?;
        assert_eq!(second.text, first.text);
        Ok(())
    }

    #[tokio::test]
    async fn observer_failed_child_does_not_consume_later_sibling_refs() -> Result<(), CoreError> {
        let connection = Arc::new(SiblingConnection::new(true));
        let observer = sibling_observer(connection.clone()).await?;
        let task = tokio::spawn(async move { observer.snapshot().await });

        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            connection.wait_for_both_children(),
        )
        .await
        .map_err(|error| CoreError::Message(error.to_string()))?;
        connection.gates[1].add_permits(1);
        connection.gates[0].add_permits(1);
        let snapshot = task
            .await
            .map_err(|error| CoreError::Message(error.to_string()))??;

        assert_eq!(
            snapshot.text,
            ["- iframe", "  - button \"A\" [ref=e1]", "- iframe"].join("\n")
        );
        assert_eq!(
            snapshot
                .refs
                .get(&crate::Ref("e1".to_string()))
                .map(|entry| entry.backend_node_id),
            Some(101)
        );
        assert!(snapshot.refs.get(&crate::Ref("e2".to_string())).is_none());
        Ok(())
    }

    #[tokio::test]
    async fn observer_reports_an_unreadable_frame_and_keeps_its_healthy_sibling()
    -> Result<(), CoreError> {
        let connection = Arc::new(SiblingConnection::new(true));
        let observer = sibling_observer(connection.clone()).await?;
        // One read for the capture, one for the ref recovery below.
        connection.gates[0].add_permits(2);
        connection.gates[1].add_permits(1);

        let snapshot = observer.snapshot().await?;

        assert_eq!(
            snapshot.text,
            ["- iframe", "  - button \"A\" [ref=e1]", "- iframe"].join("\n")
        );
        assert!(snapshot.coverage.is_partial());
        assert_eq!(
            snapshot.coverage.frame_errors,
            vec![FrameError {
                frame_id: Some(FrameId("child-b".to_string())),
                owner: Some(FrameOwner {
                    parent: None,
                    backend_node_id: 20,
                }),
                stage: FrameReadStage::Ax,
                reason: FrameReadReason::BrowserError,
            }]
        );
        let resolved = observer.resolve_ref(&crate::Ref("e1".to_string())).await?;
        assert_eq!(resolved.backend_node_id, 101);
        Ok(())
    }

    #[tokio::test]
    async fn observer_navigation_during_sibling_acquisition_retries_capture()
    -> Result<(), CoreError> {
        let connection = Arc::new(SiblingConnection::new(false));
        let observer = sibling_observer(connection.clone()).await?;
        let task = tokio::spawn(async move { observer.snapshot().await });

        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            connection.wait_for_both_children(),
        )
        .await
        .map_err(|error| CoreError::Message(error.to_string()))?;
        if let Ok(mut loader) = connection.main_loader.lock() {
            *loader = "main-loader-2".to_string();
        }
        connection.gates[0].add_permits(2);
        connection.gates[1].add_permits(2);
        let snapshot = task
            .await
            .map_err(|error| CoreError::Message(error.to_string()))??;

        assert_eq!(connection.root_ax_reads.load(Ordering::SeqCst), 2);
        assert!(snapshot.text.contains("button \"A\" [ref=e2]"));
        assert!(snapshot.text.contains("button \"B\" [ref=e1]"));
        Ok(())
    }

    #[tokio::test]
    async fn observer_skips_a_nested_frame_cycle() -> Result<(), CoreError> {
        let connection = Arc::new(SiblingConnection::new(false));
        connection.cycle_first_child.store(true, Ordering::SeqCst);
        let observer = sibling_observer(connection.clone()).await?;
        connection.gates[0].add_permits(1);
        connection.gates[1].add_permits(1);

        let snapshot = observer.snapshot().await?;

        assert_eq!(
            snapshot.text,
            [
                "- iframe",
                "  - iframe",
                "- iframe",
                "  - button \"B\" [ref=e1]"
            ]
            .join("\n")
        );
        Ok(())
    }

    #[tokio::test]
    async fn observer_nested_frames_stop_at_five_levels() -> Result<(), CoreError> {
        let connection = Arc::new(DepthConnection {
            max_child_requested: AtomicUsize::new(0),
        });
        let observer = depth_observer(connection.clone()).await?;

        let snapshot = observer.snapshot().await?;

        assert_eq!(connection.max_child_requested.load(Ordering::SeqCst), 5);
        assert_eq!(snapshot.text.lines().count(), 6);
        assert_eq!(snapshot.text.lines().last(), Some("          - iframe"));
        Ok(())
    }

    #[tokio::test]
    async fn observer_interactive_snapshot_commits_full_baseline_for_diff() -> Result<(), CoreError>
    {
        let state = harness_state(vec![
            root_with(&["2"]),
            ax_named("2", "main", "", &["3", "4"]),
            ax_named("3", "paragraph", "Intro", &[]),
            ax_button("4", "Save", 1),
        ]);
        let (connection, observer) = observer_harness(state).await?;
        let snapshot = observer
            .snapshot_with_options(SnapshotOptions {
                mode: SnapshotMode::Interactive,
                depth: None,
            })
            .await?;
        assert_eq!(snapshot.text, "- main\n  - button \"Save\" [ref=e1]");

        if let Ok(mut state) = connection.state.lock() {
            state.nodes = vec![
                root_with(&["2"]),
                ax_named("2", "main", "", &["3", "4", "5"]),
                ax_named("3", "paragraph", "Intro", &[]),
                ax_button("4", "Save", 1),
                ax_button("5", "Cancel", 2),
            ];
        }

        let diff = observer.diff().await?;
        assert_eq!(diff.added, 1);
        assert_eq!(diff.removed, 0);
        assert!(diff.text.contains("+   button \"Cancel\" [ref=e2]"));
        assert!(!diff.text.contains("+   paragraph \"Intro\""));
        Ok(())
    }
}
