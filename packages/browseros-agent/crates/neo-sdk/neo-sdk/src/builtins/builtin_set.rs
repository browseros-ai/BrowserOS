//! The one list of built-ins, and the one order of the door's rules.

use std::sync::Arc;

use neo_captcha::CaptchaSolver;

use super::{CaptchaClaims, PlaywrightFacade, PrivacyHooks, playwright_ops, solve_captcha};
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
    playwright_ops::playwright_ops(builder, &playwright)?;
    solve_captcha::solve_captcha(builder, solver, claims)
}

/// The dispatcher's rules, in their one fixed order (design D7 rule 4): the takeover lock, then
/// tab privacy, then the CAPTCHA busy rule. The door runs them in this order for every op, so an
/// owner's lock outranks a solve's claim when both would refuse. Tab privacy refuses a capture
/// of a private tab, and withholds one whose tab went private before its pixels arrived;
/// neo-playwright also asks right before each capture (`PrivacyHooks::capture_allowed`).
///
/// app.rs builds the door with these, and so does the built-ins' test harness, so a test of how
/// two rules interact runs the order production runs.
#[must_use]
pub(crate) fn door_rules(
    takeover: &Arc<TakeoverLock>,
    privacy: &Arc<PrivacyHooks>,
    claims: &Arc<CaptchaClaims>,
) -> Vec<Arc<dyn Policy>> {
    vec![
        Arc::clone(takeover) as Arc<dyn Policy>,
        Arc::clone(privacy) as Arc<dyn Policy>,
        Arc::clone(claims) as Arc<dyn Policy>,
    ]
}
