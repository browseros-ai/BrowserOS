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
//! Process shell: `config`, `paths`, `identity`, `server_file`, `activity` (OTA readiness) and
//! `app` (the composition root). `http` is the route table and its handlers; `chat` runs turns;
//! `mcp` serves the shared host at `/mcp`; `db` is the SQLite file shared with the TS server;
//! `oauth`, `klavis` and `mcp_manager` own their integrations; `js` holds the JavaScript string
//! semantics TS bakes into stored and wire values.

pub mod activity;
pub mod app;
pub mod chat;
pub mod config;
pub mod db;
pub mod http;
pub mod identity;
pub mod js;
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
