#[allow(clippy::all, unused_imports)]
#[path = "generated/mod.rs"]
pub mod models;

pub use models::*;

pub const RECORDING_INGEST_MAX_BYTES: usize = 4 * 1024 * 1024;
