//! Safe hydration of APC [`NodeKey`]s and [`AriaRefJump`] refs into live, proven isolated worlds
//! (design sections 6.2 and 6.4).
//!
//! A renderer `DomNodeId` (`backendNodeId`) is a per-process counter that restarts on cross-process
//! navigation while the main frame keeps its CDP session. Hydrating a key into an unproven world
//! could therefore pin an unrelated DOM node in the new document. [`Hydrate`] enforces section
//! 6.2's proof rule on every hydration:
//!
//! ```text
//!  Key ─▶ Case 4: doc_matches_frame(capture, key.doc, frame)?
//!           no  ─▶ retire absent doc; return StaleDocument (zero DOM.resolveNode)
//!           yes ─▶ Case 1: cached world with installed_at < capture.requested_at?
//!                    yes ─▶ DOM.resolveNode + probe resolve(Root) + in-document check
//!                             Pinned       ─▶ bind_proven_world ─▶ Hydrated
//!                             WorldGone    ─▶ Case 2: invalidate_world ─▶ WorldStale
//!                             NodeDetached ─▶ NodeDetached (world stays live)
//!                    no  ─▶ Case 3: ensure_world ─▶ fresh capture ─▶ re-check Case 4 ─▶ pin
//!
//!  Ref ─▶ TabRefTable::resolve
//!           Key + DocLookup::Bound    ─▶ zero-capture pin_in_world
//!           Key + DocLookup::Reserved ─▶ Case 3 once (ensure_world + recapture + Case 4 + pin)
//!           Retired / foreign seq     ─▶ ElementDetached (zero captures)
//!           Unknown                   ─▶ InvalidInput (zero captures)
//! ```

use std::fmt;
use std::num::NonZeroU32;

use browseros_cdp::CdpError;
use browseros_core::{CoreError, FrameId};
use neo_snapshot_apc::refs::{Ref, RefKind, RefLookup, RefTarget};
use neo_snapshot_apc::tree::{
    DocId, DocSeq, DocToken, DomNodeId, NodeKey, SnapshotTree, TreeDoc, TreeNode,
};
use serde::Deserialize;
use serde_json::json;

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::page::frame_tree::UnreadCause;
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget, Want};
use crate::resolve::{self, FramePath, FrameStart, Started};
use crate::target::{AriaRefJump, AriaRefKind, Strictness};
#[cfg(test)]
use crate::tree::CapturedTree;
use crate::tree::{AttemptStart, TabTree};
use crate::world::{DocLookup, FrameSeq, IsolatedWorld, Pin, Place, SeqLookup};
use crate::{ErrorCode, PlaywrightError};

/// Where a ref jump landed (design 6.4).
pub(crate) enum RefLanding {
    /// The ref's element, pinned in its document's proven world.
    Node(HydratedNode),
    /// The ref's document is live in its proven `world`, but the element left it: the stale ref
    /// the page's own ref engine answered as `none` with `staleRef`, which each op still decides
    /// on (a read of every match counts nothing, a wait for `detached` is met, any other op
    /// fails `element_detached`).
    Left {
        world: IsolatedWorld,
        path: FramePath,
    },
}

/// An APC [`NodeKey`] hydrated into a proven isolated world and pinned element handle.
pub(crate) struct HydratedNode {
    /// Proven isolated world hosting [`Self::key`]'s document.
    pub(crate) world: IsolatedWorld,
    /// Ancestor iframe hops from the main frame down to this node's frame.
    pub(crate) path: FramePath,
    /// Probe pin identifying the live element inside [`Self::world`].
    pub(crate) pin: Pin,
    /// Playwright preview string produced by the probe for diagnostics.
    pub(crate) preview: String,
    /// Remote CDP object ID in the current operation's object group.
    pub(crate) object_id: String,
    /// Composite APC key of the hydrated DOM node.
    pub(crate) key: NodeKey,
    /// Permanent document sequence number bound in `DocTable`.
    pub(crate) seq: FrameSeq,
}

