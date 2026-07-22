pub mod api;
pub mod app;
mod clock;
pub mod config;
pub mod db;
pub mod error;
pub mod identity;
pub mod ids;
pub mod runtime;
pub mod services;
pub mod storage;
pub mod telemetry;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub use app::{AppState, build_router};
pub use runtime::{AppRuntime, ShutdownHandle};
