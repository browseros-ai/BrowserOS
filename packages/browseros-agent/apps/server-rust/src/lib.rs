//! `server-rust`: the BrowserOS server, a drop-in Rust replacement for `apps/server` (same
//! routes, bodies, status codes, SSE bytes, sidecar config and `~/.browseros` data).
//!
//! ```text
//! apps/app ──server-api-client──► http (axum, ROUTES) ──► chat::ChatService ─► ChatEngine
//!                                   │                      ├─ BrowserOsEngine → browseros-agent ─┐
//!                                   │                      └─ AcpEngine → acp-agents ────────────┤ loopback /mcp
//!                                   └─ /mcp ◄───────── X-BrowserOS-Internal-Lease ◄──────────────┘
//!                                        browseros-mcp-host (shared with claw-server)
//! ```
//!
//! Module owners after the S0 skeleton: process shell `config`/`paths`/`identity`/`activity`/
//! `server_file`/`app`/`telemetry::logging`/`http::{mod,security,extract,errors,system}` (P3),
//! `db` (P2), `chat` core (P8), CRUD routes (P9), `oauth` (P10), `klavis` + `mcp_manager` (P12),
//! LLM tools + telemetry sinks (P13), `chat::browseros_engine` (P14), `mcp` (P15),
//! `chat::acp_engine` + `/acpx/probe` (P16).

pub mod activity;
pub mod app;
pub mod chat;
pub mod config;
pub mod db;
pub mod http;
pub mod identity;
pub mod klavis;
pub mod mcp;
pub mod mcp_manager;
pub mod oauth;
pub mod paths;
pub mod server_file;
pub mod telemetry;

pub use app::{AppInner, AppState, StartupError, build_router};

/// Reported by `--version` (Chromium reads the trimmed stdout). Starts at 0.1.0, above the TS
/// server's 0.0.x line, so OTA ordering prefers it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
