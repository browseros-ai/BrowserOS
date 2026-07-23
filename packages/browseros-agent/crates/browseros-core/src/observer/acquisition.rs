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
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
use tokio::sync::Semaphore;

const MAX_IN_FLIGHT_REQUESTS: usize = 8;
const CURSOR_SCAN_JS: &str = include_str!("../assets/cursor-augment.js");
const CURSOR_WORLD_NAME: &str = "browseros-snapshot-cursor";
static NEXT_CURSOR_NAMESPACE: AtomicU64 = AtomicU64::new(1);

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
        context: &CaptureContext,
    ) -> Result<AcquiredFrame, CoreError> {
        let target = self
            .frames
            .resolve_frame_target(self.page_id.clone(), frame_id.clone())
            .await?;
        acquire_frame_data(
            target,
            frame_id,
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
                let child_frame_id = resolve_child_frame_id(
                    &parent_session,
                    stitch.backend_node_id,
                    &context.budget,
                )
                .await?;
                if visited.contains(&child_frame_id) {
                    return None;
                }
                let result = self.acquire_frame(Some(child_frame_id), &context).await;
                Some(AcquiredChild {
                    stitch,
                    result,
                    stitch_index,
                })
            });
        }

        let mut acquired = Vec::with_capacity(stitches.len());
        while let Some(child) = pending.next().await {
            if let Some(child) = child {
                acquired.push(child);
            }
        }
        // Acquisition completion order is intentionally unordered. Re-establish the renderer's
        // stitch order here so assembly can reverse it exactly as the serialized implementation did.
        acquired.sort_by_key(|child| child.stitch_index);
        let outcome = if acquired.len() == stitches.len()
            && acquired.iter().all(|child| child.result.is_ok())
        {
            "success"
        } else {
            "partial"
        };
        trace_stage(
            &context.trace,
            parent.runtime_frame_id.as_ref(),
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
    root_session: &ProtocolSession,
    frame_documents: &HashMap<Option<FrameId>, DocumentId>,
    budget: &SnapshotBudget,
    trace: Option<&CaptureTrace>,
) -> Result<AcquiredFrame, CoreError> {
    let acquired_frame_id = frame_id.clone();
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
        target.runtime_frame_id.as_ref(),
        budget,
        trace,
    );
    let document_id =
        stable_document_id_for_frame(root_session, frame_id, frame_documents, budget, trace);
    // These stages are independent after target resolution. AX failure remains fatal, while
    // cursor and document identity retain their existing best-effort fallback semantics.
    let (nodes, cursor_hits, document_id) = tokio::join!(ax_tree, cursor_hits, document_id);

    Ok(AcquiredFrame {
        frame_id: acquired_frame_id,
        target,
        nodes: nodes?.nodes,
        cursor_hits: cursor_hits.unwrap_or_default(),
        document_id,
    })
}

async fn resolve_child_frame_id(
    session: &ProtocolSession,
    backend_node_id: i64,
    budget: &SnapshotBudget,
) -> Option<FrameId> {
    let described = budget
        .send::<DescribeNodeResult>(
            session,
            "DOM.describeNode",
            json!({ "backendNodeId": backend_node_id, "depth": 1 }),
        )
        .await
        .ok()?;
    described
        .node
        .content_document
        .and_then(|node| node.frame_id)
        .or(described.node.frame_id)
        .map(FrameId)
}

