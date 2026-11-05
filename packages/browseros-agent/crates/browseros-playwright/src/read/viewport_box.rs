//! `boundingBox` in the main frame's viewport, for an element in any frame.
//!
//! The probe measures a box in its own frame's viewport. Each parent frame on the way down
//! reports where its `<iframe>`'s document starts (its content box, past border and padding)
//! and how much a scale on the iframe or its ancestors enlarges it, and composing those, top
//! down, moves the box into the main viewport. That works the same for a same-process child
//! and an out-of-process one, because each measurement stays inside one document. It is also
//! where the page's root session dispatches input, so a click at the box's center lands on the
//! element. A rotated, skewed or flipped iframe has no axis-aligned box: `neo / unsupported`.
//!
//! Upstream reads `DOM.getBoxModel` on the frame's own session and adds the frame element's
//! border box for an out-of-process frame, which leaves out that iframe's border and padding
//! and every transform. Neo adds them; the two agree for same-process frames and for
//! borderless, untransformed iframes.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::op_scope::OpScope;
use crate::probe::{self, Point, ProbeFailure, ProbeReply, ProbeRequest, Read, Rect};
use crate::read::element_read;
use crate::resolve::{self, FramePath, Hop};
use crate::target::Strictness;
use crate::{ErrorCode, PlaywrightError};

/// The box `local` (a `Rect` in the element's own frame, or null) in the main viewport. Null
/// when the element has no layout box, and when a frame on the way is no longer rendered or no
/// longer there, as upstream returns null when it cannot place the frame.
pub(crate) async fn in_main_viewport(
    scope: &OpScope<'_>,
    path: &FramePath,
    local: Value,
) -> Result<Value, PlaywrightError> {
    let Some(rect) = element_read::read_value::<Option<Rect>>(scope, local)? else {
        return Ok(Value::Null);
    };
    // A point `p` of the innermost frame is at `origin + scale * p` in the main viewport.
    let mut origin = Point { x: 0.0, y: 0.0 };
    let mut scale = Point { x: 1.0, y: 1.0 };
    // In order, top down. The hops are independent, but a path is a handful of frames and each
    // call is one local round trip, which upstream's own recursion also pays.
    for hop in path.hops() {
        match frame_origin(scope, hop).await? {
            Some(FrameOrigin::Placed {
                x,
                y,
                scale_x,
                scale_y,
            }) => {
                origin.x += scale.x * x;
                origin.y += scale.y * y;
                scale.x *= scale_x;
                scale.y *= scale_y;
            }
            Some(FrameOrigin::Rotated(Rotated::Rotated)) => {
                return Err(scope.fail(
                    ErrorCode::Unsupported,
                    "the element is inside a rotated, skewed or flipped iframe, which has no box \
                     in page coordinates in Neo",
                ));
            }
            None => return Ok(Value::Null),
        }
    }
    Ok(json!(Rect {
        x: origin.x + scale.x * rect.x,
        y: origin.y + scale.y * rect.y,
        width: scale.x * rect.width,
        height: scale.y * rect.height,
    }))
}

/// A `frameOrigin` read's value, as protocol.ts's `FrameOrigin` types it.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FrameOrigin {
    /// Where the iframe's document starts in the parent's viewport, and its scale there.
    #[serde(rename_all = "camelCase")]
    Placed {
        x: f64,
        y: f64,
        scale_x: f64,
        scale_y: f64,
    },
    /// No axis-aligned box: a rotate, skew, flip or 3D transform.
    Rotated(Rotated),
}

/// The string `"rotated"`.
#[derive(Debug, Deserialize)]
enum Rotated {
    #[serde(rename = "rotated")]
    Rotated,
}

/// Where `hop`'s iframe's document starts in the parent's viewport; `None` when the iframe is
/// not rendered or has gone.
async fn frame_origin(
    scope: &OpScope<'_>,
    hop: &Hop,
) -> Result<Option<FrameOrigin>, PlaywrightError> {
    let iframe = hop
        .iframe
        .target()
        .map_err(|error| scope.fail(ErrorCode::Internal, &error.to_string()))?;
    let request = ProbeRequest::Read {
        target: iframe,
        strictness: Strictness::Strict,
        frame_visible: true,
        read: Read::FrameOrigin,
    };
    match probe::call(&hop.parent, &request).await {
        Ok(ProbeReply::Read { value, .. }) => element_read::read_value(scope, value.textless()),
        Ok(ProbeReply::None { .. } | ProbeReply::Detached) | Err(ProbeFailure::Gone) => Ok(None),
        Ok(other) => Err(scope.fail(
            ErrorCode::Internal,
            &format!(
                "the probe answered `{}` to a frame origin read",
                other.kind()
            ),
        )),
        Err(ProbeFailure::Cdp(error)) => Err(scope.core_error(&error)),
        Err(ProbeFailure::Broken(why)) => Err(scope.fail(ErrorCode::Internal, &why)),
        Err(failure @ ProbeFailure::Late) => Err(resolve::failed(scope, failure)),
    }
}