/// Structure only: the preview is page text and attributes, so only its length prints.
impl fmt::Debug for HydratedNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HydratedNode")
            .field("world", &self.world)
            .field("path", &self.path)
            .field("pin", &self.pin)
            .field("preview_len", &self.preview.len())
            .field("object_id", &self.object_id)
            .field("key", &self.key)
            .field("seq", &self.seq)
            .finish()
    }
}

/// Result of hydrating one [`NodeKey`] against a captured [`SnapshotTree`] (design section 6.2).
#[derive(Debug)]
pub(crate) enum HydrateOutcome {
    /// The node was resolved and pinned in a world proven to host `key.doc`.
    Hydrated(Box<HydratedNode>),
    /// A pre-capture world went `Gone` (Case 2) and was invalidated; callers retry on a new world.
    WorldStale,
    /// `key.doc` is not `frame`'s document in the capture (Case 4); the key is stale.
    StaleDocument,
    /// The document is live, but the DOM node was removed or detached from it.
    NodeDetached,
}

/// `DOM.resolveNode`'s reply subset carrying only the wrapped remote object ID.
#[derive(Deserialize)]
struct ResolvedNode {
    object: NodeObject,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeObject {
    object_id: String,
}

enum PinAttempt {
    Pinned {
        pin: Pin,
        preview: String,
        object_id: String,
    },
    WorldGone,
    NodeDetached,
}

/// What the probe answered to pinning a node by its handle ([`pin_object`]).
pub(super) enum ObjectPin {
    /// The node's element, pinned in the world, and the probe's preview of it (page text).
    Pinned { pin: Pin, preview: String },
    /// The node is detached, or not of the world's document.
    Detached,
    /// A reply pinning never gives, for the caller to word.
    Refused(ProbeReply),
}

/// Whether a frame's world could be installed: [`ensure_world`]'s answer.
pub(super) enum EnsuredWorld {
    Ready(IsolatedWorld),
    /// A world on the way went away under the install; another try installs afresh.
    Gone,
    /// The frame is not in the page any more.
    Missing,
    /// The page's frame tree could not be read down to the frame ([`Started::Unread`]): it may
    /// still be there, so nothing about it is proven gone.
    Unread(UnreadCause),
}

/// Hydrates APC [`NodeKey`]s and [`AriaRefJump`] refs into live isolated worlds under section 6.2
/// and 6.4 invariants.
pub(crate) struct Hydrate<'a> {
    page: &'a OpPage<'a>,
    scope: &'a OpScope<'a>,
    tab_tree: &'a TabTree,
}

impl<'a> Hydrate<'a> {
    /// Creates a hydrator scoped to one operation's admitted `page`, `scope`, and per-tab
    /// [`TabTree`].
    #[must_use]
    pub(crate) const fn new(
        page: &'a OpPage<'a>,
        scope: &'a OpScope<'a>,
        tab_tree: &'a TabTree,
    ) -> Self {
        Self {
            page,
            scope,
            tab_tree,
        }
    }

