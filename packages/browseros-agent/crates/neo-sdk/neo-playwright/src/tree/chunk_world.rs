//! The page side of one [`ChunkRun`] over one attempt's capture: the frame's isolated world,
//! reached through `OpPage` and `tree::hydrate`'s `resolve_node` and `pin_object`.
//!
//! Design 6.2's proof rule holds here for every node the run hands over: a key is hydrated only
//! through a world installed before the capture was requested and still alive when the
//! hydration returns. [`ChunkWorld::start`] checks the first half once, with the document the
//! capture places in the world's frame; each call's success is the second half, since V8 refuses
//! a node of another world and a world dies with its document.
//!
//! ```text
//!  start      world.installed_at < capture.requested_at ─ else WorldAfterCapture
//!             doc is the world's frame's document in the capture ─ else DocumentNotInFrame
//!
//!  call       each node ─ DOM.resolveNode in the world, RESOLVES_IN_FLIGHT at once, order kept
//!               a node gone ─▶ None, nothing sent
//!             probe::call_with_nodes(root, objects) ─▶ the reply
//!  hydrate    DOM.resolveNode ─ probe resolve(Root) ─ pin, if in the world's document
//!               ─ bind_proven_world ─▶ the pin
//!  keys       probe::pinned_nodes_in_document ─ backend ids, with the run's DocToken
//!               ─ bind_proven_world
//!
//!  any of them failing Gone ─ TabTree::invalidate_world, then the error
//! ```

use futures_util::{StreamExt as _, stream};
use neo_snapshot_apc::tree::{
    DocId, DocToken, DomNodeId, NameCache, NodeId, NodeKey, SnapshotTree,
};

use super::hydrate::{self, ObjectPin};
use super::private_mirror;
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest};
use crate::resolve::{ChunkPage, ChunkRun, QueryRoot};
use crate::tree::{CapturedTree, TabTree};
use crate::world::{IsolatedWorld, Pin};

/// `DOM.resolveNode` calls in flight for one call's nodes, as clickable marking resolved its
/// closed-root seeds before the switch-over (999 such resolves added 52 ms). Kept in
/// order here, because call node `i` is `nodes[i]`, so one slow node holds back those after it.
const RESOLVES_IN_FLIGHT: usize = 8;

/// What every frame's run in one locator attempt shares.
pub(crate) struct RunAttempt<'a> {
    pub(crate) scope: &'a OpScope<'a>,
    pub(crate) page: &'a OpPage<'a>,
    /// The tab's tree state, which is also the runs' `RefResolver`.
    pub(crate) tab: &'a TabTree,
    pub(crate) capture: &'a CapturedTree,
    /// The attempt's one name pass over `capture` (design 3.3).
    pub(crate) names: &'a NameCache<'a>,
}

/// Where a chunk's query starts, for both sides of a run.
pub(crate) enum RunRoot {
    Document,
    /// An element handle: its node resolved in the run's world, and its tree node when the
    /// capture has one (`None` is `QueryRoot::Untracked`).
    Handle {
        object: String,
        node: Option<NodeId>,
    },
}

/// Why a world cannot serve a run over this capture; either way the attempt captures again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unproven {
    /// Installed at or after the capture request (design 6.2 case 3).
    WorldAfterCapture,
    /// `doc` is not the world's frame's document in this capture, has no root node, or the
    /// handle's node is in another document (case 4).
    DocumentNotInFrame,
}

/// What [`ChunkWorld::start`] hands back: a run and its page side, made from one document and
/// one root so the two cannot disagree.
pub(crate) struct StartedRun<'a> {
    pub(crate) run: ChunkRun<'a>,
    pub(crate) page: ChunkWorld<'a>,
}

