//! Isolated worlds: one `browseros-engine` isolated world per frame document, each holding
//! Playwright's `InjectedScript` and Neo's probe, apart from the page's main world.
//!
//! ```text
//!  first use of (session, frame) in a page ── FrameWorlds::world        single-flight cell
//!     └─ IsolatedWorld::install                Page.createIsolatedWorld, then the handle to its
//!          │                                 global by number, if the session reported nothing
//!          │                                 about the number; by the new world's uniqueId
//!          │                                 while a Runtime lease has the session report its
//!          │                                 contexts (watched from before the create)
//!          └─ install_function(engine, boot { frameSeq, epoch })
//!               → described { seq, retired? }   a re-key's old seq is never handed out again
//!  every later call ── probe::call(world)    Runtime.callFunctionOn on that handle
//!  a call fails Gone ── FrameWorlds::invalidate: the cell goes and the seq retires
//! ```
//!
//! Invariants, and what enforces each:
//! - **The engine never runs in a page world.** The page's own scripts share the main world
//!   with it, so the engine lives only in its isolated world.
//! - **The handle is the isolated world's, never a page world's, as far as the session
//!   reports.** A numeric context id can name a page's world in another renderer, so the install
//!   takes the handle by the new world's `uniqueId`, or by number only across a window in which
//!   the session reported nothing about that number (`IsolatedWorld::install`). Every later call
//!   goes by the handle. Neo leaves `Runtime` off on every session, because a site can time its
//!   console calls while it is on to detect an open inspector; the session then reports no contexts, and
//!   this guard holds only while some `Runtime` lease has it report them.
//! - **One engine per world.** The install's guard on `__neoProbe` makes it idempotent, and the
//!   single-flight cell makes concurrent first uses share one install.
//! - **A seq names one document forever.** Every new document gets a new seq and none is
//!   reused, so `f7e3` resolves in document 7 or fails (`FrameWorlds`).
//! - **Restarts stay strict.** Worlds outlive a claw-sdk process, so each `Playwright` boots
//!   them with its own random [`Epoch`]: another epoch re-keys a world, which forgets every
//!   ref and reports the seq the world held, which this page then never hands out. Pages
//!   already open when the instance started take seqs from a random base from 100000 to
//!   999999 (`PageRegistry`), so an old `e5` or `f347e5` from a document this instance never
//!   met almost surely names no document of the new numbering (decisions.md, M1 review).
//! - **Liveness is learned from failures, not events.** A stale handle, a lost session or an
//!   `uninstalled` reply is [`ProbeFailure::Gone`](crate::probe::ProbeFailure); resolution
//!   re-installs and retries once, a ref jump fails `element_detached`.

mod engine_source;
mod frame_attach_log;
mod frame_seq;
mod frame_worlds;
mod isolated_world;

pub(crate) use frame_attach_log::{FrameAttachLog, FrameEvent};
pub(crate) use frame_seq::{BackendNodeId, ContextId, Epoch, FrameSeq, Pin};
pub(crate) use frame_worlds::{
    Endpoint, FrameWorlds, Installer, LinkedParent, ParentLink, Place, SeqLookup,
};
pub(crate) use isolated_world::{ContextWatch, Installed, IsolatedWorld};

#[cfg(test)]
mod live_tests;
