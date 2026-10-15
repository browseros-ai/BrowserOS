//! `tracing` setup. S0: stderr only; P3 adds the rotated `browseros-server.log` file sink.

use crate::config::EnvConfig;
use tracing_subscriber::EnvFilter;

pub fn init(env: &EnvConfig) {
    let filter = EnvFilter::try_new(env.log_level.as_deref().unwrap_or("info"))
        .unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
