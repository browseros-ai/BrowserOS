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
use neo_snapshot_apc::tree::{DocSeq, DocToken, DomNodeId, NodeKey, SnapshotTree};
use serde::Deserialize;
use serde_json::json;

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget, Want};
use crate::resolve::{self, FramePath, FrameStart, Started};
use crate::target::{AriaRefJump, AriaRefKind, Strictness};
use crate::tree::{AttemptStart, CapturedTree, TabTree};
use crate::world::{DocLookup, FrameSeq, IsolatedWorld, Pin, Place};
use crate::{ErrorCode, PlaywrightError};

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

enum EnsuredWorld {
    Ready(IsolatedWorld),
    Gone,
    Missing,
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
            EnsuredWorld::Missing => return Ok(HydrateOutcome::StaleDocument),
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
    pub(crate) async fn hydrate_ref(
        &self,
        jump: AriaRefJump,
    ) -> Result<HydratedNode, PlaywrightError> {
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
                    DocLookup::Bound { .. } | DocLookup::Reserved { .. } | DocLookup::Unknown => {
                        Err(unknown_ref(self.scope, jump))
                    }
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
                match pin_in_world(self.scope, self.page, &world, key.node).await? {
                    PinAttempt::Pinned {
                        pin,
                        preview,
                        object_id,
                    } => Ok(HydratedNode {
                        world: *world,
                        path: FramePath::from_parents(parents),
                        pin,
                        preview,
                        object_id,
                        key,
                        seq,
                    }),
                    PinAttempt::WorldGone => {
                        self.tab_tree.invalidate_world(self.page.frames(), &world);
                        Err(detached_ref(self.scope, jump))
                    }
                    PinAttempt::NodeDetached => Err(detached_ref(self.scope, jump)),
                }
            }
            DocLookup::Reserved { frame, .. } => self.hydrate_reserved_ref(jump, key, frame).await,
            DocLookup::Retired => Err(detached_ref(self.scope, jump)),
            DocLookup::Unknown => Err(unknown_ref(self.scope, jump)),
        }
    }

    /// Section 6.2 Case 3 once for a `Reserved` ref: install the frame's world, take one fresh
    /// capture to prove `key.doc` still occupies `frame`, then pin and bind to the reserved seq.
    async fn hydrate_reserved_ref(
        &self,
        jump: AriaRefJump,
        key: NodeKey,
        frame: Option<FrameId>,
    ) -> Result<HydratedNode, PlaywrightError> {
        let Some(frame) = frame else {
            let _ = self.tab_tree.retire_doc(self.page.frames(), &key.doc);
            return Err(detached_ref(self.scope, jump));
        };
        // Retry `ensure_world` once on transient `Gone` (for example a stale parent world
        // invalidated mid-hop or a context-watch churn during install), without prematurely
        // retiring `key.doc` until a missing frame or Case 4 recapture proves it gone.
        let mut ensured = None;
        for _ in 0..2 {
            match ensure_world(self.page, self.scope, self.tab_tree, &frame).await? {
                EnsuredWorld::Ready(world) => {
                    ensured = Some(world);
                    break;
                }
                EnsuredWorld::Gone => {}
                EnsuredWorld::Missing => {
                    let _ = self.tab_tree.retire_doc(self.page.frames(), &key.doc);
                    return Err(detached_ref(self.scope, jump));
                }
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
        match self.pin_and_bind(world, &key).await? {
            HydrateOutcome::Hydrated(node) => Ok(*node),
            // `DocTable` refused the binding: the document retired while it was proved.
            HydrateOutcome::StaleDocument => {
                let _ = self.tab_tree.retire_doc(self.page.frames(), &key.doc);
                Err(detached_ref(self.scope, jump))
            }
            HydrateOutcome::WorldStale | HydrateOutcome::NodeDetached => {
                Err(detached_ref(self.scope, jump))
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
pub(super) async fn resolve_node(
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
async fn ensure_world(
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
    })
}

/// `website / element_detached`: the ref's document or element is gone for good.
fn detached_ref(scope: &OpScope<'_>, jump: AriaRefJump) -> PlaywrightError {
    let text = jump.to_string();
    scope.fail(
        ErrorCode::ElementDetached,
        &ref_message(scope, &text, "is not on the page any more"),
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
