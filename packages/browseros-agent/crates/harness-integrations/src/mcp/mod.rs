mod emitter;
mod io;
mod manager;
mod paths;
mod planner;
mod types;

pub use manager::McpManager;
pub use paths::{
    detect_installed_agents, is_installed, resolve_agent_mcp_config_path, resolve_agent_surface,
};
pub use types::{
    AgentInfo, AgentScope, AgentSurface, DisconnectInput, DisconnectSummary, LinkInput,
    LinkSummary, ListLinksFilter, ListedLink, ManifestLinkEntry, ManifestServerEntry, McpServer,
    McpServerSpec, RescanEntry, RescanReport, ServerManifest, UnlinkInput, UnlinkSummary,
};
