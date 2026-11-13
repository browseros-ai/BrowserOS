//! The SDK methods claw-sdk serves itself, and the housekeeping that goes through the same door.
//!
//! ```text
//!  builtin_set ─▶ playwright_ops: every neo-playwright op ─ PlaywrightFacade, the
//!              │  process's one Playwright, built on the browser link's session
//!              │  ─ op_marks: each op's marks, and whether it is held, from its capabilities
//!              │  ─ event_release: an event pull's events through the rules, tab by tab
//!              ├▶ neo_tasks: named task records, tab lists, and child opens/closes
//!              ├▶ host_fill: host.fill and host.fillCheck, internal: the host's private typing,
//!              │  which the run service submits in a run of its own for the host's fill route
//!              └▶ solve_captcha: neo.solveCaptcha ─ CaptchaClaims (also the door's busy rule)
//!  TabJanitor::sweep ─▶ a Neo-origin run ─▶ Dispatcher::submit(page.close) per idle agent tab
//!  keep_downloads ─▶ at a run's end, the finished files its download events handed it, copied
//!                    beside the trace for the host (the run service calls it before it answers)
//!  remove_init_scripts ─▶ at a run's end, its init scripts off every tab, before it answers
//! ```
//!
//! Every built-in is a private `dispatch::Method` type, reachable only by the name it registers,
//! so nothing here runs outside the door. `builtin_set` is the one list: app.rs calls it once at
//! startup, and the registry refuses a duplicate name, so no name is ever served twice. The
//! janitor closes tabs as Neo's own ops rather than calling core directly, so the takeover lock
//! applies to it and the trace says what Neo closed.
//!
//! `neo.solveCaptcha` is the one built-in that drives a page outside the door: the solver
//! clicks through core for up to 120 s. Its claim on the tab, taken before it marks itself sent,
//! is what keeps the script's other writes off that tab meanwhile ([`CaptchaClaims`]).

mod builtin_set;
mod captcha_claims;
mod event_release;
mod host_fill;
mod keep_downloads;
mod neo_tasks;
mod op_marks;
mod playwright_facade;
mod playwright_ops;
mod solve_captcha;
mod tab_janitor;
#[cfg(test)]
mod tests;

pub(crate) use builtin_set::{BuiltinDeps, builtin_set, door_rules};
pub(crate) use captcha_claims::CaptchaClaims;
pub(crate) use host_fill::{
    CHECK as HOST_FILL_CHECK, CHECK_CODE as HOST_FILL_CHECK_CODE, ENTRY_BACKSTOP,
    FILL as HOST_FILL, FILL_CODE as HOST_FILL_CODE, FillProgress, FillValues, RecordedBox,
    boxes_args,
};
#[cfg(test)]
pub(crate) use host_fill::{EntryParts, PrivateEntry, register as register_host_fill};
pub(crate) use keep_downloads::keep_downloads;
pub(crate) use playwright_facade::PlaywrightFacade;
pub(crate) use playwright_ops::{PLAYWRIGHT_PRELUDE, remove_init_scripts};
pub(crate) use tab_janitor::TabJanitor;

#[cfg(test)]
pub(crate) use neo_tasks::neo_tasks as register_tasks;
