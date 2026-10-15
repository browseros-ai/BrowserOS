//! Session lifecycle lives in the shared MCP host. claw adds its durable journal: the
//! audit timeline and the session tab-claim ledger.

mod journal;

pub use browseros_mcp_host::sessions::*;
pub use journal::ClawSessionJournal;

#[cfg(test)]
mod manager_tests;
