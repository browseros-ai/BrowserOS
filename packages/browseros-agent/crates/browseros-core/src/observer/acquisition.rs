//! Unordered Chrome acquisition for accessibility snapshots.
//!
//! Cursor candidates and their backend-node lookups may complete in any order, but this module
//! restores scan order before exposing results to rendering. `SnapshotBudget` is shared by every
//! acquisition stage in one capture so concurrency is bounded without scheduling CDP work onto a
//! separate runtime or throttling unrelated browser commands.

use super::{
    AxTreeResult, CaptureContext, DescribeNodeResult, GetFrameTreeResult, Observer,
    collect_frame_documents,
};
use crate::{
    CoreError, FrameId, PageId, ProtocolSession,
    frames::FrameTarget,
    snapshot::{AxNode, DocumentId, IframeStitch},
};
use browseros_cdp::runtime::GetPropertiesResult;
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::json;
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::Semaphore;
use uuid::Uuid;

const MAX_IN_FLIGHT_REQUESTS: usize = 8;
const MAX_MARKER_COLLISION_RETRIES: usize = 3;
const CURSOR_SCAN_JS: &str = include_str!("../assets/cursor-augment.js");

#[derive(Clone, Copy)]
pub(super) enum SnapshotStage {
    Capture,
    Ax,
    CursorScan,
    CursorDescribe,
    DocumentValidation,
    SiblingAcquisition,
    Assembly,
    Retry,
}

impl SnapshotStage {
    #[cfg(test)]
    const ALL: &[Self] = &[
        Self::Capture,
        Self::Ax,
        Self::CursorScan,
        Self::CursorDescribe,
        Self::DocumentValidation,
        Self::SiblingAcquisition,
        Self::Assembly,
        Self::Retry,
    ];

    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::Ax => "ax",
            Self::CursorScan => "cursor_scan",
            Self::CursorDescribe => "cursor_describe",
            Self::DocumentValidation => "document_validation",
            Self::SiblingAcquisition => "sibling_acquisition",
            Self::Assembly => "assembly",
            Self::Retry => "retry",
        }
    }
}

#[derive(Clone)]
pub(super) struct CaptureTrace {
    page_id: PageId,
    attempt: usize,
}

impl CaptureTrace {
    pub(super) fn new(page_id: PageId, attempt: usize) -> Self {
        Self { page_id, attempt }
    }
}

pub(super) fn trace_stage(
    trace: &CaptureTrace,
    frame_id: Option<&FrameId>,
    stage: SnapshotStage,
    started: Instant,
    outcome: &'static str,
) {
    tracing::debug!(
        target: "browseros.snapshot",
        snapshot_stage = stage.as_str(),
        page_id = %trace.page_id,
        frame_id = frame_id.map(|frame_id| frame_id.0.as_str()).unwrap_or("main"),
        attempt = trace.attempt,
        duration_ms = started.elapsed().as_secs_f64() * 1000.0,
        outcome,
        "snapshot stage complete"
    );
}

#[derive(Clone)]
pub(super) struct SnapshotBudget {
    permits: Arc<Semaphore>,
}

impl SnapshotBudget {
    pub(super) fn new() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(MAX_IN_FLIGHT_REQUESTS)),
        }
    }

    pub(super) async fn send<R>(
        &self,
        session: &ProtocolSession,
        method: &str,
        params: Value,
    ) -> Result<R, CoreError>
    where
        R: DeserializeOwned,
    {
        // A permit represents one pending CDP response, not a worker thread. Releasing it here
        // prevents dependent operations from holding capacity while they prepare their next call.
        let permit = self
            .permits
            .acquire()
            .await
            .map_err(|error| CoreError::Message(error.to_string()))?;
        let result = session.send(method, params).await;
        drop(permit);
        result
    }
}

/// Immutable Chrome inputs for one frame. Rendering receives this only after acquisition
/// completes, so no concurrent future can observe or mutate the capture's `RefMap`.
pub(super) struct AcquiredFrame {
    pub(super) frame_id: Option<FrameId>,
    pub(super) target: FrameTarget,
    pub(super) nodes: Vec<AxNode>,
    pub(super) cursor_hits: HashMap<i64, Vec<String>>,
    pub(super) document_id: Option<DocumentId>,
}

pub(super) struct AcquiredChild {
    pub(super) stitch: IframeStitch,
    pub(super) result: Result<AcquiredFrame, CoreError>,
    stitch_index: usize,
}

impl Observer {
    pub(super) async fn acquire_frame(
        &self,
        frame_id: Option<FrameId>,
        runtime_document_id: Option<i64>,
        same_process_parent: Option<&ProtocolSession>,
        context: &CaptureContext,
    ) -> Result<AcquiredFrame, CoreError> {
        let target = self
            .frames
            .resolve_frame_target(self.page_id.clone(), frame_id.clone(), same_process_parent)
            .await?;
        let runtime_document_id = if target.cursor_uses_session_default {
            None
        } else {
            runtime_document_id
        };
        acquire_frame_data(
            target,
            frame_id,
            runtime_document_id,
            &context.root_session,
            &context.frame_documents,
            &context.budget,
            Some(&context.trace),
        )
        .await
    }

    pub(super) async fn acquire_child_frames(
        &self,
        parent: &FrameTarget,
        parent_frame_id: Option<&FrameId>,
        stitches: &[IframeStitch],
        visited: &[FrameId],
        context: &CaptureContext,
    ) -> Vec<AcquiredChild> {
        let started = Instant::now();
        let mut pending = FuturesUnordered::new();
        for (stitch_index, stitch) in stitches.iter().cloned().enumerate() {
            let parent_session = parent.session.clone();
            let visited = visited.to_vec();
            let context = context.clone();
            pending.push(async move {
                let Some(child_frame) =
                    resolve_child_frame(&parent_session, stitch.backend_node_id, &context.budget)
                        .await
                else {
                    return (None, false);
                };
                if visited.contains(&child_frame.frame_id) {
                    return (None, true);
                }
                let result = self
                    .acquire_frame(
                        Some(child_frame.frame_id),
                        child_frame.runtime_document_id,
                        Some(&parent_session),
                        &context,
                    )
                    .await;
                (
                    Some(AcquiredChild {
                        stitch,
                        result,
                        stitch_index,
                    }),
                    false,
                )
            });
        }

        let mut acquired = Vec::with_capacity(stitches.len());
        let mut cycle_skips = 0;
        while let Some((child, skipped_cycle)) = pending.next().await {
            cycle_skips += usize::from(skipped_cycle);
            if let Some(child) = child {
                acquired.push(child);
            }
        }
        // Acquisition completion order is intentionally unordered. Re-establish the renderer's
        // stitch order here so assembly can reverse it exactly as the serialized implementation did.
        acquired.sort_by_key(|child| child.stitch_index);
        let expected_children = stitches.len().saturating_sub(cycle_skips);
        let outcome = if acquired.len() == expected_children
            && acquired.iter().all(|child| child.result.is_ok())
        {
            "success"
        } else {
            "partial"
        };
        trace_stage(
            &context.trace,
            parent_frame_id,
            SnapshotStage::SiblingAcquisition,
            started,
            outcome,
        );
        acquired
    }
}

