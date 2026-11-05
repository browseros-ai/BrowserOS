//! The owner's side of hosted BrowserOS Neo (claw-sdk): the takeover parts that let zuzu's
//! owner take a tab from the agent and give it back.
//!
//! Each part is small, useful alone, and owns only its own state (design D13). A host composes
//! them over HTTP through `browseros-service`, which is the only crate that knows the wire; the
//! app adapts the types here to its dispatcher (decision 9). The parts:
//! - [`Locks`] keeps the agent off a tab. The dispatcher's lock policy asks [`Locks::check`]
//!   about every agent operation, synchronously; tab cleanup asks [`Locks::is_locked`].
//! - [`Fits`] fits a tab to the owner's viewer on Neo's one screen (Xvnc) and one window,
//!   follows the owner's own resizes, reads keyboard boxes and the browser bar for the owner's
//!   page, and puts the tab, the window and the screen back as they were.
//! - [`sweep`] reads what a tab's password, card and code fields hold into the known secrets
//!   list, through Neo's sealed world in every frame (`sealed_frames`).
//! - [`Journal`] arms that world's journal during control, so the value the owner leaves in each
//!   such field is registered: caught at its `input` event, so a navigation cannot lose it.
//!
//! ```text
//!  POST /v1/locks ─▶ BrowserTabs::page (is the tab open) ─▶ Locks::lock ─ settle ─▶ Lock
//!  dispatcher lock Policy ─▶ Locks::check (sync)       tab cleanup ─▶ Locks::is_locked
//!  Locks ─ lock.state ─▶ the service's event log ─▶ GET /v1/events
//!  POST /v1/tabs/{tab}/fit ─▶ Fits::fit ─ surface plan ─▶ Xvnc (RFB), window, viewport
//!  Xvnc ─ layouts ─▶ the screen watcher ─▶ the fits loop ─▶ refits; owner views every 750 ms
//!  POST /v1/tabs/{tabId}/sweep ─▶ sweep ─ sealed_frames, every frame ─▶ SecretSet
//!  control (display, takeover) ─▶ Journal::arm ─ binding calls ─▶ SecretSet
//! ```
//!
//! The rules live apart from the I/O, as pure functions that test alone: the lock decision,
//! the fit rule, the surface's plans, the RFB bytes and the owner's address rule.
//!
//! The display and the v1 flow join this crate next.

#![deny(missing_docs)]
#![deny(
    clippy::wildcard_imports,
    clippy::missing_panics_doc,
    clippy::needless_for_each,
    clippy::fn_params_excessive_bools,
    clippy::string_add
)]

mod browser_tabs;
mod fit_rule;
mod fits;
mod journal;
mod limits;
mod lock_decision;
mod locks;
mod owner_address;
mod owner_view;
mod part_error;
mod part_ids;
mod rfb_server;
mod screen;
mod screen_size;
mod sealed_frames;
mod surface;
mod surface_steps;
mod sweep;
#[doc(hidden)]
pub mod testing;

pub use browser_tabs::BrowserTabs;
pub use fit_rule::FitMode;
pub use fits::{BarAction, BarOutcome, BrowserBar, Fit, FitState, Fits, Fitted, Keyboard};
pub use journal::{ArmedJournal, Journal};
pub use locks::{
    Lock, LockBinding, LockEnd, LockEnded, LockTarget, LockTtl, Locks, OwnerHasControl, Reads,
};
pub use part_error::PartError;
pub use part_ids::{FitId, LockId, TakeoverId, UnknownId};
pub use screen_size::{NotAScreenSize, ScreenSize, Xvnc};
pub use sweep::{Swept, sweep};
