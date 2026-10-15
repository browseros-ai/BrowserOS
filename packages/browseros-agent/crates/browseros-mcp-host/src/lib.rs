//! BrowserOS MCP host layer shared by claw-server and server-rust.

pub mod clock;
pub mod error;
pub mod helpers;
pub mod identity;
pub mod ids;
pub mod storage;

pub use error::{HostError, HostResult};
