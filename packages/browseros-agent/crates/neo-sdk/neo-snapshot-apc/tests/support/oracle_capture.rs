//! Reads a loaded page's oracle: Playwright's ARIA engine (neo-playwright's injected.js,
//! unchanged) run over every frame of the page, in a world of its own per frame.
//!
//! ```text
//!  frames:  main session's Page.getFrameTree  (same-process frames)
//!           + fixpoint over Target.getTargets "iframe" targets whose parentFrameId is known:
//!             attach, then that session's Page.getFrameTree (its same-process subtree)
//!  per frame, on the session that owns it:
//!           createIsolatedWorld "neo-oracle" ─▶ closed-root seeds (DOMSnapshot) ─▶ resolveNode
//!           ─▶ evaluate injected.js + oracle.js ─▶ __neoOracleRead(seq, ...seeds) (JSON text)
//!           ─▶ __neoOracleNodes ─▶ describeNode each ─▶ backendNodeIds
//!           and DOM.getFrameOwner on the parent's session ─▶ owner
//! ```
//!
//! The recorder reads each fixture with it, and the live zero-conflict test each fixture and site
//! (whose frame sessions it also finds with [`frame_sites`]); the offline tests read the
//! [`Oracle`] the recorder produced.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::future::Future;
use std::time::Duration;

use browseros_cdp::{CdpClient, SessionId};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::time;

use crate::oracle::{self, Oracle, OracleAria, OracleElement, OracleFrame, OracleStates};

/// neo-playwright's probe bundle: Playwright's injected engine plus Neo's probe, as Neo ships it.
const INJECTED_JS: &str = include_str!("../../../neo-playwright/assets/injected.js");

/// The reader spliced into [`INJECTED_JS`] (see that file's header).
const ORACLE_JS: &str = include_str!("oracle.js");

/// The bundle's last top-level statement, inside its IIFE. Source inserted right after it, still
/// inside the IIFE, sees every top-level binding of the bundle by name, all of them defined.
const BUNDLE_ANCHOR: &str = "globalThis.__neoProbe ??= createProbe();";

/// The world every oracle read runs in. Isolated, so the engine's prototype patches stay out of
/// the page's own scripts and out of Neo's probe world.
const ORACLE_WORLD: &str = "neo-oracle";

/// Bound on each CDP step of one frame's read. A frame that does not answer fails the fixture
/// with its name rather than leaving a partial oracle.
const FRAME_STEP_TIMEOUT: Duration = Duration::from_secs(20);

/// The engine as the oracle runs it: the bundle with the reader spliced in, and the bundle's
/// banner line that names Playwright's version and commit.
#[derive(Debug)]
pub(crate) struct OracleEngine {
    source: String,
    banner: String,
}

impl OracleEngine {
    /// Splices oracle.js into injected.js after [`BUNDLE_ANCHOR`]. Fails when the anchor is not
    /// there exactly once: inserted anywhere else, the reader would miss bindings or run before
    /// the bundle defined them.
    pub(crate) fn new() -> Result<Self, Box<dyn Error>> {
        let anchors = INJECTED_JS.matches(BUNDLE_ANCHOR).count();
        let Some((head, tail)) = INJECTED_JS.split_once(BUNDLE_ANCHOR) else {
            return Err(
                format!("injected.js has no `{BUNDLE_ANCHOR}` to insert the oracle after").into(),
            );
        };
        if anchors != 1 {
            return Err(format!(
                "injected.js has `{BUNDLE_ANCHOR}` {anchors} times; the oracle goes after the one \
                 at the end of the bundle's IIFE"
            )
            .into());
        }
        let banner = INJECTED_JS
            .lines()
            .find(|line| line.contains("Playwright v"))
            .ok_or("injected.js has no banner line naming Playwright's version")?
            .trim_start_matches([' ', '*'])
            .to_owned();
        Ok(Self {
            source: format!("{head}{BUNDLE_ANCHOR}\n{ORACLE_JS}{tail}"),
            banner,
        })
    }

    /// injected.js's banner line naming Playwright's version and commit.
    pub(crate) fn banner(&self) -> &str {
        &self.banner
    }
}

