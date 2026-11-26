//! The live half of the zero-conflict check (design 10.2 and 10.3): what a tree read offline
//! cannot show. The edge-pack test and the site runner measure one page, loaded in a tab of its
//! own and its oracle read, the same way:
//!
//! ```text
//!  measure: frame sessions (oracle_capture::frame_sites) ─▶ before_capture
//!         ─▶ APC #1 ─▶ tree1 ─▶ check_tree(&tree1, &mut refs)
//!         ─▶ APC #2 ─▶ tree2 ─▶ check_tree(&tree2, &mut refs)    one TabRefTable, as a tab holds it
//!         ─▶ stability   both captures print the same texts and give each line the same verdict
//!         ─▶ hydration   every ref #2 printed resolves in its frame's session and world
//!         ─▶ recall      per frame and role, the oracle's set beside Rust's internal:role set
//! ```
//!
//! Capture #1 is checked through a fresh ref table, so `check_apc` on its saved bytes replays its
//! verdicts offline. Recall reads tree #1, the capture taken nearest the oracle.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::future::Future;

use browseros_cdp::{CdpClient, SessionId};
use neo_snapshot_apc::refs::TabRefTable;
use neo_snapshot_apc::select::{AccessibleText, ChunkPlan, PlannedStage, RefResolver, Scope};
use neo_snapshot_apc::tree::{BuildOptions, FrameToken, NodeKey, SnapshotTree, TreeDoc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::fixture_pages;
use crate::oracle::Oracle;
use crate::oracle_capture::{self, FrameSite};
use crate::zero_conflict::{self, ByRole, ByRoleOptions, Format, PageCheck, Verdict};

/// The world each frame's refs are hydrated in: a world of its own, as the engine's resolver
/// enters one per frame, so neither the page's scripts nor the oracle's patches reach the probe.
const HYDRATION_WORLD: &str = "neo-zero-conflict";

/// The object group every resolved node joins, released per session once the page is done.
const HYDRATION_GROUP: &str = "neo-zero-conflict";

/// Runs on a resolved node in its frame's world (design 6.2's check): connected, owned by the
/// world's document, and whether any shadow root between it and that document is closed.
/// `getRootNode()` returns a closed root too; only `host.shadowRoot` hides one.
const HYDRATION_PROBE: &str = "function () {
  let closedShadow = false;
  for (let root = this.getRootNode(); root.host; root = root.host.getRootNode()) {
    if (root.mode === 'closed') closedShadow = true;
  }
  return { connected: this.isConnected, sameDocument: this.ownerDocument === document, closedShadow };
}";

/// One capture of the page: the bytes `Page.getAnnotatedPageContent` sent and their check.
#[derive(Debug)]
pub(crate) struct Capture {
    /// The protobuf bytes, which `decode_protobuf` reads back.
    pub(crate) pb: Vec<u8>,
    pub(crate) check: PageCheck,
}

/// One page measured live.
#[derive(Debug)]
pub(crate) struct Measured {
    /// Tree #1's `contract_version`: the browser carries BrowserOS's APC contract, or the tree
    /// would not have built.
    pub(crate) contract_version: u32,
    /// Capture #1, checked through a fresh ref table.
    pub(crate) first: Capture,
    /// Capture #2, checked through the same table, as the tab's next snapshot.
    pub(crate) second: Capture,
    /// Every difference between the two captures; empty when the page printed stably.
    pub(crate) unstable: Vec<Instability>,
    /// Each distinct ref capture #2 printed, in first-printed order.
    pub(crate) hydrated: Vec<RefHydration>,
    /// The oracle's role sets beside Rust's; `None` when the caller had no oracle.
    pub(crate) recall: Option<Recall>,
}

