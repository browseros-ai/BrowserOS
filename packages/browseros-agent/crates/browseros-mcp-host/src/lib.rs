//! BrowserOS MCP host layer: the session, ownership, tab-group and dispatch logic that
//! sits between an MCP client and the `browseros-mcp` tool catalog. claw-server and
//! server-rust both serve `/mcp` through it; each plugs its own persistence, analytics
//! and branding into the seams in [`seams`], or the defaults in [`noop`].
//!
//! ```text
//!  host (claw-server, server-rust)
//!    │  HostCore { BrowserService, Sessions, TargetBindings, helpers_root }
//!    │  + HostBranding + seams (journal, analytics, tab journal, profiles,
//!    │    local tools, extra guards/effects/observers)
//!    ▼
//!  HostCtx ──► mcp_router / streamable_http_service / mcp_service
//!    │            one HostMcpService per transport session
//!    ▼
//!  HostMcpService::call_tool ── session admission (legacy id | stateless handle)
//!    ├─ local tool (name_session, host tools) ── LocalToolCall
//!    └─ catalog tool ── dispatch: guards → execute → effects → observers
//!                                        │
//!                                        ▼
//!                         browseros-mcp catalog (execute_tool on a BrowserView)
//! ```

pub mod branding;
pub mod browser;
pub mod clock;
mod ctx;
pub mod error;
pub mod helpers;
pub mod identity;
pub mod ids;
pub mod mcp;
pub mod noop;
pub mod seams;
pub mod sessions;
pub mod storage;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use branding::HostBranding;
pub use ctx::{HostCore, HostCoreConfig, HostCtx, HostCtxBuilder};
pub use error::{HostError, HostResult};
pub use mcp::{HostMcpService, McpRouteConfig, mcp_router, mcp_service, streamable_http_service};
