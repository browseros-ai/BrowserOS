//! A locator built from a child `Frame` (`frame.locator()`, and every `frame.*` method that
//! takes a selector) starts in that frame's document, as upstream's frame-scoped selectors do.
//!
//! ```text
//!  frame id (the prelude's, from page.frames) ── the page's frame tree ── main, f1, .., frame
//!  each level: DOM.getFrameOwner(child) in the parent's session ── DOM.resolveNode in the
//!    parent's isolated world ── probe resolve { root, want: frame } ── the hop's second half
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
use crate::page::OpPage;
use crate::page::frame_tree::{self, FramePlace, UnreadCause};
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget, Want};
use crate::resolve::resolver::{self, Hopped, Iframe};
use crate::resolve::{self, FramePath};
use crate::target::Strictness;
use crate::world::{BackendNodeId, IsolatedWorld};
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
        world: Box<IsolatedWorld>,
        path: FramePath,
        visible: bool,
    },
    /// A frame on the way is not there yet.
    Missing,
    /// A world on the way went away; the resolver invalidates it and runs the pass again.
    Gone(Option<IsolatedWorld>),
    /// The page's frame tree no longer lists the frame: it was detached.
    Detached,
    /// The page's frame tree could not be read down to the frame: an out-of-process frame on the
    /// way was left out, and why. The frame may still be there, so this proves nothing gone.
    Unread(UnreadCause),
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
    /// detached frame does; one the tree could not be read down to is [`Started::Unread`].
    pub(crate) async fn enter(
        &self,
        page: &OpPage<'_>,
        scope: &OpScope<'_>,
        main: IsolatedWorld,
    ) -> Result<Started, PlaywrightError> {
        // The tree's read leaves out an out-of-process frame it cannot read (one that does not
        // answer within two seconds, or refuses) rather than fail, so a frame missing from a
        // partial read may be inside one left out, still attached. Callers retire what lived in a
        // detached frame (its document, and every ref a snapshot printed there), which only a
        // whole read may prove; an unread frame keeps all of it, so the same ref works once its
        // renderer answers.
        let chain = match self.chain(page, scope).await? {
            FramePlace::Listed(chain) => chain,
            FramePlace::Absent => return Ok(Started::Detached),
            FramePlace::Unread(cause) => return Ok(Started::Unread(cause)),
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

    /// Where the page's frame tree places the start frame, from the cache or a read of the tree.
    /// Only a listed chain is cached, so a frame left unread is read again by the next attempt.
    async fn chain(
        &self,
        page: &OpPage<'_>,
        scope: &OpScope<'_>,
    ) -> Result<FramePlace, PlaywrightError> {
        if let Some(chain) = self.lock().as_ref() {
            return Ok(FramePlace::Listed(chain.clone()));
        }
        let main = page.main_endpoint();
        let tree = frame_tree::read(page.browser(), page.core_page(), &main.session)
            .await
            .map_err(|error| scope.core_error(&error))?;
        let place = tree.place(&self.frame);
        if let FramePlace::Listed(chain) = &place {
            *self.lock() = Some(chain.clone());
        }
        Ok(place)
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
    parent: &IsolatedWorld,
    child: &FrameId,
) -> Result<Hopped, PlaywrightError> {
    let crash = scope.begin_protocol(parent.session());
    let owner = parent
        .session()
        .send::<_, FrameOwner>("DOM.getFrameOwner", json!({ "frameId": child.as_str() }))
        .await
        .inspect_err(|error| crash.record(error));
    let owner = match owner {
        Ok(owner) => owner.backend_node_id,
        Err(error) if ErrorCode::from_core(&error) == ErrorCode::TargetCrashed => {
            return Err(scope.core_error(&error));
        }
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
    let crash = scope.begin_protocol(parent.session());
    let resolved = parent
        .session()
        .send::<_, ResolvedNode>("DOM.resolveNode", params)
        .await
        .inspect_err(|error| crash.record(error));
    let object = match resolved {
        Ok(resolved) => resolved.object.object_id,
        Err(error) => return gone_or(scope, error),
    };
    let request = ProbeRequest::Resolve {
        target: ProbeTarget::Root,
        strictness: Strictness::Strict,
        want: Want::Frame,
    };
    match probe::call_on(scope, page, parent, &request, &object).await {
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
    if ErrorCode::from_core(&error) == ErrorCode::TargetCrashed {
        return Err(scope.core_error(&error));
    }
    match ProbeFailure::from_core(error) {
        ProbeFailure::Gone(_) => Ok(Hopped::ParentGone),
        ProbeFailure::Cdp(CoreError::Cdp(CdpError::Protocol { .. })) => Ok(Hopped::Missing),
        failure => Err(resolve::failed(scope, failure)),
    }
}