    /// Hydrates `key` (expected in `frame`) against `capture`, applying all four cases of design
    /// section 6.2 so a backend node ID is never resolved into an unproven or replaced document.
    #[cfg(test)]
    pub(crate) async fn hydrate_key(
        &self,
        capture: &CapturedTree,
        key: &NodeKey,
        frame: &FrameId,
    ) -> Result<HydrateOutcome, PlaywrightError> {
        let main_frame = self.page.main_endpoint().frame;

        // Case 4: refuse before touching CDP if the capture does not place `key.doc` in `frame`.
        if !doc_matches_frame(capture.tree(), &main_frame, &key.doc, frame) {
            if capture.tree().document_by_token(&key.doc).is_none() {
                let _ = self.tab_tree.retire_doc(self.page.frames(), &key.doc);
            }
            return Ok(HydrateOutcome::StaleDocument);
        }

        // Case 1: a cached world installed strictly before `capture` was dispatched proves document
        // identity as soon as pinning in that world succeeds (Case 2 if it went Gone).
        if let Some(world) = self.page.frames().cached_world_for_frame(frame)
            && world.installed_at() < capture.requested_at()
        {
            return self.pin_and_bind(world, key).await;
        }

        // Case 3: no pre-capture world exists (or the cached world was installed at/after capture
        // dispatch). Ensure a world first, take one fresh capture after install so
        // `world.installed_at() < recapture.requested_at()` holds monotonically, re-check Case 4,
        // and only then pin.
        let world = match ensure_world(self.page, self.scope, self.tab_tree, frame).await? {
            EnsuredWorld::Ready(world) => world,
            EnsuredWorld::Gone => return Ok(HydrateOutcome::WorldStale),
            EnsuredWorld::Missing | EnsuredWorld::Unread(_) => {
                return Ok(HydrateOutcome::StaleDocument);
            }
        };
        let recapture = self
            .tab_tree
            .capture(self.scope, self.page, AttemptStart::now())
            .await?;
        if !doc_matches_frame(recapture.tree(), &main_frame, &key.doc, frame) {
            if recapture.tree().document_by_token(&key.doc).is_none() {
                let _ = self.tab_tree.retire_doc(self.page.frames(), &key.doc);
            }
            return Ok(HydrateOutcome::StaleDocument);
        }
        self.pin_and_bind(world, key).await
    }

    /// Hydrates a snapshot or minted ref jump (`e5`, `f7e3`, `n1`, `f2n3`) without capturing on
    /// bound documents (design section 6.4).
    ///
    /// - Bound doc: resolves and pins directly in its proven world with zero APC captures.
    /// - Reserved doc: executes section 6.2 Case 3 once (install world, recapture, verify Case 4,
    ///   pin, and bind to the reserved seq).
    /// - Retired ref or retired/foreign seq: fails `ErrorCode::ElementDetached` with zero captures.
    /// - Unknown ref on a never-issued or live seq: fails `ErrorCode::InvalidInput` with zero
    ///   captures.
    /// - An element that left its live document lands as [`RefLanding::Left`].
    pub(crate) async fn land_ref(&self, jump: AriaRefJump) -> Result<RefLanding, PlaywrightError> {
        let Some(target_ref) = jump_ref(jump) else {
            return Err(unknown_ref(self.scope, jump));
        };
        let key = match self.tab_tree.resolve_ref(&target_ref) {
            // A click proxy (a zero-size control's shown label, design 3.5) changes only how an
            // action presses the control; the ref still names the control, so hydrate that.
            RefLookup::Key(RefTarget { key, .. }) => key,
            RefLookup::Retired => return Err(detached_ref(self.scope, jump)),
            RefLookup::Unknown => {
                return match self.page.frames().lookup_doc_seq(jump.seq) {
                    DocLookup::Retired => Err(detached_ref(self.scope, jump)),
                    // A seq a live document holds, under which no snapshot printed this ref:
                    // the element is not on the page, as the page's own ref map answered before
                    // the switch-over. Only a seq the page never issued is no ref at all.
                    DocLookup::Bound { .. } | DocLookup::Reserved { .. } => {
                        Err(detached_ref(self.scope, jump))
                    }
                    DocLookup::Unknown => match self.page.frames().by_seq(jump.seq) {
                        SeqLookup::Live { .. } | SeqLookup::Retired => {
                            Err(detached_ref(self.scope, jump))
                        }
                        SeqLookup::Unknown => Err(unknown_ref(self.scope, jump)),
                    },
                };
            }
        };

        match self.page.frames().lookup_doc(&key.doc) {
            DocLookup::Bound {
                seq,
                world,
                parents,
            } => {
                // Zero-capture fast path: `world` is already proven to hold `key.doc`.
                let path = FramePath::from_parents(parents);
                match pin_in_world(self.scope, self.page, &world, key.node).await? {
                    PinAttempt::Pinned {
                        pin,
                        preview,
                        object_id,
                    } => Ok(RefLanding::Node(HydratedNode {
                        world: *world,
                        path,
                        pin,
                        preview,
                        object_id,
                        key,
                        seq,
                    })),
                    PinAttempt::WorldGone => {
                        self.tab_tree.invalidate_world(self.page.frames(), &world);
                        Err(detached_ref(self.scope, jump))
                    }
                    PinAttempt::NodeDetached => Ok(RefLanding::Left {
                        world: *world,
                        path,
                    }),
                }
            }
            // The ref table issued this ref under the seq `DocTable` reserved for its document;
            // any other seq means the two disagree about which document the ref names, and a
            // ref that might name another document never hydrates.
            DocLookup::Reserved { seq, .. } if seq != jump.seq => {
                Err(detached_ref(self.scope, jump))
            }
            DocLookup::Reserved { frame, .. } => self.hydrate_reserved_ref(jump, key, frame).await,
            DocLookup::Retired => Err(detached_ref(self.scope, jump)),
            DocLookup::Unknown => Err(unknown_ref(self.scope, jump)),
        }
    }

