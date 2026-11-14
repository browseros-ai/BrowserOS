//! The first scroll of an attempt, upstream's `_scrollRectIntoViewIfNeeded`: the protocol's
//! `DOM.scrollIntoViewIfNeeded` on the pinned element, sent in the element's own session.
//!
//! ```text
//!  act { chunk | pin, scroll: none }      resolve, pin, check the states (no scroll yet)
//!  pinned { pin }                         the element as a remote object (op's object group)
//!  DOM.scrollIntoViewIfNeeded { objectId }
//!  act { pin, scroll: protocol }          measure where the scroll left it
//! ```
//!
//! It is the same Blink scroll as the page's own `element.scrollIntoViewIfNeeded(true)`
//! (center when hidden, nearest edge when partly shown, nothing when shown, bubbling through
//! every ancestor frame, cross-site ones included), with one difference that is the reason
//! it is sent from Rust: the protocol's scroll is always instant, while the page's follows
//! the page's `scroll-behavior: smooth` and animates, moving the element under the stability
//! check and firing a scroll event per frame. The three extra messages are paid only on the
//! attempts whose rotation step is `ScrollPlan::Protocol`.

use browseros_cdp::CdpError;
use browseros_core::CoreError;
use serde_json::{Value, json};

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, ProbeFailure};
use crate::world::{IsolatedWorld, Pin};

/// Scrolls the element `pin` names into view if it is not, at once. An element that left its
/// document or has no box is not scrolled, and that is not an error: the measurement that
/// follows reports the detach or the missing box, as it would without the scroll.
pub(crate) async fn scroll_if_needed(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    pin: Pin,
) -> Result<(), ProbeFailure> {
    let Some(object) = probe::pinned_object(scope, page, world, pin).await? else {
        return Ok(());
    };
    match world
        .session()
        .send::<_, Value>("DOM.scrollIntoViewIfNeeded", json!({ "objectId": object }))
        .await
    {
        Ok(_) => Ok(()),
        Err(error) if unscrollable(&error) => Ok(()),
        Err(error) => Err(ProbeFailure::from_core(error)),
    }
}

/// The element's centre into the middle of its view at once, in its own scrollers and every
/// frame above, whatever their `scroll-behavior`: `scrollIntoView` with `block` and `inline` at
/// `center` and `behavior: 'instant'`, called on the pinned element in Neo's isolated world, so a
/// page's own override of the method is never reached. The private look's second step uses it
/// for a box larger than its view, which [`scroll_if_needed`] shows by its nearest edge
/// (protected-fill ruling R62). The protocol's call with a rectangle at the centre was measured
/// to leave that rectangle at the view's edge, under the scrollbar, where a hit finds nothing.
pub(crate) async fn centre_into_view(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    pin: Pin,
) -> Result<(), ProbeFailure> {
    let Some(object) = probe::pinned_object(scope, page, world, pin).await? else {
        return Ok(());
    };
    let params = json!({
        "objectId": object,
        "functionDeclaration": CENTRE_INTO_VIEW,
        "returnByValue": true,
        "objectGroup": scope.object_group(world.session()),
    });
    world
        .session()
        .send::<_, Value>("Runtime.callFunctionOn", params)
        .await
        .map(|_| ())
        .map_err(ProbeFailure::from_core)
}

const CENTRE_INTO_VIEW: &str = "function () { this.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' }) }";

/// The two refusals upstream turns into `error:notvisible` and `error:notconnected`
/// (crPage.ts `_scrollRectIntoViewIfNeeded`).
fn unscrollable(error: &CoreError) -> bool {
    let CoreError::Cdp(CdpError::Protocol { message, .. }) = error else {
        return false;
    };
    [
        "Node does not have a layout object",
        "Node is detached from document",
    ]
    .iter()
    .any(|refusal| message.contains(refusal))
}
