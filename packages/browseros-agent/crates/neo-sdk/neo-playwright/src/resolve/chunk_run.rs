//! One run of a [`ChunkPlan`] in one frame for one attempt: its tree stages run here, over the
//! attempt's [`SnapshotTree`], and its page stages in the frame's isolated world, through the
//! probe's `stage` request (apc-engine-design 5.4, 5.5 and 5.7). The switch-over wires it.
//!
//! A run carries one current set through the plan's stages, as upstream's `querySelectorAll`
//! carries its roots through a chunk's parts:
//!
//! ```text
//!  Root            the chunk's query root: the frame's Document, or an element handle's element
//!  Keys(matches)   what a tree stage matched, nodes of this attempt's tree, nothing hydrated
//!  Pins(matches)   what a page stage matched, pinned in the frame's world (the Document a marker)
//!
//!  stage  from  what runs                                        (q: the query root's tree node)
//!  Tree   Root  q known: TreeStage::select(Scope::Root(q)), no CDP ───────────────────▶ Keys
//!               q unknown: the run's page form, no `from` ───────────────────────────▶ Pins
//!  Tree   Pins  path 1, keys(pins); path 2, every root a node of this document and q known:
//!               select(Scope::After { root: q, roots }), nothing hydrated ──────────────▶ Keys
//!               path 3, a plain element among the roots: the page form, from = pins ──▶ Pins
//!  Page   Root  the page form, no `from` ────────────────────────────────────────────▶ Pins
//!  Page   Keys  the page form, from = the keys, hydrated (the Document stays a marker) ▶ Pins
//!  Page   Pins  the page form, from = pins ──────────────────────────────────────────▶ Pins
//! ```
//!
//! A plan's stages alternate, so a tree stage never starts from Keys; if one did it would run as
//! path 2. Every page call first evaluates the form's candidates (its tree leaves, see
//! [`ChunkPlan`]) over the frame's **whole** document and hands their matches over hydrated:
//! a role or ref match is a fact of the element alone, so the page's containment test scopes
//! them exactly, and the whole stage is one probe call however many roots it has. Path 2 exists
//! because that hydration is the expensive part: `locator('#modal').getByRole('button')` takes
//! the modal's key (one `keys` call) and finds its buttons in the tree, instead of hydrating
//! every button of the page.
//!
//! A call hands its tree nodes to the page as call nodes (`SetMember::Node`), resolved for that
//! call alone, never as pins: the probe keeps its last 512 pin entries, and a stage with more
//! candidates than that, pinned one by one, would evict its own members. An option of a collapsed
//! `<select>` is a tree node with no DOM node of its own ([`SnapshotTree::option_slot`]): its
//! select goes over as the call node, and the member names the option's index and the list's
//! length in the capture (`SetMember::Option`). [`Handover::member`] is the one place a tree node
//! becomes a member, so no other code learns that address.
//!
//! **Strict mode** is decided on the final set. When the last stage is a page call, that call
//! carries the chunk and the page writes upstream's message. When the final set is Keys, the
//! tree decides, and only the violation's previews (at most ten) are hydrated and described.
//!
//! **Staleness.** The tree answers this attempt only (design 7.1). A `detached` reply, or `None`
//! from the seam, means a member left the page since the capture: the run is stale and the
//! attempt starts over with a new capture. A candidate that left is not: a candidate set only
//! ever filters by containment, so the page drops it, as upstream's query would not find it.
//!
//! **Stale refs.** Upstream reports an `aria-ref` that names no live node only once a root reaches
//! its part, and fails the op at once on it when nothing matched (`ProbeReply::None`'s
//! `stale_ref`). A tree stage does the same. A candidate is evaluated before any root reaches it,
//! so its set goes to the page as `StageSet::StaleRef`: the part matches nothing there, and the
//! `staged` reply names the first such ref a root reached. The run keeps the first stale ref of
//! all its stages, tree and page alike.
//!
//! **Candidate errors** cross the same way: Rust gives a candidate's error upstream's class
//! ([`class_of`]) and the page throws it with that class once a root reaches the part, so the
//! page's `error` reply is the run's outcome as it stands.

