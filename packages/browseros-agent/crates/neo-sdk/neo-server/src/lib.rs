//! claw-sdk is hosted BrowserOS Neo's sidecar: the server an agent talks to instead of the
//! desktop claw-server. It exposes one MCP tool, `run`, which executes an agent's script against
//! the browser. Every SDK operation that script performs goes through one dispatcher, so policy,
//! delivery and the trace have a single front door (STYLE.md P2).
//!
//! The default-off `hosted-run` feature exports `hosted`: the same bridge, dispatcher,
//! trace, with JavaScript owned by the caller. Each connection owns a runtime
//! built by [`App::runtime`]; closing it drains runs and releases all background browser tasks.
//! It starts no listeners and queues no trace screenshots.

mod app;
mod browser;
mod builtins;
mod config;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        unused_imports,
        reason = "notes, background ops, own listings and policy fields serve the methods and \
                  rules still to land"
    )
)]
mod dispatch;
mod host;
#[cfg(feature = "hosted-run")]
pub mod hosted;
mod http;
mod logging;
mod mcp;
mod run;
mod runtime;
#[cfg(test)]
mod test_rig;
#[cfg(feature = "test-support")]
pub mod test_support;

pub use app::{App, StartError};
pub use config::{AppConfig, ConfigError};
pub use logging::Logging;