/// Measures the page of the tab `target_id`, attached as `session`, on the load it is on: frame
/// sessions, then `before_capture`, then the two captures and their checks. `oracle` is the
/// page's oracle, read on this load before `before_capture`. Child-frame sessions attached here
/// are detached again, also on failure.
///
/// Frame sessions are found before `before_capture` because that step may hang a renderer
/// (`hung-subframe`), which then answers no `Page.getFrameTree`.
pub(crate) async fn measure(
    client: &CdpClient,
    page: &str,
    session: &SessionId,
    target_id: &str,
    oracle: Option<&Oracle>,
    before_capture: impl Future<Output = Result<(), Box<dyn Error>>>,
) -> Result<Measured, Box<dyn Error>> {
    let mut attached = Vec::new();
    let measured = async {
        let sites = oracle_capture::frame_sites(client, session, target_id, &mut attached)
            .await
            .map_err(|error| format!("{page}: frame discovery: {error}"))?;
        before_capture.await?;
        // Back to back, so the second capture sees the page the first saw unless it changed
        // by itself.
        let (pb1, apc1) = fixture_pages::capture_apc(client, session).await?;
        let (pb2, apc2) = fixture_pages::capture_apc(client, session).await?;
        let tree1 = SnapshotTree::build(apc1, BuildOptions::new())
            .map_err(|error| format!("{page}: capture #1 does not build: {error}"))?;
        let tree2 = SnapshotTree::build(apc2, BuildOptions::new())
            .map_err(|error| format!("{page}: capture #2 does not build: {error}"))?;
        let mut refs = TabRefTable::new();
        let first = zero_conflict::check_tree(page, &tree1, &mut refs)?;
        let second = zero_conflict::check_tree(page, &tree2, &mut refs)?;
        let unstable = instabilities(&first, &second);
        let mut frames = Frames {
            client,
            session,
            target_id,
            sites,
            rediscovered: false,
            worlds: HashMap::new(),
        };
        let hydrated = frames.hydrate(&tree2, &second, &mut attached).await;
        frames.release().await;
        Ok::<_, Box<dyn Error>>(Measured {
            contract_version: tree1.contract_version(),
            recall: oracle.map(|oracle| recall(&tree1, oracle)),
            first: Capture {
                pb: pb1,
                check: first,
            },
            second: Capture {
                pb: pb2,
                check: second,
            },
            unstable,
            hydrated,
        })
    }
    .await;
    oracle_capture::detach_all(client, attached).await;
    measured
}

/// One difference between two captures of an unchanged page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Instability {
    /// The format's text is not byte-equal.
    Text(Format),
    /// The format's texts hold a different number of lines with a ref.
    Lines {
        format: Format,
        /// Lines with a ref in capture #1.
        first: usize,
        /// Lines with a ref in capture #2.
        second: usize,
    },
    /// A line with a ref came to another verdict; lines pair by their order in the format, so
    /// only when both captures print as many.
    Verdict {
        format: Format,
        /// The line's 1-based number in capture #1's text.
        line: usize,
        first: Verdict,
        second: Verdict,
    },
}

/// Every difference between `first` and `second`, format by format: the text, then the lines
/// with a ref in order, or how many there are when that differs.
fn instabilities(first: &PageCheck, second: &PageCheck) -> Vec<Instability> {
    let mut found = Vec::new();
    for (format, (one, two)) in Format::ALL
        .into_iter()
        .zip(first.texts.iter().zip(&second.texts))
    {
        if one != two {
            found.push(Instability::Text(format));
        }
        let lines = |check: &PageCheck| -> Vec<_> {
            check
                .lines
                .iter()
                .filter(|line| line.format == format)
                .cloned()
                .collect()
        };
        let (one, two) = (lines(first), lines(second));
        if one.len() != two.len() {
            // Lines pair by position, so after a line comes or goes every later pair would
            // differ: the count says it.
            found.push(Instability::Lines {
                format,
                first: one.len(),
                second: two.len(),
            });
            continue;
        }
        for (one, two) in one.iter().zip(&two) {
            if one.verdict != two.verdict {
                found.push(Instability::Verdict {
                    format,
                    line: one.line.number,
                    first: one.verdict.clone(),
                    second: two.verdict.clone(),
                });
            }
        }
    }
    found
}