/// Reads the oracle of the page in the tab `target_id`, attached as `session`, on the load it is
/// on; `page` names it in the oracle and in errors (a fixture's registry name, a site's id).
/// Child-frame sessions attached here are detached again, also on failure.
pub(crate) async fn read_oracle(
    client: &CdpClient,
    page: &str,
    session: &SessionId,
    target_id: &str,
    engine: &OracleEngine,
    browser: &str,
) -> Result<Oracle, Box<dyn Error>> {
    let mut attached = Vec::new();
    let read = async {
        let sites = frame_sites(client, session, target_id, &mut attached)
            .await
            .map_err(|error| format!("{page}: frame discovery: {error}"))?;
        let mut snapshots = HashMap::new();
        let mut frames = Vec::with_capacity(sites.len());
        for (seq, site) in sites.iter().enumerate() {
            let parent_session = site
                .parent
                .as_ref()
                .and_then(|parent| sites.iter().find(|other| &other.id == parent))
                .map(|parent| &parent.session);
            let frame = read_frame(client, site, seq, parent_session, engine, &mut snapshots)
                .await
                .map_err(|error| {
                    format!(
                        "{page}: oracle of frame {} ({}) failed: {error}",
                        site.url, site.id
                    )
                })?;
            frames.push(frame);
        }
        Ok::<_, Box<dyn Error>>(frames)
    }
    .await;
    detach_all(client, attached).await;
    Ok(Oracle {
        fixture: page.to_owned(),
        engine: engine.banner().to_owned(),
        browser: browser.to_owned(),
        frames: read?,
    })
}

/// `Browser.getVersion`'s product, recorded with each oracle.
pub(crate) async fn browser_product(client: &CdpClient) -> Result<String, Box<dyn Error>> {
    #[derive(Deserialize)]
    struct Version {
        product: String,
    }
    let version: Version =
        serde_json::from_value(client.send("Browser.getVersion", json!({}), None).await?)?;
    Ok(version.product)
}

/// One frame of the page and the CDP session that owns it: the only session on which its
/// isolated world can be made, its document read and its nodes resolved.
#[derive(Debug)]
pub(crate) struct FrameSite {
    /// The CDP frame id (the devtools frame token APC reports for the frame's document).
    pub(crate) id: String,
    /// The parent frame's id; `None` for the main frame.
    pub(crate) parent: Option<String>,
    /// As recorded ([`oracle::recorded_url`]).
    pub(crate) url: String,
    /// The tab's session for a frame in the tab's renderer, else the session attached to the
    /// out-of-process frame's `iframe` target that owns it.
    pub(crate) session: SessionId,
}

/// `Page.getFrameTree`'s reply.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameTreeReply {
    frame_tree: FrameTreeNode,
}

/// One frame of a session's same-process frame tree.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameTreeNode {
    frame: FrameInfo,
    #[serde(default)]
    child_frames: Vec<FrameTreeNode>,
}

/// The part of `Page.Frame` the oracle records.
#[derive(Deserialize)]
struct FrameInfo {
    id: String,
    url: String,
}

/// `Target.getTargets`'s reply.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetsReply {
    target_infos: Vec<TargetInfo>,
}

/// One target; a cross-site child frame is an `iframe` target whose id is its frame id.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetInfo {
    target_id: String,
    #[serde(rename = "type")]
    kind: String,
    parent_frame_id: Option<String>,
}