use std::collections::HashMap;
use std::fmt;
use std::ops::ControlFlow;

use neo_snapshot_apc::select::{
    self, AccessibleText, AriaTemplateError, ChunkPlan, PageForm, PlannedStage, RefResolver,
    RoleQueryError, Scope, StrictPreview, StrictViolation, TreeMatches, TreeStage, TreeStageError,
};
use neo_snapshot_apc::tree::{
    DocId, DocToken, NodeId, NodeKey, OptionSlot, SnapshotTree, TreeNode,
};

use crate::probe::{
    self, ErrorClass, PinPreview, ProbeReply, ProbeRequest, SetMember, StageMatch, StageSet,
    StrictListing,
};
use crate::resolve::strict_akas;
use crate::target::Strictness;
use crate::world::Pin;

#[cfg(test)]
mod fake_page;
#[cfg(test)]
mod tests;

/// The page side of a run: the frame's isolated world, reached from the chunk's query root.
///
/// `tree::ChunkWorld` implements it over `OpPage` and `tree::hydrate`, for the one
/// attempt whose tree the run reads. A tree node reaches the page by design 6.2: its key is
/// resolved (`DOM.resolveNode`) only in a world installed before this attempt's capture, and the
/// call that uses it proves the world still alive, since V8 refuses a node of another world. A
/// member that is gone is no error: it answers `None`, and the run goes stale.
pub(crate) trait ChunkPage {
    /// Why a call did not reach the page, such as a world that went away. A run returns it
    /// untouched, for the op's liveness rule.
    type Error;

    /// Sends a `stage` or a `describePins` in the frame's world through
    /// `probe::call_with_nodes`, whose root is the query root's node when that is an element
    /// handle (`resolver::ask`) and empty otherwise. The request's `SetMember::Node(i)`, and the
    /// select of its `SetMember::Option { select: i, .. }`, is `nodes[i]`, a node of the run's
    /// tree with a DOM node, which the seam resolves for this call alone and hands over after
    /// the root in that order. `None` when one of `nodes` no longer resolves.
    async fn call(
        &mut self,
        request: &ProbeRequest,
        nodes: &[NodeId],
    ) -> Result<Option<ProbeReply>, Self::Error>;

    /// Pins `node`'s element in the world's pin table: what outlives a call, such as the iframe
    /// a frame hop enters. `None` when it is gone. `node` has a DOM node: an option slot has
    /// none, and is pinned through a `stage` instead ([`ChunkRun::first`]).
    async fn hydrate(&mut self, node: NodeId) -> Result<Option<Pin>, Self::Error>;

    /// The key of each pinned element, in order (`probe::pinned_nodes_in_document` and the
    /// document token the world is proven to hold): `None` when one is gone, evicted or out of
    /// the world's document.
    async fn keys(&mut self, pins: &[Pin]) -> Result<Option<Vec<NodeKey>>, Self::Error>;
}

/// Where a chunk's query starts, as upstream's `querySelectorAll(selector, root)` takes it.
#[derive(Clone, Copy, Debug)]
pub(crate) enum QueryRoot {
    /// The frame's Document.
    Document,
    /// An element handle's element, by its node in the run's tree.
    Node(NodeId),
    /// An element handle's element the tree has no node for, such as a plain container the
    /// capture left out. Every stage runs in the page, which can query from it.
    Untracked,
}

/// One run of a plan: the attempt's tree and the readers all its stages share, the frame's
/// document, and where the chunk's query starts.
pub(crate) struct ChunkRun<'a> {
    tree: &'a SnapshotTree,
    /// One name pass per attempt, shared by every stage and candidate (design 3.3). It and
    /// `refs` are `Sync` because a run is borrowed across its page's awaits, and an op's future
    /// must be `Send` (`wire/route.rs`).
    text: &'a (dyn AccessibleText + Sync),
    /// The tab's ref table (engine chunk 5).
    refs: &'a (dyn RefResolver + Sync),
    /// The token every key of a pinned element of this frame must carry: a pin of another
    /// document means the world no longer holds the document this attempt captured.
    token: &'a DocToken,
    /// The frame document's root node, which stands for the Document: the scope candidates are
    /// evaluated over, and the node a `Document` match joins as.
    doc_root: NodeId,
    /// The query root's node, when the tree has one.
    root: Option<NodeId>,
}

