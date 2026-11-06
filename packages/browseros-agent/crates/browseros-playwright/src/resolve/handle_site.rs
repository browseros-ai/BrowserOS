//! An element handle's node, found again in its own document's sealed world.

use browseros_cdp::CdpError;
use browseros_core::CoreError;
use serde::Deserialize;
use serde_json::json;

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget, QueryState, Read};
use crate::resolve::{self, FramePath};
use crate::target::{ElementTarget, Strictness};
use crate::world::{SealedWorld, SeqLookup};
use crate::{ErrorCode, PlaywrightError};

/// Where an element handle's node is, for one op: its document's world, that frame's parents
/// and the node as a remote object in the op's object group.
///
/// A handle names its node by document (`seq`) and CDP backend node id, never by a pin, so
/// the probe's pin eviction cannot detach it. Once its document is gone the seq is retired and
/// the handle is detached for good; while the document lives, a node that left it is detached
/// too. That is Neo's rule for every op on a handle, where upstream still reads a detached
/// node's text through `:scope`.
pub(crate) struct HandleSite<'a> {
    page: &'a OpPage<'a>,
    scope: &'a OpScope<'a>,
    world: SealedWorld,
    path: FramePath,
    /// The node's remote object, in this world.
    node: String,
}

/// `DOM.resolveNode`'s answer.
#[derive(Deserialize)]
struct ResolvedNode {
    object: NodeObject,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeObject {
    object_id: String,
}

impl<'a> HandleSite<'a> {
    /// Finds `target`'s node, or `None` when it is gone for good: its document was replaced,
    /// or the node was removed and collected.
    pub(crate) async fn find(
        page: &'a OpPage<'a>,
        scope: &'a OpScope<'a>,
        target: &ElementTarget,
    ) -> Result<Option<Self>, PlaywrightError> {
        let (world, parents) = match page.frames().by_seq(target.seq) {
            SeqLookup::Live { world, parents } => (*world, parents),
            SeqLookup::Retired => return Ok(None),
            SeqLookup::Unknown => {
                return Err(scope.fail(
                    ErrorCode::InvalidInput,
                    "this element handle belongs to no document of this page",
                ));
            }
        };
        let params = json!({
            "backendNodeId": target.node,
            "executionContextId": world.context(),
            "objectGroup": scope.object_group(world.session()),
        });
        let resolved = world
            .session()
            .send::<_, ResolvedNode>("DOM.resolveNode", params)
            .await;
        match resolved {
            Ok(resolved) => Ok(Some(Self {
                page,
                scope,
                world,
                path: FramePath::from_parents(parents),
                node: resolved.object.object_id,
            })),
            Err(error) => match ProbeFailure::from_core(error) {
                ProbeFailure::Gone(_) => {
                    page.frames().invalidate(&world);
                    Ok(None)
                }
                // "No node with given id found": the backend id names nothing in this
                // document any more.
                ProbeFailure::Cdp(CoreError::Cdp(CdpError::Protocol { .. })) => Ok(None),
                failure => Err(resolve::failed(scope, failure)),
            },
        }
    }

    /// Sends `request`, whose target is `ProbeTarget::Root`, with the node as the call's root.
    /// A world that went away under the call answers `detached`, like a node that left.
    pub(crate) async fn call(&self, request: &ProbeRequest) -> Result<ProbeReply, PlaywrightError> {
        match probe::call_on(&self.world, request, &self.node).await {
            Ok(reply) => Ok(reply),
            Err(ProbeFailure::Gone(_)) => {
                self.page.frames().invalidate(&self.world);
                Ok(ProbeReply::Detached)
            }
            Err(failure) => Err(resolve::failed(self.scope, failure)),
        }
    }

    /// The parents of the node's frame, from the main frame down.
    pub(crate) fn path(&self) -> &FramePath {
        &self.path
    }

    /// Whether the node is still in its document. A node a script removed stays alive (and
    /// found) while something holds it, but a query rooted at it must fail as every other op
    /// on the handle does.
    pub(crate) async fn attached(&self) -> Result<bool, PlaywrightError> {
        let request = ProbeRequest::Read {
            target: ProbeTarget::Root,
            strictness: Strictness::Strict,
            frame_visible: true,
            read: Read::State {
                state: QueryState::Hidden,
            },
        };
        Ok(!matches!(self.call(&request).await?, ProbeReply::Detached))
    }

    /// The node's document's world.
    pub(crate) fn world(&self) -> &SealedWorld {
        &self.world
    }

    /// The node, as a remote object in [`HandleSite::world`], for a query rooted at it.
    pub(crate) fn node(&self) -> &str {
        &self.node
    }
}

/// `website / element_detached`, with upstream's message for an element handle whose node is
/// gone.
pub(crate) fn handle_detached(scope: &OpScope<'_>) -> PlaywrightError {
    scope.fail(
        ErrorCode::ElementDetached,
        "Element is not attached to the DOM",
    )
}
