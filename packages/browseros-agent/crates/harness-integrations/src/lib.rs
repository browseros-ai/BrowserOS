mod catalog;
mod emitter;
mod error;
mod io;
mod manager;
mod paths;
mod planner;
mod skills;
mod types;

pub use catalog::{
    AgentSurface, ConfigFormat, HarnessDefinition, HttpShape, InjectValue, KeyTransform,
    McpSurface, McpSurfaceSources, PerOsPaths, ProjectSurface, SkillSurface, StdioShape,
    detect_installed_agents, is_agent_supported, list_supported_agents,
    resolve_agent_mcp_config_path, resolve_agent_surface, resolve_harness_definition,
};
pub use error::Error;
pub use manager::McpManager;
pub use paths::is_installed;
pub use skills::{
    SkillEnvironment, SkillReconcileOutcome, SkillReconciler, SkillSpec, SkillWarning,
    TargetPlatform, resolve_agent_skill_target,
};
pub use types::{
    AgentId, AgentInfo, AgentScope, DisconnectInput, DisconnectSummary, LinkInput, LinkSummary,
    ListLinksFilter, ListedLink, ManifestLinkEntry, ManifestServerEntry, McpServer, McpServerSpec,
    McpTransport, RescanEntry, RescanReport, ServerManifest, UnlinkInput, UnlinkSummary,
};