    /// [`Self::land_ref`] for a caller that needs the element: one that left fails
    /// `element_detached`.
    #[cfg(test)]
    pub(crate) async fn hydrate_ref(
        &self,
        jump: AriaRefJump,
    ) -> Result<HydratedNode, PlaywrightError> {
        match self.land_ref(jump).await? {
            RefLanding::Node(node) => Ok(node),
            RefLanding::Left { .. } => Err(detached_ref(self.scope, jump)),
        }
    }

    /// Section 6.2 Case 3 once for a `Reserved` ref: install the frame's world, take one fresh
    /// capture to prove `key.doc` still occupies `frame`, then pin and bind to the reserved seq.
    async fn hydrate_reserved_ref(
        &self,
        jump: AriaRefJump,
        key: NodeKey,
        frame: Option<FrameId>,
    ) -> Result<RefLanding, PlaywrightError> {
        let Some(frame) = frame else {
            let _ = self.tab_tree.retire_doc(self.page.frames(), &key.doc);
            return Err(detached_ref(self.scope, jump));
        };
        // Retry `ensure_world` once on transient `Gone` (for example a stale parent world
        // invalidated mid-hop or a context-watch churn during install), without prematurely
        // retiring `key.doc` until a missing frame or Case 4 recapture proves it gone.
        let mut ensured = None;
        for _ in 0..2 {
            match ensure_world(self.page, self.scope, self.tab_tree, &frame).await {
                Ok(EnsuredWorld::Ready(world)) => {
                    ensured = Some(world);
                    break;
                }
                Ok(EnsuredWorld::Gone) => {}
                Ok(EnsuredWorld::Missing) => {
                    let _ = self.tab_tree.retire_doc(self.page.frames(), &key.doc);
                    return Err(detached_ref(self.scope, jump));
                }
                // A read that left the ref's frame out because it did not answer in time proved
                // nothing gone: the document keeps its reservation and the ref its entry, so the
                // same ref hydrates once the frame answers, and the op fails as a timeout (the
                // private look's `run_timeout`). Retiring here would let one busy moment kill the
                // ref, and every other ref printed in that document, for good.
                Ok(EnsuredWorld::Unread(UnreadCause::TimedOut)) => {
                    return Err(unanswered_ref(self.scope, jump));
                }
                // A frame tree that cannot be read (the whole tree, or the part that holds the
                // ref's frame; refused, or a frame's tree that never answered) cannot lead to the
                // ref's frame: the ref is detached as far as this op can tell (ruling R63, row 6),
                // and its document is not retired, since nothing proved it gone. The op's own
                // timeout or cancellation stays what it is.
                Ok(EnsuredWorld::Unread(UnreadCause::Refused)) => {
                    return Err(detached_ref(self.scope, jump));
                }
                Err(error) if error.error_code() == Some(ErrorCode::Internal) => {
                    return Err(detached_ref(self.scope, jump));
                }
                Err(error) => return Err(error),
            }
        }
        let Some(world) = ensured else {
            return Err(detached_ref(self.scope, jump));
        };
        let recapture = self
            .tab_tree
            .capture(self.scope, self.page, AttemptStart::now())
            .await?;
        let main_frame = self.page.main_endpoint().frame;
        if !doc_matches_frame(recapture.tree(), &main_frame, &key.doc, &frame) {
            let _ = self.tab_tree.retire_doc(self.page.frames(), &key.doc);
            return Err(detached_ref(self.scope, jump));
        }
        match self.pin_and_bind(world.clone(), &key).await? {
            HydrateOutcome::Hydrated(node) => Ok(RefLanding::Node(*node)),
            // `DocTable` refused the binding: the document retired while it was proved.
            HydrateOutcome::StaleDocument => {
                let _ = self.tab_tree.retire_doc(self.page.frames(), &key.doc);
                Err(detached_ref(self.scope, jump))
            }
            HydrateOutcome::WorldStale => Err(detached_ref(self.scope, jump)),
            // The world answered after the recapture placed the document in its frame (Case 3),
            // so it holds the document: it binds, and the element alone is gone.
            HydrateOutcome::NodeDetached => {
                if self
                    .page
                    .frames()
                    .bind_proven_world(&key.doc, &world)
                    .is_none()
                {
                    return Err(detached_ref(self.scope, jump));
                }
                match self.page.frames().lookup_doc(&key.doc) {
                    DocLookup::Bound { parents, .. } => Ok(RefLanding::Left {
                        world,
                        path: FramePath::from_parents(parents),
                    }),
                    DocLookup::Reserved { .. } | DocLookup::Retired | DocLookup::Unknown => {
                        Err(detached_ref(self.scope, jump))
                    }
                }
            }
        }
    }

