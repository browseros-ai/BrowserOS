//! Neo's host side: the host API zuzu reaches over the host socket (neo-http), and the
//! takeover lock it feeds back into the door.
//!
//! ```text
//!  startup ─ HostApi::bind: token file ─▶ host socket bound (or none) ─┐
//!        ─▶ Settings loaded from settings.json (with a socket) or the defaults (without)
//!  the link's first connect ─▶ HostApi::serve ─▶ Service::start
//!        (store, sink, session, viewports, screenshot jobs,
//!        xvnc, cdp port, settings)
//!        ─▶ TakeoverLock armed with Service::locks, AgentFront with Service::front,
//!           HostHolds with Service::tab_hold
//!        ─▶ Service::retain_tabs(a listing taken then, Service::tab_fence from before it)
//!        ─▶ tab_feed
//!        ─▶ host_service(service.router(), auth) on the socket
//!        ─▶ Service::arm_fill(FillService::door): the host's private typing
//!  door (every op, every recheck) ─▶ TakeoverLock::check ─▶ Locks::check, or
//!        Locks::check_passing for a fill's run, which may pass the one lock its host named
//!  POST /v1/tabs/{tab}/fill ─▶ FillService ─ FillLedger (its own record of request ids,
//!        beside the trace) ─ a Neo-started run with one host.fill op (fill_service.rs)
//!  idle-tab janitor ─▶ HostHolds::hold ─▶ Service::tab_hold: a lock or a fit, now or until when
//!  an op's input or capture ─▶ AgentFront::bring_forward ─▶ Front::bring_forward (agent_front.rs)
//!  tab_watch's events, in order ─▶ tab_feed ─▶ Service::tab_closed, and on a
//!        new watch start Service::retain_tabs(a listing taken then, never a queued one, and
//!        the fence taken before it)
//! ```
//!
//! The service owns the routes, the auth, the locks and the event log; Neo binds the
//! socket, hands the service what it already holds, and adapts the service's locks and the
//! browser link's tab listings to its own door. Nothing here implements a trait for the service.

mod agent_front;
mod fill_answer;
mod fill_ledger;
mod fill_request;
mod fill_service;
mod filled_tabs;
mod host_api;
mod host_holds;
mod tab_feed;
mod takeover_lock;
#[cfg(test)]
mod tests;

pub(crate) use agent_front::AgentFront;
pub(crate) use fill_ledger::FillLedger;
pub(crate) use fill_service::{FillService, FillServiceParts};
pub(crate) use filled_tabs::{FilledTabs, LiveTab};
pub(crate) use host_api::{HostApi, HostInputs};
pub(crate) use host_holds::HostHolds;
pub(crate) use takeover_lock::TakeoverLock;
