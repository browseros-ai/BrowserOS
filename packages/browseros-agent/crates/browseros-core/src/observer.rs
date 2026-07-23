use crate::{
    CoreError, FrameId, PageId, ProtocolSession, Ref,
    frames::FrameRegistry,
    pages::PageManager,
    snapshot::{
        AxNode, DiffOptions, DocumentId, RefEntry, RefMap, RenderOptions, SnapshotDiff,
        SnapshotObservation, SnapshotOptions, apply_snapshot_options, diff_snapshot_observations,
        render_snapshot,
    },
};
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

const MAX_FRAME_DEPTH: usize = 5;
const MAX_STABLE_CAPTURE_ATTEMPTS: usize = 3;
const CURSOR_SCAN_JS: &str = include_str!("assets/cursor-augment.js");

#[derive(Debug, Clone)]
pub struct SnapshotResult {
    pub text: String,
    pub refs: RefMap,
    pub url: String,
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
}

#[derive(Debug, Default)]
struct ObserverState {
    baseline: Option<SnapshotObservation>,
    refs: RefMap,
    ref_scope: Option<RefScope>,
}

pub struct Observer {
    pages: Arc<PageManager>,
    frames: Arc<FrameRegistry>,
    page_id: PageId,
    state: Mutex<ObserverState>,
}

impl Observer {
    #[must_use]
    pub fn new(pages: Arc<PageManager>, frames: Arc<FrameRegistry>, page_id: PageId) -> Self {
        Self {
            pages,
            frames,
            page_id,
            state: Mutex::new(ObserverState::default()),
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
        self.commit(result.clone()).await;
        Ok(SnapshotResult {
            text,
            refs: result.refs,
            url: result.url,
        })
    }

    pub async fn diff(&self) -> Result<SnapshotDiff, CoreError> {
        let before = self.state.lock().await.baseline.clone();
        let result = self.capture().await?;
        self.commit(result.clone()).await;
        Ok(diff_snapshot_observations(
            before.as_ref(),
            &SnapshotObservation {
                text: result.text,
                url: Some(result.url),
            },
            DiffOptions::default(),
        ))
    }

    pub async fn last_refs(&self) -> RefMap {
        self.state.lock().await.refs.clone()
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
        let _page_session = self.pages.get_session(self.page_id.clone()).await?;
        let target = self
            .frames
            .resolve_frame_target(self.page_id.clone(), entry.frame_id.clone())
            .await?;
        let mut entry_for_resolution = entry.clone();
        let resolved =
            resolve_ref_entry(&target.session, &mut entry_for_resolution, target.ax_params).await?;
        if entry_for_resolution.backend_node_id != entry.backend_node_id
            && let Some(stored) = self.state.lock().await.refs.get_mut(ref_id)
        {
            stored.backend_node_id = entry_for_resolution.backend_node_id;
        }
        Ok(resolved)
    }

    async fn capture(&self) -> Result<CaptureResult, CoreError> {
        let page_session = self.pages.get_session(self.page_id.clone()).await?;
        for _attempt in 0..MAX_STABLE_CAPTURE_ATTEMPTS {
            let before = self.read_main_frame_state(&page_session.session).await;
            let refs = self.refs_for_capture(&before).await;
            let (text, refs) = self
                .capture_frame(
                    None,
                    refs,
                    0,
                    Vec::new(),
                    page_session.session.clone(),
                    before.frame_documents.clone(),
                )
                .await?;
            let after = self.read_main_frame_state(&page_session.session).await;
            if !known_main_frame_changed(&before, &after) {
                return Ok(CaptureResult {
                    text,
                    refs,
                    url: after.url.clone(),
                    scope: ref_scope_from(&after),
                });
            }
        }
        Err(CoreError::DocumentChanged)
    }

    fn capture_frame(
        &self,
        frame_id: Option<FrameId>,
        mut refs: RefMap,
        base_depth: usize,
        mut visited: Vec<FrameId>,
        root_session: ProtocolSession,
        frame_documents: HashMap<Option<FrameId>, DocumentId>,
    ) -> BoxFuture<'_, Result<(String, RefMap), CoreError>> {
        Box::pin(async move {
            if let Some(frame_id) = &frame_id {
                if visited.contains(frame_id) {
                    return Ok((String::new(), refs));
                }
                visited.push(frame_id.clone());
            }

            let target = self
                .frames
                .resolve_frame_target(self.page_id.clone(), frame_id.clone())
                .await?;
            let nodes = fetch_ax_tree(&target.session, target.ax_params.clone()).await?;
            let cursor_hits = find_cursor_hits(&target.session).await.unwrap_or_default();
            let document_id = self
                .stable_document_id_for_frame(&root_session, frame_id.clone(), &frame_documents)
                .await;
            let mut render_opts = RenderOptions {
                refs: &mut refs,
                frame_id: frame_id.clone(),
                document_id,
                cursor_hits: Some(cursor_hits),
                base_depth,
            };
            let rendered = render_snapshot(&nodes, &mut render_opts);
            let mut text = rendered.text;
            if rendered.iframes.is_empty() || base_depth >= MAX_FRAME_DEPTH {
                return Ok((text, refs));
            }

            let mut lines = if text.is_empty() {
                Vec::new()
            } else {
                text.split('\n')
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            };
            for stitch in rendered.iframes.iter().rev() {
                let child_frame_id =
                    resolve_child_frame_id(&target.session, stitch.backend_node_id).await;
                let Some(child_frame_id) = child_frame_id else {
                    continue;
                };
                let refs_before_child = refs.clone();
                let child_result = self
                    .capture_frame(
                        Some(child_frame_id),
                        refs.clone(),
                        stitch.depth + 1,
                        visited.clone(),
                        root_session.clone(),
                        frame_documents.clone(),
                    )
                    .await;
                let child_text = match child_result {
                    Ok((child_text, child_refs)) => {
                        refs = child_refs;
                        child_text
                    }
                    Err(_err) => {
                        refs = refs_before_child;
                        String::new()
                    }
                };
                if !child_text.is_empty() {
                    lines.insert(stitch.line_index + 1, child_text);
                }
            }
            text = lines.join("\n");
            Ok((text, refs))
        })
    }

