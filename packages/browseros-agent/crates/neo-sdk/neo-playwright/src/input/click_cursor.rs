//! The optional live cursor for one click. Its page-side animation never sends device input.

use std::time::Duration;

use browseros_core::ProtocolSession;
use browseros_core::input::Point as CorePoint;
use browseros_core::pointer::{Pointer, PointerAction};
use browseros_core::screenshot::acquire_screenshot_lock;
use browseros_core::timeouts::POINTER_MOVE_DEADLINE;
use tokio::time::{self, Instant};

use crate::input::Point;
use crate::op_scope::OpScope;
use crate::{ErrorCode, PlaywrightError};

const MAX_SCREENSHOT_WAIT: Duration = Duration::from_secs(3);
const GLIDE_AND_INPUT_BUDGET: Duration =
    POINTER_MOVE_DEADLINE.saturating_add(Duration::from_secs(1));

/// Remember travel across retries, including a detached locator resolving again.
#[derive(Default)]
pub(crate) struct ClickCursor {
    enabled: bool,
    last: Option<Point>,
    glides: u8,
}

impl ClickCursor {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Self::default()
        }
    }

    /// Leave a second for input and bound the number of remeasurements on moving targets.
    pub(crate) fn needs_glide(&self, scope: &OpScope<'_>, at: Point) -> bool {
        self.enabled
            && self.glides < 3
            && self
                .last
                .is_none_or(|last| (last.x - at.x).hypot(last.y - at.y) > 2.0)
            && scope.deadline().is_none_or(|end| {
                end.saturating_duration_since(Instant::now()) >= GLIDE_AND_INPUT_BUDGET
            })
    }

    /// A screenshot may already have hidden an absent cursor. Wait until it has read its
    /// pixels before creating the canvas. Leave the move its full deadline and a second for input.
    pub(crate) async fn glide(
        &mut self,
        scope: &OpScope<'_>,
        root: &ProtocolSession,
        at: Point,
    ) -> Result<(), PlaywrightError> {
        let wait_limit = Instant::now() + MAX_SCREENSHOT_WAIT;
        let latest_start = scope.deadline().map_or(wait_limit, |end| {
            wait_limit.min(end - GLIDE_AND_INPUT_BUDGET)
        });
        let paint = async {
            let Ok(_capture) = time::timeout_at(latest_start, acquire_screenshot_lock(root)).await
            else {
                return Ok(false);
            };
            // A ready lock can win the timeout even if this task was scheduled late.
            if Instant::now() > latest_start {
                return Ok(false);
            }
            scope.recheck()?;
            let moved = Pointer::default()
                .move_to(root, PointerAction::Click, at.into())
                .await;
            Ok(moved)
        };
        let moved: Result<bool, PlaywrightError> = scope
            .within(paint)
            .await
            .map_err(|_expired| scope.timed_out(ErrorCode::NotActionable))?;
        if moved? {
            self.last = Some(at);
            self.glides += 1;
        } else {
            self.enabled = false;
        }
        Ok(())
    }

    /// Recheck ownership before painting. The caller marks delivery only after this returns.
    pub(crate) async fn ripple(
        &self,
        scope: &OpScope<'_>,
        root: &ProtocolSession,
        at: Point,
    ) -> Result<(), PlaywrightError> {
        if self.enabled && self.last.is_some() {
            scope.recheck()?;
            let pointer = Pointer::default();
            // Paint failures are best effort. If the operation itself ended, send no input.
            scope
                .within(pointer.click_effect(root, at.into()))
                .await
                .map_err(|_expired| scope.timed_out(ErrorCode::NotActionable))?;
        }
        Ok(())
    }
}

impl From<Point> for CorePoint {
    fn from(point: Point) -> Self {
        Self {
            x: point.x,
            y: point.y,
        }
    }
}

#[cfg(test)]
mod tests;