/// The page side of one [`ChunkRun`]: the frame's isolated world, which [`Self::start`] checked
/// against the capture the run reads.
pub(crate) struct ChunkWorld<'a> {
    scope: &'a OpScope<'a>,
    page: &'a OpPage<'a>,
    /// Retires the world's documents and their refs together when a call finds the world gone.
    tab: &'a TabTree,
    tree: &'a SnapshotTree,
    world: IsolatedWorld,
    /// The run's document: every node a call hands over must be one of its nodes.
    doc: DocId,
    /// `doc`'s token, which every key [`ChunkPage::keys`] answers carries, and which `DocTable`
    /// binds to this world once a call proves it.
    token: &'a DocToken,
    /// Every request's call root: an element handle's node in `world`, or none for the Document.
    root: Option<String>,
}

impl<'a> ChunkWorld<'a> {
    /// The run over `doc` from `root`, and its page side in `world`, once `world` is proven to
    /// serve this capture.
    pub(crate) fn start(
        attempt: &RunAttempt<'a>,
        world: IsolatedWorld,
        doc: DocId,
        root: RunRoot,
    ) -> Result<StartedRun<'a>, Unproven> {
        let tree = attempt.capture.tree();
        if world.installed_at() >= attempt.capture.requested_at() {
            return Err(Unproven::WorldAfterCapture);
        }
        let document = tree.document(doc).ok_or(Unproven::DocumentNotInFrame)?;
        let main_frame = attempt.page.main_endpoint().frame;
        if !hydrate::doc_matches_frame(tree, &main_frame, document.token(), world.frame()) {
            return Err(Unproven::DocumentNotInFrame);
        }
        let (query_root, call_root) = match root {
            RunRoot::Document => (QueryRoot::Document, None),
            RunRoot::Handle {
                object,
                node: Some(node),
            } => {
                if tree.node(node).is_none_or(|node| node.doc() != doc) {
                    return Err(Unproven::DocumentNotInFrame);
                }
                (QueryRoot::Node(node), Some(object))
            }
            RunRoot::Handle { object, node: None } => (QueryRoot::Untracked, Some(object)),
        };
        let run = ChunkRun::new(tree, attempt.names, attempt.tab, doc, query_root)
            .ok_or(Unproven::DocumentNotInFrame)?;
        let page = Self {
            scope: attempt.scope,
            page: attempt.page,
            tab: attempt.tab,
            tree,
            world,
            doc,
            token: document.token(),
            root: call_root,
        };
        Ok(StartedRun { run, page })
    }

    /// `node`'s DOM node id. The run hands over only nodes of its own document's tree, each an
    /// element APC names by id, so any other node is a bug in the run, not a page that changed:
    /// answering `None` for it would make the run go stale, and the op retry until its deadline
    /// and then time out with the wrong cause.
    fn dom_node(&self, node: NodeId) -> Result<DomNodeId, ProbeFailure> {
        let node = self
            .tree
            .node(node)
            .filter(|node| node.doc() == self.doc)
            .ok_or_else(|| {
                ProbeFailure::Broken(
                    "a chunk run handed its frame's world a node of another document".to_owned(),
                )
            })?;
        node.dom_node_id().ok_or_else(|| {
            ProbeFailure::Broken(
                "a chunk run handed its frame's world a node with no DOM node id".to_owned(),
            )
        })
    }

    /// Each of `nodes` as a remote object in the world, in `nodes` order: call node `i` is
    /// `nodes[i]` (`SetMember::Node`). `None` at the first node that no longer resolves, which
    /// drops the resolves still in flight.
    async fn resolve_all(&self, nodes: &[NodeId]) -> Result<Option<Vec<String>>, ProbeFailure> {
        let doms: Vec<DomNodeId> = nodes
            .iter()
            .map(|&node| self.dom_node(node))
            .collect::<Result<_, _>>()?;
        let mut resolved = stream::iter(doms)
            .map(|dom| hydrate::resolve_node(self.scope, &self.world, dom))
            .buffered(RESOLVES_IN_FLIGHT);
        let mut objects = Vec::with_capacity(nodes.len());
        while let Some(object) = resolved.next().await {
            match object? {
                Some(object) => objects.push(object),
                None => return Ok(None),
            }
        }
        Ok(Some(objects))
    }

    async fn send(
        &self,
        request: &ProbeRequest,
        nodes: &[NodeId],
    ) -> Result<Option<ProbeReply>, ProbeFailure> {
        let Some(objects) = self.resolve_all(nodes).await? else {
            return Ok(None);
        };
        let root = self.root.as_deref();
        probe::call_with_nodes(self.scope, self.page, &self.world, request, root, &objects)
            .await
            .map(Some)
    }

    async fn pin(&self, node: NodeId) -> Result<Option<Pin>, ProbeFailure> {
        let dom = self.dom_node(node)?;
        let Some(object) = hydrate::resolve_node(self.scope, &self.world, dom).await? else {
            return Ok(None);
        };
        match hydrate::pin_object(self.scope, self.page, &self.world, &object).await? {
            ObjectPin::Pinned { pin, .. } => Ok(self.bind_world().then_some(pin)),
            ObjectPin::Detached => Ok(None),
            ObjectPin::Refused(reply) => Err(ProbeFailure::Broken(format!(
                "the probe answered `{}` to pinning a hydrated node",
                reply.kind()
            ))),
        }
    }

    async fn keys_of(&self, pins: &[Pin]) -> Result<Option<Vec<NodeKey>>, ProbeFailure> {
        // Only pins in the world's document: the probe still holds an element that left the
        // page or moved into another document, and a page stage's match gone before the join
        // must not join as its node in the capture, whose subtree the next tree stage would
        // then search.
        let Some(backend_ids) =
            probe::pinned_nodes_in_document(self.scope, self.page, &self.world, pins.to_vec())
                .await?
        else {
            return Ok(None);
        };
        // The world holds the run's document (`start`, then this call's success), so every pin
        // the batch answers for is an element of that document.
        let keys: Vec<NodeKey> = backend_ids
            .into_iter()
            .map(|id| {
                private_mirror::backend_to_dom_node_id(id)
                    .map(|node| NodeKey::new(self.token.clone(), node))
            })
            .collect::<Result<_, _>>()?;
        Ok(self.bind_world().then_some(keys))
    }

    /// Records the proof a call just made in `DocTable`: this world holds the run's document.
    /// `false` when `DocTable` refuses, because the document retired, or a capture requested
    /// after the world's install saw the frame hold another one, which makes this run stale.
    fn bind_world(&self) -> bool {
        self.page
            .frames()
            .bind_proven_world(self.token, &self.world)
            .is_some()
    }

    /// Design 6.2 case 2: a world gone took its document with it. Invalidating it retires every
    /// document `DocTable` bound to the install and their refs in one step, before the run
    /// returns the error for the op's liveness rule; it is idempotent, so the op's own
    /// invalidation after this retires nothing twice.
    fn retire_if_gone<T>(&self, outcome: Result<T, ProbeFailure>) -> Result<T, ProbeFailure> {
        if let Err(ProbeFailure::Gone(_)) = &outcome {
            self.tab.invalidate_world(self.page.frames(), &self.world);
        }
        outcome
    }
}

impl ChunkPage for ChunkWorld<'_> {
    type Error = ProbeFailure;

    async fn call(
        &mut self,
        request: &ProbeRequest,
        nodes: &[NodeId],
    ) -> Result<Option<ProbeReply>, ProbeFailure> {
        let outcome = self.send(request, nodes).await;
        self.retire_if_gone(outcome)
    }

    async fn hydrate(&mut self, node: NodeId) -> Result<Option<Pin>, ProbeFailure> {
        let outcome = self.pin(node).await;
        self.retire_if_gone(outcome)
    }

    async fn keys(&mut self, pins: &[Pin]) -> Result<Option<Vec<NodeKey>>, ProbeFailure> {
        let outcome = self.keys_of(pins).await;
        self.retire_if_gone(outcome)
    }
}

#[cfg(test)]
mod tests;
