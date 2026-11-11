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
    project(scope, path, local, Projection::BoundingBox).await
}

/// Screenshot locators must retry the entire preparation when an ancestor disappears. Keep
/// this distinct from a hidden frame, which has no capturable layout rather than a new node.
pub(crate) async fn for_screenshot(
    scope: &OpScope<'_>,
    path: &FramePath,
    local: Value,
) -> Result<Value, PlaywrightError> {
    project(scope, path, local, Projection::Screenshot).await
}

#[derive(Clone, Copy)]
enum Projection {
    BoundingBox,
    Screenshot,
}

enum OriginState {
    Present(FrameOrigin),
    Hidden,
    Detached,
}

async fn project(
    scope: &OpScope<'_>,
    path: &FramePath,
    local: Value,
    projection: Projection,
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
            OriginState::Present(FrameOrigin::Placed {
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
            OriginState::Present(FrameOrigin::Rotated(Rotated::Rotated)) => {
                return Err(scope.fail(
                    ErrorCode::Unsupported,
                    "the element is inside a rotated, skewed or flipped iframe, which has no box \
                     in page coordinates in Neo",
                ));
            }
            OriginState::Hidden => return Ok(Value::Null),
            OriginState::Detached => {
                return match projection {
                    Projection::BoundingBox => Ok(Value::Null),
                    Projection::Screenshot => Err(resolve::handle_detached(scope)),
                };
            }
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

/// Keep a missing node distinct from an existing iframe without a layout box.
async fn frame_origin(scope: &OpScope<'_>, hop: &Hop) -> Result<OriginState, PlaywrightError> {
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
        Ok(ProbeReply::Read { value, .. }) => {
            let origin: Option<FrameOrigin> = element_read::read_value(scope, value.into_value())?;
            Ok(origin.map_or(OriginState::Hidden, OriginState::Present))
        }
        Ok(ProbeReply::None { .. } | ProbeReply::Detached) | Err(ProbeFailure::Gone(_)) => {
            Ok(OriginState::Detached)
        }
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

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use browseros_core::testing::TestConnection;
    use browseros_core::{CdpConnection, ProtocolSession, SessionId};
    use serde_json::{Value, json};

    use super::{for_screenshot, in_main_viewport};
    use crate::op_scope::OpScope;
    use crate::resolve::{FramePath, Hop, HopIframe};
    use crate::target::CallHeader;
    use crate::test_env::TestEnv;
    use crate::world::SealedWorld;
    use crate::{ErrorCode, Op};

    /// A vanished ancestor is retryable only for screenshots. boundingBox keeps its public
    /// null contract, and an existing hidden iframe remains null for both callers.
    #[tokio::test]
    async fn ancestor_detachment_is_distinct_from_hidden_layout() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let op = Op::from_name("page.screenshot").ok_or("screenshot op missing")?;
        let scope = OpScope::new(&env, op, CallHeader::default());
        for detached in [false, true] {
            let connection = TestConnection::new([]);
            let reply = if detached {
                json!({ "kind": "detached" })
            } else {
                json!({ "kind": "read", "value": null, "sensitive": null, "sweep": [] })
            };
            connection.always(
                "Runtime.callFunctionOn",
                json!({ "result": { "type": "object", "value": reply } }),
            )?;
            let parent = SealedWorld::for_tests(
                ProtocolSession::for_session(
                    Arc::clone(&connection) as Arc<dyn CdpConnection>,
                    SessionId::from("parent"),
                ),
                "parent",
                0,
                "global",
            );
            let mut path = FramePath::default();
            path.push(Hop {
                parent,
                iframe: HopIframe::Key("#iframe".to_owned()),
            });
            let local = json!({ "x": 1, "y": 2, "width": 3, "height": 4 });
            assert_eq!(
                in_main_viewport(&scope, &path, local.clone()).await?,
                Value::Null
            );
            let screenshot = for_screenshot(&scope, &path, local).await;
            if detached {
                assert_eq!(
                    screenshot
                        .err()
                        .ok_or("missing detachment error")?
                        .error_code(),
                    Some(ErrorCode::ElementDetached)
                );
            } else {
                assert_eq!(screenshot?, Value::Null);
            }
        }
        Ok(())
    }
}