/// One distinct printed ref and whether it hydrated.
#[derive(Clone, Debug)]
pub(crate) struct RefHydration {
    /// The ref as printed, `e5` or `f2e5`.
    pub(crate) r#ref: String,
    pub(crate) outcome: Result<Hydrated, HydrationFailure>,
}

/// A ref that hydrated: resolved in its frame's world, connected, and owned by that world's
/// document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hydrated {
    /// Documents between the ref's and the main one: 0 in the main frame, 4 in `C4`.
    pub(crate) depth: usize,
    /// A shadow root between the element and its document is closed.
    pub(crate) closed_shadow: bool,
}

/// Why a printed ref did not hydrate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HydrationFailure {
    /// The ref names no live element in the table that printed it.
    Unresolved,
    /// The element's document token names no document of the tree.
    NoDocument,
    /// The element's document carries no devtools frame token.
    NoFrameToken,
    /// No CDP session owns the document's frame, even after looking again.
    NoSession,
    /// `Page.createIsolatedWorld` failed for the frame.
    World(String),
    /// `DOM.resolveNode` failed in the frame's world.
    Resolve(String),
    /// The probe threw or answered in another shape.
    Probe(String),
    /// The node resolved and is not connected.
    Detached,
    /// The node resolved and belongs to another document than the world's.
    ForeignDocument,
}

impl HydrationFailure {
    /// The failure's kind, without its CDP detail.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Unresolved => "unresolved",
            Self::NoDocument => "no-document",
            Self::NoFrameToken => "no-frame-token",
            Self::NoSession => "no-session",
            Self::World(_) => "world",
            Self::Resolve(_) => "resolve",
            Self::Probe(_) => "probe",
            Self::Detached => "detached",
            Self::ForeignDocument => "foreign-document",
        }
    }
}

impl fmt::Display for HydrationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::World(error) | Self::Resolve(error) | Self::Probe(error) => {
                write!(f, "{}: {error}", self.kind())
            }
            Self::Unresolved
            | Self::NoDocument
            | Self::NoFrameToken
            | Self::NoSession
            | Self::Detached
            | Self::ForeignDocument => f.write_str(self.kind()),
        }
    }
}

/// The page's frames as hydration reaches them: each frame's owning session, and the world made
/// in it on first use.
struct Frames<'a> {
    client: &'a CdpClient,
    session: &'a SessionId,
    target_id: &'a str,
    sites: Vec<FrameSite>,
    /// Frame discovery ran again for a frame the first discovery did not know. Once per page:
    /// a frame added after the captures is not the one they saw.
    rediscovered: bool,
    /// Each frame's world, by frame id, or why it could not be made.
    worlds: HashMap<String, Result<World, HydrationFailure>>,
}

#[derive(Clone)]
struct World {
    session: SessionId,
    context: i64,
}

/// `Page.createIsolatedWorld`'s reply.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatedWorld {
    execution_context_id: i64,
}

/// `DOM.resolveNode`'s reply.
#[derive(Deserialize)]
struct Resolved {
    object: ResolvedObject,
}

/// The resolved node's remote object.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolvedObject {
    object_id: String,
}

/// `Runtime.callFunctionOn`'s reply with the probe's answer by value.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Probed {
    result: ProbedValue,
    exception_details: Option<Value>,
}

/// The probe's by-value result.
#[derive(Deserialize)]
struct ProbedValue {
    value: ProbeAnswer,
}

/// What [`HYDRATION_PROBE`] returns.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProbeAnswer {
    connected: bool,
    same_document: bool,
    closed_shadow: bool,
}