    async fn pin_and_bind(
        &self,
        world: IsolatedWorld,
        key: &NodeKey,
    ) -> Result<HydrateOutcome, PlaywrightError> {
        match pin_in_world(self.scope, self.page, &world, key.node).await? {
            PinAttempt::Pinned {
                pin,
                preview,
                object_id,
            } => {
                let Some(_seq) = self.page.frames().bind_proven_world(&key.doc, &world) else {
                    return Ok(HydrateOutcome::StaleDocument);
                };
                match self.page.frames().lookup_doc(&key.doc) {
                    DocLookup::Bound { seq, parents, .. } => {
                        Ok(HydrateOutcome::Hydrated(Box::new(HydratedNode {
                            world,
                            path: FramePath::from_parents(parents),
                            pin,
                            preview,
                            object_id,
                            key: key.clone(),
                            seq,
                        })))
                    }
                    DocLookup::Reserved { .. } | DocLookup::Retired | DocLookup::Unknown => {
                        Ok(HydrateOutcome::WorldStale)
                    }
                }
            }
            PinAttempt::WorldGone => {
                self.tab_tree.invalidate_world(self.page.frames(), &world);
                Ok(HydrateOutcome::WorldStale)
            }
            PinAttempt::NodeDetached => Ok(HydrateOutcome::NodeDetached),
        }
    }
}

/// Converts a parsed `aria-ref` jump into the ref table's [`Ref`]; the two parsers accept the same
/// canonical forms. `None` only for element 0, which `AriaRefJump::parse` never yields and no
/// table issues.
fn jump_ref(jump: AriaRefJump) -> Option<Ref> {
    let kind = match jump.kind {
        AriaRefKind::Snapshot => RefKind::Snapshot,
        AriaRefKind::Minted => RefKind::Minted,
    };
    let element = NonZeroU32::new(jump.element)?;
    Some(Ref::new(DocSeq::new(jump.seq.0), kind, element))
}

/// Checks whether `doc_token` is the document occupying `expected_frame` inside `tree`.
///
/// Main documents always correspond to `main_frame` even when BrowserOS `devtools_frame_token` is
/// omitted on `main_frame_data`; child documents match strictly on their reported `frame_token`.
pub(super) fn doc_matches_frame(
    tree: &SnapshotTree,
    main_frame: &FrameId,
    doc_token: &DocToken,
    expected_frame: &FrameId,
) -> bool {
    let Some(doc) = tree.document_by_token(doc_token) else {
        return false;
    };
    if doc.is_main() {
        expected_frame == main_frame
    } else {
        doc.frame_token()
            .is_some_and(|token| token.as_str() == expected_frame.as_str())
    }
}

