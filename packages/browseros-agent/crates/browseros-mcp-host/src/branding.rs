//! Product identity the shared MCP layer speaks in: server info, the stateless session
//! handle's `_meta` key, `initialize` instructions, and the product name used in
//! agent-facing error text. claw serves "BrowserOS neo"; other hosts bring their own.

use browseros_core::NativeCapabilities;
use std::{fmt, sync::Arc};

/// Builds `initialize` instructions. A function rather than a string because they depend
/// on the connected browser's native features and, for claw, on a live user setting.
pub type InstructionsFn = Arc<dyn Fn(NativeCapabilities) -> String + Send + Sync>;

#[derive(Clone)]
pub struct HostBranding {
    /// MCP `serverInfo.name`; also prefixes the session line appended to stateless results.
    pub server_name: String,
    /// MCP `serverInfo.title`.
    pub server_title: String,
    /// MCP `serverInfo.version`.
    pub server_version: String,
    /// The product an agent is told about in errors ("… session is no longer live").
    pub product_name: String,
    /// `_meta` key the stateless session handle rides under, per the MCP `_meta`
    /// convention (namespaced by owner).
    pub session_meta_key: String,
    pub instructions: InstructionsFn,
}

impl HostBranding {
    /// Neutral BrowserOS identity with the catalog's own operating guide.
    #[must_use]
    pub fn browseros(server_version: impl Into<String>) -> Self {
        Self {
            server_name: "browseros".to_string(),
            server_title: "BrowserOS".to_string(),
            server_version: server_version.into(),
            product_name: "BrowserOS".to_string(),
            session_meta_key: "com.browseros/session".to_string(),
            instructions: Arc::new(browseros_mcp::browser_mcp_instructions),
        }
    }

    #[must_use]
    pub fn instructions(&self, native: NativeCapabilities) -> String {
        (self.instructions)(native)
    }
}

impl fmt::Debug for HostBranding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostBranding")
            .field("server_name", &self.server_name)
            .field("server_title", &self.server_title)
            .field("server_version", &self.server_version)
            .field("product_name", &self.product_name)
            .field("session_meta_key", &self.session_meta_key)
            .finish_non_exhaustive()
    }
}