impl Frames<'_> {
    /// Hydrates each distinct ref `check` printed for `tree`, in first-printed order.
    async fn hydrate(
        &mut self,
        tree: &SnapshotTree,
        check: &PageCheck,
        attached: &mut Vec<SessionId>,
    ) -> Vec<RefHydration> {
        let mut seen = HashSet::new();
        let mut hydrated = Vec::new();
        for line in &check.lines {
            if !seen.insert(line.line.r#ref.as_str()) {
                continue;
            }
            let outcome = match &line.target {
                None => Err(HydrationFailure::Unresolved),
                Some(key) => self.hydrate_key(tree, key, attached).await,
            };
            hydrated.push(RefHydration {
                r#ref: line.line.r#ref.clone(),
                outcome,
            });
        }
        hydrated
    }

    /// Resolves `key` in its frame's world and probes it there.
    async fn hydrate_key(
        &mut self,
        tree: &SnapshotTree,
        key: &NodeKey,
        attached: &mut Vec<SessionId>,
    ) -> Result<Hydrated, HydrationFailure> {
        let doc = tree
            .document_by_token(&key.doc)
            .ok_or(HydrationFailure::NoDocument)?;
        let frame = doc
            .frame_token()
            .ok_or(HydrationFailure::NoFrameToken)?
            .as_str();
        let world = self.world(frame, attached).await?;
        let client = self.client;
        let resolved: Resolved = decode(
            client
                .send(
                    "DOM.resolveNode",
                    json!({
                        "backendNodeId": key.node.as_backend_node_id(),
                        "executionContextId": world.context,
                        "objectGroup": HYDRATION_GROUP,
                    }),
                    Some(&world.session),
                )
                .await,
        )
        .map_err(HydrationFailure::Resolve)?;
        let probed: Probed = decode(
            client
                .send(
                    "Runtime.callFunctionOn",
                    json!({
                        "functionDeclaration": HYDRATION_PROBE,
                        "objectId": resolved.object.object_id,
                        "returnByValue": true,
                    }),
                    Some(&world.session),
                )
                .await,
        )
        .map_err(HydrationFailure::Probe)?;
        if let Some(exception) = probed.exception_details {
            return Err(HydrationFailure::Probe(exception.to_string()));
        }
        let answer = probed.result.value;
        if !answer.connected {
            return Err(HydrationFailure::Detached);
        }
        if !answer.same_document {
            return Err(HydrationFailure::ForeignDocument);
        }
        Ok(Hydrated {
            depth: depth(tree, doc),
            closed_shadow: answer.closed_shadow,
        })
    }

    /// The world of `frame`, made on first use in the session that owns the frame.
    async fn world(
        &mut self,
        frame: &str,
        attached: &mut Vec<SessionId>,
    ) -> Result<World, HydrationFailure> {
        if let Some(world) = self.worlds.get(frame) {
            return world.clone();
        }
        if !self.sites.iter().any(|site| site.id == frame) && !self.rediscovered {
            self.rediscovered = true;
            if let Ok(sites) =
                oracle_capture::frame_sites(self.client, self.session, self.target_id, attached)
                    .await
            {
                self.sites = sites;
            }
        }
        let world = match self.sites.iter().find(|site| site.id == frame) {
            None => Err(HydrationFailure::NoSession),
            Some(site) => {
                let created: Result<CreatedWorld, String> = decode(
                    self.client
                        .send(
                            "Page.createIsolatedWorld",
                            json!({ "frameId": frame, "worldName": HYDRATION_WORLD }),
                            Some(&site.session),
                        )
                        .await,
                );
                created
                    .map(|created| World {
                        session: site.session.clone(),
                        context: created.execution_context_id,
                    })
                    .map_err(HydrationFailure::World)
            }
        };
        self.worlds.insert(frame.to_owned(), world.clone());
        world
    }

    /// Releases every node hydration resolved, one object group per session that holds some,
    /// ignoring errors: a frame that went away released its objects with it.
    async fn release(&self) {
        let mut released = BTreeSet::new();
        for world in self.worlds.values().filter_map(|world| world.as_ref().ok()) {
            if !released.insert(world.session.as_str()) {
                continue;
            }
            let _released = self
                .client
                .send(
                    "Runtime.releaseObjectGroup",
                    json!({ "objectGroup": HYDRATION_GROUP }),
                    Some(&world.session),
                )
                .await;
        }
    }
}