async fn acquire_frame_data(
    target: FrameTarget,
    frame_id: Option<FrameId>,
    runtime_document_id: Option<i64>,
    root_session: &ProtocolSession,
    frame_documents: &HashMap<Option<FrameId>, DocumentId>,
    budget: &SnapshotBudget,
    trace: Option<&CaptureTrace>,
) -> Result<AcquiredFrame, CoreError> {
    let acquired_frame_id = frame_id.clone();
    let cursor_document = if target.cursor_uses_session_default {
        CursorDocument::SessionDefault
    } else if let Some(backend_node_id) = runtime_document_id {
        CursorDocument::BackendNode(backend_node_id)
    } else {
        CursorDocument::Unavailable
    };
    let ax_tree = async {
        let started = Instant::now();
        let result = budget
            .send::<AxTreeResult>(
                &target.session,
                "Accessibility.getFullAXTree",
                target.ax_params.clone(),
            )
            .await;
        if let Some(trace) = trace {
            trace_stage(
                trace,
                acquired_frame_id.as_ref(),
                SnapshotStage::Ax,
                started,
                if result.is_ok() { "success" } else { "failure" },
            );
        }
        result
    };
    let cursor_hits = find_cursor_hits_with_trace(
        &target.session,
        cursor_document,
        acquired_frame_id.as_ref(),
        budget,
        trace,
    );
    let document_started = Instant::now();
    let document_id =
        eager_document_id_for_frame(root_session, frame_id.clone(), frame_documents, budget);
    // These stages are independent after target resolution. AX failure remains fatal, while
    // cursor and document identity retain their existing best-effort fallback semantics.
    let (nodes, cursor_hits, document_id) = tokio::join!(ax_tree, cursor_hits, document_id);
    let nodes = nodes?.nodes;
    let (document_id, document_outcome) =
        revalidate_document_after_acquisition(root_session, frame_id.as_ref(), document_id, budget)
            .await;
    if let Some(trace) = trace {
        trace_stage(
            trace,
            frame_id.as_ref(),
            SnapshotStage::DocumentValidation,
            document_started,
            document_outcome,
        );
    }

    Ok(AcquiredFrame {
        frame_id: acquired_frame_id,
        target,
        nodes,
        cursor_hits: cursor_hits.unwrap_or_default(),
        document_id,
    })
}

struct ResolvedChildFrame {
    frame_id: FrameId,
    runtime_document_id: Option<i64>,
}

async fn resolve_child_frame(
    session: &ProtocolSession,
    backend_node_id: i64,
    budget: &SnapshotBudget,
) -> Option<ResolvedChildFrame> {
    let described = budget
        .send::<DescribeNodeResult>(
            session,
            "DOM.describeNode",
            json!({ "backendNodeId": backend_node_id, "depth": 1 }),
        )
        .await
        .ok()?;
    let content_document = described.node.content_document;
    let runtime_document_id = content_document
        .as_ref()
        .and_then(|node| node.backend_node_id);
    let frame_id = content_document
        .and_then(|node| node.frame_id)
        .or(described.node.frame_id)
        .map(FrameId)?;
    Some(ResolvedChildFrame {
        frame_id,
        runtime_document_id,
    })
}

async fn eager_document_id_for_frame(
    root_session: &ProtocolSession,
    frame_id: Option<FrameId>,
    frame_documents: &HashMap<Option<FrameId>, DocumentId>,
    budget: &SnapshotBudget,
) -> Option<DocumentId> {
    let before = frame_documents.get(&frame_id).cloned();
    if frame_id.is_none() || before.is_none() {
        return before;
    }
    let latest_result = budget
        .send::<GetFrameTreeResult>(root_session, "Page.getFrameTree", json!({}))
        .await;
    let latest = latest_result
        .as_ref()
        .ok()
        .map(|result| collect_frame_documents(&result.frame_tree));
    let after = latest.and_then(|latest| latest.get(&frame_id).cloned());
    (after == before).then_some(before).flatten()
}

async fn revalidate_document_after_acquisition(
    root_session: &ProtocolSession,
    frame_id: Option<&FrameId>,
    candidate: Option<DocumentId>,
    budget: &SnapshotBudget,
) -> (Option<DocumentId>, &'static str) {
    let Some(frame_id) = frame_id else {
        return (candidate, "cached");
    };
    let Some(candidate) = candidate else {
        return (None, "fallback");
    };
    let latest_result = budget
        .send::<GetFrameTreeResult>(root_session, "Page.getFrameTree", json!({}))
        .await;
    let latest = latest_result
        .as_ref()
        .ok()
        .map(|result| collect_frame_documents(&result.frame_tree));
    let after = latest.and_then(|latest| latest.get(&Some(frame_id.clone())).cloned());
    if latest_result.is_err() {
        (None, "failure")
    } else if after.as_ref() == Some(&candidate) {
        (Some(candidate), "success")
    } else {
        (None, "changed")
    }
}

#[derive(Debug, Deserialize)]
struct RuntimeEvalResult {
    result: RemoteObject,
}

#[derive(Debug, Deserialize)]
struct RemoteObject {
    value: Option<Value>,
    #[serde(rename = "objectId")]
    object_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResolveNodeResult {
    object: RemoteObject,
}

#[derive(Clone, Copy)]
enum CursorDocument {
    SessionDefault,
    BackendNode(i64),
    Unavailable,
}

#[derive(Debug, Default, Deserialize)]
struct CursorScanResult {
    collision: bool,
    candidates: Vec<CursorHit>,
}

#[derive(Debug, Deserialize)]
struct CursorHit {
    marker: String,
    reasons: Vec<String>,
}

struct CursorCleanup {
    session: ProtocolSession,
    budget: SnapshotBudget,
    marker_attribute: Option<String>,
    marker_token: Option<String>,
    object_group: String,
    document_object_id: Option<String>,
    armed: bool,
}

impl CursorCleanup {
    fn arm_marker(&mut self, marker_attribute: String, marker_token: String) {
        self.marker_attribute = Some(marker_attribute);
        self.marker_token = Some(marker_token);
    }

    async fn clear_marker(&mut self) {
        if let (Some(marker_attribute), Some(marker_token)) =
            (&self.marker_attribute, &self.marker_token)
        {
            cleanup_cursor_marker(
                &self.session,
                &self.budget,
                marker_attribute,
                marker_token,
                self.document_object_id.as_deref(),
            )
            .await;
            self.marker_attribute = None;
            self.marker_token = None;
        }
    }

    async fn finish(&mut self) {
        self.clear_marker().await;
        release_cursor_objects(&self.session, &self.budget, &self.object_group).await;
        self.armed = false;
    }
}

impl Drop for CursorCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let session = self.session.clone();
        let budget = self.budget.clone();
        let marker_attribute = self.marker_attribute.clone();
        let marker_token = self.marker_token.clone();
        let object_group = self.object_group.clone();
        let document_object_id = self.document_object_id.clone();

        // Cancellation skips the async epilogue, so detach one idempotent cleanup task for the
        // capture token. Cleanup matches both attribute and token, preserving page-owned values
        // even if a generated attribute name collides.
        drop(handle.spawn(async move {
            if let (Some(marker_attribute), Some(marker_token)) = (marker_attribute, marker_token) {
                cleanup_cursor_marker(
                    &session,
                    &budget,
                    &marker_attribute,
                    &marker_token,
                    document_object_id.as_deref(),
                )
                .await;
            }
            release_cursor_objects(&session, &budget, &object_group).await;
        }));
    }
}

#[cfg(test)]
async fn find_cursor_hits(
    session: &ProtocolSession,
    runtime_document_id: Option<i64>,
    budget: &SnapshotBudget,
) -> Result<HashMap<i64, Vec<String>>, CoreError> {
    let document =
        runtime_document_id.map_or(CursorDocument::SessionDefault, CursorDocument::BackendNode);
    find_cursor_hits_with_trace(session, document, None, budget, None).await
}

