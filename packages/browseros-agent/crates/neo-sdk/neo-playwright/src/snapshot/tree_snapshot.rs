//! The snapshot ops' reads over the tab's APC tree (apc-engine-design 13.3 row 13, 8.2 and 9.3).
//!
//! One capture holds every frame of the tab, so a snapshot no longer reads each frame through its
//! own probe and stitches the replies: [`TabTree::aria`](crate::tree::TabTree::aria) emits the
//! whole page, child frames in place and frames the browser could not read as partial frames,
//! and prints the refs that every `aria-ref` resolves. What the tree cannot say stays the page's,
//! asked after the capture in a world installed before it, which design 6.2 proves against it:
//!
//! ```text
//!  page      the main world ─ capture_seeded (a pre-existing document's private marks first)
//!              ─ TabTree::aria(Page) ─▶ json, partial frames
//!            marked (Neo's formats, mode ai):
//!              coverSurvey ─ layers, pinned ─ pinned_nodes_in_document ─ keys ─ role, name
//!                and ref from the tree ─ action::on_top_line ─▶ the `# on top:` lines; each
//!                whole-view layer's holders mark the top-most printed nodes inside them
//!              closed-root seeds (contract field 31), the main document's and each child
//!                frame's, in its own world ─ DOM.resolveNode ─ openClosedRoots: the page's
//!                CSS and text engines see into those roots from then on
//!  locator   the resolver pins the one element ─ capture_seeded ─ the pin's node, proven in
//!              the capture's document ─ TabTree::aria(Subtree), the document for <body> or
//!              <html>, or the top-most tree nodes inside a container the tree left out
//! ```
//!
//! Each attempt captures afresh (design 7.1): a tree answers the attempt that captured it.

use std::collections::{HashMap, HashSet};
use std::iter;
use std::sync::{Arc, Mutex, PoisonError};

