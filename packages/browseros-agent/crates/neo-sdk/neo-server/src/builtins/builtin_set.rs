//! The one list of built-ins, and the one order of the door's rules.

use std::sync::Arc;

use neo_captcha::CaptchaSolver;

use super::{CaptchaClaims, PlaywrightFacade, host_fill, neo_tasks, playwright_ops, solve_captcha};
use crate::dispatch::{Policy, RegistryBuilder, RegistryError};
use crate::host::TakeoverLock;

/// What the built-ins need from startup. A plain data bag.
pub(crate) struct BuiltinDeps {
    /// The process's one CAPTCHA solver; `None` without a provider key, and then
    /// `neo.solveCaptcha` is registered but unlisted and fails `host / captcha_unavailable`.
    pub(crate) solver: Option<Arc<CaptchaSolver>>,
    /// The tabs solves hold, which the door's CAPTCHA busy rule reads too.
    pub(crate) claims: Arc<CaptchaClaims>,
    /// neo-playwright's facade, which every Playwright op runs on.
    pub(crate) playwright: Arc<PlaywrightFacade>,
}

/// Adds every built-in to `builder`. Called once at startup; a duplicate name fails the start.
pub(crate) fn builtin_set(
    builder: &mut RegistryBuilder,
    deps: BuiltinDeps,
) -> Result<(), RegistryError> {
    let BuiltinDeps {
        solver,
        claims,
        playwright,
    } = deps;
    neo_tasks::neo_tasks(builder)?;
    playwright_ops::playwright_ops(builder, &playwright)?;
    host_fill::host_fill(builder, &playwright)?;
    solve_captcha::solve_captcha(builder, solver, claims)
}

/// The dispatcher's rules, in their fixed order: the takeover lock, then the CAPTCHA busy rule.
/// An owner's lock outranks a solve's claim when both would refuse.
///
/// app.rs builds the door with these, and so does the built-ins' test harness, so a test of how
/// two rules interact runs the order production runs.
#[must_use]
pub(crate) fn door_rules(
    takeover: &Arc<TakeoverLock>,
    claims: &Arc<CaptchaClaims>,
) -> Vec<Arc<dyn Policy>> {
    vec![
        Arc::clone(takeover) as Arc<dyn Policy>,
        Arc::clone(claims) as Arc<dyn Policy>,
    ]
}