/// The document `tree` places in `frame`: the main document for the main frame, else the child
/// document whose frame token is `frame`'s id. `None` when the capture holds no document of the
/// frame, such as a frame it could not read or one that navigated since.
pub(crate) fn frame_doc(
    tree: &SnapshotTree,
    main_frame: &FrameId,
    frame: &FrameId,
) -> Option<DocId> {
    tree.documents()
        .iter()
        .find(|doc| {
            if doc.is_main() {
                frame == main_frame
            } else {
                doc.frame_token()
                    .is_some_and(|token| token.as_str() == frame.as_str())
            }
        })
        .map(TreeDoc::id)
}

/// The frame a document of `tree` lives in: the main frame, or its frame token as a frame id.
pub(super) fn doc_frame(doc: &TreeDoc, main_frame: &FrameId) -> Option<FrameId> {
    if doc.is_main() {
        Some(main_frame.clone())
    } else {
        doc.frame_token()
            .map(|token| FrameId(token.as_str().to_owned()))
    }
}

/// Resolves `node` into `world` via [`resolve_node`] and pins it with [`pin_object`], which
/// verifies world identity (V8 refuses another world's argument), and that the element is
/// connected to the world's own document.
async fn pin_in_world(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    node: DomNodeId,
) -> Result<PinAttempt, PlaywrightError> {
    let object_id = match resolve_node(scope, world, node).await {
        Ok(Some(object_id)) => object_id,
        Ok(None) => return Ok(PinAttempt::NodeDetached),
        Err(ProbeFailure::Gone(_)) => return Ok(PinAttempt::WorldGone),
        Err(failure) => return Err(resolve::failed(scope, failure)),
    };
    match pin_object(scope, page, world, &object_id).await {
        Ok(ObjectPin::Pinned { pin, preview }) => Ok(PinAttempt::Pinned {
            pin,
            preview,
            object_id,
        }),
        Ok(ObjectPin::Detached) => Ok(PinAttempt::NodeDetached),
        Ok(ObjectPin::Refused(reply)) => Err(resolve::refused(scope, reply)),
        Err(ProbeFailure::Gone(_)) => Ok(PinAttempt::WorldGone),
        Err(failure) => Err(resolve::failed(scope, failure)),
    }
}

/// `node`'s remote object in `world` (`DOM.resolveNode` with the world's context id, in the op's
/// object group), or `None` when the world lives and only the node is gone: any other CDP
/// protocol error, such as "No node with given id found". `Gone` when the world itself is gone.
///
/// A target crash or close stays `Cdp`, and is read first: CDP reports it as a protocol error,
/// which would read as a detached node, or as a lost session, which would read as a world gone,
/// and either would retry the attempt on a page that no longer exists.
pub(crate) async fn resolve_node(
    scope: &OpScope<'_>,
    world: &IsolatedWorld,
    node: DomNodeId,
) -> Result<Option<String>, ProbeFailure> {
    let params = json!({
        "backendNodeId": node.as_backend_node_id(),
        "executionContextId": world.context(),
        "objectGroup": scope.object_group(world.session()),
    });
    let crash = scope.begin_protocol(world.session());
    let resolved = world
        .session()
        .send::<_, ResolvedNode>("DOM.resolveNode", params)
        .await
        .inspect_err(|error| crash.record(error));
    match resolved {
        Ok(resolved) => Ok(Some(resolved.object.object_id)),
        Err(error)
            if matches!(
                ErrorCode::from_core(&error),
                ErrorCode::TargetCrashed | ErrorCode::TargetClosed
            ) =>
        {
            Err(ProbeFailure::Cdp(error))
        }
        Err(error) => match ProbeFailure::from_core(error) {
            ProbeFailure::Cdp(CoreError::Cdp(CdpError::Protocol { .. })) => Ok(None),
            failure => Err(failure),
        },
    }
}

