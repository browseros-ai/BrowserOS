//! `browseros`: the command line over this server's own MCP endpoint.
//!
//! The CLI owns no browser logic. This server is already the daemon, the CDP
//! client, the snapshot builder and the ref resolver, so everything here turns
//! a command line into one `tools/call` and prints the text block back. It
//! ships as a second binary of this crate because the two are released
//! together and the CLI needs no dependency the server does not already have.
//!
//! What makes it feel different from driving the same tools over MCP is state.
//! Every page-scoped tool needs `page` and every tool accepts `session`; both
//! live in a scope file, so `browseros click @e3` carries nothing the caller
//! has to remember.

pub mod capability;
pub mod client;
pub mod endpoint;
pub mod exec;
pub mod exit;
pub mod grammar;
pub mod guides;
pub mod install;
pub mod io;
pub mod plan;
pub mod refs;
pub mod render;
pub mod script;
pub mod state;
