//! Browser-mapped pointer geometry for transformed iframe paths. DOM content quads already
//! include same-process ancestor transforms; only target/session boundaries need offsets.
//! This follows Chromium Playwright's getContentQuads and clickablePoint rather than trying
//! to invert a CSS transform in the page. The ordinary interceptor still checks actual input.

use browseros_cdp::CdpError;
use browseros_core::{CoreError, FrameId, ProtocolSession};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, Point, ProbeFailure};
use crate::resolve::Resolved;
use crate::world::{IsolatedWorld, Pin};

#[derive(Deserialize)]
struct ContentQuads {
    quads: Vec<[f64; 8]>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LayoutMetrics {
    css_layout_viewport: Viewport,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Viewport {
    client_width: f64,
    client_height: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameOwner {
    backend_node_id: u64,
}

#[derive(Deserialize)]
struct BoxModel {
    model: BorderBox,
}

#[derive(Deserialize)]
struct BorderBox {
    border: [f64; 8],
}

/// Element detachment is distinct from a lost world and must not retire the frame's probe.
pub(crate) enum Placement {
    At(Point),
    Outside,
    Detached,
}

/// The root-viewport click point Chromium computed for the pinned element. `offset` is an
/// explicit position's distance from the element's local border-box origin; without it,
/// upstream clips content quads to the viewport and selects the first substantial quad.
pub(crate) async fn point(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    located: &Resolved,
    pin: Pin,
    offset: Option<Point>,
) -> Result<Placement, ProbeFailure> {
    let Some(object) = probe::pinned_object(scope, page, located.world(), pin).await? else {
        return Ok(Placement::Detached);
    };
    let Some(origin) = target_origin(located).await? else {
        return Ok(Placement::Outside);
    };
    let session = located.world().session();
    let point = match offset {
        Some(offset) => positioned_point(session, &object, origin, offset).await?,
        None => clickable_point(page, session, &object, origin).await?,
    };
    Ok(point.map_or(Placement::Outside, Placement::At))
}

async fn positioned_point(
    session: &ProtocolSession,
    object: &str,
    origin: Point,
    offset: Point,
) -> Result<Option<Point>, ProbeFailure> {
    Ok(box_origin(session, json!({ "objectId": object }))
        .await?
        .map(|border| Point {
            x: border.x + origin.x + offset.x,
            y: border.y + origin.y + offset.y,
        }))
}

async fn clickable_point(
    page: &OpPage<'_>,
    session: &ProtocolSession,
    object: &str,
    origin: Point,
) -> Result<Option<Point>, ProbeFailure> {
    let Some(quads): Option<ContentQuads> = read_geometry(
        session,
        "DOM.getContentQuads",
        json!({ "objectId": object }),
    )
    .await?
    else {
        return Ok(None);
    };
    let metrics: LayoutMetrics = page
        .root_session()
        .send("Page.getLayoutMetrics", json!({}))
        .await
        .map_err(ProbeFailure::from_core)?;
    Ok(quads.quads.into_iter().find_map(|quad| {
        Quad::from(quad)
            .translated(origin)
            .clipped(metrics.css_layout_viewport)
            .middle()
    }))
}

/// CDP quads are relative to their target's viewport. Same-process frames are already
/// mapped by Chromium. At each OOPIF boundary, upstream adds the embedding element's
/// bounding-box origin in the parent target; walking outward composes those translations.
async fn target_origin(located: &Resolved) -> Result<Option<Point>, ProbeFailure> {
    let mut session = located.world().session_id();
    let mut frame = located.world().frame();
    let mut origin = Point { x: 0.0, y: 0.0 };
    for hop in located.path().hops().iter().rev() {
        if hop.parent.session_id() != session {
            let Some(at) = embedding_origin(&hop.parent, frame).await? else {
                return Ok(None);
            };
            origin.x += at.x;
            origin.y += at.y;
            session = hop.parent.session_id();
        }
        frame = hop.parent.frame();
    }
    Ok(Some(origin))
}

async fn embedding_origin(
    parent: &IsolatedWorld,
    frame: &FrameId,
) -> Result<Option<Point>, ProbeFailure> {
    let Some(owner): Option<FrameOwner> = read_geometry(
        parent.session(),
        "DOM.getFrameOwner",
        json!({ "frameId": frame.as_str() }),
    )
    .await?
    else {
        return Ok(None);
    };
    box_origin(
        parent.session(),
        json!({ "backendNodeId": owner.backend_node_id }),
    )
    .await
}

async fn box_origin(
    session: &ProtocolSession,
    params: Value,
) -> Result<Option<Point>, ProbeFailure> {
    let model: Option<BoxModel> = read_geometry(session, "DOM.getBoxModel", params).await?;
    Ok(model.map(|model| Quad::from(model.model.border).origin()))
}

/// A connected remote object may have lost its layout since measurement. Upstream's
/// _sendMayFail treats that as missing geometry; transport/session failures retain their cause.
async fn read_geometry<T: DeserializeOwned>(
    session: &ProtocolSession,
    method: &str,
    params: Value,
) -> Result<Option<T>, ProbeFailure> {
    match session.send(method, params).await {
        Ok(value) => Ok(Some(value)),
        Err(error) if missing_layout(&error) => Ok(None),
        Err(error) => Err(ProbeFailure::from_core(error)),
    }
}

fn missing_layout(error: &CoreError) -> bool {
    let CoreError::Cdp(CdpError::Protocol { message, .. }) = error else {
        return false;
    };
    [
        "Could not compute box model",
        "Could not compute content quads",
        "No node found for given backend id",
    ]
    .iter()
    .any(|missing| message.contains(missing))
}

/// Four corners in browser winding order, retained while clipping and finding their area.
struct Quad {
    points: [Point; 4],
}

impl From<[f64; 8]> for Quad {
    fn from([x1, y1, x2, y2, x3, y3, x4, y4]: [f64; 8]) -> Self {
        Self {
            points: [
                Point { x: x1, y: y1 },
                Point { x: x2, y: y2 },
                Point { x: x3, y: y3 },
                Point { x: x4, y: y4 },
            ],
        }
    }
}

impl Quad {
    fn origin(&self) -> Point {
        self.points.iter().fold(
            Point {
                x: f64::INFINITY,
                y: f64::INFINITY,
            },
            |at, point| Point {
                x: at.x.min(point.x),
                y: at.y.min(point.y),
            },
        )
    }

    fn translated(self, origin: Point) -> Self {
        Self {
            points: self.points.map(|point| Point {
                x: point.x + origin.x,
                y: point.y + origin.y,
            }),
        }
    }

    fn clipped(self, viewport: Viewport) -> Self {
        Self {
            points: self.points.map(|point| Point {
                x: point.x.clamp(0.0, viewport.client_width),
                y: point.y.clamp(0.0, viewport.client_height),
            }),
        }
    }

    fn middle(&self) -> Option<Point> {
        let area: f64 = self
            .points
            .iter()
            .zip(self.points.iter().cycle().skip(1))
            .map(|(a, b)| (a.x * b.y - b.x * a.y) / 2.0)
            .sum();
        // Upstream admits a 1x1 target despite fractional coordinate rounding.
        if area.abs() <= 0.99 {
            return None;
        }
        Some(
            self.points
                .iter()
                .fold(Point { x: 0.0, y: 0.0 }, |at, point| Point {
                    x: at.x + point.x / 4.0,
                    y: at.y + point.y / 4.0,
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::{Quad, Viewport};
    use crate::probe::Point;

    #[test]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the SDK test contract requires Result even for infallible geometry"
    )]
    fn clips_each_corner_before_choosing_a_viewport_point() -> Result<(), Box<dyn Error>> {
        let viewport = Viewport {
            client_width: 100.0,
            client_height: 100.0,
        };
        for (name, x, y, expected) in [
            ("above-left", -30.0, -30.0, None),
            ("below-right", 110.0, 110.0, None),
            ("left-only", -30.0, 10.0, None),
            ("partial", -10.0, -10.0, Some(Point { x: 5.0, y: 5.0 })),
            ("inside", 10.0, 10.0, Some(Point { x: 20.0, y: 20.0 })),
        ] {
            let point = Quad::from([x, y, x + 20.0, y, x + 20.0, y + 20.0, x, y + 20.0])
                .clipped(viewport)
                .middle();
            assert_eq!(
                point, expected,
                "{name} must use only the viewport intersection"
            );
            if let Some(point) = point {
                assert!(
                    point.x >= 0.0 && point.y >= 0.0,
                    "{name} must not dispatch outside the viewport: {point:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn takes_the_middle_of_a_rotated_quad_and_keeps_one_pixel_targets() {
        assert_eq!(
            Quad::from([10.0, 0.0, 20.0, 10.0, 10.0, 20.0, 0.0, 10.0]).middle(),
            Some(Point { x: 10.0, y: 10.0 })
        );
        assert_eq!(
            Quad::from([0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]).middle(),
            Some(Point { x: 0.5, y: 0.5 })
        );
    }
    #[test]
    fn missing_layout_is_retryable_but_transport_and_other_protocol_errors_are_not() {
        use browseros_cdp::CdpError;
        use browseros_core::CoreError;
        assert!(super::missing_layout(&CoreError::Cdp(CdpError::Protocol {
            code: -32000,
            message: "Could not compute box model.".to_owned()
        })));
        assert!(!super::missing_layout(&CoreError::Cdp(
            CdpError::ConnectionLost
        )));
        assert!(!super::missing_layout(&CoreError::Cdp(
            CdpError::Protocol {
                code: -32601,
                message: "method not found".to_owned()
            }
        )));
    }
}