/// Pins the element `object` names (a node [`resolve_node`] resolved in `world`) with the probe's
/// `Resolve { target: Root, strictness: Strict, want: Element }`, then checks the pin with
/// [`probe::pinned_nodes_in_document`]: design 6.2 takes the node only while it is connected and
/// of the world's document. The probe's root pin checks only `isConnected`, and an element the
/// page moved into a same-origin iframe still resolves in this world, so without the second call
/// a ref or a run would act on an element of another document.
pub(super) async fn pin_object(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    object: &str,
) -> Result<ObjectPin, ProbeFailure> {
    let request = ProbeRequest::Resolve {
        target: ProbeTarget::Root,
        strictness: Strictness::Strict,
        want: Want::Element,
    };
    let (pin, preview) = match probe::call_on(scope, page, world, &request, object).await? {
        ProbeReply::Element { pin, preview, .. } => (pin, preview),
        ProbeReply::Detached | ProbeReply::None { .. } => return Ok(ObjectPin::Detached),
        other => return Ok(ObjectPin::Refused(other)),
    };
    Ok(
        match probe::pinned_nodes_in_document(scope, page, world, vec![pin]).await? {
            Some(_) => ObjectPin::Pinned { pin, preview },
            None => ObjectPin::Detached,
        },
    )
}

/// Ensures an isolated world is installed for `frame`, walking down through [`FrameStart`] for
/// child frames so ancestor parent links are recorded in `FrameWorlds`.
pub(super) async fn ensure_world(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    tab_tree: &TabTree,
    frame: &FrameId,
) -> Result<EnsuredWorld, PlaywrightError> {
    let main_endpoint = page.main_endpoint();
    let main = match page.world(&main_endpoint, Place::Main).await {
        Ok(world) => world,
        Err(ProbeFailure::Gone(_)) => return Ok(EnsuredWorld::Gone),
        Err(failure) => return Err(resolve::failed(scope, failure)),
    };
    if *frame == main_endpoint.frame {
        return Ok(EnsuredWorld::Ready(main));
    }
    // Boxed: a frame-start walk hops across arbitrary iframe depth inside hydration futures.
    let started = Box::pin(FrameStart::new(frame.clone()).enter(page, scope, main)).await?;
    Ok(match started {
        Started::Ready { world, .. } => EnsuredWorld::Ready(*world),
        Started::Gone(stale) => {
            if let Some(world) = stale {
                tab_tree.invalidate_world(page.frames(), &world);
            }
            EnsuredWorld::Gone
        }
        Started::Missing | Started::Detached => EnsuredWorld::Missing,
        Started::Unread(cause) => EnsuredWorld::Unread(cause),
    })
}

/// The isolated world of `frame`, installed if this instance has none there yet (walking down from
/// the main frame, as a frame start does): `None` when the frame is gone or a world on the way
/// went away under the install. Best effort for callers that act on a frame only if they can.
pub(crate) async fn ensure_frame_world(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    frame: &FrameId,
) -> Option<IsolatedWorld> {
    match ensure_world(page, scope, page.tree(), frame).await {
        Ok(EnsuredWorld::Ready(world)) => Some(world),
        Ok(EnsuredWorld::Gone | EnsuredWorld::Missing | EnsuredWorld::Unread(_)) | Err(_) => None,
    }
}

