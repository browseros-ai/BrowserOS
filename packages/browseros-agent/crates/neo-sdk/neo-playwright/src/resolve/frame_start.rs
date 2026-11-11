//! A locator built from a child `Frame` (`frame.locator()`, and every `frame.*` method that
//! takes a selector) starts in that frame's document, as upstream's frame-scoped selectors do.
//!
//! ```text
//!  frame id (the prelude's, from page.frames) ── the page's frame tree ── main, f1, .., frame
//!  each level: DOM.getFrameOwner(child) in the parent's session ── DOM.resolveNode in the
//!    parent's sealed world ── probe resolve { root, want: frame } ── the hop's second half
//! ```
//!
//! Each level is a hop like a `frameLocator` chunk's, so the frame's parent links are recorded
//! (a ref minted there jumps back along them), its boxes reach the main viewport, and every
//! iframe on the way counts toward visibility.

use std::sync::{Mutex, MutexGuard, PoisonError};

use browseros_cdp::CdpError;
use browseros_core::{CoreError, FrameId};
use serde::Deserialize;
use serde_json::json;

use crate::op_scope::OpScope;
use crate::page::{OpPage, frame_tree};
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget, Want};
use crate::resolve::resolver::{self, Hopped, Iframe};
use crate::resolve::{self, FramePath};
use crate::target::Strictness;
use crate::world::{BackendNodeId, SealedWorld};
use crate::{ErrorCode, PlaywrightError};

/// One op's way into the child frame its locator was built from.
pub(crate) struct FrameStart {
    frame: FrameId,
    /// The frames from the main frame's child down to `frame`, read from the page's frame tree
    /// once per op, because frame ids outlive their frames' navigations. A level whose owner
    /// is not where the tree said drops it, so the next attempt reads the tree again. A std
    /// mutex, never held across an await.
    chain: Mutex<Option<Vec<FrameId>>>,
}

/// Where a frame start got to.
pub(crate) enum Started {
    /// The frame's world, the frames above it and whether every iframe on the way is visible.
    Ready {
        /// Boxed: a world is large, and the other variants carry little.
        world: Box<SealedWorld>,
        path: FramePath,
        visible: bool,
    },
    /// A frame on the way is not there yet.
    Missing,
    /// A world on the way went away; the resolver invalidates it and runs the pass again.
    Gone(Option<SealedWorld>),
    /// The page's frame tree no longer lists the frame: it was detached.
    Detached,
}

/// `DOM.getFrameOwner`'s answer.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameOwner {
    backend_node_id: BackendNodeId,
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

impl FrameStart {
    pub(crate) fn new(frame: FrameId) -> Self {
        Self {
            frame,
            chain: Mutex::new(None),
        }
    }

    /// Walks from `main`, the main frame's world, down to the start frame. A frame no longer
    /// in the page's tree is [`Started::Detached`], which each caller fails as upstream's op on a
    /// detached frame does.
    pub(crate) async fn enter(
        &self,
        page: &OpPage<'_>,
        scope: &OpScope<'_>,
        main: SealedWorld,
    ) -> Result<Started, PlaywrightError> {
        let Some(chain) = self.chain(page, scope).await? else {
            return Ok(Started::Detached);
        };
        let mut world = main;
        let mut path = FramePath::default();
        let mut visible = true;
        for child in chain {
            match enter_child(page, scope, &world, &child).await? {
                Hopped::Into {
                    child: entered,
                    hop,
                    visible: shown,
                } if *entered.frame() == child => {
                    path.push(hop);
                    visible &= shown;
                    world = entered;
                }
                Hopped::Into { .. } | Hopped::Missing => {
                    *self.lock() = None;
                    return Ok(Started::Missing);
                }
                Hopped::ParentGone => return Ok(Started::Gone(Some(world))),
                Hopped::ChildGone => return Ok(Started::Gone(None)),
            }
        }
        Ok(Started::Ready {
            world: Box::new(world),
            path,
            visible,
        })
    }

    /// The frames down to the start frame, from the cache or the page's frame tree; `None`
    /// when the tree no longer lists it.
    async fn chain(
        &self,
        page: &OpPage<'_>,
        scope: &OpScope<'_>,
    ) -> Result<Option<Vec<FrameId>>, PlaywrightError> {
        if let Some(chain) = self.lock().as_ref() {
            return Ok(Some(chain.clone()));
        }
        let main = page.main_endpoint();
        let tree = frame_tree::read(page.browser(), page.core_page(), &main.session)
            .await
            .map_err(|error| scope.core_error(&error))?;
        let Some(chain) = tree.path_to(&self.frame) else {
            return Ok(None);
        };
        *self.lock() = Some(chain.clone());
        Ok(Some(chain))
    }

    fn lock(&self) -> MutexGuard<'_, Option<Vec<FrameId>>> {
        self.chain.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One level down: the `<iframe>` in `parent`'s document whose content frame is `child`,
/// entered the way a hop enters the one a chunk selects.
async fn enter_child(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    parent: &SealedWorld,
    child: &FrameId,
) -> Result<Hopped, PlaywrightError> {
    let owner = parent
        .session()
        .send::<_, FrameOwner>("DOM.getFrameOwner", json!({ "frameId": child.as_str() }))
        .await;
    let owner = match owner {
        Ok(owner) => owner.backend_node_id,
        // The frame's owner is not in this document (yet, or any more). That says nothing
        // about the parent's own document, so its world stays: invalidating it would retire
        // the seq that the parent's refs live under.
        Err(CoreError::Cdp(CdpError::Protocol { .. })) => return Ok(Hopped::Missing),
        Err(error) => return gone_or(scope, error),
    };
    let params = json!({
        "backendNodeId": owner,
        "executionContextId": parent.context(),
        "objectGroup": scope.object_group(parent.session()),
    });
    let resolved = parent
        .session()
        .send::<_, ResolvedNode>("DOM.resolveNode", params)
        .await;
    let object = match resolved {
        Ok(resolved) => resolved.object.object_id,
        Err(error) => return gone_or(scope, error),
    };
    let request = ProbeRequest::Resolve {
        target: ProbeTarget::Root,
        strictness: Strictness::Strict,
        want: Want::Frame,
    };
    match probe::call_on(parent, &request, &object).await {
        Ok(ProbeReply::Frame {
            pin, key, visible, ..
        }) => {
            let iframe = Iframe {
                object,
                pin,
                key,
                visible,
            };
            resolver::enter_iframe(page, scope, parent, iframe).await
        }
        Ok(ProbeReply::None { stale_ref: None } | ProbeReply::Detached) => Ok(Hopped::Missing),
        Ok(ProbeReply::NotFrame { preview }) => Err(scope.fail(
            ErrorCode::Unsupported,
            &format!(
                "this frame belongs to {preview}; Neo enters the frames of <iframe> and <frame> \
                 elements only"
            ),
        )),
        Ok(other) => Err(resolve::refused(scope, other)),
        Err(ProbeFailure::Gone(_)) => Ok(Hopped::ParentGone),
        Err(failure) => Err(resolve::failed(scope, failure)),
    }
}

/// A CDP failure on the parent's session: the parent's world is gone, the owner node is not
/// there (a protocol error), or the op fails.
fn gone_or(scope: &OpScope<'_>, error: CoreError) -> Result<Hopped, PlaywrightError> {
    match ProbeFailure::from_core(error) {
        ProbeFailure::Gone(_) => Ok(Hopped::ParentGone),
        ProbeFailure::Cdp(CoreError::Cdp(CdpError::Protocol { .. })) => Ok(Hopped::Missing),
        failure => Err(resolve::failed(scope, failure)),
    }
}
