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
use crate::page::OpPage;
use crate::probe::{
    CoverMark, Hit, PageText, ParsedSelector, Point, ProbeFailure, ProbeReply, ProbeRequest,
    ProbeTarget,
};
use crate::resolve::{self, FramePath, Hop, HopIframe};
use crate::world::IsolatedWorld;
use crate::{ErrorCode, PlaywrightError};

/// Who asks the parents. A click keeps upstream's frame rules. The private look asks with ruling
/// R60's: a frame that is only shifted is placed by its corner where upstream places none, and a
/// point that hits nothing in a parent, or hits what holds the iframe, is clear where upstream
/// misses. The page side's `frameBox` holds both (its `look` flag).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Asker {
    Click,
    Look,
}

/// Where the element's point lands in the page.
#[derive(Debug)]
pub(crate) enum Projection {
    /// The point in the main viewport's CSS pixels (`page`), and in each hop's parent from the
    /// main frame down (`levels`, `levels[0]` being `page`; empty in the main frame), for
    /// [`own_session_point`].
    At { page: Point, levels: Vec<Point> },
    /// An element of a parent frame receives events at the point instead of the iframe:
    /// upstream's description of it (`<div id="overlay"></div>`), page text of the parent;
    /// which cover it is, for a click's count; and where it was seen, for its facts: the
    /// parent's index among the path's hops, from the main frame down, and the point there.
    Covered {
        by: PageText,
        cover: Option<CoverMark>,
        hop: usize,
        at: Point,
    },
    /// A CSS transform requires browser-mapped quads rather than an additive offset. For the
    /// look, only a transform that changes the frame's size or angle.
    Transformed,
    /// An iframe on the way is gone: resolve the locator again.
    Gone,
    /// For the look only: a page above cannot show the point, after the box was scrolled into
    /// view at once in every page (`frameBox`'s `unseen`).
    Unseen,
}

/// Places `point`, in the pixels of the frame `path` leads to, in the main viewport; with
/// `hit_test`, also checks every parent's iframe receives events there, topmost first, as
/// upstream does. A parent world, or its iframe, that went away is [`Projection::Gone`].
pub(crate) async fn project(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    path: &FramePath,
    point: Point,
    hit_test: bool,
    asker: Asker,
) -> Result<Projection, PlaywrightError> {
    let broken = |why: &str| scope.fail(ErrorCode::Internal, why);
    let hops = path.hops();
    let boxes = future::join_all(
        hops.iter()
            .map(|hop| frame_box(scope, page, hop, None, asker)),
    )
    .await;
    let mut origins = Vec::with_capacity(hops.len());
    for placed in boxes {
        match placed? {
            Placed::Origin(origin) => origins.push(origin),
            Placed::NoOrigin => return Ok(Projection::Transformed),
            Placed::Hit(..) | Placed::Unseen => {
                return Err(broken("frameBox answered a hit test nobody asked for"));
            }
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
        return Ok(Projection::At {
            page: at_page,
            levels: at_level,
        });
    }
    let hits = future::join_all(
        hops.iter()
            .zip(&at_level)
            .map(|(hop, at)| frame_box(scope, page, hop, Some(*at), asker)),
    )
    .await;
    for (index, hit) in hits.into_iter().enumerate() {
        match hit? {
            Placed::Hit(Hit::Done(_), _) => {}
            Placed::Hit(Hit::Missed { by }, cover) => {
                let at = at_level.get(index).copied().unwrap_or(point);
                return Ok(Projection::Covered {
                    by,
                    cover,
                    hop: index,
                    at,
                });
            }
            Placed::Unseen => return Ok(Projection::Unseen),
            Placed::Gone => return Ok(Projection::Gone),
            Placed::Origin(_) | Placed::NoOrigin => {
                return Err(broken("frameBox answered a hit test without its hit"));
            }
        }
    }
    Ok(Projection::At {
        page: at_page,
        levels: at_level,
    })
}

/// Where the element's `point` (in its own frame's pixels) sits in the topmost frame of its
/// world's session: the out-of-process frame whose own CDP session carries input straight into
/// it. A same-process frame below that frame shares its widget, so its origins are added; the
/// frames above, in other processes, are left out. `levels` is [`Projection::At`]'s.
pub(crate) fn own_session_point(
    path: &FramePath,
    world: &IsolatedWorld,
    point: Point,
    levels: &[Point],
) -> Point {
    let session = world.session_id();
    let hops = path.hops();
    // Frame i (i < hops.len()) is hop i's parent; the element's frame is the last.
    let mut top = hops.len();
    while top > 0
        && hops
            .get(top - 1)
            .is_some_and(|hop| hop.parent.session_id() == session)
    {
        top -= 1;
    }
    levels.get(top).copied().unwrap_or(point)
}

/// One parent's answer.
enum Placed {
    Origin(Point),
    NoOrigin,
    /// The hit test's answer, and on a click's miss which cover took the point.
    Hit(Hit, Option<CoverMark>),
    /// The look's hit test found the point where this page cannot show it.
    Unseen,
    Gone,
}

async fn frame_box(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    hop: &Hop,
    point: Option<Point>,
    asker: Asker,
) -> Result<Placed, PlaywrightError> {
    let request = ProbeRequest::FrameBox {
        iframe: iframe_target(scope, hop)?,
        point,
        look: asker == Asker::Look,
    };
    Ok(
        match resolve::ask(scope, page, &hop.parent, &request, None).await {
            Ok(ProbeReply::FrameBox {
                hit: Some(_),
                unseen: true,
                ..
            }) if point.is_some() => Placed::Unseen,
            Ok(ProbeReply::FrameBox {
                hit: Some(hit),
                cover,
                ..
            }) if point.is_some() => Placed::Hit(hit, cover),
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
        },
    )
}

/// A hop's `<iframe>`, as its parent's world finds it.
pub(crate) fn iframe_target(
    scope: &OpScope<'_>,
    hop: &Hop,
) -> Result<ProbeTarget, PlaywrightError> {
    Ok(match &hop.iframe {
        HopIframe::Pin(pin) => ProbeTarget::Pin(*pin),
        HopIframe::Key(key) => {
            ProbeTarget::Chunk(ParsedSelector::frame_key(key).map_err(|error| {
                scope.fail(ErrorCode::Internal, &format!("a frame key: {error}"))
            })?)
        }
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
