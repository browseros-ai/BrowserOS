//! How the page ops call into a frame's sealed world: the frame a `frame.*` form names, found
//! the way `frame.locator` finds it, and one retry when the frame's document was replaced under
//! the world. A failure left after it is the op's through `resolve::failed`.

use std::future::Future;

use browseros_core::FrameId;

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{ProbeFailure, ProbeReply};
use crate::resolve::{self, FrameStart, Started};
use crate::world::{Endpoint, Place, SealedWorld};
use crate::{ErrorCode, PlaywrightError};

/// The world of the frame a page op's input names: absent, or the main frame's own id, is the
/// main frame; a child frame's id (from `page.frames`) is entered level by level from the main
/// frame through `FrameStart`, the route a locator built from that frame takes. A world gone on
/// the way is dropped and the walk runs once more. A frame the page's tree no longer lists, or
/// that is not where the tree says, fails `<api>: Frame was detached`, as upstream's
/// `frame.evaluate`, `waitForFunction`, `setContent` and `content` do on a detached frame
/// (measured with Playwright 1.64.1 on stock Chromium and on BrowserOS).
pub(crate) async fn frame_world(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    frame: Option<&FrameId>,
) -> Result<SealedWorld, PlaywrightError> {
    listed_frame_world(page, scope, frame)
        .await?
        .ok_or_else(|| scope.fail(ErrorCode::TargetClosed, "Frame was detached"))
}

/// [`frame_world`], with `None` for a frame the page's tree no longer lists, for an op whose
/// own failure says what a detached frame means to it (`waitForURL`'s).
pub(crate) async fn listed_frame_world(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    frame: Option<&FrameId>,
) -> Result<Option<SealedWorld>, PlaywrightError> {
    let main = page.main_endpoint();
    let child = frame.filter(|frame| **frame != main.frame);
    for _attempt in 0..2 {
        let main_world = page
            .world(&main, Place::Main)
            .await
            .map_err(|failure| resolve::failed(scope, failure))?;
        let Some(child) = child else {
            return Ok(Some(main_world));
        };
        match FrameStart::new(child.clone())
            .enter(page, scope, main_world)
            .await?
        {
            Started::Ready { world, .. } => return Ok(Some(*world)),
            Started::Missing | Started::Detached => break,
            Started::Gone(world) => {
                if let Some(world) = world {
                    page.frames().invalidate(&world);
                }
            }
        }
    }
    Ok(None)
}

/// Runs `call` in the world of `endpoint`'s frame, installing it on first use. A world whose
/// document was replaced since its install fails `Gone`: it is dropped (its seq retires) and
/// `call` runs once more in the new document's world, under a new seq. A second `Gone`, or an
/// install that fails `Gone` because the frame itself went away, is returned.
///
/// `call` must be safe to repeat after a `Gone`, also one that may have run before its document
/// went (`Ran::Maybe`): every caller's is a read, a document read or a URL match, which changes
/// nothing in the page.
pub(crate) async fn with_world<T, F, Fut>(
    page: &OpPage<'_>,
    endpoint: &Endpoint,
    place: &Place,
    call: F,
) -> Result<T, ProbeFailure>
where
    F: Fn(SealedWorld) -> Fut,
    Fut: Future<Output = Result<T, ProbeFailure>>,
{
    let world = page.world(endpoint, place.clone()).await?;
    match call(world.clone()).await {
        Err(ProbeFailure::Gone(_)) => {
            page.frames().invalidate(&world);
            call(page.world(endpoint, place.clone()).await?).await
        }
        done => done,
    }
}

/// `reply` is not one `request` allows: the page asset and this build disagree, or the probe
/// threw. The reply's own text is left out of the message unless it is the probe's error,
/// because a reply can carry page text.
pub(crate) fn unexpected(request: &str, reply: ProbeReply) -> ProbeFailure {
    match reply {
        ProbeReply::Error { message, .. } => {
            ProbeFailure::Broken(format!("the probe failed `{request}`: {message}"))
        }
        _ => ProbeFailure::Broken(format!(
            "the probe answered `{request}` with `{}`, which it does not allow",
            reply.kind()
        )),
    }
}

#[cfg(test)]
mod tests;