fn depth(tree: &SnapshotTree, doc: &TreeDoc) -> usize {
    let mut depth = 0;
    let mut parent = doc.parent_doc();
    while let Some(id) = parent {
        depth += 1;
        parent = tree.document(id).and_then(TreeDoc::parent_doc);
    }
    depth
}

/// Decodes a CDP reply into the shape a step reads; the error is the CDP or decode error's text.
fn decode<T, E>(reply: Result<Value, E>) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
    E: fmt::Display,
{
    let reply = reply.map_err(|error| error.to_string())?;
    serde_json::from_value(reply).map_err(|error| error.to_string())
}

/// How one frame's role set compares: the page's elements of that role that are not hidden for
/// ARIA (what upstream's `internal:role=<role>` returns, `roleSelectorEngine.ts` `queryRole`),
/// beside what Rust's tree stage selects for the same query in the frame's document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoleRecall {
    /// The frame's index in the oracle (its engine seq; 0 is the main frame).
    pub(crate) frame: usize,
    /// The role, as `getAriaRole` gives it.
    pub(crate) role: String,
    /// Elements of the page's set.
    pub(crate) page: usize,
    /// Elements of the page's set Rust's set holds too.
    pub(crate) found: usize,
    /// Elements Rust's set holds that the page's does not: another element of the document,
    /// one of another document, or a synthesized node with no DOM node.
    pub(crate) extra: usize,
    /// The page's elements Rust's set lacks, by backendNodeId.
    pub(crate) missing: Vec<u64>,
    /// Why Rust's set could not be read: the frame has no document in the tree, or the query
    /// failed. Every element of the page's set then counts as missing.
    pub(crate) unread: Option<String>,
}

/// Recall over every frame of the oracle and every role present there (design 10.2): what the
/// zero-conflict check alone cannot see, an element Playwright finds that the tree lacks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Recall {
    /// One entry per frame and role, frames in oracle order and roles sorted.
    pub(crate) roles: Vec<RoleRecall>,
    /// Oracle frames that no document of the tree names as its frame token.
    pub(crate) frames_without_document: usize,
}

impl Recall {
    /// Elements of every page set.
    pub(crate) fn page(&self) -> usize {
        self.roles.iter().map(|role| role.page).sum()
    }

    /// Elements of every page set that Rust's set holds too.
    pub(crate) fn found(&self) -> usize {
        self.roles.iter().map(|role| role.found).sum()
    }

    /// Elements Rust's sets hold beyond the page's.
    pub(crate) fn extra(&self) -> usize {
        self.roles.iter().map(|role| role.extra).sum()
    }
}

/// No `aria-ref` part reaches a role-only chunk, but a tree stage takes a resolver.
struct NoRefs;

impl RefResolver for NoRefs {
    fn resolve(&self, _body: &str) -> Option<NodeKey> {
        None
    }
}