/// How a run ended. The two messages carry page text (a strict violation lists previews), so
/// `Debug` prints their lengths.
pub(crate) enum ChunkOutcome {
    /// The chunk's matches, at most one under [`Strictness::Strict`].
    Matched(Matched),
    /// A strict locator matched more than one element: upstream's message, which ends the op as
    /// a `ProbeReply::Strict` does.
    Strict(String),
    /// The chunk threw, in a tree stage or in the page: what the equivalent `ProbeReply::Error`
    /// carries, so the op words it as `resolver::refused` does.
    Thrown { message: String, class: ErrorClass },
    /// A member left the page since the capture: the attempt starts over with a new one.
    Stale,
}

impl fmt::Debug for ChunkOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Matched(matched) => f.debug_tuple("Matched").field(matched).finish(),
            Self::Strict(message) => f
                .debug_struct("Strict")
                .field("message_len", &message.len())
                .finish(),
            Self::Thrown { message, class } => f
                .debug_struct("Thrown")
                .field("message_len", &message.len())
                .field("class", class)
                .finish(),
            Self::Stale => f.write_str("Stale"),
        }
    }
}

/// What a chunk matched, and the first stale ref any of its stages reported. Nothing matched
/// and a stale ref is `element_detached` at once, as a `ProbeReply::None` with a `stale_ref` is.
#[derive(Debug)]
pub(crate) struct Matched {
    pub(crate) set: ChunkSet,
    pub(crate) stale_ref: Option<String>,
}

/// The set a run ends on.
#[derive(Debug)]
pub(crate) enum ChunkSet {
    /// The last stage ran over the tree: its matches, nothing hydrated. `count()` reads its
    /// length, and an op that needs elements hydrates what it uses.
    Keys(TreeMatches),
    /// The last stage ran in the page: its matches in upstream's order, every element pinned in
    /// one entry.
    Pins(Vec<StageMatch>),
}

/// The first element of a set as a pin: the iframe a frame hop enters, or the element an action
/// takes (design 5.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum First {
    /// Nothing matched.
    Missing,
    /// The first match, pinned in the frame's world.
    Pin(Pin),
    /// The first match is the Document, which `nth` or `internal:or` kept: no element to pin.
    Document,
    /// The first match left the page since the capture.
    Stale,
    /// What pinning never answers, worded for an `internal` failure, as [`Pinned::Broken`].
    Broken(String),
}

/// A final set as the op's request names it ([`ChunkRun::pinned`]).
#[derive(Debug)]
pub(crate) enum Pinned {
    /// Every member pinned in the frame's world, in the set's order.
    Members(Vec<StageMatch>),
    /// A member left the page since the capture: the attempt starts over.
    Stale,
    /// What pinning never answers, worded for an `internal` failure.
    Broken(String),
}

/// The set a stage starts from.
enum Current {
    /// No stage ran yet: the chunk's query root.
    Root,
    /// What the stage before matched.
    Set(ChunkSet),
}

/// Where a page call starts, as its `from`.
enum Start {
    /// From the query root: no `from`.
    Root,
    /// From tree matches, which the call hands over.
    Nodes(Vec<NodeId>),
    /// From a page stage's matches, already pinned.
    Pins(Vec<StageMatch>),
}

impl Start {
    fn of(current: Current) -> Self {
        match current {
            Current::Root => Self::Root,
            Current::Set(ChunkSet::Keys(matches)) => Self::Nodes(matches.nodes().to_vec()),
            Current::Set(ChunkSet::Pins(matches)) => Self::Pins(matches),
        }
    }
}

/// The roots of a Pins set, joined to the tree (path 1).
enum Join {
    /// Every root is a node of this document: path 2.
    Nodes(Vec<NodeId>),
    /// A root the tree has no node for: path 3.
    Untracked,
    /// A root left the page, or the world holds another document.
    Stale,
    /// The seam answered fewer keys than it was asked for.
    Short,
}

