mod catalog;
mod emitter;
mod error;
mod io;
mod manager;
mod paths;
mod planner;
mod types;

pub use catalog::{
    AgentSurface, ClientConfig, ClientConfigSources, ConfigFormat, HttpShape, InjectValue,
    KeyTransform, PerOsPaths, ProjectSurface, StdioShape, detect_installed_agents,
    is_agent_supported, list_supported_agents, resolve_agent_mcp_config_path,
    resolve_agent_surface,
};
pub use error::Error;
pub use manager::Manager;
pub use paths::is_installed;
pub use types::{
    AgentId, AgentInfo, AgentScope, DisconnectInput, DisconnectSummary, LinkInput, LinkSummary,
    ListLinksFilter, ListedLink, ManifestLinkEntry, ManifestServerEntry, McpServer, McpServerSpec,
    McpTransport, RescanEntry, RescanReport, ServerManifest, UnlinkInput, UnlinkSummary,
};