/// Every frame of the page in the tab `target_id`, attached as `session`: main first, then depth
/// first with each frame's children sorted by url then id; a frame's index in this order is its
/// engine seq. Each session attached on the way is pushed onto `attached`, and the caller
/// detaches them ([`detach_all`]), on failure too.
///
/// The tab's session sees only frames in its own renderer. A cross-site child is a target of
/// its own, found by its `parentFrameId` once that parent is known; attaching to it shows its
/// same-process subtree, which may hold further cross-site children, hence the fixpoint.
pub(crate) async fn frame_sites(
    client: &CdpClient,
    session: &SessionId,
    target_id: &str,
    attached: &mut Vec<SessionId>,
) -> Result<Vec<FrameSite>, Box<dyn Error>> {
    let mut found = Vec::new();
    let main = frame_tree(client, session)
        .await
        .map_err(|error| format!("main frame {target_id}: {error}"))?;
    add_frame_tree(&mut found, main, None, session);
    loop {
        let targets: TargetsReply =
            decode(client.send("Target.getTargets", json!({}), None).await?)?;
        let known: HashSet<String> = found.iter().map(|site| site.id.clone()).collect();
        let fresh: Vec<TargetInfo> = targets
            .target_infos
            .into_iter()
            .filter(|target| {
                target.kind == "iframe"
                    && !known.contains(&target.target_id)
                    && target
                        .parent_frame_id
                        .as_ref()
                        .is_some_and(|parent| known.contains(parent))
            })
            .collect();
        if fresh.is_empty() {
            break;
        }
        for target in fresh {
            let session = attach_frame(client, &target.target_id, attached)
                .await
                .map_err(|error| format!("child frame {}: {error}", target.target_id))?;
            let tree = frame_tree(client, &session)
                .await
                .map_err(|error| format!("child frame {}: {error}", target.target_id))?;
            add_frame_tree(&mut found, tree, target.parent_frame_id, &session);
        }
    }
    Ok(in_oracle_order(found))
}

/// `Target.attachToTarget`'s reply.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Attached {
    session_id: String,
}

/// Attaches a flattened session to the cross-site child frame `target_id`, recording it in
/// `attached` so the caller of [`frame_sites`] detaches it again.
async fn attach_frame(
    client: &CdpClient,
    target_id: &str,
    attached: &mut Vec<SessionId>,
) -> Result<SessionId, Box<dyn Error>> {
    let reply: Attached = decode(
        bounded(
            "Target.attachToTarget",
            client.send(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
                None,
            ),
        )
        .await?,
    )?;
    let session = SessionId::from(reply.session_id);
    attached.push(session.clone());
    Ok(session)
}

/// Detaches every session in `attached`, ignoring errors: a frame that went away took its
/// session with it, and cleanup must not mask the read's own result.
pub(crate) async fn detach_all(client: &CdpClient, attached: Vec<SessionId>) {
    for session in attached {
        let _detached = client
            .send(
                "Target.detachFromTarget",
                json!({ "sessionId": session.as_str() }),
                None,
            )
            .await;
    }
}

async fn frame_tree(
    client: &CdpClient,
    session: &SessionId,
) -> Result<FrameTreeNode, Box<dyn Error>> {
    let reply: FrameTreeReply = decode(
        bounded(
            "Page.getFrameTree",
            client.send("Page.getFrameTree", json!({}), Some(session)),
        )
        .await?,
    )?;
    Ok(reply.frame_tree)
}

/// Adds `tree`'s frames, all owned by `session`; its root's parent is `parent`.
fn add_frame_tree(
    found: &mut Vec<FrameSite>,
    tree: FrameTreeNode,
    parent: Option<String>,
    session: &SessionId,
) {
    let mut pending = vec![(tree, parent)];
    while let Some((node, parent)) = pending.pop() {
        let id = node.frame.id;
        for child in node.child_frames {
            pending.push((child, Some(id.clone())));
        }
        found.push(FrameSite {
            url: oracle::recorded_url(&node.frame.url),
            id,
            parent,
            session: session.clone(),
        });
    }
}

/// Sorts `found` into the order [`frame_sites`] documents.
fn in_oracle_order(found: Vec<FrameSite>) -> Vec<FrameSite> {
    let mut children: HashMap<Option<String>, Vec<FrameSite>> = HashMap::new();
    for site in found {
        children.entry(site.parent.clone()).or_default().push(site);
    }
    for siblings in children.values_mut() {
        siblings.sort_by(|a, b| (&a.url, &a.id).cmp(&(&b.url, &b.id)));
        // Popped from the back below, so the first in order goes last.
        siblings.reverse();
    }
    let mut ordered = Vec::new();
    let mut pending = children.remove(&None).unwrap_or_default();
    while let Some(site) = pending.pop() {
        let below = children.remove(&Some(site.id.clone())).unwrap_or_default();
        ordered.push(site);
        pending.extend(below);
    }
    ordered
}

/// What `__neoOracleRead` returns (oracle.js); element and ref nodes index `__neoOracleNodes`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EngineRead {
    elements: Vec<EngineElement>,
    refs: Vec<EngineRef>,
    aria: OracleAria,
}