/// One candidate of a page call, evaluated over the whole document.
enum Candidates {
    /// The leaf's matches.
    Nodes(Vec<NodeId>),
    /// The leaf's error, which the page throws once a root reaches the part.
    Thrown(TreeStageError),
    /// An `aria-ref` that names no live node.
    StaleRef(String),
}

/// What one `stage` call is made of.
struct Call<'c> {
    form: &'c PageForm,
    start: Start,
    candidates: &'c [Candidates],
    strict: Option<&'c select::ParsedSelector>,
}

/// The tree nodes one call hands over, in first-use order, each once: call node `i` is
/// `nodes[i]`. Every one has a DOM node for the seam to resolve.
#[derive(Default)]
struct Handover {
    nodes: Vec<NodeId>,
    index: HashMap<NodeId, u32>,
}

impl Handover {
    /// `node` as a member of the call: the only place a tree node becomes a [`SetMember`].
    fn member(&mut self, tree: &SnapshotTree, node: NodeId, doc_root: NodeId) -> SetMember {
        if node == doc_root {
            return SetMember::Document;
        }
        match tree.option_slot(node) {
            // The option has no DOM node to resolve: its select goes over in its place, once
            // however many of its options the call names.
            Some(OptionSlot {
                select,
                index,
                count,
            }) => SetMember::Option {
                select: self.call_node(select),
                index,
                count,
            },
            None => SetMember::Node(self.call_node(node)),
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "the nodes are distinct nodes of one tree, whose arena counts them in u32"
    )]
    fn call_node(&mut self, node: NodeId) -> u32 {
        let next = self.nodes.len() as u32;
        *self.index.entry(node).or_insert_with(|| {
            self.nodes.push(node);
            next
        })
    }
}

impl<'a> ChunkRun<'a> {
    /// A run over `tree` in the frame whose document is `doc`, from `root`. `None` when the
    /// capture holds no node of that document: nothing in it can be evaluated, and the attempt
    /// starts over.
    pub(crate) fn new(
        tree: &'a SnapshotTree,
        text: &'a (dyn AccessibleText + Sync),
        refs: &'a (dyn RefResolver + Sync),
        doc: DocId,
        root: QueryRoot,
    ) -> Option<Self> {
        let document = tree.document(doc)?;
        let doc_root = document.root_node()?;
        let root = match root {
            QueryRoot::Document => Some(doc_root),
            QueryRoot::Node(node) => Some(node),
            QueryRoot::Untracked => None,
        };
        Some(Self {
            tree,
            text,
            refs,
            token: document.token(),
            doc_root,
            root,
        })
    }

    /// Runs `plan` through its stages. Under [`Strictness::Strict`] a final set of more than one
    /// element ends in [`ChunkOutcome::Strict`].
    pub(crate) async fn run<P: ChunkPage>(
        &self,
        plan: &ChunkPlan,
        strictness: Strictness,
        page: &mut P,
    ) -> Result<ChunkOutcome, P::Error> {
        let stages = match plan.stages() {
            Ok(stages) => stages,
            Err(error) => return Ok(thrown(&error)),
        };
        let strict = match strictness {
            Strictness::Strict => Some(plan.chunk()),
            Strictness::FirstMatch => None,
        };
        // Only the call that ends the chunk counts its matches against strict mode.
        let last = stages.rest().len();
        let strict_at = |position: usize| strict.filter(|_| position == last);
        let mut stale_ref = None;
        let mut set = match self
            .step(
                stages.first(),
                Current::Root,
                strict_at(0),
                &mut stale_ref,
                page,
            )
            .await?
        {
            ControlFlow::Continue(set) => set,
            ControlFlow::Break(outcome) => return Ok(outcome),
        };
        for (index, stage) in stages.rest().iter().enumerate() {
            let strict = strict_at(index + 1);
            set = match self
                .step(stage, Current::Set(set), strict, &mut stale_ref, page)
                .await?
            {
                ControlFlow::Continue(set) => set,
                ControlFlow::Break(outcome) => return Ok(outcome),
            };
        }
        if let (Some(chunk), ChunkSet::Keys(matches)) = (strict, &set)
            && let Err(violation) = matches.strict()
        {
            return self.violation(chunk, &violation, page).await;
        }
        Ok(ChunkOutcome::Matched(Matched { set, stale_ref }))
    }

