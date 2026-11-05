//! Sealed worlds: one `browseros-sdk` isolated world per frame document, each holding core's
//! seal, Playwright's `InjectedScript` and Neo's probe.
//!
//! ```text
//!  first use of (session, frame) in a page ── FrameWorlds::world        single-flight cell
//!     ├─ SessionSetup::ensure(session)       once per session: Runtime.enable (OOPIF only),
//!     │                                      then Runtime.addBinding(__neoJournal, browseros-sdk)
//!     └─ SealedWorld::install                Page.createIsolatedWorld, then the handle to its
//!          │                                 global by the new world's uniqueId (its
//!          │                                 executionContextCreated, watched from before the
//!          │                                 create), or by number for a world that existed
//!          │                                 if the session reported nothing about the number
//!          ├─ install_function(seal, engine, boot { frameSeq, epoch })
//!          │    → described { seq, retired? }   a re-key's old seq is never handed out again
//!          └─ watch_hidden_roots              a pierced DOM scan; the seal watches each closed
//!                                             shadow root, which it cannot find by itself
//!  every later call ── probe::call(world)    Runtime.callFunctionOn on that handle
//!  every sweep ── watch_hidden_roots again, then the seal's sweep (hidden_roots)
//!  a call fails Gone ── FrameWorlds::invalidate: the cell goes and the seq retires
//! ```
//!
//! Invariants, and what enforces each:
//! - **No unsealed engine world.** The seal and the engine run in one call, seal first; a
//!   throwing seal stops it before the engine exists, and `boot` refuses a world without one.
//! - **The handle is the sealed world's, never a page world's.** A numeric context id can name
//!   a page's world in another renderer, so the install takes the handle by the new world's
//!   `uniqueId`, or by number only across a window in which the session reported nothing about
//!   that number (`SealedWorld::install`). Every later call goes by the handle.
//! - **No closed root the seal cannot see.** The seal finds fields and hears input only from
//!   the document down through open roots, so the install, and every sweep before it reads,
//!   scans the document for closed roots and has the seal watch each (`hidden_roots`). A
//!   scan that fails fails the install or the sweep, so a read whose frame held a root Neo
//!   could not watch is refused.
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
mod frame_seq;
mod frame_worlds;
mod hidden_roots;
mod journal;
mod sealed_world;
mod session_setup;

pub(crate) use frame_seq::{BackendNodeId, ContextId, Epoch, FrameSeq, Pin};
pub(crate) use frame_worlds::{
    Endpoint, FrameWorlds, Installer, LinkedParent, ParentLink, Place, SeqLookup,
};
pub(crate) use journal::Journal;
pub(crate) use sealed_world::{ContextWatch, Installed, Scan, SealedWorld};
pub(crate) use session_setup::{SessionKind, SessionSetup};

#[cfg(test)]
mod live_tests;
