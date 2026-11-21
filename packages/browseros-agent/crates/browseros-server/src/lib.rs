//! The BrowserOS agent server: the local process the browser launches as
//! `resources/bin/browseros_server` and the extension talks to over HTTP on `127.0.0.1`.
//! It runs chat turns (its own agent loop over LLM providers, or Claude Code and Codex through
//! acpx), keeps conversations, providers and coding agents in the existing
//! `browseros.sqlite`, and serves the contract in `contracts/server-api/openapi.json`.
//!
//! Eight modules; a dependency may only point down this list, and
//! `tests/dependency_boundaries.rs` fails the build on an upward import:
//!
//! ```text
//!   app            the launch contract and the process; the one place modules are wired
//!   http           routes, wire types (private `http::dto`), the trust rule, the OpenAPI document
//!   conversations  every conversation: the stored transcript and the live run
//!   agent          one turn: the own loop, or a coding agent through acpx
//!   model, browser one model call per step; the browser as tools
//!   store          browseros.sqlite through SeaORM
//!   turn           the vocabulary of a turn, and the reducer behind the stream and the row
//! ```
//!
//! What a new engineer has to learn to follow a turn: `TurnEvent`, `EventSink::emit`,
//! `Conversations::send`, `TurnRunner::run_turn`, `Model::step`, `Browser::call`.

// `ChatError` is the extension's whole error envelope (nine fields), returned only on the
// failure path of a model call or a turn; boxing it would change every signature the chunks
// code against for no measurable gain.
#![allow(clippy::result_large_err)]

pub mod agent;
pub mod app;
pub mod browser;
pub mod conversations;
pub mod http;
pub mod model;
pub mod store;
pub mod turn;

/// The crate version: what `--version` prints and the browser compares against the bundled
/// server's to pick which one to launch.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
