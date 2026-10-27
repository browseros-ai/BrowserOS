//! Unordered Chrome acquisition for accessibility snapshots.
//!
//! A frame's accessibility tree, clickable elements and document identity are fetched
//! concurrently and may complete in any order. `SnapshotBudget` is shared by every acquisition
//! stage in one capture so concurrency is bounded without scheduling CDP work onto a separate
//! runtime or throttling unrelated browser commands. Nothing here runs JavaScript in the page.

use super::{
    AxTreeResult, CaptureContext, DescribeNodeResult, GetFrameTreeResult, Observer,
    clickable::{ClickableDocument, find_clickables},
    collect_frame_documents,
};
use crate::{
    CoreError, FrameId, PageId, ProtocolSession,
    frames::FrameTarget,
    snapshot::{AxNode, DocumentId, IframeStitch},
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::json;
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::Semaphore;

const MAX_IN_FLIGHT_REQUESTS: usize = 8;

#[derive(Clone, Copy)]
pub(super) enum SnapshotStage {
    Capture,
    Ax,
    CursorScan,
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
        self.run(session.send(method, params)).await
    }

    /// Holds one permit while `call` awaits a single CDP response.
    pub(super) async fn run<T>(
        &self,
        call: impl std::future::Future<Output = Result<T, CoreError>>,
    ) -> Result<T, CoreError> {
        // A permit represents one pending CDP response, not a worker thread. Releasing it here
        // prevents dependent operations from holding capacity while they prepare their next call.
        let permit = self
            .permits
            .acquire()
            .await
            .map_err(|error| CoreError::Message(error.to_string()))?;
        let result = call.await;
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
        cross_process: bool,
        context: &CaptureContext,
    ) -> Result<AcquiredFrame, CoreError> {
        let mut target = self
            .frames
            .resolve_frame_target_with_epoch(
                self.page_id.clone(),
                frame_id.clone(),
                same_process_parent,
                cross_process,
                self.access
                    .as_ref()
                    .map(|access| access.epoch_guard())
                    .transpose()?
                    .flatten(),
            )
            .await?;
        if let Some(access) = &self.access {
            target.session = access
                .frame_session(target.session, frame_id.clone())
                .await?;
        }
        let runtime_document_id = if target.cursor_uses_session_default {
            None
        } else {
            runtime_document_id
        };
        acquire_frame_data(target, frame_id, runtime_document_id, context).await
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
                        child_frame.cross_process,
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
    context: &CaptureContext,
) -> Result<AcquiredFrame, CoreError> {
    let CaptureContext {
        root_session,
        frame_documents,
        dom_snapshots,
        budget,
        trace,
    } = context;
    let acquired_frame_id = frame_id.clone();
    let clickable_document = if target.cursor_uses_session_default {
        ClickableDocument::SessionDefault
    } else if let Some(backend_node_id) = runtime_document_id {
        ClickableDocument::BackendNode(backend_node_id)
    } else {
        ClickableDocument::Unavailable
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
        trace_stage(
            trace,
            acquired_frame_id.as_ref(),
            SnapshotStage::Ax,
            started,
            if result.is_ok() { "success" } else { "failure" },
        );
        result
    };
    let cursor_hits = async {
        let started = Instant::now();
        let result =
            find_clickables(dom_snapshots, &target.session, clickable_document, budget).await;
        trace_stage(
            trace,
            acquired_frame_id.as_ref(),
            SnapshotStage::CursorScan,
            started,
            if result.is_ok() { "success" } else { "failure" },
        );
        result
    };
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
    trace_stage(
        trace,
        frame_id.as_ref(),
        SnapshotStage::DocumentValidation,
        document_started,
        document_outcome,
    );

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
    /// The parent has no contentDocument for this child, so it lives in another process (an OOPIF).
    /// Lets frame resolution attach a not-yet-registered OOPIF instead of mis-querying it on the
    /// parent session, while same-process children keep inheriting the parent session.
    cross_process: bool,
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
    let cross_process = content_document.is_none();
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
        cross_process,
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

#[cfg(test)]
mod tests {
    use super::{CaptureContext, CaptureTrace, SnapshotBudget, SnapshotStage};
    use crate::observer::clickable::DomSnapshots;
    use crate::{
        CoreError, FrameId, PageId, ProtocolSession, SessionId, connection::CdpConnection,
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
        session: Option<SessionId>,
    }

    /// Answers a capture holding two documents: the session's root (document node 900) with
    /// clickable 100, and a same-process child (document node 901) with clickables 101 and 102.
    #[derive(Default)]
    struct CaptureConnection {
        calls: Mutex<Vec<Call>>,
        fail_ax_tree: bool,
        fail_capture: bool,
        child_loader_id: Option<&'static str>,
    }

    impl CaptureConnection {
        fn calls(&self) -> Vec<Call> {
            self.calls
                .lock()
                .map(|calls| calls.clone())
                .unwrap_or_default()
        }

        fn count(&self, method: &str) -> usize {
            self.calls()
                .iter()
                .filter(|call| call.method == method)
                .count()
        }
    }

    /// A document whose body holds one pointer `div` per backend id.
    fn captured_document(document_backend_id: i64, hits: &[i64]) -> Value {
        let mut parent_index = vec![-1, 0, 1];
        let mut node_type = vec![9, 1, 1];
        let mut node_name = vec![0, 1, 2];
        let mut backend_node_id = vec![document_backend_id, 1, 2];
        let mut layout_nodes = vec![1, 2];
        for &hit in hits {
            layout_nodes.push(parent_index.len());
            parent_index.push(2);
            node_type.push(1);
            node_name.push(3);
            backend_node_id.push(hit);
        }
        json!({
            "nodes": {
                "parentIndex": parent_index,
                "nodeType": node_type,
                "nodeName": node_name,
                "backendNodeId": backend_node_id,
                "attributes": vec![Vec::<i64>::new(); backend_node_id.len()],
            },
            "layout": {
                "nodeIndex": layout_nodes,
                "styles": layout_nodes.iter().enumerate().map(|(i, _)| vec![if i < 2 { 5 } else { 4 }]).collect::<Vec<_>>(),
                "bounds": vec![vec![0.0, 0.0, 40.0, 20.0]; layout_nodes.len()],
            }
        })
    }

    impl CdpConnection for CaptureConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            _params: Value,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                if let Ok(mut calls) = self.calls.lock() {
                    calls.push(Call {
                        method: method.to_string(),
                        session: session.cloned(),
                    });
                }
                match method {
                    "DOMSnapshot.captureSnapshot" if self.fail_capture => Err(CdpError::Protocol {
                        code: -32000,
                        message: "capture failed".to_string(),
                    }),
                    "DOMSnapshot.captureSnapshot" => Ok(json!({
                        "documents": [
                            captured_document(900, &[100]),
                            captured_document(901, &[101, 102]),
                        ],
                        "strings": ["#document", "HTML", "BODY", "DIV", "pointer", "auto"],
                    })),
                    "Accessibility.getFullAXTree" if self.fail_ax_tree => Err(CdpError::Protocol {
                        code: -32000,
                        message: "AX tree failed".to_string(),
                    }),
                    "Accessibility.getFullAXTree" => Ok(json!({"nodes": []})),
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
                                    "loaderId": self.child_loader_id.unwrap_or("child-loader"),
                                    "url": "https://example.com/frame"
                                }
                            }]
                        }
                    })),
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

    fn child_documents() -> std::collections::HashMap<Option<FrameId>, String> {
        std::collections::HashMap::from([(
            Some(FrameId("child-frame".to_string())),
            "child-frame:child-loader".to_string(),
        )])
    }

    fn context(
        root_session: &ProtocolSession,
        dom_snapshots: &DomSnapshots,
        budget: &SnapshotBudget,
    ) -> CaptureContext {
        CaptureContext {
            root_session: root_session.clone(),
            frame_documents: child_documents(),
            dom_snapshots: dom_snapshots.clone(),
            budget: budget.clone(),
            trace: CaptureTrace::new(PageId(1), 0),
        }
    }

    fn target(session: &ProtocolSession, session_default: bool) -> FrameTarget {
        FrameTarget {
            session: session.clone(),
            ax_params: json!({"frameId": "child-frame"}),
            cursor_uses_session_default: session_default,
        }
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

    #[tokio::test]
    async fn acquired_frame_contains_only_raw_inputs() -> Result<(), CoreError> {
        let connection = Arc::new(CaptureConnection::default());
        let session =
            ProtocolSession::for_session(connection, SessionId::from("page-session".to_string()));

        let acquired = super::acquire_frame_data(
            target(&session, false),
            Some(FrameId("child-frame".to_string())),
            Some(901),
            &context(&session, &DomSnapshots::default(), &SnapshotBudget::new()),
        )
        .await?;

        assert!(acquired.nodes.is_empty());
        let mut hits = acquired.cursor_hits.keys().copied().collect::<Vec<_>>();
        hits.sort_unstable();
        assert_eq!(hits, vec![101, 102]);
        assert_eq!(
            acquired.document_id.as_deref(),
            Some("child-frame:child-loader")
        );
        Ok(())
    }

    #[tokio::test]
    async fn acquisition_reads_the_dom_without_running_page_script() -> Result<(), CoreError> {
        let connection = Arc::new(CaptureConnection::default());
        let session = ProtocolSession::root(connection.clone());

        super::acquire_frame_data(
            target(&session, true),
            None,
            None,
            &context(&session, &DomSnapshots::default(), &SnapshotBudget::new()),
        )
        .await?;

        let calls = connection.calls();
        assert!(
            calls
                .iter()
                .any(|call| call.method == "DOMSnapshot.captureSnapshot")
        );
        for call in &calls {
            assert!(
                !call.method.starts_with("Runtime.")
                    && !matches!(
                        call.method.as_str(),
                        "DOM.resolveNode" | "DOM.setAttributeValue" | "DOM.removeAttribute"
                    ),
                "snapshot acquisition sent {}",
                call.method
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn frames_of_one_session_share_a_capture_and_read_their_own_document()
    -> Result<(), CoreError> {
        let connection = Arc::new(CaptureConnection::default());
        let session = ProtocolSession::for_session(
            connection.clone(),
            SessionId::from("page-session".to_string()),
        );
        let snapshots = DomSnapshots::default();
        let budget = SnapshotBudget::new();

        let top = super::acquire_frame_data(
            target(&session, true),
            None,
            None,
            &context(&session, &snapshots, &budget),
        )
        .await?;
        let child = super::acquire_frame_data(
            target(&session, false),
            Some(FrameId("child-frame".to_string())),
            Some(901),
            &context(&session, &snapshots, &budget),
        )
        .await?;

        assert_eq!(
            top.cursor_hits.keys().copied().collect::<Vec<_>>(),
            vec![100]
        );
        assert_eq!(child.cursor_hits.len(), 2);
        assert_eq!(connection.count("DOMSnapshot.captureSnapshot"), 1);
        Ok(())
    }

    #[tokio::test]
    async fn another_session_gets_its_own_capture() -> Result<(), CoreError> {
        let connection = Arc::new(CaptureConnection::default());
        let page = ProtocolSession::for_session(
            connection.clone(),
            SessionId::from("page-session".to_string()),
        );
        let oopif = ProtocolSession::for_session(
            connection.clone(),
            SessionId::from("oopif-session".to_string()),
        );
        let snapshots = DomSnapshots::default();
        let budget = SnapshotBudget::new();

        for session in [&page, &oopif] {
            super::acquire_frame_data(
                target(session, true),
                None,
                None,
                &context(&page, &snapshots, &budget),
            )
            .await?;
        }

        let captured_on = connection
            .calls()
            .into_iter()
            .filter(|call| call.method == "DOMSnapshot.captureSnapshot")
            .filter_map(|call| call.session.map(|session| session.0))
            .collect::<Vec<_>>();
        assert_eq!(captured_on, vec!["page-session", "oopif-session"]);
        Ok(())
    }

    #[tokio::test]
    async fn frame_acquisition_preserves_required_and_best_effort_failures() -> Result<(), CoreError>
    {
        let frame_id = FrameId("child-frame".to_string());

        let ax_failure = Arc::new(CaptureConnection {
            fail_ax_tree: true,
            ..CaptureConnection::default()
        });
        let ax_session = ProtocolSession::root(ax_failure);
        let result = super::acquire_frame_data(
            target(&ax_session, false),
            Some(frame_id.clone()),
            Some(901),
            &context(
                &ax_session,
                &DomSnapshots::default(),
                &SnapshotBudget::new(),
            ),
        )
        .await;
        assert!(result.is_err());

        let optional_failures = Arc::new(CaptureConnection {
            fail_capture: true,
            child_loader_id: Some("changed-loader"),
            ..CaptureConnection::default()
        });
        let optional_session = ProtocolSession::root(optional_failures.clone());
        let snapshots = DomSnapshots::default();
        for _ in 0..2 {
            let acquired = super::acquire_frame_data(
                target(&optional_session, false),
                Some(frame_id.clone()),
                Some(901),
                &context(&optional_session, &snapshots, &SnapshotBudget::new()),
            )
            .await?;
            assert!(acquired.cursor_hits.is_empty());
            assert!(acquired.document_id.is_none());
        }
        // A failed capture is remembered for the attempt instead of retried per frame.
        assert_eq!(optional_failures.count("DOMSnapshot.captureSnapshot"), 1);

        let unresolved = Arc::new(CaptureConnection::default());
        let unresolved_session = ProtocolSession::root(unresolved.clone());
        let acquired = super::acquire_frame_data(
            target(&unresolved_session, false),
            Some(frame_id),
            None,
            &context(
                &unresolved_session,
                &DomSnapshots::default(),
                &SnapshotBudget::new(),
            ),
        )
        .await?;
        assert!(acquired.cursor_hits.is_empty());
        assert_eq!(unresolved.count("DOMSnapshot.captureSnapshot"), 0);
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
            _params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                match method {
                    "Accessibility.getFullAXTree" => {
                        self.mark_and_wait(0b001, self.ax_gate.clone()).await?;
                        Ok(json!({"nodes": []}))
                    }
                    "DOMSnapshot.captureSnapshot" => {
                        self.mark_and_wait(0b010, self.cursor_gate.clone()).await?;
                        Ok(json!({"documents": [], "strings": []}))
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

    fn spawn_child_acquisition(
        connection: Arc<StageOverlapConnection>,
    ) -> tokio::task::JoinHandle<Result<super::AcquiredFrame, CoreError>> {
        let session = ProtocolSession::root(connection);
        tokio::spawn(async move {
            super::acquire_frame_data(
                target(&session, false),
                Some(FrameId("child-frame".to_string())),
                Some(901),
                &context(&session, &DomSnapshots::default(), &SnapshotBudget::new()),
            )
            .await
        })
    }

    #[tokio::test]
    async fn frame_ax_cursor_and_document_acquisition_overlap() -> Result<(), CoreError> {
        let connection = Arc::new(StageOverlapConnection::new());
        let task = spawn_child_acquisition(connection.clone());

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
        let task = spawn_child_acquisition(connection.clone());

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
}
