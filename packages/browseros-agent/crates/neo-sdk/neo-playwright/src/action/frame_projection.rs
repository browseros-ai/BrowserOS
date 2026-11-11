//! Where a child frame's element sits in the main viewport, where all input goes, and whether
//! every parent frame lets the input through to it (upstream's `_checkFrameIsHitTarget`).
//!
//! ```text
//!  main ─ iframe₀ ─▶ child ─ iframe₁ ─▶ the element's frame, point p (its own pixels)
//!  round 1, all parents at once:  frameBox { iframe_k }            → origin_k (content box)
//!  round 2, all parents at once:  frameBox { iframe_k, point_k }   → hit_k
//!                                 point_k = p + origin_k + ... + origin_last
//!  page point = p + every origin
//! ```
//!
//! The levels are independent: scrolling a parent document never moves an iframe inside
//! another document, and the element's own scroll already scrolled every ancestor into place
//! (measured: `scrollIntoViewIfNeeded` inside a cross-site iframe scrolls the embedding page).

use futures_util::future;

use crate::op_scope::OpScope;
use crate::probe::{
    Hit, PageText, ParsedSelector, Point, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget,
};
use crate::resolve::{self, FramePath, Hop, HopIframe};
use crate::{ErrorCode, PlaywrightError};

/// Where the element's point lands in the page.
#[derive(Debug)]
pub(crate) enum Projection {
    /// The point in the main viewport's CSS pixels.
    At(Point),
    /// An element of a parent frame receives events at the point instead of the iframe:
    /// upstream's description of it (`<div id="overlay"></div>`), page text of the parent.
    Covered { by: PageText },
    /// A CSS transform on a parent's iframe makes the offset more than a translation.
    Transformed,
    /// An iframe on the way is gone: resolve the locator again.
    Gone,
}

/// Places `point`, in the pixels of the frame `path` leads to, in the main viewport; with
/// `hit_test`, also checks every parent's iframe receives events there, topmost first, as
/// upstream does. A parent world, or its iframe, that went away is [`Projection::Gone`].
pub(crate) async fn project(
    scope: &OpScope<'_>,
    path: &FramePath,
    point: Point,
    hit_test: bool,
) -> Result<Projection, PlaywrightError> {
    let broken = |why: &str| scope.fail(ErrorCode::Internal, why);
    let hops = path.hops();
    let boxes = future::join_all(hops.iter().map(|hop| frame_box(scope, hop, None))).await;
    let mut origins = Vec::with_capacity(hops.len());
    for placed in boxes {
        match placed? {
            Placed::Origin(origin) => origins.push(origin),
            Placed::NoOrigin => return Ok(Projection::Transformed),
            Placed::Hit(_) => return Err(broken("frameBox answered a hit test nobody asked for")),
            Placed::Gone => return Ok(Projection::Gone),
        }
    }
    // The point in each hop's parent: the element's point plus the origins of every iframe
    // from that parent down.
    let mut at_level = Vec::with_capacity(hops.len());
    let mut running = point;
    for origin in origins.iter().rev() {
        running = Point {
            x: running.x + origin.x,
            y: running.y + origin.y,
        };
        at_level.push(running);
    }
    at_level.reverse();
    let at_page = at_level.first().copied().unwrap_or(point);
    if !hit_test || hops.is_empty() {
        return Ok(Projection::At(at_page));
    }
    let hits = future::join_all(
        hops.iter()
            .zip(&at_level)
            .map(|(hop, at)| frame_box(scope, hop, Some(*at))),
    )
    .await;
    for hit in hits {
        match hit? {
            Placed::Hit(Hit::Done(_)) => {}
            Placed::Hit(Hit::Missed { by }) => return Ok(Projection::Covered { by }),
            Placed::Gone => return Ok(Projection::Gone),
            Placed::Origin(_) | Placed::NoOrigin => {
                return Err(broken("frameBox answered a hit test without its hit"));
            }
        }
    }
    Ok(Projection::At(at_page))
}

/// One parent's answer.
enum Placed {
    Origin(Point),
    NoOrigin,
    Hit(Hit),
    Gone,
}

async fn frame_box(
    scope: &OpScope<'_>,
    hop: &Hop,
    point: Option<Point>,
) -> Result<Placed, PlaywrightError> {
    let iframe = match &hop.iframe {
        HopIframe::Pin(pin) => ProbeTarget::Pin(*pin),
        HopIframe::Key(key) => {
            ProbeTarget::Chunk(ParsedSelector::frame_key(key).map_err(|error| {
                scope.fail(ErrorCode::Internal, &format!("a frame key: {error}"))
            })?)
        }
    };
    let request = ProbeRequest::FrameBox { iframe, point };
    Ok(match resolve::ask(&hop.parent, &request, None).await {
        Ok(ProbeReply::FrameBox { hit: Some(hit), .. }) if point.is_some() => Placed::Hit(hit),
        Ok(ProbeReply::FrameBox {
            origin: Some(origin),
            ..
        }) => Placed::Origin(origin),
        Ok(ProbeReply::FrameBox { origin: None, .. }) => Placed::NoOrigin,
        Ok(ProbeReply::None { .. } | ProbeReply::Detached | ProbeReply::Strict { .. })
        | Err(ProbeFailure::Gone(_)) => Placed::Gone,
        Ok(other) => {
            return Err(scope.fail(
                ErrorCode::Internal,
                &format!("frameBox answered `{}`", other.kind()),
            ));
        }
        Err(failure) => return Err(resolve::failed(scope, failure)),
    })
}

/// Upstream's `roundPoint`: two decimals, toward zero, as the page point is sent.
pub(crate) fn round_point(point: Point) -> Point {
    let round = |value: f64| (value * 100.0).trunc() / 100.0;
    Point {
        x: round(point.x),
        y: round(point.y),
    }
}