/// Compares `oracle`'s role sets with Rust's `internal:role=<role>` over `tree`, frame by frame.
/// An oracle frame is matched with the tree document whose `frame_token()` is its frame id.
fn recall(tree: &SnapshotTree, oracle: &Oracle) -> Recall {
    let names = tree.name_cache();
    let mut recall = Recall::default();
    for (index, frame) in oracle.frames.iter().enumerate() {
        let doc = tree
            .documents()
            .iter()
            .find(|doc| doc.frame_token().map(FrameToken::as_str) == Some(frame.frame.as_str()));
        recall.frames_without_document += usize::from(doc.is_none());
        let mut roles: BTreeMap<&str, BTreeSet<u64>> = BTreeMap::new();
        for element in &frame.elements {
            let set = roles.entry(element.role.as_str()).or_default();
            if !element.hidden {
                set.insert(element.backend_node_id);
            }
        }
        for (role, page) in roles {
            let selected = match doc {
                None => Err("the frame has no document in the tree".to_owned()),
                Some(doc) => select_role(tree, doc, &names, role),
            };
            let (found, extra, missing, unread) = match selected {
                Ok(rust) => {
                    let in_doc: BTreeSet<u64> = rust.in_doc.iter().copied().collect();
                    let missing: Vec<u64> = page.difference(&in_doc).copied().collect();
                    let extra = in_doc.difference(&page).count() + rust.elsewhere;
                    (page.len() - missing.len(), extra, missing, None)
                }
                Err(error) => (0, 0, page.iter().copied().collect(), Some(error)),
            };
            recall.roles.push(RoleRecall {
                frame: index,
                role: role.to_owned(),
                page: page.len(),
                found,
                extra,
                missing,
                unread,
            });
        }
    }
    recall
}

/// What Rust's `internal:role=<role>` selects in one document.
struct RoleSet {
    /// backendNodeIds of the matches in the document.
    in_doc: Vec<u64>,
    /// Matches in another document, or with no DOM node.
    elsewhere: usize,
}

/// Runs `internal:role=<role>` through `ChunkPlan` as one tree stage over `doc`, as a role
/// locator's chunk runs.
fn select_role(
    tree: &SnapshotTree,
    doc: &TreeDoc,
    names: &dyn AccessibleText,
    role: &str,
) -> Result<RoleSet, String> {
    let query = ByRole {
        role: role.to_owned(),
        options: ByRoleOptions::default(),
    };
    let plan = ChunkPlan::of(&query.chunk()).map_err(|error| error.to_string())?;
    let stages = plan.stages().map_err(|error| error.to_string())?;
    let (PlannedStage::Tree(run), []) = (stages.first(), stages.rest()) else {
        return Err(format!("{} stages", stages.iter().count()));
    };
    let scope = Scope::document(tree, doc.id()).ok_or("the document holds no node")?;
    let matches = run
        .stage()
        .select(tree, scope, names, &NoRefs)
        .map_err(|error| error.to_string())?;
    let mut set = RoleSet {
        in_doc: Vec::new(),
        elsewhere: 0,
    };
    for key in matches.keys(tree) {
        match key {
            Some(key) if &key.doc == doc.token() => {
                set.in_doc
                    .push(u64::try_from(key.node.raw()).map_err(|error| error.to_string())?);
            }
            Some(_) | None => set.elsewhere += 1,
        }
    }
    Ok(set)
}

/// The verdict's kind, with the match count where it has one, and without the selector or name
/// a few carry: what a report outside the private folder may print.
pub(crate) fn verdict_brief(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Reached { matches } | Verdict::Missed { matches } => {
            format!("{} ({matches} matches)", verdict_kind(verdict))
        }
        Verdict::Label(_)
        | Verdict::WrongFrame
        | Verdict::RefUnresolved
        | Verdict::FrameUnresolved
        | Verdict::NotOneTreeStage(_)
        | Verdict::StageError(_) => verdict_kind(verdict).to_owned(),
    }
}

/// The verdict's kind, without the selector or name a few carry: what a report outside the
/// private folder may print.
pub(crate) fn verdict_kind(verdict: &Verdict) -> &'static str {
    match verdict {
        Verdict::Reached { .. } => "reached",
        Verdict::Label(_) => "label",
        Verdict::Missed { .. } => "missed",
        Verdict::WrongFrame => "wrong-frame",
        Verdict::RefUnresolved => "ref-unresolved",
        Verdict::FrameUnresolved => "frame-unresolved",
        Verdict::NotOneTreeStage(_) => "not-one-tree-stage",
        Verdict::StageError(_) => "stage-error",
    }
}