    /// The first element of `set` as a pin. A tree set hydrates that one key and no other; an
    /// option slot, which has no key, is pinned through one `stage` ([`Self::pin_nodes`]).
    pub(crate) async fn first<P: ChunkPage>(
        &self,
        set: &ChunkSet,
        page: &mut P,
    ) -> Result<First, P::Error> {
        let node = match set {
            ChunkSet::Pins(matches) => {
                return Ok(match matches.first() {
                    None => First::Missing,
                    Some(StageMatch::Pin(pin)) => First::Pin(*pin),
                    Some(StageMatch::Document) => First::Document,
                });
            }
            ChunkSet::Keys(matches) => match matches.nodes().first() {
                None => return Ok(First::Missing),
                Some(&node) => node,
            },
        };
        if node == self.doc_root {
            return Ok(First::Document);
        }
        if self.tree.option_slot(node).is_none() {
            return Ok(page.hydrate(node).await?.map_or(First::Stale, First::Pin));
        }
        Ok(match self.pin_nodes(&[node], page).await? {
            Pinned::Members(matches) => match matches.as_slice() {
                [StageMatch::Pin(pin)] => First::Pin(*pin),
                _ => First::Broken(format!(
                    "pinning one option answered {} members, not its element",
                    matches.len()
                )),
            },
            Pinned::Stale => First::Stale,
            Pinned::Broken(why) => First::Broken(why),
        })
    }

    /// Every element of `set` pinned in the frame's world, in the set's order, for the op's own
    /// request to name (`ProbeTarget::Set`). A page stage's matches are pinned already; a tree
    /// set is pinned by [`Self::pin_nodes`].
    pub(crate) async fn pinned<P: ChunkPage>(
        &self,
        set: ChunkSet,
        page: &mut P,
    ) -> Result<Pinned, P::Error> {
        match set {
            ChunkSet::Pins(matches) => Ok(Pinned::Members(matches)),
            ChunkSet::Keys(matches) if matches.is_empty() => Ok(Pinned::Members(Vec::new())),
            ChunkSet::Keys(matches) => self.pin_nodes(matches.nodes(), page).await,
        }
    }

    /// `nodes` pinned in the frame's world, in order: handed over in one `stage` call that
    /// starts from them and has no parts of its own, so they come back exactly as they went.
    /// The only way to pin an option slot, which has no DOM node to hydrate.
    async fn pin_nodes<P: ChunkPage>(
        &self,
        nodes: &[NodeId],
        page: &mut P,
    ) -> Result<Pinned, P::Error> {
        let mut handover = Handover::default();
        let from: Vec<SetMember> = nodes
            .iter()
            .map(|&node| handover.member(self.tree, node, self.doc_root))
            .collect();
        let selector = match probe::ParsedSelector::no_parts() {
            Ok(selector) => selector,
            Err(error) => {
                return Ok(Pinned::Broken(format!(
                    "a pin stage did not encode: {error}"
                )));
            }
        };
        let request = ProbeRequest::Stage {
            selector,
            from: Some(from),
            sets: Vec::new(),
            strict: None,
        };
        Ok(match page.call(&request, &handover.nodes).await? {
            Some(ProbeReply::Staged { matches, .. }) => Pinned::Members(matches),
            Some(ProbeReply::Detached) | None => Pinned::Stale,
            Some(other) => Pinned::Broken(format!(
                "the probe answered `{}` to pinning a tree stage's matches",
                other.kind()
            )),
        })
    }

