//! Neo: Element preparation shared by pointer input and page-function drops.
//! All element waits finish before taking the tab's input turn. The caller then owns that
//! turn and its interceptor through the input acknowledgement and hit-target verdict.

use crate::playwright_methods::tab::{Arming, Geometry, NotReady, Scroll, StatesCheck, Stop, Tab};
use crate::progress::Progress;
use neo_playwright_protocol::injected::{ElementState, HitTargetAction, Point};

/// Which element states a caller needs before it can perform its action.
/// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Actionability {
    Enabled,
    Stable,
}

/// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`
impl Actionability {
    fn states(self) -> &'static [ElementState] {
        match self {
            Self::Enabled => &CLICK_STATES,
            Self::Stable => &HOVER_STATES,
        }
    }
    fn states_text(self) -> &'static str {
        match self {
            Self::Enabled => "visible, enabled and stable",
            Self::Stable => "visible and stable",
        }
    }
}

/// Only the element-preparation policy, independent of the input the caller will send.
/// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`
pub(crate) struct Aim<'a> {
    pub(crate) actionability: Actionability,
    pub(crate) hit_target: HitTargetAction,
    pub(crate) position: Option<&'a Point>,
    pub(crate) scroll: bool,
    pub(crate) force: bool,
}

/// A ready caller owns the input turn and must keep its interceptor alive until the input
/// it judges is acknowledged. Only a forced action has no interceptor. A refusal drops both
/// before another attempt can begin.
/// Neo: explicit ownership of the shared input turn and interceptor across preparation.
pub(crate) enum Readiness<T: Tab> {
    Ready {
        point: Point,
        input: T::Input,
        armed: Option<T::Armed>,
    },
    NotReady(NotReady),
}

/// The action's waits, scroll, point and hit check, in Playwright's order. No input is sent
/// here; the returned turn keeps another action from replacing this action's interceptor.
/// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`
pub(crate) async fn ready_to_perform<T: Tab>(
    tab: &T,
    element: &T::Element,
    aim: &Aim<'_>,
    retry: usize,
    progress: &Progress,
) -> Result<Readiness<T>, Stop> {
    if !aim.force {
        progress.log(format!(
            "  waiting for element to be {}",
            aim.actionability.states_text()
        ));
        match progress
            .race(tab.check_states(element, aim.actionability.states(), aim.scroll))
            .await??
        {
            StatesCheck::Ready => {}
            StatesCheck::NotConnected => return Ok(Readiness::NotReady(NotReady::NotConnected)),
            StatesCheck::Missing(state) => {
                return Ok(Readiness::NotReady(NotReady::Missing(state)));
            }
        }
        progress.log(format!("  element is {}", aim.actionability.states_text()));
    }

    progress.log("  scrolling into view if needed");
    if aim.scroll {
        let scroll = SCROLLS[retry % SCROLLS.len()];
        let scrolled = progress
            .race(tab.scroll(element, scroll, aim.position))
            .await??;
        if let Some(reason) = scrolled {
            return Ok(Readiness::NotReady(reason));
        }
    }
    progress.log("  done scrolling");

    let geometry = progress
        .race(tab.geometry(element, aim.position, aim.force))
        .await??;
    let point = match click_point(&geometry, aim.position) {
        Ok(point) => round_point(&point),
        Err(reason) => return Ok(Readiness::NotReady(reason)),
    };

    if aim.force {
        progress.log("  forcing action");
    }
    // The input phase runs from here, after every wait for the element, to the verdict. Inside
    // it the preliminary hit check, the input and the stop all see this action's own
    // interceptor.
    let input = progress.race(tab.input()).await?;
    let armed = if aim.force {
        None
    } else {
        match progress
            .race(tab.arm(element, point.clone(), aim.hit_target, input.clone()))
            .await??
        {
            Arming::Armed(armed) => Some(armed),
            Arming::NotConnected => return Ok(Readiness::NotReady(NotReady::NotConnected)),
            Arming::Intercepted(target) => {
                return Ok(Readiness::NotReady(NotReady::Intercepted(target)));
            }
        }
    };
    Ok(Readiness::Ready {
        point,
        input,
        armed,
    })
}

/// Protocol scrolling comes first; retries cycle through Playwright's three alignments.
/// Playwright: `server/dom.ts` `ElementHandle._retryPointerAction`: `undefined,`
/// Playwright: `server/dom.ts` `ElementHandle._retryPointerAction`: `{ block: 'end', inline: 'end' },`
/// Playwright: `server/dom.ts` `ElementHandle._retryPointerAction`: `{ block: 'center', inline: 'center' },`
/// Playwright: `server/dom.ts` `ElementHandle._retryPointerAction`: `{ block: 'start', inline: 'start' },`
pub(crate) const SCROLLS: [Scroll; 4] =
    [Scroll::Protocol, Scroll::End, Scroll::Center, Scroll::Start];

