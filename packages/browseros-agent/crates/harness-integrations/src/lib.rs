mod catalog;
mod error;
mod mcp;
mod skills;

pub use catalog::{
    AgentId, ConfigFormat, HarnessDefinition, HttpShape, InjectValue, KeyTransform, McpSurface,
    McpSurfaceSources, McpTransport, PerOsPaths, ProjectSurface, SkillSurface, StdioShape,
    is_agent_supported, list_supported_agents, resolve_harness_definition,
};
pub use error::Error;
pub use mcp::{
    AgentInfo, AgentScope, AgentSurface, DisconnectInput, DisconnectSummary, LinkInput,
    LinkSummary, ListLinksFilter, ListedLink, ManifestLinkEntry, ManifestServerEntry, McpManager,
    McpServer, McpServerSpec, RescanEntry, RescanReport, ServerManifest, UnlinkInput,
    UnlinkSummary, detect_installed_agents, is_installed, resolve_agent_mcp_config_path,
    resolve_agent_surface,
};
pub use skills::{
    SkillEnvironment, SkillReconcileOutcome, SkillReconciler, SkillSpec, SkillWarning,
    TargetPlatform, resolve_agent_skill_target,
};