    /// One stage from `current`, by the transfer table.
    async fn step<P: ChunkPage>(
        &self,
        stage: &PlannedStage,
        current: Current,
        strict: Option<&select::ParsedSelector>,
        stale_ref: &mut Option<String>,
        page: &mut P,
    ) -> Result<ControlFlow<ChunkOutcome, ChunkSet>, P::Error> {
        let run = match stage {
            PlannedStage::Page(form) => {
                return self
                    .page_stage(form, Start::of(current), strict, stale_ref, page)
                    .await;
            }
            PlannedStage::Tree(run) => run,
        };
        // The tree has no node for the query root, so `internal:and` and `internal:or`, which
        // evaluate from it, could not run over the tree: the page form can.
        let Some(root) = self.root else {
            return self
                .page_stage(run.page_form(), Start::of(current), strict, stale_ref, page)
                .await;
        };
        let roots = match current {
            Current::Root => None,
            Current::Set(ChunkSet::Keys(matches)) => Some(matches.nodes().to_vec()),
            Current::Set(ChunkSet::Pins(matches)) => match self.join(&matches, page).await? {
                Join::Nodes(nodes) => Some(nodes),
                Join::Untracked => {
                    return self
                        .page_stage(
                            run.page_form(),
                            Start::Pins(matches),
                            strict,
                            stale_ref,
                            page,
                        )
                        .await;
                }
                Join::Stale => return Ok(ControlFlow::Break(ChunkOutcome::Stale)),
                Join::Short => {
                    return Ok(ControlFlow::Break(internal(
                        "the page answered fewer keys than it was asked for".to_owned(),
                    )));
                }
            },
        };
        let scope = match &roots {
            None => Scope::Root(root),
            Some(roots) => Scope::After { root, roots },
        };
        Ok(
            match run.stage().select(self.tree, scope, self.text, self.refs) {
                Ok(matches) => {
                    if stale_ref.is_none() {
                        *stale_ref = matches.stale_ref().map(str::to_owned);
                    }
                    ControlFlow::Continue(ChunkSet::Keys(matches))
                }
                Err(error) => ControlFlow::Break(thrown(&error)),
            },
        )
    }

    /// Path 1: each root of `matches` as a node of this document. The Document joins as the
    /// document's root node, with no call.
    async fn join<P: ChunkPage>(
        &self,
        matches: &[StageMatch],
        page: &mut P,
    ) -> Result<Join, P::Error> {
        let pins: Vec<Pin> = matches
            .iter()
            .filter_map(|member| match member {
                StageMatch::Pin(pin) => Some(*pin),
                StageMatch::Document => None,
            })
            .collect();
        let keys: HashMap<Pin, NodeKey> = if pins.is_empty() {
            HashMap::new()
        } else {
            match page.keys(&pins).await? {
                Some(keys) => pins.iter().copied().zip(keys).collect(),
                None => return Ok(Join::Stale),
            }
        };
        let mut nodes = Vec::with_capacity(matches.len());
        for member in matches {
            let StageMatch::Pin(pin) = member else {
                nodes.push(self.doc_root);
                continue;
            };
            let Some(key) = keys.get(pin) else {
                return Ok(Join::Short);
            };
            if key.doc != *self.token {
                return Ok(Join::Stale);
            }
            match self.tree.node_by_key(key) {
                Some(node) => nodes.push(node.id()),
                None => return Ok(Join::Untracked),
            }
        }
        Ok(Join::Nodes(nodes))
    }

    /// One `stage` call: `form`'s candidates over the whole document, then the call.
    async fn page_stage<P: ChunkPage>(
        &self,
        form: &PageForm,
        start: Start,
        strict: Option<&select::ParsedSelector>,
        stale_ref: &mut Option<String>,
        page: &mut P,
    ) -> Result<ControlFlow<ChunkOutcome, ChunkSet>, P::Error> {
        let candidates: Vec<Candidates> = form
            .candidates()
            .iter()
            .map(|candidate| self.candidates(candidate))
            .collect();
        let call = Call {
            form,
            start,
            candidates: &candidates,
            strict,
        };
        let (request, nodes) = match self.request(call) {
            Ok(prepared) => prepared,
            Err(error) => return Ok(ControlFlow::Break(unencodable(&error))),
        };
        let Some(reply) = page.call(&request, &nodes).await? else {
            return Ok(ControlFlow::Break(ChunkOutcome::Stale));
        };
        let reply = match reply {
            ProbeReply::Strict {
                message,
                listing: Some(listing),
            } => ProbeReply::Strict {
                message: self.worded(message, listing, page).await?,
                listing: None,
            },
            reply => reply,
        };
        Ok(staged(reply, stale_ref))
    }