/// The enabled actionability policy, in Playwright's order.
/// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `['visible', 'enabled', 'stable', 'non-inert']`
pub(super) const CLICK_STATES: [ElementState; 4] = [
    ElementState::Visible,
    ElementState::Enabled,
    ElementState::Stable,
    ElementState::NonInert,
];
/// Hover, drag and drop wait for stability without requiring an enabled element.
/// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `['visible', 'stable']`
pub(crate) const HOVER_STATES: [ElementState; 2] = [ElementState::Visible, ElementState::Stable];
/// Playwright: `server/dom.ts` `ElementHandle._clickablePoint`
/// Playwright's point from raw geometry (`_clickablePoint`, `_offsetPoint`): clip
/// each quad to the viewport, keep quads whose area is over 0.99, take the middle of the first
/// survivor; no quads → `NotVisible`, none left → `NotInViewport`. For `Offset`, the border
/// box's top-left plus the border widths plus `position`.
pub(crate) fn click_point(
    geometry: &Geometry,
    position: Option<&Point>,
) -> Result<Point, NotReady> {
    match geometry {
        Geometry::NoQuads => Err(NotReady::NotVisible),
        Geometry::Quads { quads, .. } if quads.is_empty() => Err(NotReady::NotVisible),
        Geometry::Quads {
            quads,
            viewport: (width, height),
        } => quads
            .iter()
            .map(|quad| {
                let mut clipped = *quad;
                for (index, value) in clipped.iter_mut().enumerate() {
                    let bound = if index % 2 == 0 { *width } else { *height };
                    *value = value.max(0.0).min(bound);
                }
                clipped
            })
            .find(|quad| quad_area(quad) > 0.99)
            .map(|quad| quad_middle(&quad))
            .ok_or(NotReady::NotInViewport),
        Geometry::Offset {
            border_box,
            border: (left, top),
        } => {
            let (offset_x, offset_y) = position.map_or((0.0, 0.0), |point| (point.x, point.y));
            let x = [border_box[0], border_box[2], border_box[4], border_box[6]]
                .into_iter()
                .fold(f64::INFINITY, f64::min);
            let y = [border_box[1], border_box[3], border_box[5], border_box[7]]
                .into_iter()
                .fold(f64::INFINITY, f64::min);
            Ok(Point {
                x: x + left + offset_x,
                y: y + top + offset_y,
            })
        }
    }
}

/// Playwright: `server/dom.ts` `ElementHandle._clickablePoint`
/// `computeQuadArea`: the shoelace sum over the four corners, in Playwright's order.
fn quad_area(quad: &[f64; 8]) -> f64 {
    let mut area = 0.0;
    for corner in 0..4 {
        let next = (corner + 1) % 4;
        let (x1, y1) = (quad[corner * 2], quad[corner * 2 + 1]);
        let (x2, y2) = (quad[next * 2], quad[next * 2 + 1]);
        area += (x1 * y2 - x2 * y1) / 2.0;
    }
    area.abs()
}

/// Playwright: `server/dom.ts` `ElementHandle._clickablePoint`
/// `quadMiddlePoint`: each corner's quarter summed in order, so the float result is
/// Playwright's to the last bit.
fn quad_middle(quad: &[f64; 8]) -> Point {
    let mut point = Point { x: 0.0, y: 0.0 };
    for corner in 0..4 {
        point.x += quad[corner * 2] / 4.0;
        point.y += quad[corner * 2 + 1] / 4.0;
    }
    point
}

/// Playwright: `server/dom.ts` `roundPoint`
/// `roundPoint`: truncation to two decimals, `(x * 100 | 0) / 100`.
pub(crate) fn round_point(point: &Point) -> Point {
    Point {
        x: f64::from(to_int32(point.x * 100.0)) / 100.0,
        y: f64::from(to_int32(point.y * 100.0)) / 100.0,
    }
}

/// Playwright: `server/dom.ts` `roundPoint`
/// JS's `ToInt32`, which `| 0` applies: truncate, wrap modulo 2^32, read as signed.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "ToInt32 is defined as exactly this wrap; the value is in [0, 2^32) before the casts"
)]
fn to_int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let wrapped = value.trunc().rem_euclid(4_294_967_296.0);
    (wrapped as u32) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playwright_methods::tab::fake::OK_QUAD;
    #[test]
    fn the_click_point_is_the_middle_of_the_visible_part() {
        let quads = |quads: Vec<[f64; 8]>| Geometry::Quads {
            quads,
            viewport: (516.0, 475.0),
        };
        assert_eq!(
            click_point(&quads(vec![OK_QUAD]), None),
            Ok(Point { x: 36.75, y: 18.5 }),
            "the middle of a quad inside the viewport"
        );
        // Half the button hangs off the left edge: the clipped quad's middle.
        assert_eq!(
            click_point(
                &quads(vec![[-20.0, 8.0, 20.0, 8.0, 20.0, 28.0, -20.0, 28.0]]),
                None
            ),
            Ok(Point { x: 10.0, y: 18.0 })
        );
        assert_eq!(
            click_point(
                &quads(vec![[600.0, 8.0, 700.0, 8.0, 700.0, 28.0, 600.0, 28.0]]),
                None
            ),
            Err(NotReady::NotInViewport)
        );
        assert_eq!(
            click_point(&quads(vec![[8.0, 8.0, 9.0, 8.0, 9.0, 8.5, 8.0, 8.5]]), None),
            Err(NotReady::NotInViewport),
            "a 0.5 px² quad is under Playwright's 0.99 floor"
        );
        assert_eq!(
            click_point(&quads(Vec::new()), None),
            Err(NotReady::NotVisible)
        );
        assert_eq!(
            click_point(&Geometry::NoQuads, None),
            Err(NotReady::NotVisible)
        );
        assert_eq!(
            click_point(
                &Geometry::Offset {
                    border_box: OK_QUAD,
                    border: (2.0, 3.0),
                },
                Some(&Point { x: 5.0, y: 6.0 })
            ),
            Ok(Point { x: 15.0, y: 17.0 })
        );
    }
}
