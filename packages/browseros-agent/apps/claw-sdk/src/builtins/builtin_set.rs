//! The one list of built-ins.

use std::sync::Arc;

use browseros_captcha::CaptchaSolver;

use super::{CaptchaClaims, PlaywrightFacade, playwright_ops, solve_captcha};
use crate::dispatch::{RegistryBuilder, RegistryError};

/// What the built-ins need from startup. A plain data bag.
pub(crate) struct BuiltinDeps {
    /// The process's one CAPTCHA solver; `None` without a provider key, and then
    /// `neo.solveCaptcha` is registered but unlisted and fails `host / captcha_unavailable`.
    pub(crate) solver: Option<Arc<CaptchaSolver>>,
    /// The tabs solves hold, which the door's CAPTCHA busy rule reads too.
    pub(crate) claims: Arc<CaptchaClaims>,
    /// browseros-playwright's facade, which every Playwright op runs on.
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