    async fn commit(&self, result: CaptureResult) {
        let mut state = self.state.lock().await;
        state.baseline = Some(SnapshotObservation {
            text: result.text,
            url: Some(result.url),
        });
        state.refs = result.refs;
        state.ref_scope = result.scope;
    }

    async fn refs_for_capture(&self, state: &MainFrameState) -> RefMap {
        let current = self.state.lock().await;
        if should_reset_refs(current.ref_scope.as_ref(), state) {
            RefMap::new()
        } else {
            current.refs.fork_for_snapshot()
        }
    }

    async fn read_main_frame_state(&self, session: &ProtocolSession) -> MainFrameState {
        let result = session
            .send::<_, GetFrameTreeResult>("Page.getFrameTree", json!({}))
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

    async fn stable_document_id_for_frame(
        &self,
        root_session: &ProtocolSession,
        frame_id: Option<FrameId>,
        frame_documents: &HashMap<Option<FrameId>, DocumentId>,
    ) -> Option<DocumentId> {
        let before = frame_documents.get(&frame_id).cloned();
        if frame_id.is_none() || before.is_none() {
            return before;
        }
        let latest = self.read_frame_documents(root_session).await.ok();
        let after = latest.and_then(|latest| latest.get(&frame_id).cloned());
        if after == before { before } else { None }
    }

    async fn read_frame_documents(
        &self,
        session: &ProtocolSession,
    ) -> Result<HashMap<Option<FrameId>, DocumentId>, CoreError> {
        let result: GetFrameTreeResult = session.send("Page.getFrameTree", json!({})).await?;
        Ok(collect_frame_documents(&result.frame_tree))
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

#[derive(Debug, Deserialize)]
struct RuntimeEvalResult {
    result: RemoteValue,
}

#[derive(Debug, Deserialize)]
struct RemoteValue {
    value: Option<Value>,
    #[serde(rename = "objectId")]
    object_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CursorHit {
    marker: String,
    reasons: Vec<String>,
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

async fn find_cursor_hits(
    session: &ProtocolSession,
) -> Result<HashMap<i64, Vec<String>>, CoreError> {
    let mut hits = HashMap::new();
    let result: RuntimeEvalResult = session
        .send(
            "Runtime.evaluate",
            json!({ "expression": CURSOR_SCAN_JS, "returnByValue": true }),
        )
        .await?;
    let found = result
        .result
        .value
        .and_then(|value| serde_json::from_value::<Vec<CursorHit>>(value).ok())
        .unwrap_or_default();
    if found.is_empty() {
        return Ok(hits);
    }

    for hit in found {
        let query = format!("document.querySelector('[data-__bcid=\"{}\"]')", hit.marker);
        let result = session
            .send::<_, RuntimeEvalResult>(
                "Runtime.evaluate",
                json!({ "expression": query, "returnByValue": false }),
            )
            .await;
        let Ok(result) = result else {
            continue;
        };
        let Some(object_id) = result.result.object_id else {
            continue;
        };
        let described = session
            .send::<_, DescribeNodeResult>("DOM.describeNode", json!({ "objectId": object_id }))
            .await;
        if let Ok(described) = described
            && let Some(backend_node_id) = described.node.backend_node_id
        {
            hits.insert(backend_node_id, hit.reasons);
        }
    }
    let _ = session
        .send::<_, Value>(
            "Runtime.evaluate",
            json!({
                "expression": "document.querySelectorAll('[data-__bcid]').forEach(function(e){e.removeAttribute('data-__bcid')})",
                "returnByValue": true
            }),
        )
        .await;
    Ok(hits)
}

async fn resolve_child_frame_id(
    session: &ProtocolSession,
    backend_node_id: i64,
) -> Option<FrameId> {
    let described = session
        .send::<_, DescribeNodeResult>(
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
    use super::{RemoteValue, resolve_ref_entry};
    use crate::{
        BrowserSession, BrowserSessionHooks, CoreError, ProtocolSession,
        connection::CdpConnection,
        snapshot::{AxNode, AxValue, SnapshotMode, SnapshotOptions, refs::MintRef},
    };
    use browseros_cdp::{CdpError, CdpEvent};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use std::{
        collections::HashSet,
        sync::{Arc, Mutex},
    };
    use tokio::sync::broadcast;

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

    #[test]
    fn runtime_remote_value_deserializes_object_id() -> Result<(), serde_json::Error> {
        let value: RemoteValue = serde_json::from_value(json!({
            "type": "object",
            "objectId": "-6404171882913021072.1.1"
        }))?;

        assert_eq!(value.object_id.as_deref(), Some("-6404171882913021072.1.1"));
        Ok(())
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
        frame_tree_reads: usize,
        change_child_loader_on_second_read: bool,
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
                        Ok(json!({ "frameTree": frame_tree_value(&state) }))
                    }
                    "Accessibility.getFullAXTree" => {
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
                    "DOM.describeNode" => {
                        Ok(json!({ "node": { "contentDocument": { "frameId": "child" } } }))
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

    fn harness_state(nodes: Vec<AxNode>) -> HarnessState {
        HarnessState {
            loader_id: "loader-1".to_string(),
            url: "https://example.com/".to_string(),
            nodes,
            child_loader_id: None,
            child_nodes: Vec::new(),
            fail_ax_tree: false,
            frame_tree_reads: 0,
            change_child_loader_on_second_read: false,
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