/// Opens to the page's CSS and text engines the closed shadow roots of `doc` that the capture
/// found (contract field 31: one seed element in each), at most `limit` of them, so a locator
/// after a snapshot finds what it printed inside them and the probe's `privateMarks` walks them.
/// Whether every root the capture found was handed over: a seed whose node is gone counts as
/// handed over, a cut or a failure to resolve or to send does not, and a failure leaves the
/// roots as they were.
pub(crate) async fn open_closed_roots(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    tree: &SnapshotTree,
    doc: DocId,
    limit: Option<usize>,
) -> bool {
    let seeds: Vec<DomNodeId> = tree
        .doc_nodes(doc)
        .filter(|node| node.is_closed_root_seed())
        .filter_map(TreeNode::dom_node_id)
        .collect();
    if seeds.is_empty() {
        return true;
    }
    let cut = limit.filter(|&limit| seeds.len() > limit);
    if let Some(limit) = cut {
        scope.log(format!(
            "  closed shadow roots: a frame has {} seeds, the first {limit} sent",
            seeds.len()
        ));
    }
    let take = cut.unwrap_or(seeds.len());
    let mut objects = Vec::with_capacity(take);
    for seed in seeds.into_iter().take(take) {
        match resolve_node(scope, world, seed).await {
            Ok(Some(object)) => objects.push(object),
            Ok(None) => {}
            Err(_) => return false,
        }
    }
    if objects.is_empty() {
        return cut.is_none();
    }
    let request = ProbeRequest::OpenClosedRoots;
    let sent = probe::call_with_nodes(scope, page, world, &request, None, &objects)
        .await
        .is_ok();
    if sent {
        scope.log(format!(
            "  closed shadow roots opened to the page's engines: {}",
            objects.len()
        ));
    }
    sent && cut.is_none()
}

/// `website / element_detached`: the ref's document or element is gone for good.
fn detached_ref(scope: &OpScope<'_>, jump: AriaRefJump) -> PlaywrightError {
    let text = jump.to_string();
    scope.fail(
        ErrorCode::ElementDetached,
        &ref_message(scope, &text, "is not on the page any more"),
    )
}

/// `website / wait_timeout`: the ref's frame did not answer the frame tree's read in time. A
/// timeout, not a detach: the frame may answer a moment later, and the ref with it.
fn unanswered_ref(scope: &OpScope<'_>, jump: AriaRefJump) -> PlaywrightError {
    scope.fail(
        ErrorCode::WaitTimeout,
        &format!("{jump} is in a frame that did not answer in time; try again"),
    )
}

/// `script / invalid_input`: the ref was never issued for this page.
fn unknown_ref(scope: &OpScope<'_>, jump: AriaRefJump) -> PlaywrightError {
    let text = jump.to_string();
    scope.fail(
        ErrorCode::InvalidInput,
        &ref_message(scope, &text, "is not a ref on this page"),
    )
}

/// Formats ref failure messages identically to `resolve::resolver::ref_message`.
fn ref_message(scope: &OpScope<'_>, r#ref: &str, ordinary: &str) -> String {
    let opened_here = scope
        .admitted_tab()
        .is_some_and(|tab| scope.run_state().opened_tabs.opened_without_refs(tab));
    if opened_here {
        format!(
            "{ref} is not from this tab, which this run opened: a ref works only in the tab \
             whose snapshot printed it. A task keeps its tab across runs: await neo.task(id)"
        )
    } else {
        format!("{ref} {ordinary}; take a new snapshot")
    }
}

#[cfg(test)]
mod tests {
    use neo_snapshot_apc::refs::Ref;

    use super::jump_ref;
    use crate::target::AriaRefJump;
    use crate::test_support::TestResult;

    #[test]
    fn jump_ref_names_the_ref_the_table_prints() -> TestResult {
        for printed in ["e5", "f7e3", "n1", "f2n3"] {
            let jump = AriaRefJump::parse(printed).ok_or("valid canonical ref")?;
            let converted = jump_ref(jump).ok_or("canonical refs convert")?;
            assert_eq!(
                Some(converted),
                Ref::parse(printed),
                "aria-ref={printed} must look up the ref the table parses from {printed}"
            );
            assert_eq!(
                converted.to_string(),
                printed,
                "the table must print {printed} back"
            );
        }
        for rejected in ["f0e5", "e0", "e05", "f01e1", "x1", "e"] {
            assert!(
                AriaRefJump::parse(rejected).is_none() && Ref::parse(rejected).is_none(),
                "both parsers must reject {rejected}"
            );
        }
        Ok(())
    }
}