async fn find_cursor_hits_with_trace(
    session: &ProtocolSession,
    document: CursorDocument,
    frame_id: Option<&FrameId>,
    budget: &SnapshotBudget,
    trace: Option<&CaptureTrace>,
) -> Result<HashMap<i64, Vec<String>>, CoreError> {
    let setup_started = Instant::now();
    let capture_id = Uuid::new_v4().simple().to_string();
    let object_group = format!("browseros-snapshot-cursor-{capture_id}");
    // Arm object-group release before resolving a document wrapper: cancellation can arrive after
    // Chrome creates that remote object but before its response reaches this future.
    let mut cleanup = CursorCleanup {
        session: session.clone(),
        budget: budget.clone(),
        marker_attribute: None,
        marker_token: None,
        object_group: object_group.clone(),
        document_object_id: None,
        armed: true,
    };
    let document_object_id =
        match resolve_cursor_document(session, budget, document, &object_group).await {
            Ok(document_object_id) => document_object_id,
            Err(error) => {
                if let Some(trace) = trace {
                    trace_stage(
                        trace,
                        frame_id,
                        SnapshotStage::CursorScan,
                        setup_started,
                        "failure",
                    );
                }
                return Err(error);
            }
        };
    cleanup.document_object_id = document_object_id;
    for _attempt in 0..MAX_MARKER_COLLISION_RETRIES {
        // The random attribute isolates overlapping captures; the separate token lets cancellation
        // cleanup prove ownership before removing anything from the inspected document.
        let marker_token = Uuid::new_v4().simple().to_string();
        let marker_attribute = format!("data-__bcid-{marker_token}");
        cleanup.arm_marker(marker_attribute.clone(), marker_token.clone());
        let acquisition = CursorAcquisitionContext {
            marker_attribute: &marker_attribute,
            marker_token: &marker_token,
            object_group: &object_group,
            document_object_id: cleanup.document_object_id.as_deref(),
            frame_id,
            trace,
        };
        let result = acquire_cursor_hits(session, budget, &acquisition).await;
        cleanup.clear_marker().await;
        match result? {
            CursorAcquisition::Complete(hits) => {
                cleanup.finish().await;
                return Ok(hits);
            }
            CursorAcquisition::MarkerCollision => continue,
        }
    }
    cleanup.finish().await;
    Err(CoreError::Message(
        "Could not reserve a cursor marker namespace".to_string(),
    ))
}

async fn resolve_cursor_document(
    session: &ProtocolSession,
    budget: &SnapshotBudget,
    document: CursorDocument,
    object_group: &str,
) -> Result<Option<String>, CoreError> {
    match document {
        CursorDocument::SessionDefault => Ok(None),
        CursorDocument::Unavailable => Err(CoreError::Message(
            "Child frame document was unavailable for cursor acquisition".to_string(),
        )),
        CursorDocument::BackendNode(backend_node_id) => {
            // Resolving the child Document without an execution-context override returns its
            // default-world wrapper. Calling the scan on that object keeps page-assigned
            // `onclick` properties visible while still targeting the exact same-process frame.
            let resolved: ResolveNodeResult = budget
                .send(
                    session,
                    "DOM.resolveNode",
                    json!({
                        "backendNodeId": backend_node_id,
                        "objectGroup": object_group
                    }),
                )
                .await?;
            resolved.object.object_id.map(Some).ok_or_else(|| {
                CoreError::Message("Resolved frame document had no remote object".to_string())
            })
        }
    }
}

enum CursorAcquisition {
    Complete(HashMap<i64, Vec<String>>),
    MarkerCollision,
}

struct CursorAcquisitionContext<'a> {
    marker_attribute: &'a str,
    marker_token: &'a str,
    object_group: &'a str,
    document_object_id: Option<&'a str>,
    frame_id: Option<&'a FrameId>,
    trace: Option<&'a CaptureTrace>,
}

async fn acquire_cursor_hits(
    session: &ProtocolSession,
    budget: &SnapshotBudget,
    context: &CursorAcquisitionContext<'_>,
) -> Result<CursorAcquisition, CoreError> {
    let scan_started = Instant::now();
    let scan_result = async {
        let scan = call_document_function(
            session,
            budget,
            context.document_object_id,
            CURSOR_SCAN_JS,
            &[context.marker_attribute, context.marker_token],
            true,
            None,
        )
        .await?;
        let scan = scan
            .result
            .value
            .and_then(|value| serde_json::from_value::<CursorScanResult>(value).ok())
            .unwrap_or_default();
        if scan.collision {
            return Ok(CursorScanBatch::MarkerCollision);
        }
        let candidates = scan.candidates;
        if candidates.is_empty() {
            return Ok(CursorScanBatch::Candidates(candidates, Vec::new()));
        }

        // Markers are indexes in the full DOM scan, while `candidates` is compacted to matching
        // elements. Preserve the sparse markers in the remote array, then translate property names
        // back through each candidate's marker; otherwise skipped DOM nodes would mis-pair handles
        // and reasons. A node that disappears before collection leaves a hole instead of shifting
        // later candidates.
        let collection = call_document_function(
            session,
            budget,
            context.document_object_id,
            "function(a,t){var out=[],p=t+':';this.querySelectorAll('['+a+']').forEach(function(e){var v=e.getAttribute(a);if(v&&v.indexOf(p)===0)out[Number(v.slice(p.length))]=e;});return out;}",
            &[context.marker_attribute, context.marker_token],
            false,
            Some(context.object_group),
        )
        .await?;
        let Some(collection_id) = collection.result.object_id else {
            return Ok(CursorScanBatch::Candidates(candidates, Vec::new()));
        };
        let properties: GetPropertiesResult = budget
            .send(
                session,
                "Runtime.getProperties",
                json!({
                    "objectId": collection_id,
                    "ownProperties": true
                }),
            )
            .await?;
        let candidate_indexes = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| (candidate.marker.as_str(), index))
            .collect::<HashMap<_, _>>();
        let mut handles = properties
            .result
            .into_iter()
            .filter_map(|property| {
                let index = *candidate_indexes.get(property.name.as_str())?;
                let object_id = property.value?.object_id?;
                Some((index, object_id))
            })
            .collect::<Vec<_>>();
        handles.sort_by_key(|(index, _object_id)| *index);
        Ok::<_, CoreError>(CursorScanBatch::Candidates(candidates, handles))
    }
    .await;
    let (candidates, handles) = match scan_result {
        Ok(CursorScanBatch::Candidates(candidates, handles)) => {
            if let Some(trace) = context.trace {
                trace_stage(
                    trace,
                    context.frame_id,
                    SnapshotStage::CursorScan,
                    scan_started,
                    "success",
                );
            }
            (candidates, handles)
        }
        Ok(CursorScanBatch::MarkerCollision) => {
            if let Some(trace) = context.trace {
                trace_stage(
                    trace,
                    context.frame_id,
                    SnapshotStage::CursorScan,
                    scan_started,
                    "marker_collision",
                );
            }
            return Ok(CursorAcquisition::MarkerCollision);
        }
        Err(error) => {
            if let Some(trace) = context.trace {
                trace_stage(
                    trace,
                    context.frame_id,
                    SnapshotStage::CursorScan,
                    scan_started,
                    "failure",
                );
            }
            return Err(error);
        }
    };
    if candidates.is_empty() {
        if let Some(trace) = context.trace {
            trace_stage(
                trace,
                context.frame_id,
                SnapshotStage::CursorDescribe,
                Instant::now(),
                "skipped",
            );
        }
        return Ok(CursorAcquisition::Complete(HashMap::new()));
    }

    // Chrome may answer these in any order. Store each result in its scan-index slot and pair
    // reasons only after the whole batch completes; renderer input is therefore deterministic.
    let describe_started = Instant::now();
    let mut pending = FuturesUnordered::new();
    for (index, object_id) in handles {
        let session = session.clone();
        let budget = budget.clone();
        pending.push(async move {
            let described = budget
                .send::<DescribeNodeResult>(
                    &session,
                    "DOM.describeNode",
                    json!({ "objectId": object_id }),
                )
                .await;
            (index, described)
        });
    }

    let mut ordered = vec![None; candidates.len()];
    while let Some((index, described)) = pending.next().await {
        if let Ok(described) = described
            && let Some(backend_node_id) = described.node.backend_node_id
        {
            ordered[index] = Some(backend_node_id);
        }
    }

    let mut hits = HashMap::new();
    for (candidate, backend_node_id) in candidates.into_iter().zip(ordered) {
        if let Some(backend_node_id) = backend_node_id {
            hits.insert(backend_node_id, candidate.reasons);
        }
    }
    if let Some(trace) = context.trace {
        trace_stage(
            trace,
            context.frame_id,
            SnapshotStage::CursorDescribe,
            describe_started,
            "success",
        );
    }
    Ok(CursorAcquisition::Complete(hits))
}