    /// A violation a page stage decided, with the tree's role locators in its `aka` lines where
    /// they win (`strict_akas`): the listed pins joined to this tree's nodes, as a join does.
    /// The page's message as it stands when no line improves or a pin no longer joins.
    async fn worded<P: ChunkPage>(
        &self,
        message: String,
        listing: StrictListing,
        page: &mut P,
    ) -> Result<String, P::Error> {
        let Some(keys) = page.keys(&listing.pins).await? else {
            return Ok(message);
        };
        let nodes = keys
            .iter()
            .map(|key| self.tree.node_by_key(key).map(TreeNode::id));
        let count = usize::try_from(listing.count).unwrap_or(usize::MAX);
        Ok(
            strict_akas::with_roles(self.tree, self.text, self.refs, &listing.items, nodes)
                .map_or(message, |lines| {
                    select::strict_message(&listing.locator, count, &lines)
                }),
        )
    }

    /// The `stage` request for `call`, and the tree nodes it hands over: its start's and its
    /// candidates', each once.
    fn request(&self, call: Call<'_>) -> Result<(ProbeRequest, Vec<NodeId>), serde_json::Error> {
        let mut handover = Handover::default();
        let from: Option<Vec<SetMember>> = match call.start {
            Start::Root => None,
            Start::Nodes(nodes) => Some(
                nodes
                    .into_iter()
                    .map(|node| handover.member(self.tree, node, self.doc_root))
                    .collect(),
            ),
            Start::Pins(matches) => Some(
                matches
                    .into_iter()
                    .map(|member| match member {
                        StageMatch::Pin(pin) => SetMember::Pin(pin),
                        StageMatch::Document => SetMember::Document,
                    })
                    .collect(),
            ),
        };
        let sets: Vec<StageSet> = call
            .candidates
            .iter()
            .map(|candidate| match candidate {
                Candidates::Nodes(nodes) => StageSet::Members(
                    nodes
                        .iter()
                        .map(|&node| handover.member(self.tree, node, self.doc_root))
                        .collect(),
                ),
                Candidates::Thrown(error) => StageSet::Error {
                    message: error.to_string(),
                    class: class_of(error),
                },
                Candidates::StaleRef(body) => StageSet::StaleRef(body.clone()),
            })
            .collect();
        let request = ProbeRequest::Stage {
            selector: probe_selector(call.form.selector())?,
            from,
            sets,
            strict: call.strict.map(probe_selector).transpose()?,
        };
        Ok((request, handover.nodes))
    }

    /// A candidate over the frame's whole document: containment in each root is the page's.
    fn candidates(&self, candidate: &TreeStage) -> Candidates {
        match candidate.select(self.tree, Scope::Root(self.doc_root), self.text, self.refs) {
            Ok(matches) => match matches.stale_ref() {
                Some(stale) => Candidates::StaleRef(stale.to_owned()),
                None => Candidates::Nodes(matches.nodes().to_vec()),
            },
            Err(error) => Candidates::Thrown(error),
        }
    }

