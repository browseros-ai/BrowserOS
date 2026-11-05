//! The HTTP listeners Neo binds (design D1) and their routes.
//!
//! ```text
//!  agent listener  127.0.0.1:<ports.server>, behind Chromium's proxy, no auth: in the cell only
//!                  mirrord reaches it. Serves /mcp (stateless rmcp, behind request_hygiene,
//!                  which keeps web pages out) and /system/* for Chromium's supervisor.
//!  host listener   a Unix socket ($BROWSERCLAW_DIR/host.sock), TCP only by CLAW_HOST_TCP, none
//!                  without a usable CLAW_HOST_TOKEN_FILE (host::HostApi decides). Serves
//!                  browseros-service's host_service(routes, auth), which owns the routes, the
//!                  auth and the connect info; Neo only binds the sockets.
//! ```
//!
//! Nothing here names a browser crate (STYLE.md P2): routes that need the browser get it through
//! the parts `app.rs` hands them.

mod agent_router;
mod host_listener;
mod request_hygiene;
mod system_routes;

pub(crate) use agent_router::agent_router;
pub(crate) use host_listener::{BindError, HostListener};