enum CursorScanBatch {
    Candidates(Vec<CursorHit>, Vec<(usize, String)>),
    MarkerCollision,
}

async fn call_document_function(
    session: &ProtocolSession,
    budget: &SnapshotBudget,
    document_object_id: Option<&str>,
    function_declaration: &str,
    arguments: &[&str],
    return_by_value: bool,
    object_group: Option<&str>,
) -> Result<RuntimeEvalResult, CoreError> {
    let mut params = if let Some(object_id) = document_object_id {
        json!({
            "functionDeclaration": function_declaration,
            "objectId": object_id,
            "arguments": arguments
                .iter()
                .map(|value| json!({ "value": value }))
                .collect::<Vec<_>>(),
            "returnByValue": return_by_value
        })
    } else {
        let arguments = serde_json::to_string(arguments)
            .map_err(|error| CoreError::Message(error.to_string()))?;
        json!({
            "expression": format!("({function_declaration}).apply(document,{arguments})"),
            "returnByValue": return_by_value
        })
    };
    if let Some(object_group) = object_group {
        params["objectGroup"] = Value::String(object_group.to_string());
    }
    let method = if document_object_id.is_some() {
        "Runtime.callFunctionOn"
    } else {
        "Runtime.evaluate"
    };
    budget.send(session, method, params).await
}

async fn cleanup_cursor_marker(
    session: &ProtocolSession,
    budget: &SnapshotBudget,
    marker_attribute: &str,
    marker_token: &str,
    document_object_id: Option<&str>,
) {
    let _ = call_document_function(
        session,
        budget,
        document_object_id,
        "function(a,t){var p=t+':';this.querySelectorAll('['+a+']').forEach(function(e){var v=e.getAttribute(a);if(v&&v.indexOf(p)===0)e.removeAttribute(a);});}",
        &[marker_attribute, marker_token],
        true,
        None,
    )
    .await;
}

async fn release_cursor_objects(
    session: &ProtocolSession,
    budget: &SnapshotBudget,
    object_group: &str,
) {
    let _ = budget
        .send::<Value>(
            session,
            "Runtime.releaseObjectGroup",
            json!({ "objectGroup": object_group }),
        )
        .await;
}

