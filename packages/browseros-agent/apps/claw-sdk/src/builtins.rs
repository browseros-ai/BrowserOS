//! The SDK methods claw-sdk serves itself, and the housekeeping that goes through the same door.
//!
//! ```text
//!  builtin_set ─▶ placeholder_page: context.newPage, page.goto, page.title, page.close
//!              └▶ solve_captcha: neo.solveCaptcha ─ CaptchaClaims (also the door's busy rule)
//!  TabJanitor::sweep ─▶ a Neo-origin run ─▶ Dispatcher::submit(page.close) per idle agent tab
//! ```
//!
//! Every built-in is a private `dispatch::Method` type, reachable only by the name it registers,
//! so nothing here runs outside the door. `builtin_set` is the one list: app.rs calls it once at
//! startup, and the registry refuses a duplicate name, so these placeholder page ops cannot
//! linger beside sdk-playwright's real ones. The janitor closes tabs as Neo's own ops rather than
//! calling core directly, so the takeover lock applies to it and the trace says what Neo closed.
//!
//! `neo.solveCaptcha` is the one built-in that drives a page outside the door: the solver
//! clicks through core for up to 120 s. Its claim on the tab, taken before it marks itself sent,
//! is what keeps the script's other writes off that tab meanwhile ([`CaptchaClaims`]).

mod builtin_set;
mod captcha_claims;
mod placeholder_page;
mod solve_captcha;
mod tab_janitor;
#[cfg(test)]
mod tests;

pub(crate) use builtin_set::{BuiltinDeps, builtin_set};
pub(crate) use captcha_claims::CaptchaClaims;
pub(crate) use tab_janitor::TabJanitor;
