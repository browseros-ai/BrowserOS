//! The optional live cursor for one click. Its page-side animation never sends device input.

use std::time::Duration;

use browseros_core::ProtocolSession;
use browseros_core::input::Point as CorePoint;
use browseros_core::pointer::{Pointer, PointerAction};
use browseros_core::timeouts::POINTER_MOVE_DEADLINE;
use tokio::time::Instant;

use crate::input::Point;
use crate::op_scope::OpScope;
use crate::{ErrorCode, PlaywrightError};

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

    /// A successful glide gives the page time to move. Locator clicks must measure again.
    pub(crate) async fn glide(
        &mut self,
        scope: &OpScope<'_>,
        root: &ProtocolSession,
        at: Point,
    ) -> Result<bool, PlaywrightError> {
        // Give a moving target at most three glides and leave a second for the actual input.
        if !self.enabled
            || self.glides >= 3
            || self
                .last
                .is_some_and(|last| (last.x - at.x).hypot(last.y - at.y) <= 2.0)
        {
            return Ok(false);
        }
        scope.recheck()?;
        if scope.deadline().is_some_and(|end| {
            end.saturating_duration_since(Instant::now())
                < POINTER_MOVE_DEADLINE + Duration::from_secs(1)
        }) {
            return Ok(false);
        }
        let pointer = Pointer::default();
        let moved = scope
            .within(pointer.move_to(root, PointerAction::Click, at.into()))
            .await
            .map_err(|_expired| scope.timed_out(ErrorCode::NotActionable))?;
        if moved {
            self.last = Some(at);
            self.glides += 1;
        } else {
            self.enabled = false;
        }
        Ok(moved)
    }

    /// The caller has passed the final owner check. Snap any remaining distance before input.
    pub(crate) async fn ripple(
        &self,
        scope: &OpScope<'_>,
        root: &ProtocolSession,
        at: Point,
    ) -> Result<(), PlaywrightError> {
        if self.enabled && self.last.is_some() {
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