#[cfg(test)]
mod tests {
    use super::{SnapshotBudget, SnapshotStage, find_cursor_hits};
    use crate::{
        CoreError, FrameId, ProtocolSession, SessionId, connection::CdpConnection,
        frames::FrameTarget,
    };
    use browseros_cdp::{CdpError, CdpEvent};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    };
    use tokio::sync::{Notify, Semaphore, broadcast};

    #[derive(Debug, Clone)]
    struct Call {
        method: String,
        params: Value,
        session: Option<SessionId>,
    }

    struct CursorConnection {
        calls: Mutex<Vec<Call>>,
        candidate_markers: [usize; 3],
        marker_collisions_remaining: AtomicUsize,
        omitted_candidate: Option<usize>,
        failed_candidate: Option<usize>,
        fail_ax_tree: bool,
        fail_cursor_scan: bool,
        child_loader_id: &'static str,
    }

    #[test]
    fn snapshot_trace_stages_are_distinct_and_complete() {
        let stages = SnapshotStage::ALL
            .iter()
            .map(|stage| stage.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            stages,
            vec![
                "capture",
                "ax",
                "cursor_scan",
                "cursor_describe",
                "document_validation",
                "sibling_acquisition",
                "assembly",
                "retry",
            ]
        );
        assert_eq!(
            stages
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            stages.len()
        );
    }

    impl Default for CursorConnection {
        fn default() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                candidate_markers: [0, 1, 2],
                marker_collisions_remaining: AtomicUsize::new(0),
                omitted_candidate: None,
                failed_candidate: None,
                fail_ax_tree: false,
                fail_cursor_scan: false,
                child_loader_id: "child-loader",
            }
        }
    }

    impl CursorConnection {
        fn calls(&self) -> Vec<Call> {
            self.calls
                .lock()
                .map(|calls| calls.clone())
                .unwrap_or_default()
        }

        fn take_marker_collision(&self) -> bool {
            self.marker_collisions_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
        }
    }

    impl CdpConnection for CursorConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                if let Ok(mut calls) = self.calls.lock() {
                    calls.push(Call {
                        method: method.to_string(),
                        params: params.clone(),
                        session: session.cloned(),
                    });
                }
                match method {
                    "Runtime.evaluate"
                        if params
                            .get("expression")
                            .and_then(Value::as_str)
                            .is_some_and(|expression| expression.contains("interactiveTags")) =>
                    {
                        if self.fail_cursor_scan {
                            return Err(CdpError::Protocol {
                                code: -32000,
                                message: "cursor scan failed".to_string(),
                            });
                        }
                        if self.take_marker_collision() {
                            return Ok(json!({
                                "result": {
                                    "type": "object",
                                    "value": {"collision": true, "candidates": []}
                                }
                            }));
                        }
                        Ok(json!({
                            "result": {
                                "type": "object",
                                "value": {
                                    "collision": false,
                                    "candidates": [
                                        {"marker": self.candidate_markers[0].to_string(), "reasons": ["cursor:pointer"]},
                                        {"marker": self.candidate_markers[1].to_string(), "reasons": ["onclick"]},
                                        {"marker": self.candidate_markers[2].to_string(), "reasons": ["contenteditable"]}
                                    ]
                                }
                            }
                        }))
                    }
                    "Runtime.evaluate"
                        if params.get("returnByValue") == Some(&Value::Bool(false)) =>
                    {
                        Ok(json!({
                            "result": {
                                "type": "object",
                                "objectId": "candidate-array"
                            }
                        }))
                    }
                    "Runtime.callFunctionOn"
                        if params
                            .get("functionDeclaration")
                            .and_then(Value::as_str)
                            .is_some_and(|expression| expression.contains("interactiveTags")) =>
                    {
                        if self.fail_cursor_scan {
                            return Err(CdpError::Protocol {
                                code: -32000,
                                message: "cursor scan failed".to_string(),
                            });
                        }
                        if self.take_marker_collision() {
                            return Ok(json!({
                                "result": {
                                    "type": "object",
                                    "value": {"collision": true, "candidates": []}
                                }
                            }));
                        }
                        Ok(json!({
                            "result": {
                                "type": "object",
                                "value": {
                                    "collision": false,
                                    "candidates": [
                                        {"marker": self.candidate_markers[0].to_string(), "reasons": ["cursor:pointer"]},
                                        {"marker": self.candidate_markers[1].to_string(), "reasons": ["onclick"]},
                                        {"marker": self.candidate_markers[2].to_string(), "reasons": ["contenteditable"]}
                                    ]
                                }
                            }
                        }))
                    }
                    "Runtime.callFunctionOn"
                        if params.get("returnByValue") == Some(&Value::Bool(false)) =>
                    {
                        Ok(json!({
                            "result": {
                                "type": "object",
                                "objectId": "candidate-array"
                            }
                        }))
                    }
                    "Runtime.callFunctionOn" => Ok(json!({
                        "result": {"type": "undefined"}
                    })),
                    "DOM.resolveNode" => Ok(json!({
                        "object": {"type": "object", "objectId": "document-object"}
                    })),
                    "Runtime.evaluate" => Ok(json!({
                        "result": {"type": "undefined"}
                    })),
                    "Runtime.getProperties" => {
                        let mut properties = vec![
                            json!({
                                "name": self.candidate_markers[2].to_string(),
                                "value": {"type": "object", "objectId": "candidate-2"},
                                "configurable": true,
                                "enumerable": true
                            }),
                            json!({
                                "name": self.candidate_markers[0].to_string(),
                                "value": {"type": "object", "objectId": "candidate-0"},
                                "configurable": true,
                                "enumerable": true
                            }),
                            json!({
                                "name": self.candidate_markers[1].to_string(),
                                "value": {"type": "object", "objectId": "candidate-1"},
                                "configurable": true,
                                "enumerable": true
                            }),
                            json!({
                                "name": "length",
                                "value": {"type": "number", "value": 3},
                                "configurable": false,
                                "enumerable": false
                            }),
                        ];
                        if let Some(index) = self.omitted_candidate {
                            let name = self.candidate_markers[index].to_string();
                            properties.retain(|property| property["name"] != name);
                        }
                        Ok(json!({ "result": properties }))
                    }
                    "DOM.describeNode" => {
                        let index = params
                            .get("objectId")
                            .and_then(Value::as_str)
                            .and_then(|object_id| object_id.rsplit('-').next())
                            .and_then(|index| index.parse::<usize>().ok());
                        if index == self.failed_candidate {
                            return Err(CdpError::Protocol {
                                code: -32000,
                                message: "candidate vanished".to_string(),
                            });
                        }
                        Ok(json!({
                            "node": {
                                "backendNodeId": index.map(|index| index as i64 + 100)
                            }
                        }))
                    }
                    "Accessibility.getFullAXTree" => {
                        if self.fail_ax_tree {
                            return Err(CdpError::Protocol {
                                code: -32000,
                                message: "AX tree failed".to_string(),
                            });
                        }
                        Ok(json!({"nodes": []}))
                    }
                    "Page.getFrameTree" => Ok(json!({
                        "frameTree": {
                            "frame": {
                                "id": "main",
                                "loaderId": "main-loader",
                                "url": "https://example.com/"
                            },
                            "childFrames": [{
                                "frame": {
                                    "id": "child-frame",
                                    "loaderId": self.child_loader_id,
                                    "url": "https://example.com/frame"
                                }
                            }]
                        }
                    })),
                    "Runtime.releaseObjectGroup" => Ok(json!({})),
                    _ => Ok(json!({})),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
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

    #[tokio::test]
    async fn acquired_frame_contains_only_raw_inputs() -> Result<(), CoreError> {
        let connection = Arc::new(CursorConnection::default());
        let session =
            ProtocolSession::for_session(connection, SessionId::from("page-session".to_string()));
        let frame_id = FrameId("child-frame".to_string());
        let target = FrameTarget {
            session: session.clone(),
            ax_params: json!({"frameId": frame_id.0}),
            cursor_uses_session_default: false,
        };
        let frame_documents = std::collections::HashMap::from([(
            Some(frame_id.clone()),
            "child-frame:child-loader".to_string(),
        )]);

        let acquired = super::acquire_frame_data(
            target,
            Some(frame_id),
            Some(901),
            &session,
            &frame_documents,
            &SnapshotBudget::new(),
            None,
        )
        .await?;

        assert!(acquired.nodes.is_empty());
        assert_eq!(acquired.cursor_hits.len(), 3);
        assert_eq!(
            acquired.document_id.as_deref(),
            Some("child-frame:child-loader")
        );
        Ok(())
    }

    struct StageOverlapConnection {
        started: AtomicU8,
        started_changed: Notify,
        ax_gate: Arc<Semaphore>,
        cursor_gate: Arc<Semaphore>,
        document_gate: Arc<Semaphore>,
        document_reads: AtomicUsize,
        document_responses: AtomicUsize,
        document_responded: Notify,
        loader_changed: AtomicBool,
    }

    impl StageOverlapConnection {
        fn new() -> Self {
            Self {
                started: AtomicU8::new(0),
                started_changed: Notify::new(),
                ax_gate: Arc::new(Semaphore::new(0)),
                cursor_gate: Arc::new(Semaphore::new(0)),
                document_gate: Arc::new(Semaphore::new(0)),
                document_reads: AtomicUsize::new(0),
                document_responses: AtomicUsize::new(0),
                document_responded: Notify::new(),
                loader_changed: AtomicBool::new(false),
            }
        }

        async fn mark_and_wait(&self, bit: u8, gate: Arc<Semaphore>) -> Result<(), CdpError> {
            self.started.fetch_or(bit, Ordering::SeqCst);
            self.started_changed.notify_waiters();
            let permit = gate
                .acquire_owned()
                .await
                .map_err(|error| CdpError::Protocol {
                    code: -1,
                    message: error.to_string(),
                })?;
            permit.forget();
            Ok(())
        }

        async fn wait_for_all_stages(&self) {
            loop {
                let notified = self.started_changed.notified();
                if self.started.load(Ordering::SeqCst) == 0b111 {
                    return;
                }
                notified.await;
            }
        }

        async fn wait_for_document_response(&self, target: usize) {
            loop {
                let notified = self.document_responded.notified();
                if self.document_responses.load(Ordering::SeqCst) >= target {
                    return;
                }
                notified.await;
            }
        }
    }

    impl CdpConnection for StageOverlapConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                match method {
                    "Accessibility.getFullAXTree" => {
                        self.mark_and_wait(0b001, self.ax_gate.clone()).await?;
                        Ok(json!({"nodes": []}))
                    }
                    "Runtime.evaluate"
                        if params
                            .get("expression")
                            .and_then(Value::as_str)
                            .is_some_and(|expression| expression.contains("interactiveTags")) =>
                    {
                        self.mark_and_wait(0b010, self.cursor_gate.clone()).await?;
                        Ok(json!({
                            "result": {
                                "type": "object",
                                "value": {
                                    "collision": false,
                                    "candidates": []
                                }
                            }
                        }))
                    }
                    "Runtime.callFunctionOn"
                        if params
                            .get("functionDeclaration")
                            .and_then(Value::as_str)
                            .is_some_and(|expression| expression.contains("interactiveTags")) =>
                    {
                        self.mark_and_wait(0b010, self.cursor_gate.clone()).await?;
                        Ok(json!({
                            "result": {
                                "type": "object",
                                "value": {
                                    "collision": false,
                                    "candidates": []
                                }
                            }
                        }))
                    }
                    "Page.getFrameTree" => {
                        let read_index = self.document_reads.fetch_add(1, Ordering::SeqCst);
                        if read_index == 0 {
                            self.mark_and_wait(0b100, self.document_gate.clone())
                                .await?;
                        }
                        let child_loader_id = if self.loader_changed.load(Ordering::SeqCst) {
                            "changed-loader"
                        } else {
                            "child-loader"
                        };
                        self.document_responses.fetch_add(1, Ordering::SeqCst);
                        self.document_responded.notify_waiters();
                        Ok(json!({
                            "frameTree": {
                                "frame": {
                                    "id": "main",
                                    "loaderId": "main-loader",
                                    "url": "https://example.com/"
                                },
                                "childFrames": [{
                                    "frame": {
                                        "id": "child-frame",
                                        "loaderId": child_loader_id,
                                        "url": "https://example.com/frame"
                                    }
                                }]
                            }
                        }))
                    }
                    "DOM.resolveNode" => Ok(json!({
                        "object": {"type": "object", "objectId": "document-object"}
                    })),
                    "Runtime.callFunctionOn" => Ok(json!({"result": {"type": "undefined"}})),
                    "Runtime.evaluate" => Ok(json!({"result": {"type": "undefined"}})),
                    "Runtime.releaseObjectGroup" => Ok(json!({})),
                    _ => Ok(json!({})),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
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

    struct GatedCursorConnection {
        candidate_count: usize,
        gates: Vec<Arc<Semaphore>>,
        active: AtomicUsize,
        max_active: AtomicUsize,
        entered: Notify,
        completions: Mutex<Vec<usize>>,
        completed: Notify,
        cleanup_evaluations: AtomicUsize,
        released_groups: AtomicUsize,
        cleaned: Notify,
        document_resolve_gate: Arc<Semaphore>,
        document_resolve_calls: AtomicUsize,
        document_resolve_started: Notify,
    }

    impl GatedCursorConnection {
        fn new(candidate_count: usize) -> Self {
            Self {
                candidate_count,
                gates: (0..candidate_count)
                    .map(|_index| Arc::new(Semaphore::new(0)))
                    .collect(),
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
                entered: Notify::new(),
                completions: Mutex::new(Vec::new()),
                completed: Notify::new(),
                cleanup_evaluations: AtomicUsize::new(0),
                released_groups: AtomicUsize::new(0),
                cleaned: Notify::new(),
                document_resolve_gate: Arc::new(Semaphore::new(0)),
                document_resolve_calls: AtomicUsize::new(0),
                document_resolve_started: Notify::new(),
            }
        }

        async fn wait_for_entries(&self, target: usize) {
            loop {
                let notified = self.entered.notified();
                if self.max_active.load(Ordering::SeqCst) >= target {
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

        async fn wait_for_cleanup(&self) {
            loop {
                let notified = self.cleaned.notified();
                if self.released_groups.load(Ordering::SeqCst) > 0 {
                    return;
                }
                notified.await;
            }
        }

        async fn wait_for_document_resolve(&self) {
            loop {
                let notified = self.document_resolve_started.notified();
                if self.document_resolve_calls.load(Ordering::SeqCst) > 0 {
                    return;
                }
                notified.await;
            }
        }

        fn completion_order(&self) -> Vec<usize> {
            self.completions
                .lock()
                .map(|completions| completions.clone())
                .unwrap_or_default()
        }
    }

    impl CdpConnection for GatedCursorConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                match method {
                    "Runtime.evaluate"
                        if params
                            .get("expression")
                            .and_then(Value::as_str)
                            .is_some_and(|expression| expression.contains("interactiveTags")) =>
                    {
                        let candidates = (0..self.candidate_count)
                            .map(|index| {
                                json!({
                                    "marker": index.to_string(),
                                    "reasons": [format!("reason-{index}")]
                                })
                            })
                            .collect::<Vec<_>>();
                        Ok(json!({
                            "result": {
                                "type": "object",
                                "value": {
                                    "collision": false,
                                    "candidates": candidates
                                }
                            }
                        }))
                    }
                    "Runtime.evaluate"
                        if params.get("returnByValue") == Some(&Value::Bool(false)) =>
                    {
                        Ok(json!({
                            "result": {
                                "type": "object",
                                "objectId": "candidate-array"
                            }
                        }))
                    }
                    "Runtime.evaluate" => {
                        if params
                            .get("expression")
                            .and_then(Value::as_str)
                            .is_some_and(|expression| expression.contains("removeAttribute(a)"))
                        {
                            self.cleanup_evaluations.fetch_add(1, Ordering::SeqCst);
                        }
                        Ok(json!({"result": {"type": "undefined"}}))
                    }
                    "Runtime.getProperties" => {
                        let properties = (0..self.candidate_count)
                            .map(|index| {
                                json!({
                                    "name": index.to_string(),
                                    "value": {
                                        "type": "object",
                                        "objectId": format!("candidate-{index}")
                                    },
                                    "configurable": true,
                                    "enumerable": true
                                })
                            })
                            .collect::<Vec<_>>();
                        Ok(json!({ "result": properties }))
                    }
                    "DOM.describeNode" => {
                        let Some(index) = params
                            .get("objectId")
                            .and_then(Value::as_str)
                            .and_then(|object_id| object_id.rsplit('-').next())
                            .and_then(|index| index.parse::<usize>().ok())
                        else {
                            return Err(CdpError::Protocol {
                                code: -1,
                                message: "missing candidate index".to_string(),
                            });
                        };
                        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                        self.max_active.fetch_max(active, Ordering::SeqCst);
                        self.entered.notify_waiters();
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
                        self.active.fetch_sub(1, Ordering::SeqCst);
                        if let Ok(mut completions) = self.completions.lock() {
                            completions.push(index);
                        }
                        self.completed.notify_waiters();
                        Ok(json!({"node": {"backendNodeId": index as i64 + 100}}))
                    }
                    "DOM.resolveNode" => {
                        self.document_resolve_calls.fetch_add(1, Ordering::SeqCst);
                        self.document_resolve_started.notify_waiters();
                        let permit = self
                            .document_resolve_gate
                            .clone()
                            .acquire_owned()
                            .await
                            .map_err(|error| CdpError::Protocol {
                                code: -1,
                                message: error.to_string(),
                            })?;
                        permit.forget();
                        Ok(json!({
                            "object": {"type": "object", "objectId": "document-object"}
                        }))
                    }
                    "Runtime.releaseObjectGroup" => {
                        self.released_groups.fetch_add(1, Ordering::SeqCst);
                        self.cleaned.notify_waiters();
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
            _session: Option<&'a SessionId>,
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

    #[tokio::test]
    async fn cursor_scan_batches_handles_and_restores_candidate_pairing() -> Result<(), CoreError> {
        let connection = Arc::new(CursorConnection::default());
        let session = ProtocolSession::for_session(
            connection.clone(),
            SessionId::from("page-session".to_string()),
        );

        let hits = find_cursor_hits(&session, None, &SnapshotBudget::new()).await?;

        assert_eq!(hits.get(&100), Some(&vec!["cursor:pointer".to_string()]));
        assert_eq!(hits.get(&101), Some(&vec!["onclick".to_string()]));
        assert_eq!(hits.get(&102), Some(&vec!["contenteditable".to_string()]));

        let calls = connection.calls();
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.method == "Runtime.getProperties")
                .count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.method == "DOM.describeNode")
                .count(),
            3
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.method == "Runtime.releaseObjectGroup")
                .count(),
            1
        );
        assert!(calls.iter().all(
            |call| call.session.as_ref().map(|session| session.0.as_str()) == Some("page-session")
        ));
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.method == "Runtime.evaluate")
                .count(),
            3
        );
        Ok(())
    }

    #[tokio::test]
    async fn sparse_cursor_markers_restore_compact_candidate_order() -> Result<(), CoreError> {
        let connection = Arc::new(CursorConnection {
            candidate_markers: [12, 37, 90],
            ..CursorConnection::default()
        });
        let session = ProtocolSession::root(connection);

        let hits = find_cursor_hits(&session, None, &SnapshotBudget::new()).await?;

        assert_eq!(hits.get(&100), Some(&vec!["cursor:pointer".to_string()]));
        assert_eq!(hits.get(&101), Some(&vec!["onclick".to_string()]));
        assert_eq!(hits.get(&102), Some(&vec!["contenteditable".to_string()]));
        Ok(())
    }

    #[tokio::test]
    async fn cursor_resolution_omits_vanished_candidates_and_cleans_its_namespace()
    -> Result<(), CoreError> {
        let connection = Arc::new(CursorConnection {
            omitted_candidate: Some(1),
            failed_candidate: Some(2),
            ..CursorConnection::default()
        });
        let session = ProtocolSession::root(connection.clone());

        let hits = find_cursor_hits(&session, None, &SnapshotBudget::new()).await?;

        assert_eq!(hits.len(), 1);
        assert_eq!(hits.get(&100), Some(&vec!["cursor:pointer".to_string()]));
        let calls = connection.calls();
        let cleanup = calls.iter().find(|call| {
            call.method == "Runtime.evaluate"
                && call
                    .params
                    .get("expression")
                    .and_then(Value::as_str)
                    .is_some_and(|expression| expression.contains("removeAttribute(a)"))
        });
        let released = calls
            .iter()
            .find(|call| call.method == "Runtime.releaseObjectGroup");
        assert!(cleanup.is_some());
        assert!(released.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn cursor_scans_use_unique_markers() -> Result<(), CoreError> {
        let connection = Arc::new(CursorConnection::default());
        let session = ProtocolSession::root(connection.clone());
        let budget = SnapshotBudget::new();

        let _first = find_cursor_hits(&session, None, &budget).await?;
        let _second = find_cursor_hits(&session, None, &budget).await?;

        let scan_expressions = connection
            .calls()
            .into_iter()
            .filter(|call| call.method == "Runtime.evaluate")
            .filter_map(|call| {
                call.params
                    .get("expression")
                    .and_then(Value::as_str)
                    .filter(|expression| expression.contains("interactiveTags"))
                    .map(ToString::to_string)
            })
            .collect::<Vec<_>>();
        assert_eq!(scan_expressions.len(), 2);
        assert_ne!(scan_expressions[0], scan_expressions[1]);
        assert!(
            scan_expressions
                .iter()
                .all(|expression| expression.contains("data-__bcid-"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn colliding_page_marker_is_preserved_and_retried() -> Result<(), CoreError> {
        let connection = Arc::new(CursorConnection {
            marker_collisions_remaining: AtomicUsize::new(1),
            ..CursorConnection::default()
        });
        let session = ProtocolSession::root(connection.clone());

        let hits = find_cursor_hits(&session, None, &SnapshotBudget::new()).await?;

        assert_eq!(hits.len(), 3);
        let calls = connection.calls();
        let scans = calls
            .iter()
            .filter(|call| {
                call.method == "Runtime.evaluate"
                    && call
                        .params
                        .get("expression")
                        .and_then(Value::as_str)
                        .is_some_and(|expression| expression.contains("interactiveTags"))
            })
            .collect::<Vec<_>>();
        assert_eq!(scans.len(), 2);
        assert_ne!(scans[0].params["expression"], scans[1].params["expression"]);
        let cleanups = calls
            .iter()
            .filter_map(|call| {
                call.params
                    .get("expression")
                    .and_then(Value::as_str)
                    .filter(|expression| expression.contains("removeAttribute(a)"))
            })
            .collect::<Vec<_>>();
        assert_eq!(cleanups.len(), 2);
        assert!(
            cleanups
                .iter()
                .all(|expression| expression.contains("v.indexOf(p)===0"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn child_cursor_scan_uses_main_world_document_in_target_session() -> Result<(), CoreError>
    {
        let connection = Arc::new(CursorConnection::default());
        let session = ProtocolSession::for_session(
            connection.clone(),
            SessionId::from("page-session".to_string()),
        );

        let _hits = find_cursor_hits(&session, Some(901), &SnapshotBudget::new()).await?;

        let calls = connection.calls();
        let resolved_document = calls.iter().find(|call| call.method == "DOM.resolveNode");
        assert_eq!(
            resolved_document.and_then(|call| call.params.get("backendNodeId")),
            Some(&json!(901))
        );
        assert!(calls.iter().all(|call| {
            call.session
                .as_ref()
                .is_some_and(|session| session.0 == "page-session")
        }));
        let scan = calls.iter().find(|call| {
            call.method == "Runtime.callFunctionOn"
                && call
                    .params
                    .get("functionDeclaration")
                    .and_then(Value::as_str)
                    .is_some_and(|function| function.contains("interactiveTags"))
        });
        assert_eq!(
            scan.and_then(|call| call.params.get("objectId")),
            Some(&json!("document-object"))
        );
        assert!(
            scan.and_then(|call| call.params.get("functionDeclaration"))
                .and_then(Value::as_str)
                .is_some_and(|function| function.contains("el.onclick!==null"))
        );
        assert!(
            calls
                .iter()
                .all(|call| call.method != "Page.createIsolatedWorld")
        );
        Ok(())
    }

    #[tokio::test]
    async fn oopif_cursor_scan_uses_target_session_default_main_world() -> Result<(), CoreError> {
        let connection = Arc::new(CursorConnection::default());
        let session = ProtocolSession::for_session(
            connection.clone(),
            SessionId::from("oopif-session".to_string()),
        );

        let _hits = find_cursor_hits(&session, None, &SnapshotBudget::new()).await?;

        let calls = connection.calls();
        let scan = calls.iter().find(|call| {
            call.method == "Runtime.evaluate"
                && call
                    .params
                    .get("expression")
                    .and_then(Value::as_str)
                    .is_some_and(|expression| expression.contains("interactiveTags"))
        });
        assert!(scan.is_some());
        assert_eq!(
            scan.and_then(|call| call.session.as_ref())
                .map(|session| session.0.as_str()),
            Some("oopif-session")
        );
        assert!(calls.iter().all(|call| call.method != "DOM.resolveNode"));
        assert!(
            calls
                .iter()
                .all(|call| call.params.get("contextId").is_none())
        );
        Ok(())
    }

    #[tokio::test]
    async fn cursor_describe_requests_never_exceed_capture_budget() -> Result<(), CoreError> {
        let connection = Arc::new(GatedCursorConnection::new(12));
        let session = ProtocolSession::root(connection.clone());
        let task =
            tokio::spawn(
                async move { find_cursor_hits(&session, None, &SnapshotBudget::new()).await },
            );

        connection.wait_for_entries(8).await;
        assert_eq!(connection.max_active.load(Ordering::SeqCst), 8);
        assert_eq!(connection.active.load(Ordering::SeqCst), 8);
        for gate in &connection.gates {
            gate.add_permits(1);
        }

        let hits = task
            .await
            .map_err(|error| CoreError::Message(error.to_string()))??;
        assert_eq!(hits.len(), 12);
        assert_eq!(connection.max_active.load(Ordering::SeqCst), 8);
        Ok(())
    }

    #[tokio::test]
    async fn out_of_order_cursor_describes_keep_candidate_reasons_aligned() -> Result<(), CoreError>
    {
        let connection = Arc::new(GatedCursorConnection::new(3));
        let session = ProtocolSession::root(connection.clone());
        let task =
            tokio::spawn(
                async move { find_cursor_hits(&session, None, &SnapshotBudget::new()).await },
            );

        connection.wait_for_entries(3).await;
        connection.gates[2].add_permits(1);
        connection.wait_for_completions(1).await;
        connection.gates[0].add_permits(1);
        connection.wait_for_completions(2).await;
        connection.gates[1].add_permits(1);

        let hits = task
            .await
            .map_err(|error| CoreError::Message(error.to_string()))??;
        assert_eq!(connection.completion_order(), vec![2, 0, 1]);
        assert_eq!(hits.get(&100), Some(&vec!["reason-0".to_string()]));
        assert_eq!(hits.get(&101), Some(&vec!["reason-1".to_string()]));
        assert_eq!(hits.get(&102), Some(&vec!["reason-2".to_string()]));
        Ok(())
    }

    #[tokio::test]
    async fn cancelling_cursor_resolution_still_cleans_marker_and_object_group()
    -> Result<(), CoreError> {
        let connection = Arc::new(GatedCursorConnection::new(1));
        let session = ProtocolSession::root(connection.clone());
        let task =
            tokio::spawn(
                async move { find_cursor_hits(&session, None, &SnapshotBudget::new()).await },
            );

        connection.wait_for_entries(1).await;
        task.abort();
        assert!(task.await.is_err());
        connection.wait_for_cleanup().await;
        assert_eq!(connection.cleanup_evaluations.load(Ordering::SeqCst), 1);
        assert_eq!(connection.released_groups.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn cancelling_document_resolution_still_releases_its_object_group()
    -> Result<(), CoreError> {
        let connection = Arc::new(GatedCursorConnection::new(1));
        let session = ProtocolSession::root(connection.clone());
        let task = tokio::spawn(async move {
            find_cursor_hits(&session, Some(901), &SnapshotBudget::new()).await
        });

        connection.wait_for_document_resolve().await;
        task.abort();
        assert!(task.await.is_err());
        connection.wait_for_cleanup().await;
        assert_eq!(connection.cleanup_evaluations.load(Ordering::SeqCst), 0);
        assert_eq!(connection.released_groups.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn frame_ax_cursor_and_document_acquisition_overlap() -> Result<(), CoreError> {
        let connection = Arc::new(StageOverlapConnection::new());
        let session = ProtocolSession::root(connection.clone());
        let frame_id = FrameId("child-frame".to_string());
        let target = FrameTarget {
            session: session.clone(),
            ax_params: json!({"frameId": frame_id.0}),
            cursor_uses_session_default: false,
        };
        let frame_documents = std::collections::HashMap::from([(
            Some(frame_id.clone()),
            "child-frame:child-loader".to_string(),
        )]);
        let task = tokio::spawn(async move {
            super::acquire_frame_data(
                target,
                Some(frame_id),
                Some(901),
                &session,
                &frame_documents,
                &SnapshotBudget::new(),
                None,
            )
            .await
        });

        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            connection.wait_for_all_stages(),
        )
        .await
        .map_err(|error| CoreError::Message(error.to_string()))?;
        connection.ax_gate.add_permits(1);
        connection.cursor_gate.add_permits(1);
        connection.document_gate.add_permits(1);

        let acquired = task
            .await
            .map_err(|error| CoreError::Message(error.to_string()))??;
        assert!(acquired.nodes.is_empty());
        assert!(acquired.cursor_hits.is_empty());
        assert_eq!(
            acquired.document_id.as_deref(),
            Some("child-frame:child-loader")
        );
        Ok(())
    }

    #[tokio::test]
    async fn child_navigation_after_eager_validation_uses_fallback_refs() -> Result<(), CoreError> {
        let connection = Arc::new(StageOverlapConnection::new());
        let session = ProtocolSession::root(connection.clone());
        let frame_id = FrameId("child-frame".to_string());
        let target = FrameTarget {
            session: session.clone(),
            ax_params: json!({"frameId": frame_id.0}),
            cursor_uses_session_default: false,
        };
        let frame_documents = std::collections::HashMap::from([(
            Some(frame_id.clone()),
            "child-frame:child-loader".to_string(),
        )]);
        let task = tokio::spawn(async move {
            super::acquire_frame_data(
                target,
                Some(frame_id),
                Some(901),
                &session,
                &frame_documents,
                &SnapshotBudget::new(),
                None,
            )
            .await
        });

        connection.wait_for_all_stages().await;
        connection.document_gate.add_permits(1);
        connection.wait_for_document_response(1).await;
        connection.loader_changed.store(true, Ordering::SeqCst);
        connection.ax_gate.add_permits(1);
        connection.cursor_gate.add_permits(1);

        let acquired = task
            .await
            .map_err(|error| CoreError::Message(error.to_string()))??;
        assert!(acquired.document_id.is_none());
        assert_eq!(connection.document_reads.load(Ordering::SeqCst), 2);
        Ok(())
    }

    #[tokio::test]
    async fn frame_acquisition_preserves_required_and_best_effort_failures() -> Result<(), CoreError>
    {
        let frame_id = FrameId("child-frame".to_string());
        let frame_documents = std::collections::HashMap::from([(
            Some(frame_id.clone()),
            "child-frame:child-loader".to_string(),
        )]);

        let ax_failure = Arc::new(CursorConnection {
            fail_ax_tree: true,
            ..CursorConnection::default()
        });
        let ax_session = ProtocolSession::root(ax_failure);
        let ax_target = FrameTarget {
            session: ax_session.clone(),
            ax_params: json!({"frameId": frame_id.0}),
            cursor_uses_session_default: false,
        };
        let result = super::acquire_frame_data(
            ax_target,
            Some(frame_id.clone()),
            Some(901),
            &ax_session,
            &frame_documents,
            &SnapshotBudget::new(),
            None,
        )
        .await;
        assert!(result.is_err());

        let optional_failures = Arc::new(CursorConnection {
            fail_cursor_scan: true,
            child_loader_id: "changed-loader",
            ..CursorConnection::default()
        });
        let optional_session = ProtocolSession::root(optional_failures);
        let optional_target = FrameTarget {
            session: optional_session.clone(),
            ax_params: json!({"frameId": frame_id.0}),
            cursor_uses_session_default: false,
        };
        let acquired = super::acquire_frame_data(
            optional_target,
            Some(frame_id),
            Some(901),
            &optional_session,
            &frame_documents,
            &SnapshotBudget::new(),
            None,
        )
        .await?;
        assert!(acquired.cursor_hits.is_empty());
        assert!(acquired.document_id.is_none());
        Ok(())
    }
}
