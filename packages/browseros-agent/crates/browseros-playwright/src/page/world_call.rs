//! How the page ops call into a frame's sealed world: one retry when the frame's document was
//! replaced under the world. A failure left after it is the op's through `resolve::failed`.

use std::future::Future;

use browseros_core::FrameId;

use crate::PlaywrightError;
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{ProbeFailure, ProbeReply};
use crate::world::{Endpoint, Place, SealedWorld};

/// The frame a page op's input names: absent is the main frame, the only one these ops serve
/// yet. `frame.evaluate`, `frame.title` and the other frame forms answer `not built yet`.
pub(crate) fn main_frame_only(
    scope: &OpScope<'_>,
    frame: Option<&FrameId>,
) -> Result<(), PlaywrightError> {
    match frame {
        None => Ok(()),
        Some(_) => Err(scope.not_built()),
    }
}

/// Runs `call` in the world of `endpoint`'s frame, installing it on first use. A world whose
/// document was replaced since its install fails `Gone`: it is dropped (its seq retires) and
/// `call` runs once more in the new document's world, under a new seq. A second `Gone`, or an
/// install that fails `Gone` because the frame itself went away, is returned.
///
/// `call` must be safe to repeat after a `Gone`, also one that may have run before its document
/// went (`Ran::Maybe`): every caller's is a read, a sweep, a document read or a URL match, which
/// changes nothing in the page.
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