use browseros_core::FrameId;
use neo_snapshot::{AriaChild, AriaNode, Call, Snapshot};
use neo_snapshot_apc::refs::{Ref, RefLookup, RefTarget};
use neo_snapshot_apc::tree::{
    AriaRoot, AriaTree, DocId, DocSeq, NameCache, NameVisibility, NodeId, NodeKey, SnapshotTree,
    TreeDoc, TreeNode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::aria_snapshot::{PartialFrame, SnapshotResult, SnapshotTop};
use crate::action::{self, OnTopNote};
use crate::backoff::Backoff;
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{
    self, PinnedLayer, ProbeFailure, ProbeReply, ProbeRequest, SnapshotMode, SurveyPhases, Want,
};
use crate::resolve::{self, Landed, Resolver};
use crate::target::{LocatorTarget, Strictness};
use crate::tree::{
    AttemptStart, CapturedTree, TabTree, TreeAsk, backend_to_dom_node_id, ensure_frame_world,
    frame_doc, open_closed_roots,
};
use crate::world::{IsolatedWorld, Pin, Place};
use crate::{ErrorCode, PlaywrightError};

/// What one tree read asks for.
#[derive(Clone, Copy, Debug)]
pub(super) struct TreeRead {
    pub(super) mode: SnapshotMode,
    pub(super) depth: Option<i64>,
    pub(super) boxes: bool,
    /// Neo's marks beside the tree's own: the cover survey and the closed-root seeds.
    pub(super) marked: bool,
}

impl TreeRead {
    fn ask(self, root: AriaRoot) -> TreeAsk {
        TreeAsk {
            ai: self.mode == SnapshotMode::Ai,
            root,
            depth: depth_of(self.depth),
            boxes: self.boxes,
        }
    }
}

/// The op's depth as the emitter takes it: upstream's falsy zero, and a negative depth, which
/// upstream's engine reads as no limit at the root, both cut nothing.
fn depth_of(depth: Option<i64>) -> Option<usize> {
    depth.and_then(|depth| usize::try_from(depth).ok())
}

/// The page's snapshot. The main frame is the op: while the capture holds no main document
/// yet, or its world went away under a navigation, it tries again on the outer backoff until
/// the op's timeout, as upstream polls for the body.
pub(super) async fn read_page(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    read: TreeRead,
) -> Result<SnapshotResult, PlaywrightError> {
    let backoff = Backoff::outer(scope.timeout());
    for attempt in 0.. {
        if scope.sleep(backoff.wait(attempt)).await.is_err() {
            return Err(scope.timed_out(ErrorCode::WaitTimeout));
        }
        let reading = Mutex::new(None);
        let Ok(once) = scope.within(page_once(scope, page, read, &reading)).await else {
            // The op's deadline cut the capture: the timeout's error ends at the phase that ran
            // out, with how long it had run.
            if let Some(started) = reading
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
            {
                scope.log(format!(
                    "  reading the main frame's tree: unfinished after {} ms",
                    Instant::now().duration_since(started).as_millis()
                ));
            }
            return Err(scope.timed_out(ErrorCode::WaitTimeout));
        };
        if let Some(result) = once? {
            return Ok(result);
        }
    }
    Err(scope.timed_out(ErrorCode::WaitTimeout))
}

/// One attempt of [`read_page`]: `None` to try again.
async fn page_once(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    read: TreeRead,
    reading: &Mutex<Option<Instant>>,
) -> Result<Option<SnapshotResult>, PlaywrightError> {
    // From here the op is reading the page's tree, which a timeout's call log names.
    let started = Instant::now();
    *reading.lock().unwrap_or_else(PoisonError::into_inner) = Some(started);
    // The main world first: what the survey and the seeds pin in it is proven against this
    // capture only by a world installed before the capture was requested.
    let world = match page.world(&page.main_endpoint(), Place::Main).await {
        Ok(world) => world,
        Err(ProbeFailure::Gone(_)) => return Ok(None),
        Err(failure) => return Err(resolve::failed(scope, failure)),
    };
    // Upstream polls the main frame for `body,frameset` before it reads it
    // (`ariaSnapshotJSONForFrame`): a document still parsing towards its body, behind a
    // parser-blocking script, is read again rather than answered as an empty page. The page is
    // asked, not the tree: the browser's tree leaves out a body that draws nothing, as an empty
    // or a `display: none` one.
    match world.document_has_body().await {
        Ok(true) => {}
        Ok(false) => return Ok(None),
        Err(error) => match ProbeFailure::from_core(error) {
            // The main world went away, as under a navigation. Dropped, the next attempt
            // installs one in the page's current document; kept, it would fail the same way
            // until the op times out.
            ProbeFailure::Gone(_) => {
                page.tree().invalidate_world(page.frames(), &world);
                return Ok(None);
            }
            failure => return Err(resolve::failed(scope, failure)),
        },
    }
    let captured = page
        .tree()
        .capture_seeded(scope, page, AttemptStart::now())
        .await?;
    *reading.lock().unwrap_or_else(PoisonError::into_inner) = None;
    let tree = captured.tree();
    let main_frame = page.main_endpoint().frame;
    let Some(doc) = frame_doc(tree, &main_frame, world.frame()) else {
        return Ok(None);
    };
    if tree.document(doc).and_then(TreeDoc::root_node).is_none() {
        return Ok(None);
    }
    let mut snapshot = emitted(scope, page, tree, read.ask(AriaRoot::Page))?;
    label_partial_frames(scope, page, tree, &mut snapshot).await;
    scope.log(format!(
        "  read the main frame's tree in {} ms",
        started.elapsed().as_millis()
    ));
    let mut on_top = Vec::new();
    if read.marked {
        match survey(scope, page, &world, &captured, doc, &mut snapshot.tree).await {
            Ok(tops) => on_top = tops,
            // The main world went away: the page is navigating, and this tree is the old one.
            Err(Surveyed::Gone) => {
                page.tree().invalidate_world(page.frames(), &world);
                return Ok(None);
            }
            Err(Surveyed::Failed(error)) => return Err(error),
        }
        open_closed_roots(scope, page, &world, tree, doc, Some(MAX_CLOSED_ROOT_SEEDS)).await;
        open_child_closed_roots(scope, page, tree).await;
    }
    Ok(Some(result_of(snapshot, on_top)))
}

/// `locator.ariaSnapshot`'s read: the locator's one element, through the resolver's frame hops
/// and ref jumps, then its subtree from a capture taken after it was pinned. A strict violation,
/// a stale ref and an error the page throws end it at once.
pub(super) async fn read_locator(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    target: &LocatorTarget,
    strict: Strictness,
    read: TreeRead,
) -> Result<SnapshotResult, PlaywrightError> {
    let resolver = Resolver::new(page, scope, target, strict);
    scope.log(format!("waiting for {}", target.display));
    let backoff = Backoff::outer(scope.timeout());
    for attempt in 0.. {
        if scope.sleep(backoff.wait(attempt)).await.is_err() {
            return Err(scope.timed_out(ErrorCode::NoMatch));
        }
        // A locator's element is strict in both modes, as upstream's locator.ariaSnapshot is.
        let request = |target, _frame_visible| ProbeRequest::Resolve {
            target,
            strictness: Strictness::Strict,
            want: Want::Element,
        };
        let Ok(attempt) = scope.within(resolver.attempt_at(request)).await else {
            return Err(scope.timed_out(ErrorCode::NoMatch));
        };
        let Some(Landed { reply, world, .. }) = attempt? else {
            continue;
        };
        let pin = match reply {
            ProbeReply::Element { pin, .. } => pin,
            ProbeReply::None { stale_ref: None } | ProbeReply::Detached => continue,
            other => return Err(resolver.refused(other)),
        };
        let Ok(once) = scope
            .within(element_once(scope, page, &world, pin, read))
            .await
        else {
            return Err(scope.timed_out(ErrorCode::NoMatch));
        };
        if let Some(result) = once? {
            return Ok(result);
        }
    }
    Err(scope.timed_out(ErrorCode::NoMatch))
}

/// One attempt at the pinned element's subtree: `None` when it left its document or its world
/// went away, so the locator resolves again.
async fn element_once(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    pin: Pin,
    read: TreeRead,
) -> Result<Option<SnapshotResult>, PlaywrightError> {
    let Some(found) = element_in_tree(scope, page, world, pin).await? else {
        return Ok(None);
    };
    let Some(roots) = found.roots(scope, page, world, pin).await? else {
        return Ok(None);
    };
    let tree = found.captured.tree();
    let mut snapshot = Snapshot::default();
    for root in roots {
        let part = emitted(scope, page, tree, read.ask(root))?;
        snapshot.tree.extend(part.tree);
        snapshot.partial.extend(part.partial);
    }
    label_partial_frames(scope, page, tree, &mut snapshot).await;
    if read.marked {
        open_closed_roots(
            scope,
            page,
            world,
            tree,
            found.doc,
            Some(MAX_CLOSED_ROOT_SEEDS),
        )
        .await;
    }
    Ok(Some(result_of(snapshot, Vec::new())))
}

/// An element a resolver pinned, found in a capture taken after it was pinned: its document in
/// the capture, and its tree node when the capture has one.
pub(crate) struct InTree {
    pub(crate) captured: Arc<CapturedTree>,
    pub(crate) doc: DocId,
    /// The element's key, whether or not the capture has a node for it.
    pub(crate) key: NodeKey,
    pub(crate) node: Option<NodeId>,
}

impl InTree {
    /// Where the element's aria tree starts: its node, or, for an element the tree has no node
    /// for, [`untracked_roots`]. `None` when the element went away.
    pub(crate) async fn roots(
        &self,
        scope: &OpScope<'_>,
        page: &OpPage<'_>,
        world: &IsolatedWorld,
        pin: Pin,
    ) -> Result<Option<Vec<AriaRoot>>, PlaywrightError> {
        match self.node {
            Some(node) => Ok(Some(vec![AriaRoot::Subtree(node)])),
            None => untracked_roots(scope, page, world, pin, self.captured.tree(), self.doc).await,
        }
    }
}

/// `pin`'s element in a fresh capture of the tab: `None` when it left its document, or its world
/// went away (which this invalidates), so the caller resolves again.
///
/// The world pinned the element, so it was installed before the capture was requested; its
/// answer after the capture proves it held the capture's document throughout (design 6.2), and
/// the pin's node is a node of that document.
pub(crate) async fn element_in_tree(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    pin: Pin,
) -> Result<Option<InTree>, PlaywrightError> {
    let captured = page
        .tree()
        .capture_seeded(scope, page, AttemptStart::now())
        .await?;
    let dom = match probe::pinned_nodes_in_document(scope, page, world, vec![pin]).await {
        Ok(Some(ids)) => match ids.first().map(|&id| backend_to_dom_node_id(id)) {
            Some(Ok(dom)) => dom,
            Some(Err(failure)) => return Err(resolve::failed(scope, failure)),
            None => return Ok(None),
        },
        Ok(None) => return Ok(None),
        Err(ProbeFailure::Gone(_)) => {
            page.tree().invalidate_world(page.frames(), world);
            return Ok(None);
        }
        Err(failure) => return Err(resolve::failed(scope, failure)),
    };
    let tree = captured.tree();
    let main_frame = page.main_endpoint().frame;
    let Some(doc) = frame_doc(tree, &main_frame, world.frame()) else {
        return Ok(None);
    };
    let Some(document) = tree.document(doc) else {
        return Ok(None);
    };
    let key = NodeKey::new(document.token().clone(), dom);
    let node = tree.node_by_key(&key).map(TreeNode::id);
    Ok(Some(InTree {
        captured,
        doc,
        key,
        node,
    }))
}

/// `pin`'s ref in the tab's ref table: the one a snapshot printed for the element, else one
/// minted for it under its document's seq, which `getByRef` resolves (`locator.ref()`). `None`
/// when the element left its document or its world went away.
pub(crate) async fn element_ref(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    pin: Pin,
) -> Result<Option<String>, PlaywrightError> {
    let Some(found) = element_in_tree(scope, page, world, pin).await? else {
        return Ok(None);
    };
    let Some(seq) = found
        .captured
        .tree()
        .document(found.doc)
        .and_then(TreeDoc::seq)
    else {
        return Ok(None);
    };
    Ok(Some(page.tree().ref_for(seq, &found.key).to_string()))
}

/// The aria snapshot a failed `expect` that found nothing reports, upstream's
/// `ariaSnapshotForExpectFailure(document.body, { mode: 'default' })` in the frame the matcher
/// started in (`frame`, else the main frame), from a fresh capture. `None` when the capture
/// holds no document of that frame, or it prints nothing.
pub(crate) async fn expect_body_snapshot(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    frame: Option<&FrameId>,
) -> Option<String> {
    let captured = page
        .tree()
        .capture_seeded(scope, page, AttemptStart::now())
        .await
        .ok()?;
    let tree = captured.tree();
    let main_frame = page.main_endpoint().frame;
    let doc = frame_doc(tree, &main_frame, frame.unwrap_or(&main_frame))?;
    let ask = TreeAsk {
        ai: false,
        root: AriaRoot::Document(doc),
        depth: None,
        boxes: false,
    };
    let snapshot = page.tree().aria(tree, ask).ok()?.into_snapshot();
    let text = neo_snapshot::render(&snapshot, Call::Playwright).into_text();
    Some(text).filter(|text| !text.is_empty())
}

/// `DOM.describeNode`'s subtree of an element, read for its node name and the backend ids of
/// everything under it, shadow roots and same-process frames included (`pierce`).
#[derive(Deserialize)]
struct Described {
    node: DescribedNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedNode {
    node_name: String,
    #[serde(default)]
    children: Vec<Value>,
    #[serde(default)]
    shadow_roots: Vec<Value>,
}

/// Where the subtree of an element the tree has no node for starts. `<body>` and `<html>` are
/// the document's, which the document root prints; any other such element is a plain container
/// the retention rule left out, whose top-most tree nodes print in its place, as a plain `div`
/// prints nothing of its own in the page's snapshot. `None` when the element went away.
async fn untracked_roots(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    pin: Pin,
    tree: &SnapshotTree,
    doc: DocId,
) -> Result<Option<Vec<AriaRoot>>, PlaywrightError> {
    let object = match probe::pinned_object(scope, page, world, pin).await {
        Ok(Some(object)) => object,
        Ok(None) | Err(ProbeFailure::Gone(_)) => return Ok(None),
        Err(failure) => return Err(resolve::failed(scope, failure)),
    };
    let crash = scope.begin_protocol(world.session());
    let described = world
        .session()
        .send::<_, Described>(
            "DOM.describeNode",
            json!({ "objectId": object, "depth": -1, "pierce": true }),
        )
        .await
        .inspect_err(|error| crash.record(error));
    let described = match described {
        Ok(described) => described.node,
        Err(error) => {
            return match ProbeFailure::from_core(error) {
                ProbeFailure::Gone(_) => Ok(None),
                failure => Err(resolve::failed(scope, failure)),
            };
        }
    };
    if matches!(described.node_name.as_str(), "BODY" | "HTML" | "#document") {
        return Ok(Some(vec![AriaRoot::Document(doc)]));
    }
    let inside = backend_ids_below(described.children, described.shadow_roots);
    Ok(Some(top_most(tree, doc, &inside)))
}

/// Every `backendNodeId` in a described subtree, walked with a stack of its own: a page's DOM
/// can be nested deeper than a call stack.
fn backend_ids_below(children: Vec<Value>, shadow_roots: Vec<Value>) -> HashSet<i64> {
    let mut ids = HashSet::new();
    let mut pending: Vec<Value> = children.into_iter().chain(shadow_roots).collect();
    while let Some(mut node) = pending.pop() {
        if let Some(id) = node.get("backendNodeId").and_then(Value::as_i64) {
            ids.insert(id);
        }
        for key in ["children", "shadowRoots", "pseudoElements"] {
            if let Some(Value::Array(more)) = node.get_mut(key).map(Value::take) {
                pending.extend(more);
            }
        }
    }
    ids
}

/// The tree nodes of `doc` whose DOM node is in `inside` and that have no such ancestor, in
/// document order: the roots the container's own subtree prints as.
fn top_most(tree: &SnapshotTree, doc: DocId, inside: &HashSet<i64>) -> Vec<AriaRoot> {
    let mut roots = Vec::new();
    let mut covered_until: Option<NodeId> = None;
    for node in tree.doc_nodes(doc) {
        if covered_until.is_some_and(|end| node.id() < end) {
            continue;
        }
        let Some(dom) = node.dom_node_id() else {
            continue;
        };
        if inside.contains(&dom.as_backend_node_id()) {
            roots.push(AriaRoot::Subtree(node.id()));
            covered_until = Some(node.subtree_end());
        }
    }
    roots
}

/// `tree`'s aria tree under `ask`, as a snapshot: the nodes and the frames not read. A tree
/// nested deeper than a snapshot carries is the snapshot's own refusal, which says how to retry.
fn emitted(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    tree: &SnapshotTree,
    ask: TreeAsk,
) -> Result<Snapshot, PlaywrightError> {
    page.tree()
        .aria(tree, ask)
        .map(AriaTree::into_snapshot)
        .map_err(|too_deep| scope.fail(ErrorCode::Unsupported, &too_deep.to_string()))
}

/// `DOM.describeNode`'s attributes of an element, as name and value in turn.
#[derive(Deserialize)]
struct DescribedAttributes {
    node: AttributesOf,
}

#[derive(Deserialize)]
struct AttributesOf {
    #[serde(default)]
    attributes: Vec<String>,
}

/// Names each frame the capture could not read by its iframe's `src`, as the `# partial` line
/// always named it, where the tree knows the frame only by its frame token (a frame that never
/// committed or timed out has no URL in the capture). Read only for an iframe of the main
/// document, whose element the root session describes by its node id; any other keeps its
/// token. Best effort: a label is never worth failing a snapshot over.
async fn label_partial_frames(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    tree: &SnapshotTree,
    snapshot: &mut Snapshot,
) {
    for partial in &mut snapshot.partial {
        if partial.frame.contains("://") {
            continue;
        }
        let Some(frame) = tree.partial_frames().iter().find(|frame| {
            frame
                .frame_token
                .as_ref()
                .is_some_and(|token| token.as_str() == partial.frame)
        }) else {
            continue;
        };
        let (Some(parent), Some(iframe)) = (tree.document(frame.parent_doc), &frame.iframe_key)
        else {
            continue;
        };
        if !parent.is_main() {
            continue;
        }
        let session = page.root_session();
        let crash = scope.begin_protocol(&session);
        let described = session
            .send::<_, DescribedAttributes>(
                "DOM.describeNode",
                json!({ "backendNodeId": iframe.node.as_backend_node_id() }),
            )
            .await
            .inspect_err(|error| crash.record(error));
        let Ok(described) = described else {
            continue;
        };
        let src = described
            .node
            .attributes
            .as_chunks::<2>()
            .0
            .iter()
            .find(|[name, _]| name == "src")
            .map(|[_, value]| value.clone());
        let Some(src) = src else {
            continue;
        };
        let resolved = parent
            .url()
            .and_then(|base| url::Url::parse(base).ok())
            .and_then(|base| base.join(&src).ok())
            .map_or(src, String::from);
        partial.frame = resolved;
    }
}

/// The op's output from an emitted snapshot.
fn result_of(snapshot: Snapshot, on_top: Vec<SnapshotTop>) -> SnapshotResult {
    SnapshotResult {
        json: snapshot.tree,
        partial: snapshot
            .partial
            .into_iter()
            .map(|frame| PartialFrame {
                r#ref: frame.r#ref,
                frame: frame.frame,
                reason: frame.reason,
            })
            .collect(),
        unmarked: None,
        on_top,
        document: None,
        snapshot_call: None,
        closed_roots: None,
    }
}

/// Why the survey has no answer.
enum Surveyed {
    /// The main world went away under it.
    Gone,
    Failed(PlaywrightError),
}

/// The `# on top:` lines of a marked page snapshot (design 8.2): the page's survey finds the
/// layers and pins what each answer names; the tree names them. Each whole-view layer with words
/// also marks the top-most printed nodes inside its holders, which the v1 text moves under it.
async fn survey(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    captured: &CapturedTree,
    doc: DocId,
    json: &mut [AriaNode],
) -> Result<Vec<SnapshotTop>, Surveyed> {
    let failed = |failure| Surveyed::Failed(resolve::failed(scope, failure));
    let reply = match probe::call(scope, page, world, &ProbeRequest::CoverSurvey).await {
        Ok(reply) => reply,
        Err(ProbeFailure::Gone(_)) => return Err(Surveyed::Gone),
        Err(failure) => return Err(failed(failure)),
    };
    // The survey is best effort, as it was inside the page's snapshot: an answer it cannot give
    // (the page threw, or answered what a survey does not) leaves the snapshot without its lines.
    let ProbeReply::Surveyed {
        layers,
        tested,
        survey_ms,
        phases,
    } = reply
    else {
        scope.log("  cover survey or content move skipped (page or engine shape)".to_owned());
        return Ok(Vec::new());
    };
    log_survey(scope, &phases, tested, survey_ms);
    if layers.is_empty() {
        return Ok(Vec::new());
    }
    let pins: Vec<Pin> = layers.iter().flat_map(layer_pins).collect();
    let tree = captured.tree();
    let named = match PinNames::read(scope, page, world, tree, doc, pins).await {
        Ok(Some(named)) => named,
        // A pin left the page since the survey: the layers are not worded this time.
        Ok(None) => {
            scope.log("  cover survey or content move skipped (page or engine shape)".to_owned());
            return Ok(Vec::new());
        }
        Err(ProbeFailure::Gone(_)) => return Err(Surveyed::Gone),
        Err(failure) => return Err(failed(failure)),
    };
    let mut tops = Vec::with_capacity(layers.len());
    let mut held: Vec<(String, Vec<NodeKey>)> = Vec::new();
    for layer in &layers {
        let Some(own) = named.named(layer.pin) else {
            continue;
        };
        let controls: Vec<Named> = layer
            .controls
            .iter()
            .filter_map(|&pin| named.named(pin))
            .collect();
        let control_facts: Vec<(&str, &Value, &str)> = controls
            .iter()
            .map(|control| (control.role, &control.name, control.r#ref.as_str()))
            .collect();
        let note = OnTopNote {
            role: own.role,
            name: &own.name,
            reference: &own.r#ref,
            blank: layer.blank,
            loading: layer.loading,
            whole_view: layer.whole_view,
            controls: &control_facts,
            first_words: layer
                .first_words
                .as_ref()
                .and_then(|words| words.as_value().as_str()),
        };
        tops.push(SnapshotTop {
            r#ref: own.r#ref.clone(),
            line: action::on_top_line(&note),
            whole_view: layer.whole_view,
        });
        if layer.whole_view && !layer.blank && !layer.holders.is_empty() {
            let holders = layer
                .holders
                .iter()
                .filter_map(|&pin| named.key(pin).cloned())
                .collect();
            held.push((own.r#ref, holders));
        }
    }
    if !held.is_empty() {
        mark_on_top(page, tree, json, &held);
        carry_holder_text(tree, json, &held);
    }
    Ok(tops)
}

/// Every element a layer's answer names, pinned: the layer, its controls and its holders.
fn layer_pins(layer: &PinnedLayer) -> impl Iterator<Item = Pin> + '_ {
    iter::once(layer.pin)
        .chain(layer.controls.iter().copied())
        .chain(layer.holders.iter().copied())
}

/// The survey's lines in the call log, as the page's own survey wrote them.
fn log_survey(scope: &OpScope<'_>, phases: &SurveyPhases, tested: u32, survey_ms: f64) {
    scope.log(format!(
        "  cover survey phases: hits {:.3} ms, count {:.3} ms, naming {:.3} ms, linking {:.3} ms, marks {:.3} ms",
        phases.hits, phases.count, phases.naming, phases.linking, phases.marks
    ));
    if phases.stopped {
        scope.log(format!(
            "  cover survey stopped at its budget after {tested} controls"
        ));
    }
    scope.log(format!(
        "  cover survey: {tested} controls tested in {survey_ms:.3} ms"
    ));
}

/// An element a page answer names, as the tree names it: its role, its accessible name and the
/// ref a snapshot printed for it, else one minted for it under the document's seq, which
/// `getByRef` resolves.
pub(crate) struct Named {
    /// As a snapshot line prints it ([`printed_role`]).
    pub(crate) role: &'static str,
    /// The name as the answer's lines quote it, a JSON string.
    pub(crate) name: Value,
    pub(crate) r#ref: String,
}

/// The elements one page answer pinned, keyed in a capture taken after the answer and named
/// from it (design 8.2): the page finds what covers, the tree says what it is. The `# on top:`
/// survey and a covered action's answer both name through it.
///
/// The world that pinned them was installed before the capture was requested, so its answer to
/// the keying after the capture proves it held the capture's document throughout (design 6.2),
/// and each pin's node is a node of that document. Pins read before the capture and named from
/// an older one could name another element under a reused node id.
pub(crate) struct PinNames<'a> {
    tab: &'a TabTree,
    tree: &'a SnapshotTree,
    names: NameCache<'a>,
    seq: DocSeq,
    keys: HashMap<Pin, NodeKey>,
}

impl<'a> PinNames<'a> {
    /// Keys `pins`, which `world` answered, in `doc` of `tree`. `Ok(None)` when one left the
    /// world's document since the answer, or the capture gave the document no seq to mint refs
    /// under: the answer is not named this time.
    pub(crate) async fn read(
        scope: &OpScope<'_>,
        page: &'a OpPage<'_>,
        world: &IsolatedWorld,
        tree: &'a SnapshotTree,
        doc: DocId,
        mut pins: Vec<Pin>,
    ) -> Result<Option<Self>, ProbeFailure> {
        // Each pin once: the batch's deep serialization sends a node it has already sent as a
        // reference with no value, and a layer named by its own holder repeats that pin.
        let mut seen = HashSet::with_capacity(pins.len());
        pins.retain(|&pin| seen.insert(pin));
        let Some(ids) = probe::pinned_nodes_in_document(scope, page, world, pins.clone()).await?
        else {
            return Ok(None);
        };
        let Some(document) = tree.document(doc) else {
            return Ok(None);
        };
        let Some(seq) = document.seq() else {
            return Ok(None);
        };
        let mut keys = HashMap::with_capacity(pins.len());
        for (pin, id) in pins.into_iter().zip(ids) {
            let dom = backend_to_dom_node_id(id)?;
            keys.insert(pin, NodeKey::new(document.token().clone(), dom));
        }
        Ok(Some(Self {
            tab: page.tree(),
            tree,
            names: NameCache::new(tree),
            seq,
            keys,
        }))
    }

    /// The capture the pins were keyed in.
    pub(crate) fn tree(&self) -> &'a SnapshotTree {
        self.tree
    }

    /// The pinned element's key, whether or not the capture has a node for it. `None` for a pin
    /// the answer did not carry.
    pub(crate) fn key(&self, pin: Pin) -> Option<&NodeKey> {
        self.keys.get(&pin)
    }

    /// The pinned element's tree node; `None` for an element the retention rule left out.
    pub(crate) fn node(&self, pin: Pin) -> Option<NodeId> {
        self.tree.node_by_key(self.key(pin)?).map(TreeNode::id)
    }

    /// The pinned element as the tree names it. Its ref is minted in the tab's table when no
    /// snapshot printed one, so every ref an answer prints resolves.
    pub(crate) fn named(&self, pin: Pin) -> Option<Named> {
        let key = self.key(pin)?;
        let node = self.tree.node_by_key(key);
        let role = printed_role(node);
        let name = node.map_or_else(String::new, |node| {
            self.names
                .name(node.id(), NameVisibility::VisibleOnly)
                .text()
                .to_owned()
        });
        Some(Named {
            role,
            name: Value::String(name),
            r#ref: self.tab.ref_for(self.seq, key).to_string(),
        })
    }
}

/// A named element's role as its line prints it, the words the page's answers have always used:
/// `iframe` for a frame, the tree's role, and `generic` for an element with no role of its own
/// (a plain container, one the retention rule left out of the tree, or a presentational one),
/// as a snapshot prints a plain `div` it keeps.
fn printed_role(node: Option<&TreeNode>) -> &'static str {
    match node {
        Some(node) if node.is_iframe_boundary() => "iframe",
        Some(node) => node
            .role()
            .filter(|role| !role.is_presentational())
            .map_or("generic", |role| role.as_str()),
        None => "generic",
    }
}

/// Marks each printed node whose element is inside a whole-view layer's holder with that
/// layer's ref, top-most only: a marked node's children are not looked at, as the page's
/// survey marked its JSON. A printed node is known by its ref, so a node that printed none is
/// passed through to its children.
fn mark_on_top(
    page: &OpPage<'_>,
    tree: &SnapshotTree,
    json: &mut [AriaNode],
    held: &[(String, Vec<NodeKey>)],
) {
    let mut pending: Vec<&mut AriaNode> = json.iter_mut().collect();
    while let Some(node) = pending.pop() {
        let key = node
            .r#ref
            .as_deref()
            .and_then(Ref::parse)
            .and_then(|printed| match page.tree().resolve_ref(&printed) {
                RefLookup::Key(RefTarget { key, .. }) => Some(key),
                RefLookup::Retired | RefLookup::Unknown => None,
            });
        let layer = key.and_then(|key| {
            held.iter()
                .find(|(_, holders)| holders.iter().any(|holder| tree.contains(holder, &key)))
                .map(|(layer, _)| layer.clone())
        });
        if let Some(layer) = layer {
            node.on_top = Some(layer);
            continue;
        }
        pending.extend(node.children.iter_mut().filter_map(|child| match child {
            AriaChild::Node(node) => Some(node.as_mut()),
            AriaChild::Text(_) => None,
        }));
    }
}

/// Moves each held layer's ref-less holder text with the layer. A holder the tree emits no node
/// for (a plain or `display: contents` container, apc-engine-design 3.5) prints its own text in
/// its place, beside the nodes [`mark_on_top`] marked, where upstream's holder `generic` carried
/// it along. The survey names such a holder itself when the layer's faces share it, or names the
/// faces, a backdrop and its dialog, when they share none, and then their parent is that holder.
/// Where its text and the marked nodes sit side by side in one children list, they are wrapped
/// in one `generic` that carries the mark in their place, so the text prints in the layer's
/// block. Text the tree masks is never matched (`SnapshotTree::node_text`), so such a holder is
/// left as it is.
fn carry_holder_text(tree: &SnapshotTree, json: &mut [AriaNode], held: &[(String, Vec<NodeKey>)]) {
    for (layer, holders) in held {
        let Some(texts) = holder_texts(tree, holders) else {
            continue;
        };
        let mut pending: Vec<&mut AriaNode> = json.iter_mut().collect();
        while let Some(node) = pending.pop() {
            if wrap_with_layer(&mut node.children, layer, &texts) {
                break;
            }
            pending.extend(node.children.iter_mut().filter_map(|child| match child {
                AriaChild::Node(node) => Some(node.as_mut()),
                AriaChild::Text(_) => None,
            }));
        }
    }
}

/// The text that prints beside a layer's marked nodes, as the emitted tree writes strings: the
/// direct text of each holder that emits no node of its own, and that of the parent the holders
/// share when that parent holds nothing else that prints. `None` when there is none.
fn holder_texts(tree: &SnapshotTree, holders: &[NodeKey]) -> Option<Vec<String>> {
    let nodes: Vec<&TreeNode> = holders
        .iter()
        .filter_map(|holder| tree.node_by_key(holder))
        .collect();
    let mut texts: Vec<String> = nodes
        .iter()
        .filter(|node| emits_no_node(tree, node))
        .flat_map(|node| tree.children(node.id()))
        .filter_map(|child| tree.node_text(child.id()))
        .map(printed_text)
        .filter(|text| !text.is_empty())
        .collect();
    texts.extend(shared_parent_texts(tree, &nodes).unwrap_or_default());
    (!texts.is_empty()).then_some(texts)
}

/// The direct text of the parent `nodes` share, when that parent emits no node and holds nothing
/// else but text and empty boxes that print nothing, as a backdrop beside its dialog.
fn shared_parent_texts(tree: &SnapshotTree, nodes: &[&TreeNode]) -> Option<Vec<String>> {
    let parent = tree.node(nodes.first()?.parent()?)?;
    if !emits_no_node(tree, parent) || nodes.iter().any(|node| node.parent() != Some(parent.id())) {
        return None;
    }
    let mut texts = Vec::new();
    for child in tree.children(parent.id()) {
        if nodes.iter().any(|node| node.id() == child.id()) {
            continue;
        }
        if let Some(text) = tree.node_text(child.id()) {
            let text = printed_text(text);
            if !text.is_empty() {
                texts.push(text);
            }
        } else if !(emits_no_node(tree, child) && tree.children(child.id()).next().is_none()) {
            return None;
        }
    }
    Some(texts)
}

/// Whether the ai tree prints no line of `node`'s own: a role-less plain container (design 3.5),
/// whose children print in its place.
fn emits_no_node(tree: &SnapshotTree, node: &TreeNode) -> bool {
    node.role().is_none() && tree.fold().is_plain_prunable_container(node.id())
}

/// A text node's text as the emitter prints it, whitespace runs folded to one space.
fn printed_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Wraps, in `children`, the one run of the nodes marked `layer` and the strings in `texts`
/// that holds both, side by side and nothing else, into one `generic` marked `layer`. Whether it
/// did.
fn wrap_with_layer(children: &mut Vec<AriaChild>, layer: &str, texts: &[String]) -> bool {
    let belongs = |child: &AriaChild| match child {
        AriaChild::Node(node) => node.on_top.as_deref() == Some(layer),
        AriaChild::Text(text) => texts.iter().any(|own| own == text),
    };
    let Some(first) = children.iter().position(belongs) else {
        return false;
    };
    let end = children
        .iter()
        .skip(first)
        .position(|child| !belongs(child))
        .map_or(children.len(), |run| first + run);
    let run = children.get(first..end).unwrap_or_default();
    let marked = run.iter().any(|child| matches!(child, AriaChild::Node(_)));
    let text = run.iter().any(|child| matches!(child, AriaChild::Text(_)));
    if !marked || !text {
        return false;
    }
    let mut wrapped: Vec<AriaChild> = children.drain(first..end).collect();
    for child in &mut wrapped {
        if let AriaChild::Node(node) = child {
            node.on_top = None;
        }
    }
    let holder = AriaNode {
        role: "generic".to_owned(),
        name: None,
        checked: None,
        disabled: None,
        expanded: None,
        active: None,
        inert: None,
        on_top: Some(layer.to_owned()),
        invalid: None,
        level: None,
        pressed: None,
        selected: None,
        aria_hidden: None,
        r#ref: None,
        cursor: None,
        r#box: None,
        url: None,
        placeholder: None,
        text: None,
        children: wrapped,
    };
    children.insert(first, AriaChild::Node(Box::new(holder)));
    true
}

/// [`open_closed_roots`](crate::tree::open_closed_roots) for each child document of the page whose
/// capture marked a seed, in its
/// frame's world, installed now if this instance has none there yet: the page's snapshot opened
/// every frame's closed roots it read, and a locator into a frame (`frameLocator(..).getByLabel`)
/// finds what is inside them only once its world holds them. A seed of a document the frame has
/// since replaced resolves to nothing there, or to a node of no closed root, and opens nothing.
async fn open_child_closed_roots(scope: &OpScope<'_>, page: &OpPage<'_>, tree: &SnapshotTree) {
    let main_frame = page.main_endpoint().frame;
    for document in tree.documents() {
        if document.is_main() {
            continue;
        }
        let seeded = tree
            .doc_nodes(document.id())
            .any(TreeNode::is_closed_root_seed);
        let Some(frame) = document
            .frame_token()
            .map(|token| FrameId(token.as_str().to_owned()))
        else {
            continue;
        };
        if !seeded || frame == main_frame {
            continue;
        }
        if let Some(world) = ensure_frame_world(scope, page, &frame).await {
            open_closed_roots(
                scope,
                page,
                &world,
                tree,
                document.id(),
                Some(MAX_CLOSED_ROOT_SEEDS),
            )
            .await;
        }
    }
}

/// The most closed-root seeds one document hands its page engines per snapshot, as the clickable
/// scan sent: each is a `DOM.resolveNode`, so a page built from thousands of closed components does
/// not make every snapshot wait on them. The tree reads and prints every root either way; only the
/// page's CSS and text engines stop at the cut.
const MAX_CLOSED_ROOT_SEEDS: usize = 1_000;

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use std::error::Error;

    use neo_snapshot::{AriaChild, AriaNode};
    use neo_snapshot_apc::proto::FormControlType;
    use neo_snapshot_apc::tree::{
        ApcBuilder, BuildOptions, DocToken, DomNodeId, NodeBuilder, NodeKey, SnapshotTree,
    };

    use super::{backend_ids_below, depth_of, holder_texts, wrap_with_layer};

    /// e15's page: a `main` holding a button, then a `display: contents` holder (20) whose own
    /// text precedes a whole-view backdrop (22) and a dialog (23) holding a button.
    fn layer_page() -> Result<SnapshotTree, Box<dyn Error>> {
        let button = |dom: i32, name: &str| {
            NodeBuilder::form_control(FormControlType::ButtonButton)
                .dom_id(dom)
                .tag("button")
                .bounding_box(0, 0, 80, 24)
                .child(NodeBuilder::text(name))
        };
        let body = NodeBuilder::element("body")
            .dom_id(3)
            .bounding_box(0, 0, 1280, 800)
            .child(
                NodeBuilder::element("main")
                    .dom_id(4)
                    .bounding_box(0, 0, 1280, 100)
                    .child(button(5, "Outside box")),
            )
            .child(
                NodeBuilder::container()
                    .dom_id(20)
                    .display_contents(true)
                    .child(NodeBuilder::text("Please read  this whole message"))
                    .child(
                        NodeBuilder::container()
                            .dom_id(22)
                            .bounding_box(0, 0, 1280, 800),
                    )
                    .child(
                        NodeBuilder::element("section")
                            .dom_id(23)
                            .role("dialog")
                            .aria_label("Ref-less block")
                            .bounding_box(400, 200, 400, 300)
                            .child(button(24, "Dismiss")),
                    ),
            );
        let apc = ApcBuilder::new("doc-layer")
            .root(NodeBuilder::root().child(NodeBuilder::element("html").child(body)))
            .build();
        Ok(SnapshotTree::build(apc, BuildOptions::new())?)
    }

    fn keys(doms: &[i32]) -> Result<Vec<NodeKey>, Box<dyn Error>> {
        let token = DocToken::new("doc-layer").ok_or("a document token")?;
        Ok(doms
            .iter()
            .map(|&dom| NodeKey::new(token.clone(), DomNodeId::new(dom)))
            .collect())
    }

    #[test]
    fn tests_that_a_layers_holder_text_is_found_whichever_holders_the_survey_names()
    -> Result<(), Box<dyn Error>> {
        let tree = layer_page()?;
        let text = Some(vec!["Please read this whole message".to_owned()]);
        assert_eq!(
            holder_texts(&tree, &keys(&[20])?),
            text,
            "the shared holder itself, its own text"
        );
        assert_eq!(
            holder_texts(&tree, &keys(&[22, 23])?),
            text,
            "the backdrop and the dialog, their parent's text"
        );
        assert_eq!(
            holder_texts(&tree, &keys(&[23])?),
            text,
            "the dialog alone, beside an empty backdrop"
        );
        assert_eq!(
            holder_texts(&tree, &keys(&[4])?),
            None,
            "a holder that prints, beside other content"
        );
        Ok(())
    }

    fn node(role: &str, on_top: Option<&str>) -> Result<AriaChild, serde_json::Error> {
        let mut node: AriaNode = serde_json::from_value(json!({ "role": role }))?;
        node.on_top = on_top.map(str::to_owned);
        Ok(AriaChild::Node(Box::new(node)))
    }

    #[test]
    fn tests_that_a_layers_holder_text_is_wrapped_with_its_marked_nodes()
    -> Result<(), Box<dyn Error>> {
        let text = || AriaChild::Text("Please read this whole message".to_owned());
        let mut children = vec![node("main", None)?, text(), node("dialog", Some("e5"))?];
        let texts = vec!["Please read this whole message".to_owned()];
        assert!(wrap_with_layer(&mut children, "e5", &texts));
        assert_eq!(children.len(), 2, "main, then the layer's one block");
        let Some(AriaChild::Node(block)) = children.get(1) else {
            return Err("the layer's block".into());
        };
        assert_eq!(block.role, "generic");
        assert_eq!(block.on_top.as_deref(), Some("e5"));
        assert_eq!(block.children.first(), Some(&text()));
        assert!(
            block.children.iter().all(|child| match child {
                AriaChild::Node(inner) => inner.on_top.is_none(),
                AriaChild::Text(_) => true,
            }),
            "the block carries the mark, not its nodes"
        );
        Ok(())
    }

    #[test]
    fn tests_that_text_apart_from_the_layer_or_a_layer_alone_is_left_in_place()
    -> Result<(), Box<dyn Error>> {
        let text = || AriaChild::Text("Elsewhere".to_owned());
        let texts = vec!["Elsewhere".to_owned()];
        let mut apart = vec![text(), node("main", None)?, node("dialog", Some("e5"))?];
        assert!(
            !wrap_with_layer(&mut apart, "e5", &texts),
            "not side by side"
        );
        assert_eq!(apart.len(), 3);
        let mut alone = vec![node("main", None)?, node("dialog", Some("e5"))?];
        assert!(
            !wrap_with_layer(&mut alone, "e5", &texts),
            "no text to carry"
        );
        assert_eq!(alone.len(), 2);
        Ok(())
    }

    #[test]
    fn tests_that_the_depth_option_keeps_upstreams_falsy_zero_and_cuts_nothing_below_it() {
        assert_eq!(depth_of(None), None);
        assert_eq!(
            depth_of(Some(0)),
            Some(0),
            "the emitter reads zero as no cut"
        );
        assert_eq!(depth_of(Some(3)), Some(3));
        assert_eq!(depth_of(Some(-1)), None);
    }

    #[test]
    fn tests_that_a_described_subtree_yields_every_backend_id_under_it_shadow_roots_included() {
        let children = vec![json!({
            "backendNodeId": 11,
            "children": [{ "backendNodeId": 12, "children": [] }],
            "shadowRoots": [{ "backendNodeId": 13, "children": [{ "backendNodeId": 14 }] }],
        })];
        let ids = backend_ids_below(children, Vec::new());
        assert_eq!(ids, HashSet::from([11, 12, 13, 14]));
    }
}
