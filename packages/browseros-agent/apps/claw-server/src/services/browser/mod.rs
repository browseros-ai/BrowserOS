//! The CDP connection lives in the shared MCP host; claw keeps the tab registry that
//! observes each browser session for recording attribution and ledger inheritance.

mod tab_registry;

pub use browseros_mcp_host::browser::*;
pub use tab_registry::{TabRegistry, TabRegistryObserver};
