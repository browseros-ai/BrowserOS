//! Carries out a surface plan against the browser and the screen, one step at a time, in the
//! plan's order. Each browser step has its own deadline; the screen resize has its own 2 s.
//!
//! A claim stops at a failed screen resize, which is its error: the owner would see the wrong
//! size. Any other failed step is logged and the plan goes on, and the plan reports itself as
//! applied partly. So a release always reaches its last step, and the agent always gets its
//! tab back as far as the browser allows.

use crate::limits::STEP_TIMEOUT;
use crate::screen::Screen;
use crate::surface::{Plan, Step};
use crate::{BrowserTabs, Fitted, PartError};
use std::future::Future;

/// Why a plan runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlanFor {
    /// A fit claims the surface: a failed screen resize ends the plan and fails the claim.
    Claim,
    /// The screen changed, or a fit ends: every step runs, whatever failed before it.
    Change,
}

/// Runs `plan`. `Partial` when any step failed.
///
/// # Errors
/// For a claim, the screen resize's error; nothing else fails a plan.
pub(crate) async fn run(
    plan: Plan,
    browser: &BrowserTabs,
    screen: &Screen,
    purpose: PlanFor,
) -> Result<Fitted, PartError> {
    let mut fitted = Fitted::Full;
    for step in plan.0 {
        let outcome = match &step {
            Step::Screen(size) => screen.resize(*size).await,
            Step::Activate(tab) => bounded(browser.activate(*tab)).await,
            Step::ResetZoom(tab) => bounded(browser.reset_zoom(*tab)).await,
            Step::Window(window, state) => bounded(browser.set_window_state(*window, *state)).await,
            Step::Viewport(tab, request) => bounded(browser.set_viewport(*tab, *request)).await,
        };
        let Err(error) = outcome else {
            continue;
        };
        if purpose == PlanFor::Claim && matches!(step, Step::Screen(_)) {
            return Err(error);
        }
        // A tab that closed fails its own steps; that is expected when its fit ends with it.
        if matches!(error, PartError::TabNotFound(_)) {
            tracing::debug!(?step, %error, "a fit step found its tab closed");
        } else {
            tracing::warn!(?step, %error, "a fit step failed; the plan goes on");
        }
        fitted = Fitted::Partial;
    }
    Ok(fitted)
}

async fn bounded(step: impl Future<Output = Result<(), PartError>>) -> Result<(), PartError> {
    tokio::time::timeout(STEP_TIMEOUT, step)
        .await
        .unwrap_or(Err(PartError::TimedOut("a fit step")))
}
