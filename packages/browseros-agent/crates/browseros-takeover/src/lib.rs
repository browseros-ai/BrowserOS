//! The owner's side of hosted BrowserOS Neo (claw-sdk): the takeover parts that let zuzu's
//! owner take a tab from the agent and give it back.
//!
//! Each part is small, useful alone, and owns only its own state (design D13). A host composes
//! them over HTTP through `browseros-service`, which is the only crate that knows the wire; the
//! app adapts the types here to its dispatcher (decision 9). This step has one part:
//! - [`Locks`] keeps the agent off a tab. The dispatcher's lock policy asks [`Locks::check`]
//!   about every agent operation, synchronously; tab cleanup asks [`Locks::is_locked`].
//!
//! ```text
//!  POST /v1/locks ─▶ BrowserTabs::page (is the tab open) ─▶ Locks::lock ─ settle ─▶ Lock
//!  dispatcher lock Policy ─▶ Locks::check (sync)       tab cleanup ─▶ Locks::is_locked
//!  Locks ─ lock.state ─▶ the service's event log ─▶ GET /v1/events
//! ```
//!
//! The fit, the sweep, the journal, the display and the v1 flow join this crate next.

#![deny(missing_docs)]
#![deny(
    clippy::wildcard_imports,
    clippy::missing_panics_doc,
    clippy::needless_for_each,
    clippy::fn_params_excessive_bools,
    clippy::string_add
)]

mod browser_tabs;
mod limits;
mod lock_decision;
mod locks;
mod part_error;
mod part_ids;

pub use browser_tabs::BrowserTabs;
pub use locks::{
    Lock, LockBinding, LockEnd, LockEnded, LockTarget, LockTtl, Locks, OwnerHasControl, Reads,
};
pub use part_error::PartError;
pub use part_ids::{FitId, LockId, TakeoverId, UnknownId};
