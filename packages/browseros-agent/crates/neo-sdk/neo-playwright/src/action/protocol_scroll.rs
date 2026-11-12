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
use crate::probe::{self, ProbeFailure};
use crate::world::{IsolatedWorld, Pin};

/// Scrolls the element `pin` names into view if it is not, at once. An element that left its
/// document or has no box is not scrolled, and that is not an error: the measurement that
/// follows reports the detach or the missing box, as it would without the scroll.
pub(super) async fn scroll_if_needed(
    scope: &OpScope<'_>,
    world: &IsolatedWorld,
    pin: Pin,
) -> Result<(), ProbeFailure> {
    let Some(object) = probe::pinned_object(scope, world, pin).await? else {
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