/// One element's facts as oracle.js writes them.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EngineElement {
    node: usize,
    role: String,
    name: String,
    description: String,
    error_message: String,
    hidden: bool,
    visible: bool,
    states: OracleStates,
}

/// One ai-mode ref and the element it names.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EngineRef {
    r#ref: String,
    node: usize,
}

/// A `Runtime.RemoteObject`, as far as the oracle reads one.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteObject {
    object_id: Option<String>,
    value: Option<Value>,
}

/// The reply of `Runtime.evaluate` and `Runtime.callFunctionOn`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ran {
    result: RemoteObject,
    exception_details: Option<Value>,
}

/// `DOM.getFrameOwner`'s reply.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameOwner {
    backend_node_id: u64,
}

/// `Page.createIsolatedWorld`'s reply.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct World {
    execution_context_id: i64,
}

/// `DOM.resolveNode`'s reply.
#[derive(Deserialize)]
struct Resolved {
    object: RemoteObject,
}

/// `Runtime.getProperties`'s reply.
#[derive(Deserialize)]
struct Properties {
    result: Vec<Property>,
}

/// One own property of the nodes array: an index and the element under it.
#[derive(Deserialize)]
struct Property {
    name: String,
    value: Option<RemoteObject>,
}

/// `DOM.describeNode`'s reply.
#[derive(Deserialize)]
struct Described {
    node: DescribedNode,
}

/// The part of `DOM.Node` the join needs.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedNode {
    backend_node_id: u64,
}