async fn stable_document_id_for_frame(
    root_session: &ProtocolSession,
    frame_id: Option<FrameId>,
    frame_documents: &HashMap<Option<FrameId>, DocumentId>,
    budget: &SnapshotBudget,
    trace: Option<&CaptureTrace>,
) -> Option<DocumentId> {
    let started = Instant::now();
    let before = frame_documents.get(&frame_id).cloned();
    if frame_id.is_none() || before.is_none() {
        if let Some(trace) = trace {
            trace_stage(
                trace,
                frame_id.as_ref(),
                SnapshotStage::DocumentValidation,
                started,
                "cached",
            );
        }
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
    let stable = after == before;
    if let Some(trace) = trace {
        trace_stage(
            trace,
            frame_id.as_ref(),
            SnapshotStage::DocumentValidation,
            started,
            if latest_result.is_err() {
                "failure"
            } else if stable {
                "success"
            } else {
                "changed"
            },
        );
    }
    if stable { before } else { None }
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
struct GetPropertiesResult {
    result: Vec<PropertyDescriptor>,
}

#[derive(Debug, Deserialize)]
struct PropertyDescriptor {
    name: String,
    value: Option<RemoteObject>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateIsolatedWorldResult {
    execution_context_id: i64,
}

#[derive(Debug, Deserialize)]
struct CursorHit {
    reasons: Vec<String>,
}

struct CursorCleanup {
    session: ProtocolSession,
    budget: SnapshotBudget,
    marker_attribute: String,
    object_group: String,
    context_id: Option<i64>,
    armed: bool,
}

impl CursorCleanup {
    async fn finish(&mut self) {
        cleanup_cursor(
            &self.session,
            &self.budget,
            &self.marker_attribute,
            &self.object_group,
            self.context_id,
        )
        .await;
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
        let object_group = self.object_group.clone();
        let context_id = self.context_id;

        // Cancellation skips the async epilogue, so detach one idempotent cleanup
        // task for the capture namespace. Candidate requests are never spawned.
        drop(handle.spawn(async move {
            cleanup_cursor(
                &session,
                &budget,
                &marker_attribute,
                &object_group,
                context_id,
            )
            .await;
        }));
    }
}

#[cfg(test)]
async fn find_cursor_hits(
    session: &ProtocolSession,
    frame_id: Option<&FrameId>,
    budget: &SnapshotBudget,
) -> Result<HashMap<i64, Vec<String>>, CoreError> {
    find_cursor_hits_with_trace(session, frame_id, budget, None).await
}

async fn find_cursor_hits_with_trace(
    session: &ProtocolSession,
    frame_id: Option<&FrameId>,
    budget: &SnapshotBudget,
    trace: Option<&CaptureTrace>,
) -> Result<HashMap<i64, Vec<String>>, CoreError> {
    let namespace = NEXT_CURSOR_NAMESPACE.fetch_add(1, Ordering::Relaxed);
    // Overlapping captures share a document, so both marker cleanup and object release need a
    // capture-owned namespace. One capture must never remove another capture's temporary handles.
    let marker_attribute = format!("data-__bcid-{namespace}");
    let object_group = format!("browseros-snapshot-cursor-{namespace}");
    let setup_started = Instant::now();
    let context_id = match execution_context(session, frame_id, budget).await {
        Ok(context_id) => context_id,
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
    let mut cleanup = CursorCleanup {
        session: session.clone(),
        budget: budget.clone(),
        marker_attribute: marker_attribute.clone(),
        object_group: object_group.clone(),
        context_id,
        armed: true,
    };

    let result = acquire_cursor_hits(
        session,
        budget,
        &marker_attribute,
        &object_group,
        context_id,
        frame_id,
        trace,
    )
    .await;
    cleanup.finish().await;
    result
}

async fn execution_context(
    session: &ProtocolSession,
    frame_id: Option<&FrameId>,
    budget: &SnapshotBudget,
) -> Result<Option<i64>, CoreError> {
    let Some(frame_id) = frame_id else {
        return Ok(None);
    };
    let result: CreateIsolatedWorldResult = budget
        .send(
            session,
            "Page.createIsolatedWorld",
            json!({
                "frameId": frame_id.0,
                "worldName": CURSOR_WORLD_NAME
            }),
        )
        .await?;
    Ok(Some(result.execution_context_id))
}

async fn acquire_cursor_hits(
    session: &ProtocolSession,
    budget: &SnapshotBudget,
    marker_attribute: &str,
    object_group: &str,
    context_id: Option<i64>,
    frame_id: Option<&FrameId>,
    trace: Option<&CaptureTrace>,
) -> Result<HashMap<i64, Vec<String>>, CoreError> {
    let scan_started = Instant::now();
    let scan_result = async {
        let marker_literal = serde_json::to_string(marker_attribute)
            .map_err(|error| CoreError::Message(error.to_string()))?;
        let scan_expression =
            CURSOR_SCAN_JS.replace("__BROWSEROS_CURSOR_MARKER__", &marker_literal);
        let scan: RuntimeEvalResult = budget
            .send(
                session,
                "Runtime.evaluate",
                evaluate_params(scan_expression, true, None, context_id),
            )
            .await?;
        let candidates = scan
            .result
            .value
            .and_then(|value| serde_json::from_value::<Vec<CursorHit>>(value).ok())
            .unwrap_or_default();
        if candidates.is_empty() {
            return Ok((candidates, Vec::new()));
        }

        // Marker values are original scan indexes. Preserve them as sparse array positions so a
        // node that disappears before collection leaves a hole instead of shifting later reasons.
        let collection_expression = format!(
            "(function(a){{var out=[];document.querySelectorAll('['+a+']').forEach(function(e){{out[Number(e.getAttribute(a))]=e;}});return out;}})({marker_literal})"
        );
        let collection: RuntimeEvalResult = budget
            .send(
                session,
                "Runtime.evaluate",
                evaluate_params(collection_expression, false, Some(object_group), context_id),
            )
            .await?;
        let Some(collection_id) = collection.result.object_id else {
            return Ok((candidates, Vec::new()));
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
        let mut handles = properties
            .result
            .into_iter()
            .filter_map(|property| {
                let index = property.name.parse::<usize>().ok()?;
                let object_id = property.value?.object_id?;
                (index < candidates.len()).then_some((index, object_id))
            })
            .collect::<Vec<_>>();
        handles.sort_by_key(|(index, _object_id)| *index);
        Ok::<_, CoreError>((candidates, handles))
    }
    .await;
    let (candidates, handles) = match scan_result {
        Ok(result) => {
            if let Some(trace) = trace {
                trace_stage(
                    trace,
                    frame_id,
                    SnapshotStage::CursorScan,
                    scan_started,
                    "success",
                );
            }
            result
        }
        Err(error) => {
            if let Some(trace) = trace {
                trace_stage(
                    trace,
                    frame_id,
                    SnapshotStage::CursorScan,
                    scan_started,
                    "failure",
                );
            }
            return Err(error);
        }
    };
    if candidates.is_empty() {
        if let Some(trace) = trace {
            trace_stage(
                trace,
                frame_id,
                SnapshotStage::CursorDescribe,
                Instant::now(),
                "skipped",
            );
        }
        return Ok(HashMap::new());
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
    if let Some(trace) = trace {
        trace_stage(
            trace,
            frame_id,
            SnapshotStage::CursorDescribe,
            describe_started,
            "success",
        );
    }
    Ok(hits)
}

fn evaluate_params(
    expression: String,
    return_by_value: bool,
    object_group: Option<&str>,
    context_id: Option<i64>,
) -> Value {
    let mut params = json!({
        "expression": expression,
        "returnByValue": return_by_value
    });
    if let Some(object_group) = object_group {
        params["objectGroup"] = Value::String(object_group.to_string());
    }
    if let Some(context_id) = context_id {
        params["contextId"] = Value::Number(context_id.into());
    }
    params
}

async fn cleanup_cursor(
    session: &ProtocolSession,
    budget: &SnapshotBudget,
    marker_attribute: &str,
    object_group: &str,
    context_id: Option<i64>,
) {
    if let Ok(marker_literal) = serde_json::to_string(marker_attribute) {
        let expression = format!(
            "(function(a){{document.querySelectorAll('['+a+']').forEach(function(e){{e.removeAttribute(a);}});}})({marker_literal})"
        );
        let _ = budget
            .send::<Value>(
                session,
                "Runtime.evaluate",
                evaluate_params(expression, true, None, context_id),
            )
            .await;
    }
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
        atomic::{AtomicU8, AtomicUsize, Ordering},
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
                        Ok(json!({
                            "result": {
                                "type": "object",
                                "value": [
                                    {"marker": "0", "reasons": ["cursor:pointer"]},
                                    {"marker": "1", "reasons": ["onclick"]},
                                    {"marker": "2", "reasons": ["contenteditable"]}
                                ]
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
                    "Runtime.evaluate" => Ok(json!({
                        "result": {"type": "undefined"}
                    })),
                    "Runtime.getProperties" => {
                        let mut properties = vec![
                            json!({
                                "name": "2",
                                "value": {"type": "object", "objectId": "candidate-2"}
                            }),
                            json!({
                                "name": "0",
                                "value": {"type": "object", "objectId": "candidate-0"}
                            }),
                            json!({
                                "name": "1",
                                "value": {"type": "object", "objectId": "candidate-1"}
                            }),
                            json!({
                                "name": "length",
                                "value": {"type": "number", "value": 3}
                            }),
                        ];
                        if let Some(index) = self.omitted_candidate {
                            let name = index.to_string();
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
                    "Page.createIsolatedWorld" => Ok(json!({"executionContextId": 7})),
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
            runtime_frame_id: Some(frame_id.clone()),
        };
        let frame_documents = std::collections::HashMap::from([(
            Some(frame_id.clone()),
            "child-frame:child-loader".to_string(),
        )]);

        let acquired = super::acquire_frame_data(
            target,
            Some(frame_id),
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
    }

    impl StageOverlapConnection {
        fn new() -> Self {
            Self {
                started: AtomicU8::new(0),
                started_changed: Notify::new(),
                ax_gate: Arc::new(Semaphore::new(0)),
                cursor_gate: Arc::new(Semaphore::new(0)),
                document_gate: Arc::new(Semaphore::new(0)),
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
                                "value": []
                            }
                        }))
                    }
                    "Page.getFrameTree" => {
                        self.mark_and_wait(0b100, self.document_gate.clone())
                            .await?;
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
                                        "loaderId": "child-loader",
                                        "url": "https://example.com/frame"
                                    }
                                }]
                            }
                        }))
                    }
                    "Page.createIsolatedWorld" => Ok(json!({"executionContextId": 7})),
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
                                "value": candidates
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
                                    }
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
        assert!(calls.iter().all(|call| {
            call.params
                .get("expression")
                .and_then(Value::as_str)
                .is_none_or(|expression| !expression.contains("document.querySelector('"))
        }));
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
    async fn child_cursor_scan_uses_frame_context_in_target_session() -> Result<(), CoreError> {
        let connection = Arc::new(CursorConnection::default());
        let session = ProtocolSession::for_session(
            connection.clone(),
            SessionId::from("oopif-session".to_string()),
        );

        let _hits = find_cursor_hits(
            &session,
            Some(&crate::FrameId("child-frame".to_string())),
            &SnapshotBudget::new(),
        )
        .await?;

        let calls = connection.calls();
        let create_world = calls
            .iter()
            .find(|call| call.method == "Page.createIsolatedWorld");
        assert_eq!(
            create_world.and_then(|call| call.params.get("frameId")),
            Some(&json!("child-frame"))
        );
        assert!(calls.iter().all(|call| {
            call.session
                .as_ref()
                .is_some_and(|session| session.0 == "oopif-session")
        }));
        assert!(calls.iter().all(|call| {
            call.method != "Runtime.evaluate" || call.params.get("contextId") == Some(&json!(7))
        }));
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
    async fn frame_ax_cursor_and_document_acquisition_overlap() -> Result<(), CoreError> {
        let connection = Arc::new(StageOverlapConnection::new());
        let session = ProtocolSession::root(connection.clone());
        let frame_id = FrameId("child-frame".to_string());
        let target = FrameTarget {
            session: session.clone(),
            ax_params: json!({"frameId": frame_id.0}),
            runtime_frame_id: Some(frame_id.clone()),
        };
        let frame_documents = std::collections::HashMap::from([(
            Some(frame_id.clone()),
            "child-frame:child-loader".to_string(),
        )]);
        let task = tokio::spawn(async move {
            super::acquire_frame_data(
                target,
                Some(frame_id),
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
            runtime_frame_id: Some(frame_id.clone()),
        };
        let result = super::acquire_frame_data(
            ax_target,
            Some(frame_id.clone()),
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
            runtime_frame_id: Some(frame_id.clone()),
        };
        let acquired = super::acquire_frame_data(
            optional_target,
            Some(frame_id),
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
