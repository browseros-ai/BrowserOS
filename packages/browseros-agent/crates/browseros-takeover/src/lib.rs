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
//! - [`Displays`] shows Neo's one screen over RFB, the only way a host reaches Xvnc: view
//!   displays to spectators, with no input and never while a private tab is in front; control
//!   displays to the owner, whose first input waits until every open tab is private. A view can
//!   keep a tab, or a task's tab, in front.
//!
//! [`Takeovers`] is the v1 flow over them, zuzu's two moments: the owner opens the link (the tab
//! goes private, the agent is locked out, spectators are held off, the journal is armed and the
//! tab fitted), and the owner is done (all of it given back, the lock last). It holds nothing
//! but what it took from the parts, so a new flow replaces it without touching them (D13).
//!
//! ```text
//!  POST /v1/locks ─▶ BrowserTabs::page (is the tab open) ─▶ Locks::lock ─ settle ─▶ Lock
//!  dispatcher lock Policy ─▶ Locks::check (sync)       tab cleanup ─▶ Locks::is_locked
//!  Locks ─ lock.state ─▶ the service's event log ─▶ GET /v1/events
//!  POST /v1/tabs/{tab}/fit ─▶ Fits::fit ─ surface plan ─▶ Xvnc (RFB), window, viewport
//!  Xvnc ─ layouts ─▶ the screen watcher ─▶ the fits loop ─▶ refits; owner views every 750 ms
//!  POST /v1/tabs/{tabId}/sweep ─▶ sweep ─ sealed_frames, every frame ─▶ SecretSet
//!  control (display, takeover) ─▶ Journal::arm ─ binding calls ─▶ SecretSet
//!  GET /v1/display ─▶ Displays::admit ─ upgrade ─▶ Admitted::serve ─ RFB, filtered ─▶ Xvnc
//!    control: first input ─▶ views close ─▶ every open tab stamped, then private ─▶ the input
//!      goes on
//!    view: follow ─ the task's operations ─▶ Browser.activateTab
//!    views: tab.private, a 250 ms poll ─ a private tab in front ─▶ every view closes
//!  POST /v1/takeovers ─▶ Takeovers::start ─▶ private, lock, views held, journal, fit
//!  release, 10 s before the TTL, tab closed ─▶ Takeovers::release ─▶ displays closed, sweep,
//!    journal disarmed, fit restored, views back, unlock
//! ```
//!
//! The rules live apart from the I/O, as pure functions that test alone: the lock decision,
//! the fit rule, the surface's plans, the RFB bytes both ways and the display's verdict table,
//! the owner's address rule and follow's next activation, private targets included.

#![deny(missing_docs)]
#![deny(
    clippy::wildcard_imports,
    clippy::missing_panics_doc,
    clippy::needless_for_each,
    clippy::fn_params_excessive_bools,
    clippy::string_add
)]

mod browser_tabs;
mod displays;
mod fit_rule;
mod fits;
mod follow;
mod front_tab;
mod journal;
mod limits;
mod lock_decision;
mod locks;
mod owner_address;
mod owner_view;
mod part_error;
mod part_ids;
mod rfb_client;
mod rfb_server;
mod screen;
mod screen_size;
mod sealed_frames;
mod surface;
mod surface_steps;
mod sweep;
mod takeovers;
#[doc(hidden)]
pub mod testing;

pub use browser_tabs::BrowserTabs;
pub use displays::{
    Admitted, DisplayBinding, DisplayEnd, DisplayRequest, Displays, Follow, TaintStamps, ViewsHold,
};
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
pub use takeovers::{
    ReleaseReason, Takeover, TakeoverRequest, TakeoverState, TakeoverTtl, Takeovers,
};