/// Reads one frame's document with the engine. `seq` names its refs (`f<seq>e<n>`; bare `e<n>`
/// in the main frame) as Neo's per-frame probes do. `snapshots` caches each session's DOM
/// capture, which covers every frame that session owns.
async fn read_frame(
    client: &CdpClient,
    site: &FrameSite,
    seq: usize,
    parent_session: Option<&SessionId>,
    engine: &OracleEngine,
    snapshots: &mut HashMap<SessionId, Captured>,
) -> Result<OracleFrame, Box<dyn Error>> {
    let session = &site.session;
    let owner = match (&site.parent, parent_session) {
        (None, _) => None,
        // DOM.getFrameOwner answers only on the session that owns the parent document.
        (Some(_), Some(parent_session)) => {
            let owner: FrameOwner = decode(
                bounded(
                    "DOM.getFrameOwner",
                    client.send(
                        "DOM.getFrameOwner",
                        json!({ "frameId": site.id }),
                        Some(parent_session),
                    ),
                )
                .await?,
            )?;
            Some(owner.backend_node_id)
        }
        (Some(parent), None) => {
            return Err(format!("its parent frame {parent} was not found").into());
        }
    };

    // Page.createIsolatedWorld succeeds only on the session that owns the frame. A world of its
    // own per read: the engine's prototype patches never reach the page or Neo's probe world.
    let world: World = decode(
        bounded(
            "Page.createIsolatedWorld",
            client.send(
                "Page.createIsolatedWorld",
                json!({
                    "frameId": site.id,
                    "worldName": ORACLE_WORLD,
                    "grantUniveralAccess": true,
                }),
                Some(session),
            ),
        )
        .await?,
    )?;
    let context = world.execution_context_id;
    let seeds = seed_objects(client, site, context, snapshots).await?;
    let read = run_engine(client, session, context, engine, seq, seeds).await?;

    let backend_ids = node_backend_ids(client, session, context).await?;
    let backend_id = |node: usize| {
        backend_ids
            .get(node)
            .copied()
            .ok_or_else(|| format!("__neoOracleNodes has no element {node}"))
    };
    let mut elements = Vec::with_capacity(read.elements.len());
    for element in read.elements {
        elements.push(OracleElement {
            backend_node_id: backend_id(element.node)?,
            role: element.role,
            name: element.name,
            description: element.description,
            error_message: element.error_message,
            hidden: element.hidden,
            visible: element.visible,
            states: element.states,
        });
    }
    let mut refs = BTreeMap::new();
    for engine_ref in read.refs {
        refs.insert(engine_ref.r#ref, backend_id(engine_ref.node)?);
    }
    Ok(OracleFrame {
        frame: site.id.clone(),
        parent: site.parent.clone(),
        owner,
        url: site.url.clone(),
        elements,
        refs,
        aria: read.aria,
    })
}

/// The closed-root seeds of `site`'s document as `Runtime.callFunctionOn` arguments, each a
/// remote object in the oracle's world `context`.
async fn seed_objects(
    client: &CdpClient,
    site: &FrameSite,
    context: i64,
    snapshots: &mut HashMap<SessionId, Captured>,
) -> Result<Vec<Value>, Box<dyn Error>> {
    let session = &site.session;
    if !snapshots.contains_key(session) {
        let captured = decode(
            bounded(
                "DOMSnapshot.captureSnapshot",
                client.send(
                    "DOMSnapshot.captureSnapshot",
                    json!({ "computedStyles": [] }),
                    Some(session),
                ),
            )
            .await?,
        )?;
        snapshots.insert(session.clone(), captured);
    }
    let seed_ids = snapshots
        .get(session)
        .map(|captured| closed_root_seeds(captured, &site.id))
        .unwrap_or_default();
    let mut seeds = Vec::with_capacity(seed_ids.len());
    for backend_node_id in seed_ids {
        let resolved: Resolved = decode(
            bounded(
                "DOM.resolveNode",
                client.send(
                    "DOM.resolveNode",
                    json!({ "backendNodeId": backend_node_id, "executionContextId": context }),
                    Some(session),
                ),
            )
            .await?,
        )?;
        let object_id = resolved
            .object
            .object_id
            .ok_or("DOM.resolveNode gave a closed-root seed no object id")?;
        seeds.push(json!({ "objectId": object_id }));
    }
    Ok(seeds)
}

/// Evaluates the patched bundle in the world `context` and runs `__neoOracleRead` there.
async fn run_engine(
    client: &CdpClient,
    session: &SessionId,
    context: i64,
    engine: &OracleEngine,
    seq: usize,
    seeds: Vec<Value>,
) -> Result<EngineRead, Box<dyn Error>> {
    let installed: Ran = decode(
        bounded(
            "Runtime.evaluate (engine)",
            client.send(
                "Runtime.evaluate",
                json!({ "expression": engine.source, "contextId": context }),
                Some(session),
            ),
        )
        .await?,
    )?;
    // oracle.js throws here, naming the binding, when the bundle no longer defines one it reads.
    if let Some(exception) = installed.exception_details {
        return Err(format!("installing the engine threw: {exception}").into());
    }

    let mut arguments = vec![json!({ "value": seq })];
    arguments.extend(seeds);
    let ran: Ran = decode(
        bounded(
            "Runtime.callFunctionOn (__neoOracleRead)",
            client.send(
                "Runtime.callFunctionOn",
                json!({
                    "functionDeclaration":
                        "function (seq, ...seeds) { return globalThis.__neoOracleRead(seq, ...seeds); }",
                    "executionContextId": context,
                    "arguments": arguments,
                    "returnByValue": true,
                }),
                Some(session),
            ),
        )
        .await?,
    )?;
    if let Some(exception) = ran.exception_details {
        return Err(format!("__neoOracleRead threw: {exception}").into());
    }
    let text = ran
        .result
        .value
        .as_ref()
        .and_then(Value::as_str)
        .ok_or("__neoOracleRead returned no JSON text")?;
    // An ARIA tree nests two JSON levels per element; deep-nesting's passes serde's default 128.
    let mut reader = serde_json::Deserializer::from_str(text);
    reader.disable_recursion_limit();
    let read = EngineRead::deserialize(&mut reader)?;
    reader.end()?;
    Ok(read)
}

/// The backendNodeId of each element in the world's `__neoOracleNodes`, by index.
async fn node_backend_ids(
    client: &CdpClient,
    session: &SessionId,
    context: i64,
) -> Result<Vec<u64>, Box<dyn Error>> {
    let nodes: Ran = decode(
        bounded(
            "Runtime.evaluate (__neoOracleNodes)",
            client.send(
                "Runtime.evaluate",
                json!({ "expression": "globalThis.__neoOracleNodes", "contextId": context }),
                Some(session),
            ),
        )
        .await?,
    )?;
    let array = nodes
        .result
        .object_id
        .ok_or("__neoOracleNodes is not an object")?;
    let properties: Properties = decode(
        bounded(
            "Runtime.getProperties",
            client.send(
                "Runtime.getProperties",
                json!({ "objectId": array, "ownProperties": true }),
                Some(session),
            ),
        )
        .await?,
    )?;
    let mut by_index = BTreeMap::new();
    for property in properties.result {
        let Ok(index) = property.name.parse::<usize>() else {
            continue;
        };
        let object_id = property
            .value
            .and_then(|value| value.object_id)
            .ok_or_else(|| format!("__neoOracleNodes[{index}] is not a node"))?;
        let described: Described = decode(
            bounded(
                "DOM.describeNode",
                client.send(
                    "DOM.describeNode",
                    json!({ "objectId": object_id }),
                    Some(session),
                ),
            )
            .await?,
        )?;
        by_index.insert(index, described.node.backend_node_id);
    }
    let ids: Vec<u64> = by_index.values().copied().collect();
    if by_index.keys().copied().ne(0..ids.len()) {
        return Err("__neoOracleNodes is not a dense array".into());
    }
    Ok(ids)
}

/// `DOMSnapshot.captureSnapshot`'s reply: flat arrays, so a page of any depth reads within
/// serde's nesting limit (`DOM.getDocument`'s nested nodes do not on deep-nesting).
#[derive(Deserialize)]
struct Captured {
    documents: Vec<CapturedDocument>,
    strings: Vec<String>,
}

/// One document of a capture: the session's own frame or a same-process child frame.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapturedDocument {
    /// Index into [`Captured::strings`].
    frame_id: usize,
    nodes: CapturedNodes,
}