    /// A violation the tree decided: its previews handed over and described in one call, and
    /// upstream's message assembled from the page's words.
    async fn violation<P: ChunkPage>(
        &self,
        chunk: &select::ParsedSelector,
        violation: &StrictViolation,
        page: &mut P,
    ) -> Result<ChunkOutcome, P::Error> {
        let previews = violation.preview_nodes();
        if previews.contains(&self.doc_root) {
            // `describePins` words elements only, and upstream's own listing cannot word the
            // Document either: its selector generator takes an element.
            return Ok(internal(
                "a strict-mode violation lists the Document, which has no preview".to_owned(),
            ));
        }
        let mut handover = Handover::default();
        let members: Vec<SetMember> = previews
            .iter()
            .map(|&node| handover.member(self.tree, node, self.doc_root))
            .collect();
        let request = match probe_selector(chunk) {
            Ok(chunk) => ProbeRequest::DescribePins {
                members,
                chunk: Some(chunk),
            },
            Err(error) => return Ok(unencodable(&error)),
        };
        let Some(reply) = page.call(&request, &handover.nodes).await? else {
            return Ok(ChunkOutcome::Stale);
        };
        Ok(match reply {
            ProbeReply::PinsDescribed {
                locator: Some(locator),
                items,
            } if items.len() == previews.len() => {
                // The page's generator offers no role locator; the tree's goes in where it wins.
                let nodes = previews.iter().map(|&node| Some(node));
                let lines = strict_akas::with_roles(self.tree, self.text, self.refs, &items, nodes)
                    .unwrap_or_else(|| {
                        items
                            .into_iter()
                            .map(|PinPreview { preview, aka }| StrictPreview { preview, aka })
                            .collect()
                    });
                ChunkOutcome::Strict(violation.message(&locator, &lines))
            }
            ProbeReply::Detached => ChunkOutcome::Stale,
            ProbeReply::Error { message, class, .. } => ChunkOutcome::Thrown { message, class },
            other => internal(format!(
                "the probe answered `{}` to describing a strict-mode violation's previews",
                other.kind()
            )),
        })
    }
}

/// A `stage` reply as the run's next set, or how the run ends. A stale ref the page reached
/// counts only when no earlier stage of the run reported one.
fn staged(
    reply: ProbeReply,
    stale_ref: &mut Option<String>,
) -> ControlFlow<ChunkOutcome, ChunkSet> {
    ControlFlow::Break(match reply {
        ProbeReply::Staged {
            matches,
            stale_ref: reached,
        } => {
            if stale_ref.is_none() {
                *stale_ref = reached;
            }
            return ControlFlow::Continue(ChunkSet::Pins(matches));
        }
        ProbeReply::Strict { message, .. } => ChunkOutcome::Strict(message),
        ProbeReply::Detached => ChunkOutcome::Stale,
        ProbeReply::Error { message, class, .. } => ChunkOutcome::Thrown { message, class },
        other => internal(format!(
            "the probe answered `{}`, which a stage does not take",
            other.kind()
        )),
    })
}

/// A tree part's error as the probe would answer the same error thrown in the page.
fn thrown(error: &TreeStageError) -> ChunkOutcome {
    ChunkOutcome::Thrown {
        message: error.to_string(),
        class: class_of(error),
    }
}

/// The class the probe gives a tree part's error thrown in the page: the one decision both a
/// tree stage's error and a candidate's error set take their class from.
fn class_of(error: &TreeStageError) -> ErrorClass {
    match error {
        // `parseAttributeSelector` throws `InvalidSelectorError`, which the probe classifies
        // `selector` (its message stands alone). A template the script wrote wrong is the
        // selector's fault too, worded the same way.
        TreeStageError::Role(RoleQueryError::AttributeSelector(_))
        | TreeStageError::AriaTemplate(
            AriaTemplateError::Body { .. } | AriaTemplateError::Template(_),
        ) => ErrorClass::Selector,
        // `createStacklessError` and the role engine's validation throw a plain `Error`, which
        // the probe classifies `script` (`Error: ` before its message).
        TreeStageError::Role(_) | TreeStageError::NthWithCapture => ErrorClass::Script,
        // A template Neo cannot match where the script asked: Neo's limit, not the script's.
        TreeStageError::AriaTemplate(
            AriaTemplateError::TooDeep(_) | AriaTemplateError::OutsideTree,
        ) => ErrorClass::Unsupported,
    }
}

fn internal(message: String) -> ChunkOutcome {
    ChunkOutcome::Thrown {
        message,
        class: ErrorClass::Internal,
    }
}

fn unencodable(error: &serde_json::Error) -> ChunkOutcome {
    internal(format!("a chunk's page form did not encode: {error}"))
}

/// The probe's opaque `ParsedSelector` of a typed one: the same JSON, which the page reads with
/// the vendored parser's types.
fn probe_selector(
    selector: &select::ParsedSelector,
) -> Result<probe::ParsedSelector, serde_json::Error> {
    serde_json::to_value(selector).and_then(serde_json::from_value)
}
