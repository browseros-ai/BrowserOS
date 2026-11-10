//! Neo: Serves one MCP run tool over the current runtime, including retained full results,
//! output budgets, artifacts and image blocks. Clones share one browser connection and
//! runtime so per-tab input ordering survives separate stateless HTTP requests.
//!
//! The host owns process boot and its listener. This crate owns /mcp, request hygiene,
//! admission, cancellation and cleanup. Shutdown cancels requests, waits for owned work,
//! drains runtime cleanup, then disconnects the browser. It owns no Claw sessions or effects.

mod artifacts;
mod binary;
mod budget;
mod output;
mod service;

use std::path::PathBuf;

pub use service::NeoSdkServer;

/// The browser endpoint and shared data directory supplied by the embedding process.
/// Artifact and retained-result files remain available across this server's requests.
#[derive(Debug, Clone)]
pub struct Config {
    pub cdp_port: u16,
    pub state_dir: PathBuf,
}

impl Config {
    #[must_use]
    pub fn new(cdp_port: u16, state_dir: PathBuf) -> Self {
        Self {
            cdp_port,
            state_dir,
        }
    }
}
