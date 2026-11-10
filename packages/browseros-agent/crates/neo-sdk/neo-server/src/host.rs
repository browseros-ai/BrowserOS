//! Neo's host side: the host API zuzu reaches over the host socket (neo-service), and the
//! takeover lock it feeds back into the door.
//!
//! ```text
//!  startup ─ HostApi::bind: token file ─▶ host socket bound (or none) ─┐
//!  the link's first connect ─▶ HostApi::serve ─ after a replaced trace store, every open tab
//!        made private (FlagRecovery) ─ Service::start(store, sink, privacy, session, viewports,
//!        screenshot jobs, xvnc, cdp port)
//!        ─▶ TakeoverLock armed with Service::locks
//!        ─▶ Service::retain_tabs(a listing taken then, Service::tab_fence from before it)
//!        ─▶ tab_feed
//!        ─▶ host_service(service.router(), auth) on the socket
//!  door (every op, every recheck) ─▶ TakeoverLock::check ─▶ Locks::check
//!  idle-tab janitor ─▶ TakeoverLock::is_locked
//!  tab_watch's events, in order ─▶ tab_feed ─▶ Service::tab_closed | tab_opened, and on a
//!        new watch start Service::retain_tabs(a listing taken then, never a queued one, and
//!        the fence taken before it)
//! ```
//!
//! The service owns the routes, the auth, the locks, tab privacy and the event log; Neo binds the
//! socket, hands the service what it already holds, and adapts the service's locks and the
//! browser link's tab listings to its own door. Nothing here implements a trait for the service.

mod flag_recovery;
mod host_api;
mod tab_feed;
mod takeover_lock;
#[cfg(test)]
mod tests;

pub(crate) use flag_recovery::FlagRecovery;
pub(crate) use host_api::{HostApi, HostInputs};
pub(crate) use takeover_lock::TakeoverLock;