/// The node columns the seed search reads.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapturedNodes {
    #[serde(default)]
    node_type: Vec<i64>,
    #[serde(default)]
    backend_node_id: Vec<u64>,
    /// The type of the shadow root each listed node sits in.
    shadow_root_type: Option<RareStrings>,
    pseudo_type: Option<RareStrings>,
}

/// `DOMSnapshot`'s sparse string column: `value[i]` (a string index) belongs to node `index[i]`.
#[derive(Deserialize)]
struct RareStrings {
    index: Vec<usize>,
    value: Vec<usize>,
}

/// Elements inside closed shadow roots of frame `frame_id`'s document. A script in any world
/// gets `null` from a closed host; handed one node inside the root, `openClosedRoots` opens that
/// root to the oracle's world, as Neo's marked snapshots do. Every such element is a seed, so a
/// root nested in another closed root is opened as well.
fn closed_root_seeds(captured: &Captured, frame_id: &str) -> Vec<u64> {
    let Some(document) = captured.documents.iter().find(|document| {
        captured.strings.get(document.frame_id).map(String::as_str) == Some(frame_id)
    }) else {
        return Vec::new();
    };
    let nodes = &document.nodes;
    let Some(shadow) = &nodes.shadow_root_type else {
        return Vec::new();
    };
    let pseudo: HashSet<usize> = nodes
        .pseudo_type
        .as_ref()
        .map(|pseudo| pseudo.index.iter().copied().collect())
        .unwrap_or_default();
    let mut seeds = Vec::new();
    for (&node, &kind) in shadow.index.iter().zip(&shadow.value) {
        let closed = captured.strings.get(kind).map(String::as_str) == Some("closed");
        let element = nodes.node_type.get(node) == Some(&1);
        if closed && element && !pseudo.contains(&node) {
            seeds.extend(nodes.backend_node_id.get(node).copied());
        }
    }
    seeds
}

/// Decodes a CDP reply into the typed shape the step reads.
fn decode<T: DeserializeOwned>(reply: Value) -> Result<T, Box<dyn Error>> {
    Ok(serde_json::from_value(reply)?)
}

/// Bounds one per-frame CDP step by [`FRAME_STEP_TIMEOUT`]; the caller names the frame.
async fn bounded<F, E>(step: &str, send: F) -> Result<Value, Box<dyn Error>>
where
    F: Future<Output = Result<Value, E>>,
    E: Error + 'static,
{
    match time::timeout(FRAME_STEP_TIMEOUT, send).await {
        Ok(Ok(reply)) => Ok(reply),
        Ok(Err(error)) => Err(format!("{step}: {error}").into()),
        Err(_elapsed) => Err(format!("{step} did not answer within {FRAME_STEP_TIMEOUT:?}").into()),
    }
}
