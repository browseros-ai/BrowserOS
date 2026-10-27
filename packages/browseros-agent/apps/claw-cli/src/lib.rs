//! `browseros`: a command line over claw-server's MCP endpoint.
//!
//! The CLI owns no browser logic. claw-server is already the daemon, the CDP
//! client, the snapshot builder and the ref resolver, so everything here turns
//! a command line into one `tools/call` and prints the text block back.
//!
//! What makes it feel different from MCP is state. Every page-scoped tool
//! needs `page` and every tool accepts `session`; both live in a scope file, so
//! `browseros click @e3` carries nothing the caller has to remember.

pub mod cli;
pub mod client;
pub mod endpoint;
pub mod exec;
pub mod exit;
pub mod io;
pub mod plan;
pub mod refs;
pub mod render;
pub mod script;
pub mod state;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
